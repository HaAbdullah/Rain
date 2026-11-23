//! `rain-backend --stop`: asks a running backend to stop. Foundation sections 1.3 and 2.6.
//!
//! With `--force` a backend that accepted the stop but is still there after the wait is ended by
//! its process id (the id that its own `GET /ping` gives, never found by an image name).

use std::time::Duration;

use anyhow::{Context, bail};
use hyper::Method;
use rain_pipe::{PipeClient, pipe_exists};
use rain_types::env::pipe_path;
use rain_types::limits::API_PREFIX;
use rain_types::{Ping, ShutdownBody, StopReason};

use crate::APP_NAME;
use crate::platform::process::terminate_pid;
use crate::server::builtin::SHUTDOWN_ROUTE;

/// How long to wait until the pipe is gone after the request was accepted.
const STOP_WAIT: Duration = Duration::from_secs(5);
const STOP_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// Nothing listened on the pipe.
    NotRunning,
    /// The backend accepted the request and the pipe is gone.
    Stopped,
    /// The backend did not leave in time and its process was ended (`--force`).
    Killed,
}

/// Sends the shutdown request and waits until the pipe has vanished. An error means that the
/// backend refused, could not be reached, or did not stop within `STOP_WAIT` and `force` is off.
pub async fn stop(pipe_name: &str, force: bool) -> anyhow::Result<StopOutcome> {
    let client = PipeClient::new(pipe_name);
    let body = serde_json::to_vec(&ShutdownBody {
        reason: StopReason::Stop,
        force: None,
    })?;
    let path = format!("{API_PREFIX}{SHUTDOWN_ROUTE}");

    let answer = client
        .request(Method::POST, &path, Some(body.into()), &[])
        .await;
    match answer {
        Ok(raw) => {
            if let Err(e) = raw.into_success() {
                bail!("the backend refused to stop: {}", e.message);
            }
        }
        Err(e) if e.reason.as_deref() == Some("backendDown") => return Ok(StopOutcome::NotRunning),
        Err(e) => bail!("could not ask the backend to stop: {}", e.message),
    }

    if wait_until_gone(pipe_name).await {
        return Ok(StopOutcome::Stopped);
    }
    if !force {
        bail!(
            "the backend accepted the stop but the pipe {} is still there after {STOP_WAIT:?}; use --stop --force to end it",
            pipe_path(pipe_name)
        );
    }
    end_by_pid(&client).await?;
    if wait_until_gone(pipe_name).await {
        Ok(StopOutcome::Killed)
    } else {
        bail!("the backend process was ended but the pipe is still there")
    }
}

/// Reads the process id from `GET /ping` and ends that process. Only a program that says it is
/// the Rain backend is ended.
async fn end_by_pid(client: &PipeClient) -> anyhow::Result<()> {
    let ping: Ping = client
        .json(Method::GET, &format!("{API_PREFIX}/ping"), None::<&()>)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.message))
        .context("the backend does not answer, so its process id is unknown")?;
    if ping.app != APP_NAME {
        bail!("the program on the pipe is not the Rain backend, so it is not ended");
    }
    terminate_pid(ping.pid).context("could not end the backend process")
}

async fn wait_until_gone(pipe_name: &str) -> bool {
    tokio::time::timeout(STOP_WAIT, async {
        while pipe_exists(pipe_name) {
            tokio::time::sleep(STOP_POLL).await;
        }
    })
    .await
    .is_ok()
}
