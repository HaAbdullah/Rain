//! Idle jobs on the harness (section 03 10.2). The machine is a fake: a test says how long the PC
//! has been idle and whether a game runs. The programs are tiny made-up ones (`test_tool`, `ping`).
//! The waits of the module (sampling every 30 s, "a minute" of a time limit) are shortened
//! through the `Tuning` seam.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use common::TestBackend;
use futures_util::future::BoxFuture;
use hyper::StatusCode;
use rain_backend::modules::idle::{GamesState, MachineIdle};
use rain_backend::modules::jobs::Tuning;
use rain_types::{Health, IdleJob, IdleResource, IdleState, LogLevel, LogLine, SourceState};
use serde_json::json;

// ---- fakes and helpers ----

/// How long the PC has been idle, as the test says. `None` when `refuse` is set.
#[derive(Default)]
struct FakeIdle {
    ms: AtomicU64,
    refuse: AtomicBool,
}

impl FakeIdle {
    fn set_minutes(&self, minutes: u64) {
        self.ms.store(minutes * 60_000, Ordering::SeqCst);
    }

    fn set_ms(&self, ms: u64) {
        self.ms.store(ms, Ordering::SeqCst);
    }
}

impl MachineIdle for FakeIdle {
    fn idle_ms(&self) -> Option<u64> {
        if self.refuse.load(Ordering::SeqCst) {
            None
        } else {
            Some(self.ms.load(Ordering::SeqCst))
        }
    }
}

#[derive(Default)]
struct FakeGames(AtomicBool);

impl GamesState for FakeGames {
    fn any_game_running(&self) -> BoxFuture<'_, bool> {
        Box::pin(std::future::ready(self.0.load(Ordering::SeqCst)))
    }
}

fn tuning() -> Arc<Tuning> {
    Arc::new(Tuning {
        catch_up_delay: Duration::from_secs(3600),
        minute: Duration::from_millis(100),
        log_flush: Duration::from_millis(20),
        idle_slow: Duration::from_millis(40),
        idle_fast: Duration::from_millis(40),
        ..Tuning::default()
    })
}

struct Setup {
    backend: TestBackend,
    idle: Arc<FakeIdle>,
    games: Arc<FakeGames>,
}

async fn start_with(text: &str) -> Setup {
    let idle = Arc::new(FakeIdle::default());
    let games = Arc::new(FakeGames::default());
    let backend = TestBackend::builder()
        .file("idle-jobs.toml", text)
        .with::<dyn MachineIdle>(idle.clone())
        .with::<dyn GamesState>(games.clone())
        .with::<Tuning>(tuning())
        .start()
        .await;
    Setup {
        backend,
        idle,
        games,
    }
}

/// The path of a program that `cargo test` builds from `examples/` (see `tests/jobs.rs`).
fn example(name: &str) -> PathBuf {
    let file = format!("{name}.exe");
    let exe = std::env::current_exe().expect("the test program has a path");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = exe
        .ancestors()
        .map(|folder| folder.join("examples").join(&file))
        .chain(["debug", "release"].map(|profile| {
            manifest
                .join("../../target")
                .join(profile)
                .join("examples")
                .join(&file)
        }));
    candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| {
            panic!("{file} is missing: run `cargo build -p rain-backend --examples`")
        })
}

/// One idle job as TOML. Strings are literals, so Windows paths need no escaping.
fn idle_toml(
    id: &str,
    resource: &str,
    enabled: bool,
    command: &str,
    args: &[&str],
    extra: &str,
) -> String {
    let args: Vec<String> = args.iter().map(|a| format!("'{a}'")).collect();
    format!(
        "[[idle_job]]\nid = '{id}'\nname = 'Job {id}'\nenabled = {enabled}\nresource = '{resource}'\nidle_after_minutes = 1\ncommand = '{command}'\nargs = [{}]\n{extra}\n",
        args.join(", ")
    )
}

/// A job that prints a line every 100 ms for `ticks` ticks.
fn ticker(id: &str, resource: &str, enabled: bool, ticks: u32, extra: &str) -> String {
    let tool = example("test_tool");
    idle_toml(
        id,
        resource,
        enabled,
        &tool.display().to_string(),
        &["ticker", &ticks.to_string(), "100"],
        extra,
    )
}

async fn jobs(backend: &TestBackend) -> Vec<IdleJob> {
    backend.get("/idle-jobs").await
}

async fn job(backend: &TestBackend, id: &str) -> IdleJob {
    jobs(backend)
        .await
        .into_iter()
        .find(|job| job.id == id)
        .unwrap_or_else(|| panic!("no idle job {id}"))
}

async fn wait_job(backend: &TestBackend, id: &str, done: impl Fn(&IdleJob) -> bool) -> IdleJob {
    let polling = async {
        loop {
            let current = job(backend, id).await;
            if done(&current) {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), polling)
        .await
        .unwrap_or_else(|_| panic!("idle job {id} never reached the state the test waits for"))
}

async fn log(backend: &TestBackend, id: &str) -> Vec<LogLine> {
    backend.get(&format!("/idle-jobs/{id}/log")).await
}

fn has_line(lines: &[LogLine], text: &str) -> bool {
    lines.iter().any(|line| line.text == text)
}

fn ticks(lines: &[LogLine]) -> usize {
    lines.iter().filter(|l| l.text.starts_with("tick ")).count()
}

// ---- the files and the list ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_jobs_list_empty_then_commented_file_written() {
    let backend = TestBackend::start().await;
    assert!(jobs(&backend).await.is_empty());
    let written = std::fs::read_to_string(backend.data_dir.join("idle-jobs.toml")).unwrap();
    assert!(written.starts_with("schema_version = 1"));
    assert!(written.contains("# [[idle_job]]") && written.contains("rain-progress: 42%"));
    assert!(!written.contains("\n[[idle_job]]"));
    assert!(
        !backend.state.idle().sampler_active(),
        "no idle job, no sampler"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_typical_folder_lists_two_idle_jobs() {
    let backend = TestBackend::with_fixture("typical").await;
    let list = jobs(&backend).await;
    assert_eq!(list.len(), 2);
    assert_eq!(
        (list[0].id.as_str(), list[0].resource, list[0].enabled),
        ("idle-cpu-burner", IdleResource::Cpu, true)
    );
    assert_eq!(
        (list[1].id.as_str(), list[1].resource, list[1].enabled),
        ("idle-gpu-wait", IdleResource::Gpu, false)
    );
    assert_eq!(
        list[0].state,
        IdleState::Queued,
        "a job that never ran is queued"
    );
    assert_eq!(list[0].idle_after_minutes, 1440);
    assert_eq!(list[0].progress, 0.0);
    // The enabled job needs a sampler; it waits for a whole day of quiet, so nothing starts.
    assert!(backend.state.idle().sampler_active());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_toggle_writes_enabled_keeps_comments() {
    let text = "schema_version = 1\n# mine\n\n# first\n[[idle_job]]\nid = 'a'\nname = 'A'\nenabled = false # keep me\ncommand = 'cmd'\n\n# no id here\n[[idle_job]]\nname = 'Second thing'\ncommand = 'cmd'\n";
    let setup = start_with(text).await;
    let backend = &setup.backend;
    let path = backend.data_dir.join("idle-jobs.toml");

    let (status, body) = backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], true);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        text.replace("enabled = false # keep me", "enabled = true # keep me")
    );

    let (status, _) = backend
        .send(
            "PATCH",
            "/idle-jobs/idle-second-thing",
            Some(json!({ "enabled": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains(
            "# no id here\n[[idle_job]]\nid = 'idle-second-thing'\nname = 'Second thing'"
        )
    );
    assert!(after.contains("command = 'cmd'\nenabled = true\n"));

    let (status, body) = backend
        .send(
            "PATCH",
            "/idle-jobs/nobody",
            Some(json!({ "enabled": true })),
        )
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownIdleJob"))
    );
    let (status, _) = backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": 3 })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_idle_entries_are_skipped_and_named_in_health() {
    let text = format!(
        "schema_version = 1\n\n[[idle_job]]\nname = 'No command'\n\n{}",
        idle_toml("good", "cpu", false, "cmd", &[], "")
    );
    let setup = start_with(&text).await;
    let backend = &setup.backend;
    let ids: Vec<_> = jobs(backend).await.into_iter().map(|j| j.id).collect();
    assert_eq!(ids, ["good"]);
    let health: Health = backend.get("/health").await;
    let issues: Vec<_> = health
        .config_issues
        .iter()
        .filter(|i| i.file == "idle-jobs.toml")
        .collect();
    assert_eq!(issues.len(), 1, "{health:?}");
    assert!(issues[0].message.contains("`command` is missing"));
    assert_eq!(issues[0].line, Some(3));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_idle_log_route_is_empty_for_a_job_that_never_ran_and_refuses_an_unknown_one() {
    let setup = start_with(&idle_toml("a", "cpu", false, "cmd", &[], "")).await;
    assert!(log(&setup.backend, "a").await.is_empty());
    let (status, body) = setup
        .backend
        .send("GET", "/idle-jobs/nobody/log", None)
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownIdleJob"))
    );
    let (status, body) = setup
        .backend
        .send("POST", "/idle-jobs/nobody/run", None)
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownIdleJob"))
    );
}

// ---- the sampler ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_sampler_only_while_enabled() {
    let text = ticker("a", "cpu", false, 600, "");
    let setup = start_with(&text).await;
    let backend = &setup.backend;
    let idle = backend.state.idle();
    assert!(!idle.sampler_active(), "all idle jobs are disabled");
    assert_eq!(idle.sampler_starts(), 0);

    backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert!(idle.sampler_active(), "started when a job is enabled");
    backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": false })))
        .await;
    assert!(
        !idle.sampler_active(),
        "ended when the last job is disabled"
    );

    backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert!(idle.sampler_active());
    // Game mode: the sampler is cancelled, and a run that is going on is not touched.
    setup.idle.set_minutes(5);
    wait_job(backend, "a", |j| j.state == IdleState::Running).await;
    idle.wind_down().pause(true);
    assert!(!idle.sampler_active());
    assert_eq!(idle.wind_down().counts().running, 1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        job(backend, "a").await.state,
        IdleState::Running,
        "a running run is left alone"
    );
    idle.wind_down().pause(false);
    assert!(idle.sampler_active(), "started again after game mode");
    assert_eq!(idle.sampler_starts(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sampler_is_not_started_while_game_mode_is_on() {
    let setup = start_with(&ticker("a", "cpu", true, 10, "")).await;
    let idle = setup.backend.state.idle();
    assert!(idle.sampler_active());
    idle.wind_down().pause(true);
    assert!(!idle.sampler_active());
    // Enabling or reloading while game mode is on does not bring it back.
    setup
        .backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": false })))
        .await;
    setup
        .backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert!(!idle.sampler_active());
}

// ---- runs ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_runs_pauses_resumes() {
    let setup = start_with(&ticker("a", "cpu", true, 30, "")).await;
    let backend = &setup.backend;

    // Not idle long enough (30 s of a minute): nothing starts.
    setup.idle.set_ms(30_000);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(job(backend, "a").await.state, IdleState::Queued);

    // A minute of quiet: it starts.
    setup.idle.set_minutes(1);
    wait_job(backend, "a", |j| j.state == IdleState::Running).await;

    // The user comes back: the run is frozen, and a frozen program prints nothing.
    setup.idle.set_ms(1_000);
    let paused = wait_job(backend, "a", |j| j.state == IdleState::Paused).await;
    assert!(paused.eta_minutes.is_none());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = ticks(&log(backend, "a").await);
    tokio::time::sleep(Duration::from_millis(900)).await;
    let after = ticks(&log(backend, "a").await);
    assert_eq!(
        before, after,
        "the program made progress while it was frozen"
    );
    assert!(has_line(
        &log(backend, "a").await,
        "Paused: the PC is in use"
    ));
    assert_eq!(
        backend.state.idle().wind_down().counts().running,
        1,
        "a frozen run counts as running"
    );

    // A minute of quiet again: it goes on and ends.
    setup.idle.set_minutes(2);
    wait_job(backend, "a", |j| j.state == IdleState::Running).await;
    let done = wait_job(backend, "a", |j| j.state == IdleState::Success).await;
    assert_eq!(done.progress, 1.0);
    assert!(done.last_run_at.is_some());
    let lines = log(backend, "a").await;
    assert_eq!(lines[0].text, "Started after 1 min idle");
    assert_eq!(lines[1].text, "Using the CPU");
    assert!(has_line(&lines, "Resumed after 2 min idle"));
    assert!(has_line(&lines, "tick 30"));
    assert!(has_line(&lines, "Process exited with code 0"));
    assert_eq!(
        lines.iter().filter(|l| l.level == LogLevel::Warn).count(),
        1
    );
    assert_eq!(backend.state.idle().wind_down().counts().running, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_resource_exclusive() {
    let text = format!(
        "{}{}{}",
        ticker("cpu-1", "cpu", true, 200, ""),
        ticker("cpu-2", "cpu", true, 200, ""),
        ticker("gpu-1", "gpu", true, 200, ""),
    );
    let setup = start_with(&text).await;
    let backend = &setup.backend;
    setup.idle.set_minutes(5);
    wait_job(backend, "cpu-1", |j| j.state == IdleState::Running).await;
    wait_job(backend, "gpu-1", |j| j.state == IdleState::Running).await;
    // The second CPU job waits, and "Run now" says who is using the CPU.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(job(backend, "cpu-2").await.state, IdleState::Queued);
    let (status, body) = backend.send("POST", "/idle-jobs/cpu-2/run", None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("resourceBusy"))
    );
    assert_eq!(body["message"], "Job cpu-1 is using the CPU right now");
    assert_eq!(job(backend, "cpu-2").await.state, IdleState::Queued);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_run_now_ignores_idle() {
    // A disabled job, and a PC that is in use: "Run now" starts it, and it is never frozen.
    let setup = start_with(&ticker("a", "cpu", false, 15, "")).await;
    let backend = &setup.backend;
    setup.idle.set_ms(0);
    let (status, started) = backend.send("POST", "/idle-jobs/a/run", None).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["state"], "running");

    // A second call returns the running job.
    let (status, again) = backend.send("POST", "/idle-jobs/a/run", None).await;
    assert_eq!(
        (status, again["state"].as_str()),
        (StatusCode::OK, Some("running"))
    );

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        job(backend, "a").await.state,
        IdleState::Running,
        "a manual run is not frozen"
    );
    let done = wait_job(backend, "a", |j| j.state == IdleState::Success).await;
    assert_eq!(done.progress, 1.0);
    let lines = log(backend, "a").await;
    assert_eq!(lines[0].text, "Started by Run now");
    assert!(!has_line(&lines, "Paused: the PC is in use"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_disable_stops_process() {
    let setup = start_with(&ticker("a", "cpu", true, 600, "")).await;
    let backend = &setup.backend;
    setup.idle.set_minutes(5);
    wait_job(backend, "a", |j| j.state == IdleState::Running).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (status, body) = backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": false })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stopped = wait_job(backend, "a", |j| j.state == IdleState::Paused && !j.enabled).await;
    assert!(stopped.progress < 1.0);
    // The process is gone: no run is counted, and the log says why.
    wait_counts_running(backend, 0).await;
    let lines = log(backend, "a").await;
    assert!(has_line(&lines, "Stopped: the job was switched off"));
    let count = ticks(&lines);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        ticks(&log(backend, "a").await),
        count,
        "nothing runs any more"
    );
    assert!(!backend.state.idle().sampler_active());

    // Switched on again: queued, from the start.
    backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": true })))
        .await;
    let queued = wait_job(backend, "a", |j| j.enabled && j.state != IdleState::Paused).await;
    assert!(matches!(
        queued.state,
        IdleState::Queued | IdleState::Running
    ));
}

async fn wait_counts_running(backend: &TestBackend, expected: u32) {
    let polling = async {
        while backend.state.idle().wind_down().counts().running != expected {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), polling)
        .await
        .expect("the number of runs reached the wanted value");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_progress_reported() {
    let tool = example("test_tool");
    let text = idle_toml(
        "a",
        "cpu",
        false,
        &tool.display().to_string(),
        &[
            "progress",
            "350",
            "rain-progress: 10%",
            "rain-eta: 7",
            "rain-progress: 50%",
            "rain-progress: 50.2%",
            "rain-progress: 20%",
            "rain-progress: 0.9",
        ],
        "",
    );
    let setup = start_with(&text).await;
    let backend = &setup.backend;
    backend.send("POST", "/idle-jobs/a/run", None).await;

    // While it runs: progress never goes down, and the reported eta shows.
    let mut seen = Vec::new();
    let mut eta_seen = false;
    loop {
        let now = job(backend, "a").await;
        seen.push(now.progress);
        eta_seen |= now.eta_minutes == Some(7);
        if now.state == IdleState::Success {
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    assert!(
        eta_seen,
        "the reported eta is shown while running: {seen:?}"
    );
    assert!(
        seen.windows(2).all(|w| w[0] <= w[1]),
        "progress went down: {seen:?}"
    );
    assert!(seen.iter().any(|p| (*p - 0.5).abs() < 0.01), "{seen:?}");
    assert_eq!(*seen.last().unwrap(), 1.0);
    let lines = log(backend, "a").await;
    assert!(
        has_line(&lines, "rain-progress: 50%"),
        "the report lines are in the log too"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_progress_changes_under_one_percent_are_not_published() {
    let tool = example("test_tool");
    let text = idle_toml(
        "a",
        "cpu",
        false,
        &tool.display().to_string(),
        &[
            "progress",
            "1200",
            "rain-progress: 50%",
            "rain-progress: 50.4%",
            "rain-progress: 60%",
        ],
        "",
    );
    let setup = start_with(&text).await;
    let backend = &setup.backend;
    let mut stream = backend.stream().await;
    backend.send("POST", "/idle-jobs/a/run", None).await;

    // Frames: the start (0), 50 %, and then 60 % (50.4 % was a change of 0.4 points: no frame),
    // and the end (1.0). Each report is 1.2 s apart, far more than the 250 ms of the throttle.
    let mut progress = Vec::new();
    loop {
        let frame = stream.expect_topic("idleJobs").await;
        let now = frame.payload[0]["progress"].as_f64().unwrap();
        let state = frame.payload[0]["state"].as_str().unwrap().to_owned();
        progress.push(now);
        if state == "success" {
            break;
        }
    }
    assert!(
        progress.iter().any(|p| (*p - 0.5).abs() < 0.001),
        "{progress:?}"
    );
    assert!(
        !progress.iter().any(|p| (*p - 0.504).abs() < 0.0005),
        "{progress:?}"
    );
    assert!(
        progress.iter().any(|p| (*p - 0.6).abs() < 0.001),
        "{progress:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_machine_none_and_a_running_game_hold_everything_back() {
    let setup = start_with(&ticker("a", "cpu", true, 10, "")).await;
    let backend = &setup.backend;

    // Windows refuses to say: the user counts as present, and the source says so.
    setup.idle.refuse.store(true, Ordering::SeqCst);
    let polling = async {
        loop {
            let health: Health = backend.get("/health").await;
            if let Some(source) = health.sources.iter().find(|s| s.name == "idle-sampler") {
                return source.clone();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    };
    let source = tokio::time::timeout(Duration::from_secs(10), polling)
        .await
        .expect("the source appeared");
    assert_eq!(source.state, SourceState::Failed);
    assert_eq!(
        source.message.as_deref(),
        Some("Idle detection is not available")
    );
    assert_eq!(job(backend, "a").await.state, IdleState::Queued);

    // It works again, but a game runs: a game pad does not move the input clock.
    setup.idle.refuse.store(false, Ordering::SeqCst);
    setup.idle.set_minutes(30);
    setup.games.0.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(job(backend, "a").await.state, IdleState::Queued);
    let health: Health = backend.get("/health").await;
    assert!(
        health.sources.iter().all(|s| s.name != "idle-sampler"),
        "the source is cleared"
    );

    setup.games.0.store(false, Ordering::SeqCst);
    wait_job(backend, "a", |j| j.state == IdleState::Running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_wind_down() {
    let setup = start_with(&ticker("a", "cpu", true, 600, "")).await;
    let backend = &setup.backend;
    setup.idle.set_minutes(5);
    wait_job(backend, "a", |j| j.state == IdleState::Running).await;
    let idle = backend.state.idle();
    assert_eq!(idle.wind_down().counts().enabled, 1);

    // Game mode: nothing new starts by hand either.
    idle.wind_down().pause(true);
    backend.state.set_game_mode_active(true);
    let (status, body) = backend.send("POST", "/idle-jobs/a/run", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the running job is simply returned: {body}"
    );

    let stopped = idle.wind_down().stop_all(Duration::from_secs(5)).await;
    assert_eq!(stopped.len(), 1);
    assert_eq!(stopped[0].name, "Job a");
    let after = wait_job(backend, "a", |j| j.state == IdleState::Queued).await;
    assert_eq!(after.progress, 0.0);
    assert!(has_line(&log(backend, "a").await, "Stopped by game mode"));
    assert_eq!(idle.wind_down().counts().running, 0);
    assert_eq!(*idle.wind_down().running_watch().borrow(), 0);
    let (status, body) = backend.send("POST", "/idle-jobs/a/run", None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("gameMode"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_restart_rebuilds_state() {
    let text = format!(
        "{}{}{}",
        ticker("done", "gpu", false, 2, ""),
        ticker("cut", "cpu", false, 600, ""),
        ticker("cut-by-crash", "cpu", false, 600, ""),
    );
    let mut setup = start_with(&text).await;
    {
        let backend = &setup.backend;
        // One run that ends well, one that Rain stops, and one row that a crash left behind.
        backend.send("POST", "/idle-jobs/done/run", None).await;
        wait_job(backend, "done", |j| j.state == IdleState::Success).await;
        backend.send("POST", "/idle-jobs/cut/run", None).await;
        wait_job(backend, "cut", |j| j.state == IdleState::Running).await;
        backend
            .state
            .db
            .call(|c| {
                c.execute(
                    "INSERT INTO idle_run (id, job_id, started_at, status, manual)
                     VALUES ('irun-crash', 'cut-by-crash', '2026-11-21T09:00:00.000Z', 'running', 0)",
                    [],
                )
            })
            .await
            .unwrap();
        // `restart` is refused while a run goes on: stop, then start again.
        backend.request_stop("stop").await;
    }
    setup.backend.restart_following_machine_file().await;
    let backend = &setup.backend;

    let done = job(backend, "done").await;
    assert_eq!(
        done.state,
        IdleState::Success,
        "what finished well stays so"
    );
    assert_eq!(done.progress, 1.0);
    assert!(done.last_run_at.is_some());
    assert_eq!(
        job(backend, "cut").await.state,
        IdleState::Queued,
        "a stopped run starts again"
    );
    assert_eq!(job(backend, "cut").await.progress, 0.0);
    assert_eq!(job(backend, "cut-by-crash").await.state, IdleState::Queued);
    assert!(has_line(
        &log(backend, "cut").await,
        "Stopped because Rain stopped"
    ));
    assert!(has_line(
        &log(backend, "cut-by-crash").await,
        "Rain stopped while this run was in progress"
    ));
    // Nothing runs: no sampler (all disabled), no process.
    assert_eq!(backend.state.idle().wind_down().counts().running, 0);
    setup.idle.set_minutes(0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_newest_five_idle_runs_keep_their_log() {
    let setup = start_with(&ticker("a", "cpu", false, 1, "")).await;
    let backend = &setup.backend;
    let mut first_run_id = None;
    for n in 1..=6 {
        backend.send("POST", "/idle-jobs/a/run", None).await;
        let polling = async {
            loop {
                let ran: i64 = backend
                    .state
                    .db
                    .call(|c| {
                        c.query_row(
                            "SELECT COUNT(*) FROM idle_run WHERE status != 'running'",
                            [],
                            |r| r.get(0),
                        )
                    })
                    .await
                    .unwrap();
                if ran >= n.min(5) && job(backend, "a").await.state == IdleState::Success {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), polling)
            .await
            .expect("the run ended");
        if n == 1 {
            first_run_id = backend
                .state
                .db
                .call(|c| c.query_row("SELECT id FROM idle_run", [], |r| r.get::<_, String>(0)))
                .await
                .ok();
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let kept: i64 = backend
        .state
        .db
        .call(|c| c.query_row("SELECT COUNT(*) FROM idle_run", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(kept, 5);
    let first = first_run_id.expect("the first run id");
    let rows: i64 = backend
        .state
        .db
        .call(move |c| {
            c.query_row(
                "SELECT COUNT(*) FROM jobs_run_log WHERE run_id = ?1",
                [first],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "the log of the sixth-newest run was deleted with it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_idle_topic_carries_the_list_and_a_hand_edit_reaches_it() {
    let setup = start_with(&ticker("a", "cpu", false, 5, "")).await;
    let backend = &setup.backend;
    let mut stream = backend.stream().await;
    let (status, _) = backend
        .send("PATCH", "/idle-jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK);
    let frame = stream.expect_topic("idleJobs").await;
    assert_eq!(frame.payload[0]["enabled"], true);

    let edited = format!(
        "{}{}",
        ticker("a", "cpu", true, 5, ""),
        ticker("b", "gpu", false, 5, "")
    );
    std::fs::write(backend.data_dir.join("idle-jobs.toml"), edited).unwrap();
    loop {
        let frame = stream.expect_topic("idleJobs").await;
        if frame.payload.as_array().is_some_and(|list| list.len() == 2) {
            assert_eq!(frame.payload[1]["resource"], "gpu");
            break;
        }
    }
}
