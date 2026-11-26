//! Load Action configuration and resolve selected enum options into executable argv.

mod file;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::num::NonZeroUsize;
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
    queue: QueueConfig,
    enums: Enums,
    groups: IndexMap<String, Group>,
    actions: IndexMap<String, Action>,
}

struct Action {
    label: String,
    group: Option<String>,
    expose: Exposure,
    execution: Arc<Execution>,
    bindings: BTreeMap<String, String>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Exposure {
    web: bool,
    webhook: Option<Webhook>,
}

impl Exposure {
    fn webhook_token(&self) -> Option<&str> {
        self.webhook.as_ref().map(|webhook| webhook.token.as_str())
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Webhook {
    token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    label: String,
    max_concurrent: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueueConfig {
    pub max_wait_seconds: u32,
    pub max_pending: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_wait_seconds: 60,
            max_pending: 16,
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupView>,
    pub inputs: Vec<InputView>,
}

#[derive(Serialize)]
pub struct GroupView {
    pub id: String,
    pub label: String,
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

#[derive(Serialize)]
pub struct SelectionLabel<'a> {
    pub label: &'a str,
    pub value: &'a str,
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
    pub group_limit: Option<GroupLimit>,
}

#[derive(Debug)]
pub struct GroupLimit {
    pub id: String,
    pub max_concurrent: NonZeroUsize,
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
        std::iter::once(self.admin_token())
            .chain(
                self.actions
                    .values()
                    .filter_map(|action| action.expose.webhook_token()),
            )
            .map(str::to_owned)
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
                !action.expose.webhook_token().is_some_and(is_placeholder),
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

    pub fn queue(&self) -> QueueConfig {
        self.queue
    }

    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    /// List Actions with a web execution entry point and their permitted selections.
    pub fn web_actions(&self) -> Vec<ActionView> {
        self.actions
            .iter()
            .filter(|(_, action)| action.expose.web)
            .map(|(id, action)| ActionView {
                id: id.clone(),
                label: action.label.clone(),
                group: action.group.as_ref().map(|id| GroupView {
                    id: id.clone(),
                    label: self.groups[id].label.clone(),
                }),
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
        self.actions.get(id)?.expose.webhook_token()
    }

    pub fn has_web_action(&self, id: &str) -> bool {
        self.actions.get(id).is_some_and(|action| action.expose.web)
    }

    pub fn action_label(&self, id: &str) -> Option<&str> {
        self.actions.get(id).map(|action| action.label.as_str())
    }

    /// Look up display labels for a run's recorded selections in the current configuration.
    pub fn selection_labels<'a>(
        &'a self,
        action: &str,
        target: &str,
        selections: &'a BTreeMap<String, String>,
    ) -> BTreeMap<&'a str, SelectionLabel<'a>> {
        let action = self.actions.get(action);
        let target = self.actions.get(target);
        selections
            .iter()
            .map(|(name, value)| {
                let input = action
                    .filter(|action| !action.bindings.contains_key(name))
                    .and_then(|action| action.execution.inputs.get(name))
                    .or_else(|| target.and_then(|target| target.execution.inputs.get(name)));
                let selection = match input {
                    Some(input) => SelectionLabel {
                        label: &input.label,
                        value: self.enums[&input.enum_id]
                            .get(value)
                            .map_or(value.as_str(), |option| option.label.as_str()),
                    },
                    None => SelectionLabel { label: name, value },
                };
                (name.as_str(), selection)
            })
            .collect()
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
            group_limit: action.group.as_ref().and_then(|id| {
                self.groups[id]
                    .max_concurrent
                    .map(|max_concurrent| GroupLimit {
                        id: id.clone(),
                        max_concurrent,
                    })
            }),
            selections,
        })
    }
}
