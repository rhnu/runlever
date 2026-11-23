use std::process::{ExitStatus, Stdio};
use std::str;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::Invocation;

use super::now;
use super::redact::Redactor;
use super::store::Store;

const READ_SIZE: usize = 8 * 1024;
const QUEUE_CAPACITY: usize = 64;
const WRITE_BATCH_SIZE: usize = 32;

pub(super) struct OutputChunk {
    pub(super) stream: &'static str,
    pub(super) text: String,
    pub(super) at: i64,
}

pub(super) async fn execute(
    store: &Arc<Store>,
    id: i64,
    invocation: Invocation,
    redactor: &Redactor,
) -> Result<ExitStatus> {
    let mut command = Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &invocation.cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().with_context(|| {
        format!(
            "start executable {:?} with working directory {:?}",
            redactor.redact(&invocation.program),
            invocation.cwd.as_deref().map(|cwd| redactor.redact(cwd)),
        )
    })?;
    let stdout = child.stdout.take().context("open the child stdout pipe")?;
    let stderr = child.stderr.take().context("open the child stderr pipe")?;
    let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
    let collect = async move {
        let wait = async { child.wait().await.context("wait for the child process") };
        let (status, (), ()) = tokio::try_join!(
            wait,
            read_output(stdout, "stdout", sender.clone(), redactor),
            read_output(stderr, "stderr", sender, redactor),
        )?;
        Ok(status)
    };

    // A read failure closes the pipes and drops the child. Drain the chunks
    // already in the queue before reporting it. A storage failure stops the
    // child immediately because its output can no longer be persisted.
    let (exit, ()) = tokio::try_join!(
        async { Ok::<_, anyhow::Error>(collect.await) },
        write_output(store, id, receiver),
    )?;
    exit
}

async fn read_output(
    mut reader: impl AsyncRead + Unpin,
    stream: &'static str,
    sender: mpsc::Sender<OutputChunk>,
    redactor: &Redactor,
) -> Result<()> {
    let mut chunk = [0; READ_SIZE];
    let mut buffer = Vec::with_capacity(READ_SIZE + 3);
    let mut pending = String::new();
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .with_context(|| format!("read child {stream}"))?;
        buffer.extend_from_slice(&chunk[..count]);
        let eof = count == 0;
        pending.push_str(&decode_buffer(&mut buffer, eof));
        let text = redactor.redact_output(&mut pending, eof);
        if !text.is_empty() {
            sender
                .send(OutputChunk {
                    stream,
                    text,
                    at: now(),
                })
                .await
                .with_context(|| format!("queue child {stream} for storage"))?;
        }
        if eof {
            return Ok(());
        }
    }
}

async fn write_output(
    store: &Arc<Store>,
    id: i64,
    mut receiver: mpsc::Receiver<OutputChunk>,
) -> Result<()> {
    let mut seq = 1;
    let mut batch = Vec::new();
    loop {
        let count = receiver.recv_many(&mut batch, WRITE_BATCH_SIZE).await;
        if count == 0 {
            return Ok(());
        }
        store.append(id, seq, std::mem::take(&mut batch)).await?;
        seq += count as i64;
    }
}

/// Decodes complete characters and keeps up to three bytes for the next read.
fn decode_buffer(buffer: &mut Vec<u8>, eof: bool) -> String {
    let mut text = String::with_capacity(buffer.len());
    let mut remaining = buffer.as_slice();
    let keep = loop {
        match str::from_utf8(remaining) {
            Ok(valid) => {
                text.push_str(valid);
                break 0;
            }
            Err(error) => {
                let (valid, invalid) = remaining.split_at(error.valid_up_to());
                text.push_str(str::from_utf8(valid).expect("validated UTF-8 prefix"));
                match error.error_len() {
                    Some(length) => {
                        text.push(char::REPLACEMENT_CHARACTER);
                        remaining = &invalid[length..];
                    }
                    None if eof => {
                        text.push(char::REPLACEMENT_CHARACTER);
                        break 0;
                    }
                    None => break invalid.len(),
                }
            }
        }
    };
    buffer.drain(..buffer.len() - keep);
    text
}

#[cfg(test)]
mod tests;
