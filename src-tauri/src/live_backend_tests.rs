//! Tests that run the real `rain-backend.exe` and drive the shell's logic against it: starting the
//! backend, `api_request`, the relay, and the facts report. No window is created: that logic uses
//! no Tauri type (see the module comments of `backend.rs`, `bridge.rs` and `relay.rs`).
//!
//! Every test has its own pipe and its own temporary folders, and ends the backend it started.
//! The program must be built before: `cargo build -p rain-backend` (`npm run check` does it).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use hyper::Method;
use hyper::body::Bytes;
use rain_pipe::pipe_exists;
use rain_types::{ApiError, ErrorCode, Frame, Health, Ping, ShellFacts};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::backend::{
    Backend, BackendProgram, BackendState, BackendStatus, StatusSink, backend_exe_beside,
};
use crate::bridge;
use crate::relay::{self, Host};
use crate::test_server::{FakeServer, Reply};

/// How long a test waits for something that needs a process to start or stop.
const PATIENCE: Duration = Duration::from_secs(15);

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// The backend program that `cargo build -p rain-backend` left in `target\debug`.
///
/// The test program itself sits deeper (Cargo keeps its intermediate files in a separate build
/// folder), so this looks in every folder above it, closest first.
fn built_backend_exe() -> PathBuf {
    let test_exe = std::env::current_exe().expect("the test program knows its path");
    test_exe
        .ancestors()
        .skip(1)
        .map(|folder| backend_exe_beside(&folder.join("rain.exe")))
        .find(|exe| exe.is_file())
        .expect("rain-backend.exe is missing: run `cargo build -p rain-backend` first")
}

/// A backend of its own: a unique pipe, temporary folders, and a record of every state change.
struct Fixture {
    backend: Backend,
    program: BackendProgram,
    pipe_name: String,
    states: Arc<Mutex<Vec<BackendStatus>>>,
    _temp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with_exe(built_backend_exe())
    }

    fn with_exe(exe: PathBuf) -> Self {
        Self::with_program(exe, |_| {})
    }

    /// `configure` may change the program before the shell logic gets it (the upgrade rule, say).
    fn with_program(exe: PathBuf, configure: impl FnOnce(&mut BackendProgram)) -> Self {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let pipe_name = format!(
            "rain-shelltest-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let env: Vec<(OsString, OsString)> = vec![
            ("RAIN_PIPE".into(), pipe_name.clone().into()),
            ("RAIN_DATA_DIR".into(), temp.path().join("data").into()),
            ("RAIN_LOCAL_DIR".into(), temp.path().join("local").into()),
        ];
        let states = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&states);
        let sink: StatusSink =
            Arc::new(move |status| recorder.lock().unwrap().push(status.clone()));
        let mut program = BackendProgram {
            env,
            ..BackendProgram::at(exe)
        };
        configure(&mut program);
        Self {
            backend: Backend::new(&pipe_name, program.clone(), sink),
            program,
            pipe_name,
            states,
            _temp: temp,
        }
    }

    /// Another owner of the same backend, such as a script or a second shell: it starts the
    /// program on this fixture's pipe, and the fixture's own `backend` learns nothing of it.
    fn outside_starter(&self) -> Backend {
        Backend::new(&self.pipe_name, self.program.clone(), Arc::new(|_| {}))
    }

    fn states(&self) -> Vec<BackendState> {
        self.states
            .lock()
            .unwrap()
            .iter()
            .map(|status| status.state)
            .collect()
    }

    fn last_status(&self) -> Option<BackendStatus> {
        self.states.lock().unwrap().last().cloned()
    }

    fn last_state(&self) -> Option<BackendState> {
        self.last_status().map(|status| status.state)
    }

    async fn ping(&self) -> Ping {
        self.backend
            .client()
            .json(Method::GET, "/api/v1/ping", None::<&()>)
            .await
            .expect("the backend answers a ping")
    }

    /// A call the way the page makes it, through the shell's checks.
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, ApiError> {
        bridge::handle(&self.backend, method, path, body, None).await
    }

    /// A call around the shell's checks, for the routes that the shell refuses on purpose.
    async fn raw(&self, method: &str, path: &str, body: Value) -> u16 {
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let body = Some(Bytes::from(body.to_string()));
        let answer = self.backend.client().request(method, path, body, &[]).await;
        answer.expect("the backend answered").status.as_u16()
    }

    async fn ask_to_stop(&self, reason: &str) {
        let status = self
            .raw(
                "POST",
                "/api/v1/internal/shutdown",
                json!({ "reason": reason }),
            )
            .await;
        assert_eq!(status, 204);
    }
}

impl Drop for Fixture {
    /// A test that fails half way must not leave a backend behind.
    fn drop(&mut self) {
        // Only a backend that the shell logic started needs ending, and then `states` is not empty.
        if self.states.lock().unwrap().is_empty() {
            return;
        }
        let backend = self.backend.clone();
        let pipe_name = self.pipe_name.clone();
        // `drop` runs inside the test's runtime, which must not be blocked: use a thread of our own.
        let _ = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime")
                .block_on(end_backend(&backend, &pipe_name));
        })
        .join();
    }
}

/// Waits for a backend that may still be starting, asks it to stop, and waits for its pipe to go.
async fn end_backend(backend: &Backend, pipe_name: &str) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && !pipe_exists(pipe_name) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let body = Bytes::from(json!({ "reason": "stop" }).to_string());
    let _ = backend
        .client()
        .request(Method::POST, "/api/v1/internal/shutdown", Some(body), &[])
        .await;
    while Instant::now() < deadline && pipe_exists(pipe_name) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// What the relay needs from the app, recorded instead of shown.
struct FakeHost {
    frames: Mutex<Vec<Frame>>,
    exited: AtomicBool,
}

impl FakeHost {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            frames: Mutex::new(Vec::new()),
            exited: AtomicBool::new(false),
        })
    }

    fn topics(&self) -> Vec<String> {
        let frames = self.frames.lock().unwrap();
        frames.iter().map(|frame| frame.topic.clone()).collect()
    }
}

impl Host for FakeHost {
    fn emit_frame(&self, frame: &Frame) {
        self.frames.lock().unwrap().push(frame.clone());
    }

    fn shell_facts(&self) -> ShellFacts {
        ShellFacts {
            hotkey_registered: false,
            hotkey_error: Some("already taken".to_owned()),
            shell_exe: r"C:\fake\rain.exe".to_owned(),
            shell_version: "0.0.0-test".to_owned(),
            window_visible: false,
        }
    }

    fn exit_app(&self) {
        self.exited.store(true, Ordering::SeqCst);
    }
}

/// Polls until `condition` holds, or fails the test after `PATIENCE`.
async fn eventually(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Runs the relay in the background. Cancel the returned token to end it.
fn start_relay(fixture: &Fixture, host: &Arc<FakeHost>, exit_grace: Duration) -> CancellationToken {
    let stop = CancellationToken::new();
    tokio::spawn(relay::run(
        fixture.backend.clone(),
        Arc::clone(host) as Arc<dyn Host>,
        exit_grace,
        stop.clone(),
    ));
    stop
}

/// Ends a process by its id, the way Task Manager does: it gets no chance to say goodbye.
#[allow(clippy::disallowed_methods)] // a test may start the system's own tool
fn kill_process(pid: u32) {
    let output = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .expect("taskkill ran");
    assert!(
        output.status.success(),
        "taskkill could not end process {pid}"
    );
}

// ---- starting the backend ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_silent_pipe_gets_one_backend_however_many_callers_ask() {
    let fixture = Fixture::new();
    assert!(!pipe_exists(&fixture.pipe_name));

    for result in join_all((0..5).map(|_| fixture.backend.ensure())).await {
        result.expect("every caller finds the backend running");
    }
    let first = fixture.ping().await;
    assert_eq!(first.app, "rain-backend");
    assert!(first.sandbox, "a backend with its own pipe is a sandbox");
    assert_eq!(fixture.states(), [BackendState::Starting]);

    // A backend that answers is left alone: same process, no second start.
    fixture.backend.ensure().await.unwrap();
    assert_eq!(fixture.ping().await.pid, first.pid);
    assert_eq!(fixture.states(), [BackendState::Starting]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_program_is_reported_and_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::with_exe(dir.path().join("rain-backend.exe"));
    let error = fixture.backend.ensure().await.unwrap_err();
    assert_eq!(error.reason.as_deref(), Some("backendMissing"));
    assert!(fixture.states().is_empty());
    assert!(!pipe_exists(&fixture.pipe_name));
}

/// A program that ends at once and never opens the pipe.
fn system_tool_that_exits() -> PathBuf {
    let windows = std::env::var_os("SystemRoot").expect("Windows sets SystemRoot");
    PathBuf::from(windows).join(r"System32\whoami.exe")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_backend_that_never_answers_ends_in_down_and_is_not_started_forever() {
    let fixture = Fixture::with_exe(system_tool_that_exits());
    for attempt in 1..=3 {
        let error = fixture.backend.ensure().await.unwrap_err();
        assert_eq!(
            error.reason.as_deref(),
            Some("backendDown"),
            "start {attempt}"
        );
        assert_eq!(fixture.last_state(), Some(BackendState::Down));
    }
    // The fourth start within a minute is refused without starting anything.
    let error = fixture.backend.ensure().await.unwrap_err();
    assert_eq!(error.reason.as_deref(), Some("backendCrashLoop"));
    assert_eq!(error.message, "Rain backend keeps stopping. See the log.");
    assert_eq!(
        fixture
            .states()
            .iter()
            .filter(|s| **s == BackendState::Starting)
            .count(),
        3
    );
}

// ---- api_request ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_page_gets_the_answers_of_the_backend_and_the_first_call_starts_it() {
    let fixture = Fixture::new();

    // The pipe is silent: the call starts the backend and is tried again (foundation 6.7).
    let ping: Ping = serde_json::from_value(fixture.call("GET", "/ping", None).await.unwrap())
        .expect("the answer is a ping");
    assert_eq!(ping.app, "rain-backend");
    assert_eq!(fixture.states(), [BackendState::Starting]);

    // Settings: a read, a change with a body, and a value outside its allowed set.
    let settings = fixture.call("GET", "/settings", None).await.unwrap();
    assert_eq!(
        settings["hotkeyAvailable"],
        json!(true),
        "no shell has reported yet"
    );
    let patched = fixture
        .call(
            "PATCH",
            "/settings",
            Some(json!({ "resetAfterSeconds": 300 })),
        )
        .await
        .unwrap();
    assert_eq!(patched["resetAfterSeconds"], json!(300));
    assert_eq!(
        fixture.call("GET", "/settings", None).await.unwrap()["resetAfterSeconds"],
        json!(300)
    );

    // An error body of the backend reaches the page unchanged.
    let error = fixture
        .call(
            "PATCH",
            "/settings",
            Some(json!({ "resetAfterSeconds": 45 })),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Invalid);
    assert_eq!(error.reason.as_deref(), Some("badBody"));
    let error = fixture
        .call("GET", "/nothing-here", None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.reason.as_deref(), Some("noRoute"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_shell_refuses_internal_routes_and_answers_its_own_path() {
    let fixture = Fixture::new();
    fixture.backend.ensure().await.unwrap();

    let error = fixture
        .call(
            "POST",
            "/internal/shutdown",
            Some(json!({ "reason": "stop" })),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Invalid);
    assert_eq!(error.reason.as_deref(), Some("badRequest"));
    fixture.ping().await; // the backend did not stop

    let local = fixture.call("GET", "/local/backend", None).await.unwrap();
    assert_eq!(local, json!({ "state": "starting" }));
    let error = fixture.call("GET", "/local/other", None).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_to_a_stopped_game_mode_backend_is_answered_with_stopped() {
    let fixture = Fixture::new();
    fixture.backend.ensure().await.unwrap();
    let host = FakeHost::new();
    let relay = start_relay(&fixture, &host, Duration::from_millis(10));
    eventually("the backend is up", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;

    fixture.ask_to_stop("gameMode").await;
    eventually("the shell exits", || host.exited.load(Ordering::SeqCst)).await;
    relay.cancel();

    assert_eq!(fixture.last_state(), Some(BackendState::Stopped));
    assert!(
        !pipe_exists(&fixture.pipe_name),
        "the shell waited for the pipe to close"
    );
    // Nothing starts the backend again, however it is asked.
    for error in [
        fixture.backend.ensure().await.unwrap_err(),
        fixture.call("GET", "/ping", None).await.unwrap_err(),
    ] {
        assert_eq!(error.code, ErrorCode::Unavailable);
        assert_eq!(error.reason.as_deref(), Some("stopped"));
        assert_eq!(error.message, "Rain has stopped");
    }
    assert!(!pipe_exists(&fixture.pipe_name));
}

// ---- the relay ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_relay_follows_a_backend_through_up_stop_start_and_crash() {
    let fixture = Fixture::new();
    let host = FakeHost::new();
    let relay = start_relay(&fixture, &host, Duration::from_millis(10));

    // 1. The backend is not running; the first ensure starts it and the stream says hello.
    fixture.backend.ensure().await.unwrap();
    eventually("the backend is up", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    let first_id = fixture
        .last_status()
        .unwrap()
        .start_id
        .expect("up has a start id");
    assert_eq!(
        &host.topics()[..2],
        ["hello", "resync"],
        "the page sees the frames in order"
    );
    assert_eq!(fixture.states(), [BackendState::Starting, BackendState::Up]);

    // 2. The facts of the shell were reported before the page was told: Settings shows them.
    let health: Health =
        serde_json::from_value(fixture.call("GET", "/health", None).await.unwrap()).unwrap();
    assert!(health.shell.reported);
    assert_eq!(health.shell.hotkey_registered, Some(false));
    let settings = fixture.call("GET", "/settings", None).await.unwrap();
    assert_eq!(settings["hotkeyAvailable"], json!(false));

    // 3. A stop on request: the state is `stopped` and the relay goes quiet.
    fixture.ask_to_stop("stop").await;
    eventually("the state is stopped", || {
        fixture.last_state() == Some(BackendState::Stopped)
    })
    .await;
    assert!(host.topics().contains(&"lifecycle".to_owned()));
    eventually("the pipe is gone", || !pipe_exists(&fixture.pipe_name)).await;
    assert!(
        !host.exited.load(Ordering::SeqCst),
        "only game mode ends the shell"
    );

    // 4. The next call starts the backend again, and the relay finds it without being told.
    fixture.call("GET", "/ping", None).await.unwrap();
    eventually("the backend is up again", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    let second_id = fixture.last_status().unwrap().start_id.unwrap();
    assert_ne!(first_id, second_id, "a new process has a new start id");

    // 5. A crash: no goodbye. The shell sees `down`, starts a new backend, and is up again.
    let before_crash = fixture.states().len();
    kill_process(fixture.ping().await.pid);
    eventually("the backend is up after the crash", || {
        fixture.last_state() == Some(BackendState::Up) && fixture.states().len() > before_crash + 2
    })
    .await;
    let crash_states = &fixture.states()[before_crash..];
    assert_eq!(
        crash_states,
        [BackendState::Down, BackendState::Starting, BackendState::Up]
    );
    let third_id = fixture.last_status().unwrap().start_id.unwrap();
    assert!(third_id != first_id && third_id != second_id);

    relay.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_backend_started_outside_the_shell_after_a_stop_is_noticed_by_the_next_call() {
    let fixture = Fixture::new();
    let host = FakeHost::new();
    let relay = start_relay(&fixture, &host, Duration::from_millis(10));
    fixture.backend.ensure().await.unwrap();
    eventually("the backend is up", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    let first_id = fixture.last_status().unwrap().start_id.unwrap();

    // `npm run backend:stop` and then `backend:start`: the shell does not start the new backend.
    fixture.ask_to_stop("stop").await;
    eventually("the state is stopped", || {
        fixture.last_state() == Some(BackendState::Stopped)
    })
    .await;
    eventually("the pipe is gone", || !pipe_exists(&fixture.pipe_name)).await;
    fixture.outside_starter().ensure().await.unwrap();
    assert_eq!(fixture.last_state(), Some(BackendState::Stopped));

    // The call is answered at once, and the relay is woken: it opens the stream, hears the hello.
    fixture.call("GET", "/ping", None).await.unwrap();
    eventually("the shell sees the new backend", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    assert_ne!(fixture.last_status().unwrap().start_id.unwrap(), first_id);

    // And it is watched again: a crash of this backend is answered with a new one.
    let before_crash = fixture.states().len();
    kill_process(fixture.ping().await.pid);
    eventually("the backend is up after the crash", || {
        fixture.states().len() > before_crash + 2 && fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    relay.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_backend_is_not_given_a_second_one() {
    use tokio::net::windows::named_pipe::ServerOptions;

    let fixture = Fixture::with_exe(system_tool_that_exits());
    // A pipe that exists and never answers: a backend too busy to reply within the ping timeout.
    let _busy = ServerOptions::new()
        .create(format!(r"\\.\pipe\{}", fixture.pipe_name))
        .expect("a pipe of our own");

    let error = fixture.backend.ensure().await.unwrap_err();
    assert_eq!(error.reason.as_deref(), Some("timeout"));
    assert!(fixture.states().is_empty(), "nothing was started");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_goes_on_when_the_page_stops_waiting_for_its_call() {
    // The program never opens the pipe, so a start that runs to its end must report `down`.
    let fixture = Fixture::with_exe(system_tool_that_exits());
    let error = bridge::handle(&fixture.backend, "GET", "/ping", None, Some(50))
        .await
        .unwrap_err();
    assert_eq!(error.reason.as_deref(), Some("timeout"));

    eventually("the start ended in down", || {
        fixture.states() == [BackendState::Starting, BackendState::Down]
    })
    .await;
}

// ---- the upgrade rule (foundation section 2.5) ----

/// A shell that expects another version than the backend program reports, with the rule switched
/// on (the app switches it off in a sandbox, and every test runs in one).
fn expecting_a_newer_version(program: &mut BackendProgram) {
    program.version = "999.0.0".to_owned();
    program.upgrade = true;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_backend_of_another_version_is_replaced_once() {
    let fixture = Fixture::with_program(built_backend_exe(), expecting_a_newer_version);
    fixture.backend.ensure().await.unwrap(); // nothing runs yet: a plain start
    let old = fixture.ping().await;

    fixture.backend.ensure().await.unwrap();

    let new = fixture.ping().await;
    assert_ne!(new.pid, old.pid, "the old backend was replaced");
    assert_ne!(new.start_id, old.start_id);
    // The new backend has the same (wrong) version. The shell does not ask again: a backend that
    // can never match must not be restarted in a loop.
    fixture.backend.ensure().await.unwrap();
    assert_eq!(fixture.ping().await.pid, new.pid);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_backend_of_the_expected_version_is_left_alone() {
    let fixture = Fixture::with_program(built_backend_exe(), |program| program.upgrade = true);
    fixture.backend.ensure().await.unwrap();
    let first = fixture.ping().await;

    fixture.backend.ensure().await.unwrap();

    assert_eq!(fixture.ping().await.pid, first.pid);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_a_sandbox_the_upgrade_rule_does_not_apply() {
    let fixture = Fixture::with_program(built_backend_exe(), |program| {
        program.version = "999.0.0".to_owned(); // upgrade stays false
    });
    fixture.backend.ensure().await.unwrap();
    let first = fixture.ping().await;

    fixture.backend.ensure().await.unwrap();

    assert_eq!(fixture.ping().await.pid, first.pid);
}

/// A backend that reports itself as old and busy, and refuses to stop like the real one does.
fn busy_old_backend() -> FakeServer {
    FakeServer::start(|seen| match (seen.method.as_str(), seen.path()) {
        ("GET", "/api/v1/ping") => Reply::json(
            200,
            &serde_json::to_value(Ping {
                app: "rain-backend".to_owned(),
                version: "0.0.1".to_owned(),
                api_version: 1,
                start_id: "old".to_owned(),
                pid: 1,
                exe_path: r"C:\Old\rain-backend.exe".to_owned(),
                sandbox: false,
                busy: true,
            })
            .unwrap(),
        ),
        ("POST", "/api/v1/internal/shutdown") => Reply::json(
            409,
            &json!({ "code": "conflict", "message": "Rain is busy", "reason": "busy" }),
        ),
        _ => Reply::json(404, &json!({ "code": "notFound", "message": "no" })),
    })
}

fn shutdown_requests(server: &FakeServer) -> Vec<Value> {
    server
        .seen()
        .iter()
        .filter(|seen| seen.path() == "/api/v1/internal/shutdown")
        .map(|seen| serde_json::from_slice(&seen.body).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_backend_is_kept_and_not_asked_again() {
    let server = busy_old_backend();
    let mut program = BackendProgram::at(system_tool_that_exits());
    program.upgrade = true;
    let backend = Backend::new(server.pipe_name(), program, Arc::new(|_| {}));

    backend.ensure().await.expect("a busy backend is kept");
    backend.ensure().await.expect("and still kept");

    assert_eq!(
        shutdown_requests(&server),
        [json!({ "reason": "upgrade" })],
        "one polite request, with the reason upgrade and no force"
    );
    assert_eq!(
        backend.status().state,
        BackendState::Down,
        "nothing was started, nothing was stopped"
    );
}

// ---- the exit after game mode, and a second launch during it ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_launch_during_the_grace_cancels_the_exit_and_starts_rain_again() {
    let fixture = Fixture::new();
    fixture.backend.ensure().await.unwrap();
    let host = FakeHost::new();
    let grace = Duration::from_secs(2);
    let relay = start_relay(&fixture, &host, grace);
    eventually("the backend is up", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    let first_id = fixture.last_status().unwrap().start_id.unwrap();

    fixture.ask_to_stop("gameMode").await;
    eventually("the pipe is gone", || !pipe_exists(&fixture.pipe_name)).await;
    assert_eq!(fixture.last_state(), Some(BackendState::Stopped));

    // The user starts Rain by hand inside the grace.
    fixture.backend.relaunch().await.unwrap();
    eventually("the new backend is up", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    assert_ne!(fixture.last_status().unwrap().start_id.unwrap(), first_id);

    tokio::time::sleep(grace + Duration::from_millis(500)).await;
    assert!(
        !host.exited.load(Ordering::SeqCst),
        "the cancelled exit did not happen"
    );
    relay.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_second_launch_the_shell_exits_after_the_grace() {
    let fixture = Fixture::new();
    fixture.backend.ensure().await.unwrap();
    let host = FakeHost::new();
    let relay = start_relay(&fixture, &host, Duration::from_millis(300));
    eventually("the backend is up", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;

    fixture.ask_to_stop("gameMode").await;
    eventually("the shell exits", || host.exited.load(Ordering::SeqCst)).await;
    assert!(!pipe_exists(&fixture.pipe_name));
    relay.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waking_the_backend_does_not_undo_a_game_mode_stop() {
    let fixture = Fixture::new();
    fixture.backend.ensure().await.unwrap();
    let host = FakeHost::new();
    let relay = start_relay(&fixture, &host, Duration::from_secs(5));
    eventually("the backend is up", || {
        fixture.last_state() == Some(BackendState::Up)
    })
    .await;
    fixture.ask_to_stop("gameMode").await;
    eventually("the pipe is gone", || !pipe_exists(&fixture.pipe_name)).await;

    // The hotkey and a call from the page both only wake; neither may start the backend.
    fixture.backend.wake().await.unwrap();
    assert!(fixture.call("GET", "/ping", None).await.is_err());

    assert!(!pipe_exists(&fixture.pipe_name));
    assert_eq!(fixture.last_state(), Some(BackendState::Stopped));
    relay.cancel();
}

// ---- the shell tells the backend when a fact changes ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_fact_is_reported_to_the_backend() {
    let fixture = Fixture::new();
    fixture.backend.ensure().await.unwrap();
    let facts = crate::facts::Facts::new();
    facts.set_hotkey(Err("already taken".into()));
    assert!(facts.retry_hotkey(|| Ok(())), "the other program let go");

    crate::facts::report(&fixture.backend.client(), &facts.snapshot(true))
        .await
        .unwrap();

    let health: Health =
        serde_json::from_value(fixture.call("GET", "/health", None).await.unwrap()).unwrap();
    assert!(health.shell.reported);
    assert_eq!(health.shell.hotkey_registered, Some(true));
    let settings = fixture.call("GET", "/settings", None).await.unwrap();
    assert_eq!(settings["hotkeyAvailable"], json!(true));
}
