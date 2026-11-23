use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params, types::Type};

use crate::config::Invocation;

use super::process::OutputChunk;
use super::{LogEvent, Run, RunFilter, now};

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;
const RUN_PAGE_SIZE: i64 = 50;
const EVENT_PAGE_SIZE: i64 = 256;

const RUN_COLUMNS: &str = "id, action, label, target, selections, source, status, started_at,
    finished_at, exit_code, error, log_state = 'expired'";

pub(super) struct Store {
    connection: Mutex<Connection>,
}

pub(super) enum Claim {
    Busy(i64),
    Created(i64, Invocation),
}

impl Store {
    pub(super) fn open(state_dir: &Path, retention_days: u32) -> Result<Self> {
        fs::create_dir_all(state_dir)
            .with_context(|| format!("create state directory {}", state_dir.display()))?;
        let path = state_dir.join("runs.sqlite3");
        let mut connection = Connection::open(&path)
            .with_context(|| format!("open SQLite history {}", path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;

                 BEGIN;
                 CREATE TABLE IF NOT EXISTS runs (
                     id INTEGER PRIMARY KEY,
                     action TEXT NOT NULL,
                     label TEXT NOT NULL,
                     target TEXT NOT NULL,
                     selections TEXT NOT NULL,
                     source TEXT NOT NULL,
                     lock_key TEXT,
                     status TEXT NOT NULL CHECK (status IN (
                         'running', 'succeeded', 'failed', 'interrupted'
                     )),
                     started_at INTEGER NOT NULL,
                     finished_at INTEGER,
                     exit_code INTEGER,
                     error TEXT,
                     log_state TEXT NOT NULL DEFAULT 'empty'
                         CHECK (log_state IN ('empty', 'present', 'expired'))
                 );
                 CREATE UNIQUE INDEX IF NOT EXISTS active_lock
                     ON runs (lock_key) WHERE status = 'running' AND lock_key IS NOT NULL;
                 CREATE INDEX IF NOT EXISTS pending_log_expiry
                     ON runs (finished_at) WHERE log_state = 'present' AND finished_at IS NOT NULL;
                 CREATE TABLE IF NOT EXISTS log_events (
                     run_id INTEGER NOT NULL REFERENCES runs (id),
                     seq INTEGER NOT NULL,
                     stream TEXT NOT NULL CHECK (stream IN ('stdout', 'stderr')),
                     text TEXT NOT NULL,
                     at INTEGER NOT NULL,
                     PRIMARY KEY (run_id, seq)
                 ) WITHOUT ROWID;
                 COMMIT;",
            )
            .context("initialize the run history tables")?;
        let interrupted = connection
            .execute(
                "UPDATE runs
                 SET status = 'interrupted', finished_at = ?1,
                     error = 'the service stopped before the run finished'
                 WHERE status = 'running'",
                [now()],
            )
            .context("mark runs left active by the previous service instance as interrupted")?;
        if interrupted > 0 {
            tracing::warn!(interrupted, "recovered active runs");
        }
        expire_logs(&mut connection, retention_days).context("expire run output at startup")?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub(super) async fn claim(
        self: &Arc<Self>,
        invocation: Invocation,
        source: String,
    ) -> Result<Claim> {
        self.access(move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(lock_key) = &invocation.lock {
                let existing = transaction
                    .query_row(
                        "SELECT id FROM runs WHERE lock_key = ?1 AND status = 'running'",
                        [lock_key],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(id) = existing {
                    return Ok(Claim::Busy(id));
                }
            }
            let selections = serde_json::to_string(&invocation.selections)
                .context("serialize run selections")?;
            transaction.execute(
                "INSERT INTO runs (
                     action, label, target, selections, source, lock_key, status, started_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7)",
                params![
                    invocation.action,
                    invocation.label,
                    invocation.target,
                    selections,
                    source,
                    invocation.lock,
                    now(),
                ],
            )?;
            let id = transaction.last_insert_rowid();
            transaction.commit()?;
            Ok(Claim::Created(id, invocation))
        })
        .await
    }

    pub(super) async fn get(self: &Arc<Self>, id: i64) -> Result<Option<Run>> {
        self.access(move |connection| {
            let sql = format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1");
            Ok(connection.query_row(&sql, [id], read_run).optional()?)
        })
        .await
        .with_context(|| format!("read run {id}"))
    }

    pub(super) async fn list(self: &Arc<Self>, filter: RunFilter) -> Result<Vec<Run>> {
        self.access(move |connection| {
            let last_id = filter
                .before
                .map_or(i64::MAX, |before| before.saturating_sub(1));
            let sql = format!(
                "SELECT {RUN_COLUMNS} FROM runs
                 WHERE id <= ?1
                   AND (?2 IS NULL OR action = ?2)
                   AND (?3 IS NULL OR status = ?3)
                   AND (?4 IS NULL OR source = ?4)
                 ORDER BY id DESC
                 LIMIT ?5"
            );
            let mut statement = connection.prepare_cached(&sql)?;
            let rows = statement.query_map(
                params![
                    last_id,
                    filter.action,
                    filter.status,
                    filter.source,
                    RUN_PAGE_SIZE
                ],
                read_run,
            )?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
        .context("list runs")
    }

    pub(super) async fn events(self: &Arc<Self>, id: i64, after: i64) -> Result<Vec<LogEvent>> {
        self.access(move |connection| {
            let mut statement = connection.prepare_cached(
                "SELECT seq, stream, text, at FROM log_events
                 WHERE run_id = ?1 AND seq > ?2
                 ORDER BY seq
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(params![id, after, EVENT_PAGE_SIZE], |row| {
                Ok(LogEvent {
                    seq: row.get(0)?,
                    stream: row.get(1)?,
                    text: row.get(2)?,
                    at: row.get(3)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
        .with_context(|| format!("read output for run {id} after event {after}"))
    }

    pub(super) async fn append(
        self: &Arc<Self>,
        id: i64,
        first_seq: i64,
        events: Vec<OutputChunk>,
    ) -> Result<()> {
        self.access(move |connection| {
            let transaction = connection.transaction()?;
            if first_seq == 1 {
                transaction.execute("UPDATE runs SET log_state = 'present' WHERE id = ?1", [id])?;
            }
            {
                let mut statement = transaction.prepare_cached(
                    "INSERT INTO log_events (run_id, seq, stream, text, at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )?;
                for (offset, event) in events.into_iter().enumerate() {
                    let seq = first_seq + offset as i64;
                    statement.execute(params![id, seq, event.stream, event.text, event.at])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await
        .with_context(|| format!("store output for run {id}"))
    }

    pub(super) async fn finish(
        self: &Arc<Self>,
        id: i64,
        status: &'static str,
        exit_code: Option<i32>,
        message: Option<String>,
    ) -> Result<()> {
        self.access(move |connection| {
            let rows = connection.execute(
                "UPDATE runs SET status = ?2, finished_at = ?3, exit_code = ?4, error = ?5
                 WHERE id = ?1 AND status = 'running'",
                params![id, status, now(), exit_code, message],
            )?;
            ensure!(rows == 1, "run {id} is no longer active");
            Ok(())
        })
        .await
        .with_context(|| format!("record completion of run {id}"))
    }

    pub(super) async fn cleanup(self: &Arc<Self>, retention_days: u32) -> Result<()> {
        self.access(move |connection| expire_logs(connection, retention_days))
            .await
            .context("expire run output")
    }

    async fn access<T: Send + 'static>(
        self: &Arc<Self>,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut connection = store
                .connection
                .lock()
                .map_err(|_| anyhow!("the SQLite connection mutex is poisoned"))?;
            operation(&mut connection)
        })
        .await
        .context("join SQLite history task")?
    }
}

fn read_run(row: &Row<'_>) -> rusqlite::Result<Run> {
    let selections: String = row.get(4)?;
    let selections = serde_json::from_str(&selections).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
    })?;
    Ok(Run {
        id: row.get(0)?,
        action: row.get(1)?,
        label: row.get(2)?,
        target: row.get(3)?,
        selections,
        source: row.get(5)?,
        status: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        exit_code: row.get(9)?,
        error: row.get(10)?,
        logs_expired: row.get(11)?,
    })
}

fn retention_cutoff(at: i64, retention_days: u32) -> i64 {
    at - i64::from(retention_days) * SECONDS_PER_DAY
}

fn expire_logs(connection: &mut Connection, retention_days: u32) -> Result<()> {
    let cutoff = retention_cutoff(now(), retention_days);
    let transaction = connection.transaction()?;
    transaction.execute(
        "DELETE FROM log_events WHERE run_id IN (
             SELECT id FROM runs WHERE log_state = 'present' AND finished_at <= ?1
         )",
        [cutoff],
    )?;
    let expired = transaction.execute(
        "UPDATE runs SET log_state = 'expired' WHERE log_state = 'present' AND finished_at <= ?1",
        [cutoff],
    )?;
    transaction.commit()?;
    if expired > 0 {
        tracing::info!(expired, "expired run output");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
