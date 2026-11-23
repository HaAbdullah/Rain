//! Keeps the Rain backend running, and knows what state it is in.
//!
//! The backend is a separate program that lives on after the window closes. The shell finds out
//! whether it runs by asking the pipe, starts it when the pipe is silent, and tracks its state
//! from what the live stream reports (foundation sections 2.1 to 2.4).
//!
//! Nothing here uses a Tauri type, so all of it is tested with a real backend process.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hyper::Method;
use hyper::body::Bytes;
use rain_pipe::{PipeClient, pipe_exists};
use rain_types::limits::{
    API_PREFIX, BACKEND_READY_POLL, BACKEND_READY_WAIT, CLIENT_MAX_INFLIGHT, SHUTDOWN_BUDGET,
    SPAWN_GUARD_MAX, SPAWN_GUARD_WINDOW, UPGRADE_WAIT,
};
use rain_types::{ApiError, ErrorCode, Ping, ShutdownBody, StopReason};
use serde::Serialize;
use tokio::sync::{Mutex as AsyncMutex, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::AbortHandle;
use tracing::{info, warn};

/// File name of the backend program. It sits next to the shell's own program.
const BACKEND_EXE: &str = "rain-backend.exe";

/// The value of `Ping.app` that proves the program on the pipe is Rain's backend.
const BACKEND_APP_NAME: &str = "rain-backend";

/// How long a `ping` may take before the pipe counts as silent (foundation section 2.2, step 1).
const PING_TIMEOUT: Duration = Duration::from_secs(1);

/// How long the request to stop an outdated backend may take. The backend answers before it stops.
const SHUTDOWN_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// How often to look whether the pipe of a stopping backend has disappeared.
const PIPE_GONE_POLL: Duration = Duration::from_millis(100);

// Process creation flags (foundation section 2.3). `DETACHED_PROCESS` and `CREATE_NO_WINDOW`
// exclude each other, so only the first is used.
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
/// Windows answers a spawn with this error when the parent's job forbids breakaway.
const ERROR_ACCESS_DENIED: i32 = 5;

// ---- the errors the shell creates itself (foundation section 6.6) ----

/// An error that the shell makes up, as opposed to one the backend sent.
pub(crate) fn shell_error(code: ErrorCode, reason: &str, message: &str) -> ApiError {
    ApiError {
        code,
        message: message.to_owned(),
        source: Some("backend".to_owned()),
        reason: Some(reason.to_owned()),
    }
}

pub(crate) fn backend_down() -> ApiError {
    shell_error(
        ErrorCode::Unavailable,
        "backendDown",
        "Rain backend is not running",
    )
}

fn backend_missing() -> ApiError {
    shell_error(
        ErrorCode::Unavailable,
        "backendMissing",
        "Rain backend program was not found",
    )
}

fn crash_loop() -> ApiError {
    shell_error(
        ErrorCode::Unavailable,
        "backendCrashLoop",
        "Rain backend keeps stopping. See the log.",
    )
}

fn stopped() -> ApiError {
    shell_error(ErrorCode::Unavailable, "stopped", "Rain has stopped")
}

pub(crate) fn timeout_error() -> ApiError {
    shell_error(
        ErrorCode::Unavailable,
        "timeout",
        "Rain backend did not answer in time",
    )
}

/// A ping failure that means "nobody answers on the pipe", as opposed to "something is wrong
/// with the pipe". Only silence leads to a start of the backend.
fn is_silence(error: &ApiError) -> bool {
    matches!(error.reason.as_deref(), Some("backendDown" | "timeout"))
}

// ---- the state of the backend as the shell sees it (foundation section 2.1) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendState {
    /// Spawn requested, pipe not yet answering.
    Starting,
    /// The stream is connected and said `hello`.
    Up,
    /// Nobody answers and no stop was announced: a crash, or never started.
    Down,
    /// The backend announced that it stops for good (`gameMode`, `stop`).
    Stopped,
    /// The backend announced that it stops to be replaced (`upgrade`, `restart`).
    Restarting,
}

/// The payload of the page event `rain:backend` and the answer of `GET /local/backend`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub state: BackendState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_id: Option<String>,
}

impl BackendStatus {
    fn of(state: BackendState) -> Self {
        Self {
            state,
            start_id: None,
        }
    }
}

/// Things that happen to the backend, from the shell's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The shell started the backend program.
    Spawned,
    /// The program was started but never answered.
    StartFailed,
    /// The stream said `hello`: the backend is up. `start_id` names this run of the backend.
    Hello { start_id: String },
    /// The stream said `lifecycle`: a planned stop is under way.
    Stopping(StopReason),
    /// The stream ended or could not be opened.
    Lost,
    /// The user started Rain again while it was ending for game mode: that stop is cancelled.
    Revived,
}

/// The rules of foundation section 2.1 as a plain value, so a test can walk through them.
pub struct StateMachine {
    status: BackendStatus,
    /// Set by a `lifecycle` frame; it tells a planned stop from a crash when the stream ends.
    announced_stop: Option<StopReason>,
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            status: BackendStatus::of(BackendState::Down),
            announced_stop: None,
        }
    }

    /// Applies an event. Returns the new status when the state changed.
    pub fn apply(&mut self, event: Event) -> Option<BackendStatus> {
        let next = match event {
            Event::Spawned => {
                self.announced_stop = None;
                BackendStatus::of(BackendState::Starting)
            }
            Event::StartFailed => BackendStatus::of(BackendState::Down),
            Event::Revived => {
                self.announced_stop = None;
                BackendStatus::of(BackendState::Down)
            }
            Event::Hello { start_id } => {
                self.announced_stop = None;
                BackendStatus {
                    state: BackendState::Up,
                    start_id: Some(start_id),
                }
            }
            Event::Stopping(reason) => {
                self.announced_stop = Some(reason);
                BackendStatus::of(match reason {
                    StopReason::GameMode | StopReason::Stop => BackendState::Stopped,
                    StopReason::Upgrade | StopReason::Restart => BackendState::Restarting,
                })
            }
            // After a planned stop the stream ends as announced; the state already says so. While
            // the backend starts, a connection that does not work yet is expected.
            Event::Lost => match (self.announced_stop, self.status.state) {
                (Some(_), _) | (None, BackendState::Starting) => return None,
                (None, _) => BackendStatus::of(BackendState::Down),
            },
        };
        (next != self.status).then(|| {
            self.status = next.clone();
            next
        })
    }

    pub fn status(&self) -> &BackendStatus {
        &self.status
    }

    pub fn announced_stop(&self) -> Option<StopReason> {
        self.announced_stop
    }
}

// ---- the spawn guard (foundation section 2.2, step 4) ----

/// Allows at most `SPAWN_GUARD_MAX` starts of the backend in any `SPAWN_GUARD_WINDOW`, so a
/// backend that dies at once is not restarted forever.
///
/// The caller passes the time, so a test can use a made-up clock.
#[derive(Default)]
pub struct SpawnGuard {
    spawns: VecDeque<Instant>,
}

impl SpawnGuard {
    /// Records a start at `now` and returns `Ok`, or returns `Err` with the time that is left
    /// until a start is allowed again. A refusal is not a start: it does not push that time away.
    pub fn try_acquire(&mut self, now: Instant) -> Result<(), Duration> {
        while let Some(oldest) = self.spawns.front() {
            if now.saturating_duration_since(*oldest) < SPAWN_GUARD_WINDOW {
                break;
            }
            self.spawns.pop_front();
        }
        if self.spawns.len() >= SPAWN_GUARD_MAX as usize {
            let oldest = self.spawns.front().copied().unwrap_or(now);
            return Err(SPAWN_GUARD_WINDOW.saturating_sub(now.saturating_duration_since(oldest)));
        }
        self.spawns.push_back(now);
        Ok(())
    }
}

// ---- starting the program (foundation section 2.3) ----

/// The backend program and what it needs to start.
#[derive(Debug, Clone)]
pub struct BackendProgram {
    pub exe: PathBuf,
    /// Variables added to the environment of the new process. Empty in the app, where the backend
    /// inherits the shell's environment; a test sets its own pipe and folders here.
    pub env: Vec<(OsString, OsString)>,
    /// The version the shell expects the running backend to have: the shell's own.
    pub version: String,
    /// Whether a running backend of another version or path is asked to stop (the upgrade rule,
    /// foundation section 2.5). Off in a sandbox, where the dev scripts restart the backend.
    pub upgrade: bool,
}

/// Where the backend program sits: next to the shell's own program.
pub fn backend_exe_beside(shell_exe: &Path) -> PathBuf {
    shell_exe.with_file_name(BACKEND_EXE)
}

/// Two spellings of one file path: Windows ignores case, accepts `/` for `\`, and a `\\?\`
/// prefix adds nothing.
fn same_file_path(a: &str, b: &Path) -> bool {
    let simple = |text: &str| {
        text.trim_start_matches(r"\\?\")
            .replace('/', r"\")
            .to_lowercase()
    };
    simple(a) == simple(&b.to_string_lossy())
}

impl BackendProgram {
    /// A program at `exe` that inherits the environment and never asks a running backend to stop.
    pub fn at(exe: PathBuf) -> Self {
        Self {
            exe,
            env: Vec::new(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            upgrade: false,
        }
    }

    /// The backend next to the running shell. If the shell cannot tell where it is, the path stays
    /// empty, which `locate` reports as "program not found".
    pub fn beside_shell() -> Self {
        let exe = std::env::current_exe()
            .map(|shell_exe| backend_exe_beside(&shell_exe))
            .unwrap_or_default();
        Self {
            upgrade: !rain_types::env::sandbox(),
            ..Self::at(exe)
        }
    }

    /// True when the backend that answered is not the one the shell would start.
    fn is_outdated(&self, running: &Ping) -> bool {
        running.version != self.version || !same_file_path(&running.exe_path, &self.exe)
    }

    /// The program's path, or the error "not found" when the file is not there.
    fn locate(&self) -> Result<&Path, ApiError> {
        if self.exe.is_file() {
            Ok(&self.exe)
        } else {
            Err(backend_missing())
        }
    }

    /// Starts the program so that it outlives the shell: no console, own process group, outside
    /// the shell's job object. Never waits for it and never kills it.
    fn spawn_detached(&self, exe: &Path) -> io::Result<()> {
        // `clippy.toml` forbids `Command::new` so that nothing starts a process by accident. The
        // shell starting the backend is the reason this one place exists (foundation 2.3).
        #[allow(clippy::disallowed_methods)]
        let spawn = |flags: u32| {
            std::process::Command::new(exe)
                .envs(self.env.iter().map(|(name, value)| (name, value)))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(flags)
                .spawn()
        };
        let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        // The shell may run inside a job that forbids breakaway (a terminal, a task runner).
        // Windows then fails the spawn with "access denied", and the retry without breakaway works.
        let child = match spawn(detached | CREATE_BREAKAWAY_FROM_JOB) {
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => spawn(detached),
            other => other,
        }?;
        drop(child); // `Child` does not kill on drop; the backend is meant to keep running
        Ok(())
    }
}

// ---- the backend as one handle ----

/// Called with every new status, so the page can be told (`rain:backend`).
pub type StatusSink = Arc<dyn Fn(&BackendStatus) + Send + Sync>;

struct Inner {
    client: PipeClient,
    pipe_name: String,
    program: BackendProgram,
    machine: Mutex<StateMachine>,
    sink: StatusSink,
    /// Holds the spawn guard. Locked for the whole of `ensure`, so callers that ask at the same
    /// time share one start.
    start_lock: AsyncMutex<SpawnGuard>,
    /// Wakes the relay: the backend was just started or found, so the stream should connect now.
    reconnect: Notify,
    /// Limits the calls from the page that are in flight at once.
    inflight: Arc<Semaphore>,
    /// Set when the shell has asked an outdated backend to stop, whatever the answer was: the rule
    /// is tried once per shell run, so a backend that cannot match (another spelling of its path,
    /// say) is never restarted in a loop.
    upgrade_tried: AtomicBool,
    /// The task that ends the shell after game mode, while it waits. A second launch cancels it.
    exit_task: Mutex<Option<AbortHandle>>,
}

/// Everything the shell does with the backend process. Cheap to clone: all clones share one state.
#[derive(Clone)]
pub struct Backend {
    inner: Arc<Inner>,
}

impl Backend {
    pub fn new(pipe_name: &str, program: BackendProgram, sink: StatusSink) -> Self {
        Self {
            inner: Arc::new(Inner {
                client: PipeClient::new(pipe_name),
                pipe_name: pipe_name.to_owned(),
                program,
                machine: Mutex::new(StateMachine::new()),
                sink,
                start_lock: AsyncMutex::new(SpawnGuard::default()),
                reconnect: Notify::new(),
                inflight: Arc::new(Semaphore::new(CLIENT_MAX_INFLIGHT)),
                upgrade_tried: AtomicBool::new(false),
                exit_task: Mutex::new(None),
            }),
        }
    }

    /// The one pooled client of the shell: calls, the relay, the facts and the media handler share it.
    pub fn client(&self) -> PipeClient {
        self.inner.client.clone()
    }

    pub fn pipe_name(&self) -> &str {
        &self.inner.pipe_name
    }

    pub fn status(&self) -> BackendStatus {
        self.machine().status().clone()
    }

    /// The reason of a stop that the backend announced, until it is started again.
    pub fn announced_stop(&self) -> Option<StopReason> {
        self.machine().announced_stop()
    }

    /// Notified when the relay should open the stream now instead of waiting for its backoff.
    pub fn reconnect(&self) -> &Notify {
        &self.inner.reconnect
    }

    /// Takes one of the 32 places for a call from the page; the 33rd call waits here.
    pub async fn acquire_inflight(&self) -> OwnedSemaphorePermit {
        Arc::clone(&self.inner.inflight)
            .acquire_owned()
            .await
            .expect("the semaphore is never closed")
    }

    /// True when no backend should be running by the shell's own account: it crashed, or it was
    /// stopped on request (`stop`). A stop for game mode is not idle: Rain is ending.
    pub fn is_idle(&self) -> bool {
        let machine = self.machine();
        match machine.status().state {
            BackendState::Down => true,
            BackendState::Stopped => machine.announced_stop() == Some(StopReason::Stop),
            BackendState::Starting | BackendState::Up | BackendState::Restarting => false,
        }
    }

    /// A call was answered. When the shell thinks the backend is idle, someone else started it (a
    /// script, a second shell): the relay is told to open the stream, so the state becomes `up`
    /// and a later crash is seen.
    pub fn note_answered(&self) {
        if self.is_idle() {
            self.inner.reconnect.notify_one();
        }
    }

    /// Tells the state machine what happened and passes a change on to the sink.
    pub fn apply(&self, event: Event) {
        let changed = self.machine().apply(event);
        if let Some(status) = changed {
            info!(state = ?status.state, start_id = ?status.start_id, "backend state changed");
            (self.inner.sink)(&status);
        }
    }

    /// Remembers the task that will end the shell after game mode, so a second launch can cancel it.
    pub fn set_exit_task(&self, task: AbortHandle) {
        let old = self.exit_slot().replace(task);
        if let Some(old) = old {
            old.abort();
        }
    }

    /// The user started Rain again while it was ending for game mode (foundation 2.6, section 06
    /// 6.8). Cancels the pending exit and forgets the stop, so that the backend may be started
    /// again. Returns true when there was a game mode stop to cancel.
    pub fn revive_after_game_mode(&self) -> bool {
        if let Some(task) = self.exit_slot().take() {
            task.abort();
        }
        let ended_for_game_mode = self.announced_stop() == Some(StopReason::GameMode);
        if ended_for_game_mode {
            info!("Rain was started again during game mode's exit; the exit is cancelled");
            self.apply(Event::Revived);
        }
        ended_for_game_mode
    }

    /// Brings the backend back for a user who summoned the window while it was down (foundation
    /// section 2.2). A backend that was stopped for game mode stays stopped.
    pub async fn wake(&self) -> Result<(), ApiError> {
        if self.is_idle() {
            self.ensure().await?;
        }
        Ok(())
    }

    /// A second launch of the shell. Unlike `wake` it also overrides game mode's stop, because a
    /// person who starts Rain by hand wants it, not the exit that is under way.
    pub async fn relaunch(&self) -> Result<(), ApiError> {
        if self.revive_after_game_mode() {
            // The old backend may still be closing its pipe. A new one started now would see it
            // answer, leave it alone, and the old one would then disappear.
            wait_until_pipe_is_gone(self.pipe_name(), SHUTDOWN_BUDGET).await;
        }
        self.wake().await
    }

    fn exit_slot(&self) -> MutexGuard<'_, Option<AbortHandle>> {
        self.inner
            .exit_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn machine(&self) -> MutexGuard<'_, StateMachine> {
        // A poisoned lock only means that some other thread panicked; the state is still usable.
        self.inner
            .machine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes sure a backend answers on the pipe, starting one if the pipe is silent
    /// (foundation section 2.2). A running backend of another version or path is replaced first
    /// (section 2.5), unless it is busy.
    ///
    /// Callers that ask at the same time share one start. The spawn guard allows 3 starts in 60 s.
    pub async fn ensure(&self) -> Result<(), ApiError> {
        let mut guard = self.inner.start_lock.lock().await;
        if self.announced_stop() == Some(StopReason::GameMode) {
            // Game mode ended Rain on purpose; the shell is about to exit and must not undo it.
            return Err(stopped());
        }
        if let Some(running) = self.answers().await?
            && !self.replace_if_outdated(&running).await?
        {
            // Someone else started it (a script, an earlier shell). The relay may not know yet.
            if self.status().state != BackendState::Up {
                self.inner.reconnect.notify_one();
            }
            return Ok(());
        }
        if pipe_exists(self.pipe_name()) {
            // The pipe is there but the ping went unanswered: the backend is busy, not gone. A
            // second program could not use the pipe and would only use up the spawn guard.
            return Err(timeout_error());
        }
        let exe = self.inner.program.locate()?;
        if let Err(wait) = guard.try_acquire(Instant::now()) {
            warn!(
                retry_in_secs = wait.as_secs() + 1,
                "the backend was started 3 times in a minute; not starting it again yet"
            );
            return Err(crash_loop());
        }
        info!(exe = %exe.display(), "starting the backend");
        self.inner.program.spawn_detached(exe).map_err(|e| {
            warn!(error = %e, "could not start the backend program");
            backend_down()
        })?;
        self.apply(Event::Spawned);
        let ready = self.wait_until_ready().await;
        if ready.is_err() {
            self.apply(Event::StartFailed);
        }
        self.inner.reconnect.notify_one();
        ready
    }

    /// The upgrade rule (foundation section 2.5). When the backend that answers is not the one
    /// beside the shell, asks it to stop and waits for its pipe to go. Returns `Ok(true)` when it
    /// has gone and a new one must be started, `Ok(false)` when the running one stays: it is
    /// current, it is busy, or the rule was already tried in this run of the shell.
    async fn replace_if_outdated(&self, running: &Ping) -> Result<bool, ApiError> {
        let program = &self.inner.program;
        if !program.upgrade || !program.is_outdated(running) {
            return Ok(false);
        }
        if self.inner.upgrade_tried.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        info!(
            running_version = %running.version,
            expected_version = %program.version,
            "the running backend is not the one beside the shell; asking it to stop"
        );
        if !self.ask_to_stop_for_upgrade().await {
            return Ok(false);
        }
        wait_until_pipe_is_gone(self.pipe_name(), UPGRADE_WAIT).await;
        if pipe_exists(self.pipe_name()) {
            warn!("the old backend did not leave in time");
            return Err(backend_down());
        }
        Ok(true)
    }

    /// `POST /internal/shutdown` with reason `upgrade`. True when the backend agreed. A busy
    /// backend answers 409 and keeps its work; the shell then lives with it until its next start.
    async fn ask_to_stop_for_upgrade(&self) -> bool {
        let body = ShutdownBody {
            reason: StopReason::Upgrade,
            force: None,
        };
        let body = serde_json::to_vec(&body).expect("a shutdown body is always valid JSON");
        let path = format!("{API_PREFIX}/internal/shutdown");
        let call = self
            .inner
            .client
            .request(Method::POST, &path, Some(Bytes::from(body)), &[]);
        let answer = tokio::time::timeout(SHUTDOWN_REQUEST_TIMEOUT, call).await;
        match answer {
            Ok(Ok(raw)) => match raw.into_success() {
                Ok(_) => true,
                Err(error) => {
                    warn!(message = %error.message, reason = ?error.reason,
                        "the old backend stays: it did not agree to stop");
                    false
                }
            },
            Ok(Err(error)) => {
                warn!(message = %error.message, "could not ask the old backend to stop");
                false
            }
            Err(_) => {
                warn!("the old backend did not answer the request to stop");
                false
            }
        }
    }

    /// Asks the pipe whether the backend answers. `Ok(None)` means silence; an error means that
    /// something else is wrong (another program owns the pipe name, access denied).
    async fn answers(&self) -> Result<Option<Ping>, ApiError> {
        match self.ping().await {
            Ok(ping) if ping.app == BACKEND_APP_NAME => Ok(Some(ping)),
            Ok(_) => Err(shell_error(
                ErrorCode::Unavailable,
                "pipeDenied",
                "Another program answers on the Rain backend pipe",
            )),
            Err(error) if is_silence(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn ping(&self) -> Result<Ping, ApiError> {
        let path = format!("{API_PREFIX}/ping");
        let call = self.inner.client.json(Method::GET, &path, None::<&()>);
        tokio::time::timeout(PING_TIMEOUT, call)
            .await
            .unwrap_or_else(|_| Err(timeout_error()))
    }

    /// Polls the pipe every 50 ms until the new backend answers, for at most 5 s.
    async fn wait_until_ready(&self) -> Result<(), ApiError> {
        let deadline = Instant::now() + BACKEND_READY_WAIT;
        loop {
            if self.answers().await?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(backend_down());
            }
            tokio::time::sleep(BACKEND_READY_POLL).await;
        }
    }
}

/// Waits until the pipe name has disappeared, which means that the backend has really ended.
/// Gives up after `longest`.
pub(crate) async fn wait_until_pipe_is_gone(pipe_name: &str, longest: Duration) {
    let deadline = Instant::now() + longest;
    while pipe_exists(pipe_name) && Instant::now() < deadline {
        tokio::time::sleep(PIPE_GONE_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hello(id: &str) -> Event {
        Event::Hello {
            start_id: id.to_owned(),
        }
    }

    fn state_after(machine: &mut StateMachine, event: Event) -> BackendState {
        machine.apply(event);
        machine.status().state
    }

    // ---- the state machine ----

    #[test]
    fn a_new_machine_is_down() {
        assert_eq!(StateMachine::new().status().state, BackendState::Down);
    }

    #[test]
    fn a_normal_start_goes_through_starting_to_up() {
        let mut machine = StateMachine::new();
        assert_eq!(
            machine.apply(Event::Spawned),
            Some(BackendStatus::of(BackendState::Starting))
        );
        assert_eq!(
            machine.apply(hello("a")),
            Some(BackendStatus {
                state: BackendState::Up,
                start_id: Some("a".into())
            })
        );
    }

    #[test]
    fn an_event_that_changes_nothing_reports_nothing() {
        let mut machine = StateMachine::new();
        assert_eq!(machine.apply(Event::Lost), None);
        machine.apply(hello("a"));
        assert_eq!(machine.apply(hello("a")), None);
    }

    #[test]
    fn a_new_hello_with_another_start_id_is_a_change() {
        let mut machine = StateMachine::new();
        machine.apply(hello("a"));
        assert!(machine.apply(hello("b")).is_some());
    }

    #[test]
    fn a_lost_stream_without_an_announcement_is_a_crash() {
        let mut machine = StateMachine::new();
        machine.apply(hello("a"));
        assert_eq!(state_after(&mut machine, Event::Lost), BackendState::Down);
    }

    #[test]
    fn a_connection_that_fails_while_the_backend_starts_is_not_a_crash() {
        let mut machine = StateMachine::new();
        machine.apply(Event::Spawned);
        assert_eq!(machine.apply(Event::Lost), None);
        assert_eq!(machine.status().state, BackendState::Starting);
    }

    #[test]
    fn a_start_that_never_answers_ends_in_down() {
        let mut machine = StateMachine::new();
        machine.apply(Event::Spawned);
        assert_eq!(
            state_after(&mut machine, Event::StartFailed),
            BackendState::Down
        );
    }

    #[test]
    fn announced_stops_map_to_stopped_or_restarting() {
        let cases = [
            (StopReason::GameMode, BackendState::Stopped),
            (StopReason::Stop, BackendState::Stopped),
            (StopReason::Upgrade, BackendState::Restarting),
            (StopReason::Restart, BackendState::Restarting),
        ];
        for (reason, expected) in cases {
            let mut machine = StateMachine::new();
            machine.apply(hello("a"));
            assert_eq!(state_after(&mut machine, Event::Stopping(reason)), expected);
            assert_eq!(machine.announced_stop(), Some(reason));
        }
    }

    #[test]
    fn the_end_of_the_stream_after_an_announced_stop_keeps_the_state() {
        let mut machine = StateMachine::new();
        machine.apply(hello("a"));
        machine.apply(Event::Stopping(StopReason::Stop));
        assert_eq!(machine.apply(Event::Lost), None);
        assert_eq!(machine.status().state, BackendState::Stopped);
    }

    #[test]
    fn starting_again_clears_the_announced_stop() {
        let mut machine = StateMachine::new();
        machine.apply(Event::Stopping(StopReason::Stop));
        machine.apply(Event::Spawned);
        assert_eq!(machine.announced_stop(), None);
    }

    #[test]
    fn a_hello_clears_the_announced_stop() {
        let mut machine = StateMachine::new();
        machine.apply(Event::Stopping(StopReason::Restart));
        machine.apply(hello("b"));
        assert_eq!(machine.announced_stop(), None);
        assert_eq!(machine.status().state, BackendState::Up);
    }

    #[test]
    fn the_status_serialises_as_the_page_expects() {
        let up = BackendStatus {
            state: BackendState::Up,
            start_id: Some("19a3f1c2-1f4c".into()),
        };
        assert_eq!(
            serde_json::to_value(&up).unwrap(),
            json!({ "state": "up", "startId": "19a3f1c2-1f4c" })
        );
        assert_eq!(
            serde_json::to_value(BackendStatus::of(BackendState::Restarting)).unwrap(),
            json!({ "state": "restarting" })
        );
    }

    // ---- the spawn guard, on a made-up clock ----

    #[test]
    fn the_guard_allows_three_starts_in_a_minute_and_refuses_the_fourth() {
        let start = Instant::now();
        let mut guard = SpawnGuard::default();
        assert!(guard.try_acquire(start).is_ok());
        assert!(guard.try_acquire(start + Duration::from_secs(10)).is_ok());
        assert!(guard.try_acquire(start + Duration::from_secs(20)).is_ok());
        assert!(guard.try_acquire(start + Duration::from_secs(30)).is_err());
        // A refusal is not a start: it does not push the window forward.
        assert!(guard.try_acquire(start + Duration::from_secs(59)).is_err());
    }

    #[test]
    fn the_guard_allows_a_start_again_when_the_oldest_has_left_the_window() {
        let start = Instant::now();
        let mut guard = SpawnGuard::default();
        for second in [0, 10, 20] {
            assert!(
                guard
                    .try_acquire(start + Duration::from_secs(second))
                    .is_ok()
            );
        }
        assert!(guard.try_acquire(start + SPAWN_GUARD_WINDOW).is_ok()); // the first start is 60 s old
        let again = start + SPAWN_GUARD_WINDOW + Duration::from_secs(1);
        assert!(guard.try_acquire(again).is_err());
    }

    #[test]
    fn a_refusal_says_how_long_until_a_start_is_allowed() {
        let start = Instant::now();
        let mut guard = SpawnGuard::default();
        for second in [0, 10, 20] {
            guard
                .try_acquire(start + Duration::from_secs(second))
                .unwrap();
        }
        let wait = guard
            .try_acquire(start + Duration::from_secs(45))
            .unwrap_err();
        assert_eq!(wait, Duration::from_secs(15)); // the first start leaves the window at 60 s
    }

    // ---- finding the program ----

    #[test]
    fn the_backend_program_sits_next_to_the_shell() {
        let shell = Path::new(r"C:\Apps\Rain\Rain.exe");
        assert_eq!(
            backend_exe_beside(shell),
            Path::new(r"C:\Apps\Rain\rain-backend.exe")
        );
    }

    #[test]
    fn a_program_that_is_there_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("rain.exe");
        let program = BackendProgram::at(backend_exe_beside(&shell));
        std::fs::write(&program.exe, b"").unwrap();
        assert_eq!(program.locate().unwrap(), program.exe);
    }

    #[test]
    fn a_missing_program_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let program = BackendProgram::at(backend_exe_beside(&dir.path().join("rain.exe")));
        let error = program.locate().unwrap_err();
        assert_eq!(error.code, ErrorCode::Unavailable);
        assert_eq!(error.reason.as_deref(), Some("backendMissing"));
        assert_eq!(error.message, "Rain backend program was not found");
    }

    #[test]
    fn a_folder_with_the_program_name_is_not_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let program = BackendProgram::at(dir.path().join(BACKEND_EXE));
        std::fs::create_dir(&program.exe).unwrap();
        assert!(program.locate().is_err());
    }

    // ---- the upgrade rule: which backend counts as outdated ----

    fn ping_of(version: &str, exe_path: &str) -> Ping {
        Ping {
            app: BACKEND_APP_NAME.to_owned(),
            version: version.to_owned(),
            api_version: 1,
            start_id: "a".to_owned(),
            pid: 1,
            exe_path: exe_path.to_owned(),
            sandbox: false,
            busy: false,
        }
    }

    fn installed_program() -> BackendProgram {
        let mut program = BackendProgram::at(PathBuf::from(r"C:\Apps\Rain\rain-backend.exe"));
        program.version = "0.2.0".to_owned();
        program
    }

    #[test]
    fn the_same_version_at_the_same_path_is_current() {
        let ping = ping_of("0.2.0", r"C:\Apps\Rain\rain-backend.exe");
        assert!(!installed_program().is_outdated(&ping));
    }

    #[test]
    fn another_version_is_outdated() {
        let ping = ping_of("0.1.0", r"C:\Apps\Rain\rain-backend.exe");
        assert!(installed_program().is_outdated(&ping));
    }

    #[test]
    fn another_path_is_outdated() {
        let ping = ping_of("0.2.0", r"D:\Old\Rain\rain-backend.exe");
        assert!(installed_program().is_outdated(&ping));
    }

    #[test]
    fn spellings_of_one_path_are_the_same_path() {
        let exe = Path::new(r"C:\Apps\Rain\rain-backend.exe");
        for spelling in [
            r"c:\apps\rain\RAIN-BACKEND.EXE",
            "C:/Apps/Rain/rain-backend.exe",
            r"\\?\C:\Apps\Rain\rain-backend.exe",
        ] {
            assert!(same_file_path(spelling, exe), "{spelling}");
        }
        assert!(!same_file_path(r"C:\Apps\Rain\rain.exe", exe));
    }

    #[test]
    fn a_stop_for_game_mode_can_be_cancelled_and_nothing_else_can() {
        let mut machine = StateMachine::new();
        machine.apply(hello("a"));
        machine.apply(Event::Stopping(StopReason::GameMode));
        assert_eq!(
            state_after(&mut machine, Event::Revived),
            BackendState::Down
        );
        assert_eq!(machine.announced_stop(), None);
    }

    // ---- the errors of the shell ----

    #[test]
    fn only_silence_leads_to_a_start() {
        let reason = |text: &str| shell_error(ErrorCode::Unavailable, text, "x");
        assert!(is_silence(&reason("backendDown")));
        assert!(is_silence(&reason("timeout")));
        assert!(!is_silence(&reason("pipeDenied")));
        assert!(!is_silence(&reason("transport")));
    }

    #[test]
    fn the_messages_are_the_ones_of_the_foundation() {
        assert_eq!(backend_down().message, "Rain backend is not running");
        assert_eq!(
            crash_loop().message,
            "Rain backend keeps stopping. See the log."
        );
        assert_eq!(stopped().message, "Rain has stopped");
        assert_eq!(
            timeout_error().message,
            "Rain backend did not answer in time"
        );
        for error in [backend_down(), crash_loop(), stopped(), timeout_error()] {
            assert_eq!(error.code, ErrorCode::Unavailable);
            assert_eq!(error.source.as_deref(), Some("backend"));
        }
    }
}
