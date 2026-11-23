//! Load Action configuration and resolve selected enum options into executable argv.

mod file;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

type Inputs = IndexMap<String, Input>;
type Enums = IndexMap<String, IndexMap<String, EnumOption>>;

pub struct Catalog {
    listen: SocketAddr,
    origin: String,
    admin_token: String,
    retention_days: u32,
    log_request_params: bool,
    enums: Enums,
    actions: IndexMap<String, Action>,
}

struct Action {
    label: String,
    webhook_token: Option<String>,
    execution: Arc<Execution>,
    bindings: BTreeMap<String, String>,
}

impl Action {
    fn visible_inputs(&self) -> impl Iterator<Item = (&String, &Input)> {
        self.execution
            .inputs
            .iter()
            .filter(|(name, _)| !self.bindings.contains_key(*name))
    }
}

struct Input {
    enum_id: String,
    label: String,
}

struct Execution {
    target: String,
    inputs: Inputs,
    program: String,
    args: Vec<Argument>,
    cwd: Option<Argument>,
    lock: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumOption {
    label: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Argument {
    Literal(String),
    Parameter(Parameter),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameter {
    param: String,
}

#[derive(Serialize)]
pub struct ActionView {
    pub id: String,
    pub label: String,
    pub inputs: Vec<InputView>,
}

#[derive(Serialize)]
pub struct InputView {
    pub id: String,
    pub label: String,
    pub options: Vec<OptionView>,
}

#[derive(Serialize)]
pub struct OptionView {
    pub id: String,
    pub label: String,
}

#[derive(Debug)]
pub struct Invocation {
    pub action: String,
    pub label: String,
    pub target: String,
    pub selections: BTreeMap<String, String>,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub lock: Option<String>,
}

#[derive(Debug, Error)]
enum ResolveError {
    #[error("unknown action")]
    UnknownAction,
    #[error("unexpected action input")]
    UnexpectedInput,
    #[error("missing action input `{0}`")]
    MissingInput(String),
    #[error("unknown enum option for action input `{0}`")]
    UnknownOption(String),
}

pub fn normalize_origin(origin: &str) -> Result<String> {
    let url = Url::parse(origin).context("invalid server.origin")?;
    ensure!(
        url.scheme() == "https"
            && url.path() == "/"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "server.origin must be an HTTPS origin"
    );
    Ok(url.origin().ascii_serialization())
}

impl Catalog {
    /// Read and validate the configuration before any Action can be selected.
    pub fn load(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("read configuration {}", path.display()))?;
        Self::parse(&source).with_context(|| format!("load configuration {}", path.display()))
    }

    pub fn admin_token(&self) -> &str {
        &self.admin_token
    }

    /// Credentials known to the service that must be masked in captured process output.
    pub fn secrets(&self) -> Vec<String> {
        std::iter::once(self.admin_token.clone())
            .chain(
                self.actions
                    .values()
                    .filter_map(|action| action.webhook_token.clone()),
            )
            .collect()
    }

    /// Require the example credentials to be replaced before starting the server.
    pub fn check_secrets(&self) -> Result<()> {
        let is_placeholder = |token: &str| {
            matches!(
                token.trim(),
                "RUNLEVER_GENERATED_ADMIN_TOKEN" | "RUNLEVER_GENERATED_FORGEJO_TOKEN"
            )
        };
        ensure!(
            !is_placeholder(&self.admin_token),
            "replace the example admin token before serving"
        );
        for (id, action) in &self.actions {
            ensure!(
                !action.webhook_token.as_deref().is_some_and(is_placeholder),
                "replace the example webhook token for action {id:?} before serving"
            );
        }
        Ok(())
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn listen(&self) -> SocketAddr {
        self.listen
    }

    pub fn retention_days(&self) -> u32 {
        self.retention_days
    }

    pub fn log_request_params(&self) -> bool {
        self.log_request_params
    }

    /// Return only the Action and option labels, IDs, and permitted selections.
    pub fn actions(&self) -> Vec<ActionView> {
        self.actions
            .iter()
            .map(|(id, action)| ActionView {
                id: id.clone(),
                label: action.label.clone(),
                inputs: action
                    .visible_inputs()
                    .map(|(name, input)| InputView {
                        id: name.clone(),
                        label: input.label.clone(),
                        options: self.enums[&input.enum_id]
                            .iter()
                            .map(|(id, option)| OptionView {
                                id: id.clone(),
                                label: option.label.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect()
    }

    pub fn webhook_token(&self, id: &str) -> Option<&str> {
        self.actions.get(id)?.webhook_token.as_deref()
    }

    pub fn has_action(&self, id: &str) -> bool {
        self.actions.contains_key(id)
    }

    /// Validate public option IDs, then substitute the configured values into argv and cwd.
    pub fn resolve(&self, id: &str, inputs: &BTreeMap<String, String>) -> Result<Invocation> {
        let action = self.actions.get(id).ok_or(ResolveError::UnknownAction)?;
        let execution = &action.execution;

        if inputs
            .keys()
            .any(|name| !execution.inputs.contains_key(name) || action.bindings.contains_key(name))
        {
            return Err(ResolveError::UnexpectedInput.into());
        }

        let mut selections = action.bindings.clone();
        for (name, input) in action.visible_inputs() {
            let option = inputs
                .get(name)
                .ok_or_else(|| ResolveError::MissingInput(name.clone()))?;
            if !self.enums[&input.enum_id].contains_key(option) {
                return Err(ResolveError::UnknownOption(name.clone()).into());
            }
            selections.insert(name.clone(), option.clone());
        }

        // Configuration validation guarantees that every parameter refers to an input.
        let resolve = |argument: &Argument| match argument {
            Argument::Literal(value) => value.clone(),
            Argument::Parameter(Parameter { param }) => {
                let enum_id = &execution.inputs[param].enum_id;
                self.enums[enum_id][&selections[param]].value.clone()
            }
        };

        Ok(Invocation {
            action: id.to_owned(),
            label: action.label.clone(),
            target: execution.target.clone(),
            program: execution.program.clone(),
            args: execution.args.iter().map(resolve).collect(),
            cwd: execution.cwd.as_ref().map(resolve),
            lock: execution.lock.as_ref().map(|name| {
                let enum_id = &execution.inputs[name].enum_id;
                format!("{enum_id}:{}", selections[name])
            }),
            selections,
        })
    }
}
