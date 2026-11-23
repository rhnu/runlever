mod process;
mod redact;
mod schedule;
mod store;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, Notify};
use tokio::time::{Instant, sleep_until};
use tracing::Instrument;

use crate::config::{Invocation, QueueConfig};

use self::redact::Redactor;
use self::schedule::{Admission, Pending, Schedule, Transition};
use self::store::Store;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const DISPATCH_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Accepts and schedules commands and exposes their persistent run history.
#[derive(Clone)]
pub struct Engine(Arc<EngineState>);

struct EngineState {
    store: Arc<Store>,
    redactor: Redactor,
    queue: QueueConfig,
    schedule: Mutex<Schedule>,
    changed: Arc<Notify>,
}

#[derive(Debug, Error)]
pub enum StartError {
    #[error("the run queue is full")]
    QueueFull,
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
    pub submitted_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub logs_expired: bool,
}

impl Run {
    pub fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "running")
    }
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
    pub fn open(
        state_dir: &Path,
        retention_days: u32,
        secrets: Vec<String>,
        queue: QueueConfig,
    ) -> Result<Self> {
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

        let state = Arc::new(EngineState {
            store,
            redactor: Redactor::new(secrets),
            queue,
            schedule: Mutex::new(Schedule::default()),
            changed: Arc::new(Notify::new()),
        });
        let engine = Arc::downgrade(&state);
        let changed = Arc::clone(&state.changed);
        runtime.spawn(async move {
            loop {
                let notified = changed.notified();
                let Some(state) = engine.upgrade() else {
                    break;
                };
                let deadline = {
                    let mut schedule = state.schedule.lock().await;
                    match state.dispatch(&mut schedule).await {
                        Ok(()) => schedule.pending.front().map(|run| run.deadline),
                        Err(error) => {
                            tracing::error!(?error, "could not dispatch queued runs");
                            Some(Instant::now() + DISPATCH_RETRY_DELAY)
                        }
                    }
                };
                drop(state);
                if let Some(deadline) = deadline {
                    tokio::select! {
                        _ = notified => {}
                        _ = sleep_until(deadline) => {}
                    }
                } else {
                    notified.await;
                }
            }
        });
        Ok(Self(state))
    }

    /// Accepts a command for immediate or later execution, independently of its request.
    pub async fn start(&self, invocation: Invocation, source: &str) -> Result<i64, StartError> {
        let state = Arc::clone(&self.0);
        let source = source.to_owned();
        // Keep acceptance independent from a request canceled during the history write.
        tokio::spawn(async move { state.admit(invocation, source).await })
            .await
            .context("join run admission task")?
    }

    pub async fn get(&self, id: i64) -> Result<Option<Run>> {
        self.0.store.get(id).await
    }

    pub async fn list(&self, filter: RunFilter) -> Result<Vec<Run>> {
        self.0.store.list(filter).await
    }

    pub async fn events(&self, id: i64, after: i64) -> Result<Vec<LogEvent>> {
        self.0.store.events(id, after).await
    }
}

impl EngineState {
    async fn admit(
        self: &Arc<Self>,
        invocation: Invocation,
        source: String,
    ) -> Result<i64, StartError> {
        let mut schedule = self.schedule.lock().await;
        self.dispatch(&mut schedule).await?;
        let admission = schedule.admit(&invocation, self.queue.max_pending);
        if admission == Admission::Full {
            return Err(StartError::QueueFull);
        }
        let start_now = admission == Admission::Running;
        let action = self.redactor.redact(&invocation.action);
        let (id, invocation) = self
            .store
            .create(invocation, source, start_now)
            .await
            .with_context(|| format!("create run for action {action:?}"))?;
        if start_now {
            self.launch(&mut schedule, id, invocation);
        } else {
            schedule.pending.push_back(Pending {
                id,
                invocation,
                deadline: Instant::now() + Duration::from_secs(self.queue.max_wait_seconds.into()),
            });
            tracing::info!(run_id = id, action, "queued run");
            self.changed.notify_one();
        }
        Ok(id)
    }

    async fn dispatch(self: &Arc<Self>, schedule: &mut Schedule) -> Result<()> {
        while let Some((index, transition)) = schedule.next(Instant::now()) {
            let id = schedule.pending[index].id;
            match transition {
                Transition::Start => self.store.promote(id).await?,
                Transition::Expire => self.store.expire(id).await?,
            }
            // Keep the request in memory until the status write succeeds.
            let run = schedule
                .pending
                .remove(index)
                .expect("selected pending run");
            match transition {
                Transition::Start => self.launch(schedule, id, run.invocation),
                Transition::Expire => tracing::info!(run_id = id, "queued run timed out"),
            }
        }
        Ok(())
    }

    fn launch(self: &Arc<Self>, schedule: &mut Schedule, id: i64, invocation: Invocation) {
        schedule.reserve(&invocation);
        let span = tracing::info_span!(
            "run",
            run_id = id,
            action = %self.redactor.redact(&invocation.action),
            target = %self.redactor.redact(&invocation.target)
        );
        tokio::spawn(Arc::clone(self).execute(id, invocation).instrument(span));
    }

    async fn execute(self: Arc<Self>, id: i64, invocation: Invocation) {
        tracing::info!("starting run");
        let result = process::execute(&self.store, id, &invocation, &self.redactor).await;
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
        let mut schedule = self.schedule.lock().await;
        if let Err(error) = self.store.finish(id, status, exit_code, message).await {
            tracing::error!(?error, status, exit_code, "could not record run completion");
        } else {
            tracing::info!(status, exit_code, "finished run");
        }
        schedule.release(&invocation);
        self.changed.notify_one();
    }
}

impl Drop for EngineState {
    fn drop(&mut self) {
        self.changed.notify_one();
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
        |error| -(error.duration().as_secs() as i64),
        |duration| duration.as_secs() as i64,
    )
}
