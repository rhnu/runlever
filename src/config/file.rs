use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use super::{
    Action, Argument, Catalog, Enums, Execution, Exposure, Group, Input, Inputs, Parameter,
    QueueConfig, normalize_origin,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use indexmap::IndexMap;
use serde::{Deserialize, de::IgnoredAny};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    server: Server,
    #[serde(default)]
    logs: Logs,
    #[serde(default)]
    queue: QueueConfig,
    #[serde(default)]
    enums: Enums,
    #[serde(default)]
    groups: IndexMap<String, Group>,
    #[serde(default)]
    actions: IndexMap<String, ActionFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Server {
    listen: Option<SocketAddr>,
    origin: Option<String>,
    admin_token: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Logs {
    retention_days: Option<u32>,
    request_parameters: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionFile {
    label: String,
    group: Option<String>,
    inputs: Option<IndexMap<String, InputFile>>,
    lock: Option<Parameter>,
    exec: Option<Exec>,
    call: Option<String>,
    bind: Option<BTreeMap<String, String>>,
    #[serde(default)]
    expose: Exposure,
    #[serde(rename = "webhook")]
    legacy_webhook: Option<IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InputFile {
    EnumId(String),
    Labeled(LabeledInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabeledInput {
    #[serde(rename = "enum")]
    enum_id: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exec {
    program: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<Argument>,
    cwd: Option<Argument>,
}

struct Definition {
    label: String,
    group: Option<String>,
    expose: Exposure,
    body: Body,
}

enum Body {
    Exec(Arc<Execution>),
    Call {
        target: String,
        bind: BTreeMap<String, String>,
    },
}

impl Catalog {
    pub(super) fn parse(source: &str) -> Result<Self> {
        let location = |error: &toml::de::Error| {
            // TOML errors may contain raw input lines and values, including credential fields.
            if let Some(span) = error.span() {
                let prefix = &source.as_bytes()[..span.start.min(source.len())];
                let line = prefix.iter().filter(|&&byte| byte == b'\n').count() + 1;
                let column = prefix
                    .iter()
                    .rev()
                    .take_while(|&&byte| byte != b'\n')
                    .count()
                    + 1;
                anyhow!("invalid TOML or configuration fields at line {line}, byte column {column}")
            } else {
                anyhow!("invalid TOML or configuration fields")
            }
        };
        let deserializer = toml::de::Deserializer::parse(source).map_err(|e| location(&e))?;
        let config: ConfigFile =
            serde_path_to_error::deserialize(deserializer).map_err(|error| {
                location(error.inner()).context(format!("configuration field {}", error.path()))
            })?;
        let listen = config
            .server
            .listen
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 7380)));
        let origin = normalize_origin(
            config
                .server
                .origin
                .as_deref()
                .unwrap_or("https://localhost"),
        )?;
        ensure!(
            !config.server.admin_token.trim().is_empty(),
            "server.admin_token must be nonempty"
        );
        let retention_days = config.logs.retention_days.unwrap_or(30);
        ensure!(retention_days > 0, "logs.retention_days must be positive");
        ensure!(
            config.queue.max_wait_seconds > 0,
            "queue.max_wait_seconds must be positive"
        );

        validate_enums(&config.enums)?;
        for (id, group) in &config.groups {
            ensure!(valid_id(id), "invalid group ID {id:?}");
            ensure!(
                !group.label.trim().is_empty(),
                "group {id:?} must have a nonempty label"
            );
        }
        let definitions = config
            .actions
            .into_iter()
            .map(|(id, action)| {
                if let Some(group) = &action.group {
                    ensure!(
                        config.groups.contains_key(group),
                        "action {id:?} refers to unknown group {group:?}"
                    );
                }
                let definition = Definition::new(&id, action, &config.enums)
                    .with_context(|| format!("action {id:?}"))?;
                Ok((id, definition))
            })
            .collect::<Result<IndexMap<_, _>>>()?;
        validate_webhook_tokens(&config.server.admin_token, &definitions)?;
        let actions = compile_actions(&definitions, &config.enums)?;

        Ok(Self {
            listen,
            origin,
            admin_token: config.server.admin_token,
            retention_days,
            log_request_params: config.logs.request_parameters.unwrap_or(true),
            queue: config.queue,
            enums: config.enums,
            groups: config.groups,
            actions,
        })
    }
}

fn valid_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_enums(enums: &Enums) -> Result<()> {
    for (enum_id, options) in enums {
        ensure!(valid_id(enum_id), "invalid enum ID {enum_id:?}");
        ensure!(!options.is_empty(), "enum {enum_id:?} must contain options");
        for (option_id, option) in options {
            ensure!(
                valid_id(option_id),
                "invalid option ID {option_id:?} in enum {enum_id:?}"
            );
            ensure!(
                !option.label.trim().is_empty(),
                "option {option_id:?} in enum {enum_id:?} must have a nonempty label"
            );
            ensure!(
                !option.value.contains('\0'),
                "option {option_id:?} in enum {enum_id:?} contains a NUL byte"
            );
        }
    }
    Ok(())
}

fn validate_webhook_tokens(
    admin_token: &str,
    definitions: &IndexMap<String, Definition>,
) -> Result<()> {
    let mut used_tokens = HashMap::new();
    for (id, definition) in definitions {
        if let Some(token) = definition.expose.webhook_token() {
            ensure!(
                !token.trim().is_empty(),
                "action {id:?} webhook token must be nonempty"
            );
            ensure!(
                token != admin_token,
                "action {id:?} webhook token must differ from the admin token"
            );
            if let Some(other_id) = used_tokens.insert(token, id) {
                bail!("actions {other_id:?} and {id:?} must use different webhook tokens");
            }
        }
    }
    Ok(())
}

impl Definition {
    fn new(id: &str, action: ActionFile, enums: &Enums) -> Result<Self> {
        ensure!(valid_id(id), "invalid action ID");
        ensure!(
            action.legacy_webhook.is_none(),
            "move webhook.token to expose.webhook.token"
        );
        ensure!(!action.label.trim().is_empty(), "label must be nonempty");

        let body = match (action.exec, action.call) {
            (Some(exec), None) => {
                ensure!(action.bind.is_none(), "bind requires call");
                Body::Exec(Arc::new(Execution::new(
                    id,
                    exec,
                    action.inputs.unwrap_or_default(),
                    action.lock,
                    enums,
                )?))
            }
            (None, Some(target)) => {
                ensure!(
                    action.inputs.is_none() && action.lock.is_none(),
                    "call derives inputs and lock from the target"
                );
                Body::Call {
                    target,
                    bind: action.bind.unwrap_or_default(),
                }
            }
            _ => bail!("specify exactly one of exec or call"),
        };
        Ok(Self {
            label: action.label,
            group: action.group,
            expose: action.expose,
            body,
        })
    }
}

impl Execution {
    fn new(
        id: &str,
        exec: Exec,
        inputs: IndexMap<String, InputFile>,
        lock: Option<Parameter>,
        enums: &Enums,
    ) -> Result<Self> {
        let mut declared_inputs = Inputs::with_capacity(inputs.len());
        for (name, input) in inputs {
            ensure!(valid_id(&name), "invalid input name {name:?}");
            let (enum_id, label) = match input {
                InputFile::EnumId(enum_id) => (enum_id, name.clone()),
                InputFile::Labeled(LabeledInput { enum_id, label }) => (enum_id, label),
            };
            ensure!(
                enums.contains_key(&enum_id),
                "input {name:?} refers to unknown enum {enum_id:?}"
            );
            ensure!(
                !label.trim().is_empty(),
                "input {name:?} must have a nonempty label"
            );
            declared_inputs.insert(name, Input { enum_id, label });
        }
        let inputs = declared_inputs;
        for (index, argument) in exec.args.iter().enumerate() {
            validate_argument(argument, &inputs).with_context(|| format!("exec.args[{index}]"))?;
        }
        if let Some(cwd) = &exec.cwd {
            validate_argument(cwd, &inputs).context("exec.cwd")?;
            let has_empty_path = match cwd {
                Argument::Literal(path) => path.is_empty(),
                Argument::Parameter(Parameter { param }) => enums[&inputs[param].enum_id]
                    .values()
                    .any(|option| option.value.is_empty()),
            };
            ensure!(!has_empty_path, "exec.cwd must be nonempty");
        }
        let lock = lock.map(|lock| lock.param);
        if let Some(name) = &lock {
            ensure!(
                inputs.contains_key(name),
                "lock refers to unknown input {name:?}"
            );
        }

        let (program, mut args) = match (exec.program, exec.command) {
            (Some(program), None) => {
                validate_executable(&program).context("exec.program")?;
                (program, Vec::new())
            }
            (None, Some(command)) => {
                validate_executable(&command).context("exec.command")?;
                // The trusted command string receives configured arguments as "$1", "$2", and so on.
                let args = ["-c".into(), command, "runlever".into()]
                    .into_iter()
                    .map(Argument::Literal)
                    .collect();
                ("/bin/sh".into(), args)
            }
            _ => bail!("specify exactly one of exec.program or exec.command"),
        };
        args.extend(exec.args);
        Ok(Self {
            target: id.to_owned(),
            inputs,
            program,
            args,
            cwd: exec.cwd,
            lock,
        })
    }
}

fn validate_executable(value: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "must be nonempty");
    ensure!(!value.contains('\0'), "must not contain a NUL byte");
    Ok(())
}

fn validate_argument(argument: &Argument, inputs: &Inputs) -> Result<()> {
    match argument {
        Argument::Literal(value) => ensure!(!value.contains('\0'), "must not contain a NUL byte"),
        Argument::Parameter(Parameter { param }) => {
            ensure!(inputs.contains_key(param), "unknown input {param:?}");
        }
    }
    Ok(())
}

fn compile_actions(
    definitions: &IndexMap<String, Definition>,
    enums: &Enums,
) -> Result<IndexMap<String, Action>> {
    let mut compiled = HashMap::<&str, Action>::with_capacity(definitions.len());
    for (id, _) in definitions {
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut current = id.as_str();
        while !compiled.contains_key(current) {
            let definition = &definitions[current];
            ensure!(seen.insert(current), "action call cycle at {current:?}");
            path.push((current, definition));
            match &definition.body {
                Body::Exec(_) => break,
                Body::Call { target, .. } => {
                    ensure!(
                        definitions.contains_key(target),
                        "action {current:?} calls unknown action {target:?}"
                    );
                    current = target;
                }
            }
        }
        for (id, definition) in path.into_iter().rev() {
            let (execution, bindings, inherited_group) = match &definition.body {
                Body::Exec(execution) => (Arc::clone(execution), BTreeMap::new(), None),
                Body::Call { target, bind } => {
                    let target = &compiled[target.as_str()];
                    let bindings = extend_bindings(target, bind, enums)
                        .with_context(|| format!("action {id:?}"))?;
                    (
                        Arc::clone(&target.execution),
                        bindings,
                        target.group.as_ref(),
                    )
                }
            };
            if let (Some(group), Some(inherited)) = (&definition.group, inherited_group) {
                ensure!(
                    group == inherited,
                    "action {id:?} cannot override inherited group {inherited:?} with {group:?}"
                );
            }
            compiled.insert(
                id,
                Action {
                    label: definition.label.clone(),
                    group: inherited_group.or(definition.group.as_ref()).cloned(),
                    expose: definition.expose.clone(),
                    execution,
                    bindings,
                },
            );
        }
    }

    Ok(definitions
        .keys()
        .map(|id| {
            let action = compiled
                .remove(id.as_str())
                .expect("every validated Action has been compiled");
            (id.clone(), action)
        })
        .collect())
}

fn extend_bindings(
    target: &Action,
    bind: &BTreeMap<String, String>,
    enums: &Enums,
) -> Result<BTreeMap<String, String>> {
    let mut bindings = target.bindings.clone();
    for (name, option) in bind {
        let input = target
            .execution
            .inputs
            .get(name)
            .with_context(|| format!("bind refers to unknown input {name:?}"))?;
        ensure!(
            !bindings.contains_key(name),
            "bind refers to already bound input {name:?}"
        );
        ensure!(
            enums[&input.enum_id].contains_key(option),
            "bind for input {name:?} refers to unknown option {option:?}"
        );
        bindings.insert(name.clone(), option.clone());
    }
    Ok(bindings)
}
