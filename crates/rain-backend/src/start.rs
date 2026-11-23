//! Starting the backend, and the order in which it stops. Foundation sections 2.4 and 2.6.

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use anyhow::Context;
use axum::Router;
use rain_types::limits::{MODULE_STOP_BUDGET, PIPE_PENDING_INSTANCES, SHUTDOWN_BUDGET};
use rain_types::{ConfigIssue, StopReason};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::core::state::AppState;
use crate::modules::{self, MODULES};
use crate::server::pipe::{PipeListener, serve};
use crate::server::router;
use crate::store::db::OpenOutcome;

/// What Windows says when a pipe of that name exists already: "access denied" (os error 5).
const ERROR_ACCESS_DENIED: i32 = 5;

/// How long a stop waits for the open streams to write their last frame (step 2 of section 2.6).
const STREAM_TAIL_WAIT: Duration = Duration::from_millis(500);

/// The file name under which problems of the database are reported in `GET /health`.
const DATABASE_FILE: &str = "rain.db";

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// Another backend owns the pipe. This is not a failure: the caller just ends quietly.
    #[error("another Rain backend is already running on this pipe")]
    AlreadyRunning,
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// A backend that is running in this process.
pub struct Running {
    pub state: AppState,
    stopped: JoinHandle<()>,
}

impl Running {
    /// Waits until the backend has stopped.
    pub async fn wait(self) -> anyhow::Result<()> {
        self.stopped
            .await
            .context("the shutdown task ended abnormally")
    }
}

/// Runs the backend until it is stopped. This is what `main` calls.
pub async fn run(config: Config) -> Result<(), StartError> {
    let state = AppState::builder(config).build();
    let running = start(state, Router::new()).await?;
    running.wait().await?;
    Ok(())
}

/// Creates the pipe, starts the modules and begins to serve. Returns when the backend is ready.
///
/// The pipe is created first, with `first_pipe_instance(true)`: a second backend on the same pipe
/// name fails right there with [`StartError::AlreadyRunning`] (single instance, section 2.4).
///
/// `extra` holds routes that are added to the API. The product passes an empty router; a test
/// adds routes that fail on purpose.
pub async fn start(state: AppState, extra: Router<AppState>) -> Result<Running, StartError> {
    let listener = match PipeListener::bind(&state.config.pipe_name, PIPE_PENDING_INSTANCES) {
        Ok(listener) => listener,
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
            return Err(StartError::AlreadyRunning);
        }
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context("could not create the Rain pipe")
                .into());
        }
    };

    report_startup_issues(&state);
    // First run: the data folder may not exist. A failure is not fatal: every file is then
    // reported as unsaved when a write is tried.
    if let Err(e) = state.hub.ensure_data_dir().await {
        warn!(reason = %e.body.message, "the data folder could not be created");
    }
    open_database(&state).await;
    modules::migrate_all(&state, MODULES).await;
    modules::init_all(&state, MODULES).await?;
    // Every module has opened its files by now, so the watcher finds them registered.
    state.hub.start_watching();
    modules::start_all(&state, MODULES).await;

    let serve_stop = CancellationToken::new();
    let app = router::build(state.clone(), extra);
    let server = tokio::spawn({
        let state = state.clone();
        let serve_stop = serve_stop.clone();
        async move {
            serve(
                listener,
                app,
                serve_stop.clone(),
                state.config.limits.clone(),
            )
            .await;
            // `serve` ends on its own only when it cannot accept clients any more. Then the whole
            // backend stops, so that the shell sees the stream end and starts a new one.
            if !serve_stop.is_cancelled() {
                state.shutdown.request(StopReason::Restart);
            }
        }
    });
    let stopped = tokio::spawn(run_shutdown(state.clone(), server, serve_stop));

    info!(
        version = env!("CARGO_PKG_VERSION"),
        pipe = %state.config.pipe_name,
        sandbox = state.config.sandbox,
        start_id = %state.start_id,
        "backend started"
    );
    Ok(Running { state, stopped })
}

/// Problems that were found while the configuration was made (a `machine.toml` that cannot be
/// used) go to `GET /health`, grouped by file.
fn report_startup_issues(state: &AppState) {
    let mut by_file: BTreeMap<&str, Vec<ConfigIssue>> = BTreeMap::new();
    for issue in &state.config.startup_issues {
        by_file
            .entry(issue.file.as_str())
            .or_default()
            .push(issue.clone());
    }
    for (file, issues) in by_file {
        state.health.set_file_issues(file, issues);
    }
}

/// Opens `rain.db`. A damaged file is moved aside and a new one is started; a database that
/// cannot be a file at all lives in memory. Either is reported to the user as an issue of the
/// file `rain.db`; the backend starts in every case (foundation 10.5).
async fn open_database(state: &AppState) {
    let now_ms = state
        .clock
        .now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let message = match state.db.open(now_ms).await {
        Ok(OpenOutcome::Normal) => return,
        Ok(OpenOutcome::Recovered { damaged_file }) => {
            error!("the database was damaged; a new one was started");
            let name = damaged_file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!(
                "The database was damaged. Rain started a new one and kept the old file as {name}."
            )
        }
        Ok(OpenOutcome::InMemory) => {
            "The database file cannot be used. Rain keeps its history in memory until it stops."
                .to_owned()
        }
        Err(e) => {
            error!(error = %e, "the database thread could not start");
            "The database cannot be used. Rain runs without history.".to_owned()
        }
    };
    state.health.set_file_issues(
        DATABASE_FILE,
        vec![ConfigIssue {
            file: DATABASE_FILE.to_owned(),
            line: None,
            message,
        }],
    );
}

/// Waits for a stop request, then stops everything in the order of foundation section 2.6.
///
/// 1. Marking the stop and storing its reason is done by `ShutdownHandle::request`. It also makes
///    every open stream write its `lifecycle` frame and end.
/// 2. Wait until the streams have written that frame (at most `STREAM_TAIL_WAIT`).
/// 3. Stop the modules, in reverse order, in parallel, each with its own budget.
/// 4. Close the pipe and wait for the open connections (at most `shutdown_grace`).
/// 5. Stop the file watcher, checkpoint and close the database.
///
/// The whole stop has a budget; after that the backend ends anyway.
async fn run_shutdown(state: AppState, mut server: JoinHandle<()>, serve_stop: CancellationToken) {
    state.shutdown.token().cancelled().await;
    info!(reason = ?state.shutdown.reason(), "stopping");

    let in_order = async {
        state.shutdown.wait_for_streams(STREAM_TAIL_WAIT).await;
        modules::stop_all(&state, MODULES, MODULE_STOP_BUDGET).await;
        serve_stop.cancel();
        let _ = (&mut server).await;
        state.hub.stop_watching();
        state.db.close().await;
    };
    if tokio::time::timeout(SHUTDOWN_BUDGET, in_order)
        .await
        .is_err()
    {
        error!("the stop took longer than its budget; the backend ends anyway");
        server.abort();
    }
    info!("backend stopped");
}
