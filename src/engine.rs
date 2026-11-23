mod process;
mod redact;
mod store;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::Instrument;

use crate::config::Invocation;

use self::redact::Redactor;
use self::store::{Claim, Store};

const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Owns running commands and their persistent run history.
#[derive(Clone)]
pub struct Engine {
    store: Arc<Store>,
    redactor: Arc<Redactor>,
}

#[derive(Debug, Error)]
pub enum StartError {
    #[error("run {0} holds the requested lock")]
    Busy(i64),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Default, Deserialize)]
pub struct RunFilter {
    pub action: Option<String>,
    pub status: Option<String>,
    pub source: Option<String>,
    pub before: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Run {
    pub id: i64,
    pub action: String,
    pub label: String,
    pub target: String,
    pub selections: BTreeMap<String, String>,
    pub source: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub logs_expired: bool,
}

#[derive(Debug, Serialize)]
pub struct LogEvent {
    pub seq: i64,
    pub stream: String,
    pub text: String,
    pub at: i64,
}

impl Engine {
    /// Opens the history after the caller has acquired the state directory lock.
    pub fn open(state_dir: &Path, retention_days: u32, secrets: Vec<String>) -> Result<Self> {
        let runtime = tokio::runtime::Handle::try_current()
            .context("open the run engine inside the Tokio runtime")?;
        let store = Arc::new(Store::open(state_dir, retention_days)?);
        let weak_store = Arc::downgrade(&store);
        runtime.spawn(async move {
            loop {
                tokio::time::sleep(CLEANUP_INTERVAL).await;
                let Some(store) = weak_store.upgrade() else {
                    break;
                };
                if let Err(error) = store.cleanup(retention_days).await {
                    tracing::error!(?error, "could not expire run output");
                }
            }
        });

        Ok(Self {
            store,
            redactor: Arc::new(Redactor::new(secrets)),
        })
    }

    /// Claims the invocation's lock and starts a command independently of its request.
    pub async fn start(&self, invocation: Invocation, source: &str) -> Result<i64, StartError> {
        let engine = self.clone();
        let source = source.to_owned();
        // The task must still launch accepted commands if the request is canceled
        // while the SQLite transaction is committing.
        tokio::spawn(async move {
            let action = engine.redactor.redact(&invocation.action);
            let claim = engine
                .store
                .claim(invocation, source)
                .await
                .with_context(|| format!("create run for action {action:?}"))?;
            let (id, invocation) = match claim {
                Claim::Busy(id) => return Err(StartError::Busy(id)),
                Claim::Created(id, invocation) => (id, invocation),
            };
            let span = tracing::info_span!(
                "run",
                run_id = id,
                action = %action,
                target = %engine.redactor.redact(&invocation.target)
            );
            tokio::spawn(engine.execute(id, invocation).instrument(span));
            Ok(id)
        })
        .await
        .context("claim and start run task")?
    }

    pub async fn get(&self, id: i64) -> Result<Option<Run>> {
        self.store.get(id).await
    }

    pub async fn list(&self, filter: RunFilter) -> Result<Vec<Run>> {
        self.store.list(filter).await
    }

    pub async fn events(&self, id: i64, after: i64) -> Result<Vec<LogEvent>> {
        self.store.events(id, after).await
    }

    async fn execute(self, id: i64, invocation: Invocation) {
        tracing::info!("starting run");
        let result = process::execute(&self.store, id, invocation, &self.redactor).await;
        let (status, exit_code, message) = match result {
            Ok(status) if status.success() => ("succeeded", status.code(), None),
            Ok(status) => (
                "failed",
                status.code(),
                Some(
                    self.redactor
                        .redact(&format!("command exited with {status}")),
                ),
            ),
            Err(error) => {
                tracing::error!(
                    error = %self.redactor.redact(&format!("{error:?}")),
                    "could not execute or capture run"
                );
                (
                    "failed",
                    None,
                    Some(self.redactor.redact(&format!("{error:#}"))),
                )
            }
        };
        if let Err(error) = self.store.finish(id, status, exit_code, message).await {
            tracing::error!(?error, status, exit_code, "could not record run completion");
        } else {
            tracing::info!(status, exit_code, "finished run");
        }
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
        |error| -(error.duration().as_secs() as i64),
        |duration| duration.as_secs() as i64,
    )
}
