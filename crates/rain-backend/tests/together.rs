//! The parts of the backend wired together. Each test here fails when one wire between two
//! modules is missing: search over every type, pins, a launch of a project and of a game through
//! the one engine, idle jobs and games, game mode with its real neighbours, sessions with projects
//! and jobs, the daily note through the one opener, and the settings that call other modules.
//!
//! The world is the typical data folder (jobs and idle jobs of made-up names) plus a throwaway
//! project folder, game folder and Claude folder. Git, programs, windows, processes, the process
//! table of Claude and the opener are fakes: no program of the user is started. The only real
//! programs are the tiny `ping` and `cmd` jobs that the typical fixture already holds.

#![allow(clippy::unwrap_used)] // a test panics with the failing line, which is its message

#[path = "common/claude.rs"]
mod claude;
mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use claude::{ClaudeHome, FakeForwarder, FakeProbe, NoExits, envelope, event};
use common::fakes::{FakeAppSource, FakeClock, FakeOpener, FakeProcesses, FakeWindows};
use common::{PATIENCE, StreamReader, TestBackend};
use futures_util::future::BoxFuture;
use hyper::StatusCode;
use rain_backend::core::launch_contract::{OpenError, OpenRequest, Opener, Started};
use rain_backend::modules::gamemode::{CloseTimings, Timings};
use rain_backend::modules::games::{ProcessWaiter, SteamRegistry};
use rain_backend::modules::idle::MachineIdle;
use rain_backend::modules::jobs::Tuning;
use rain_backend::modules::launcher::AppSource;
use rain_backend::modules::projects::{GitCall, GitError, GitOutput, GitRunner, ToolEnv};
use rain_backend::modules::sessions::{ExitSource, ForwarderProgram, ProcessProbe};
use rain_backend::platform::process::ProcessLister;
use rain_backend::platform::windows::WindowLister;
use rain_types::{
    ClaudeSession, Frame, GameLibrary, GameModePhase, GameModeState, IdleJob, IdleState, LaunchRun,
    LogLevel, LogLine, Project, RecapOutcome, RunStatus, ScheduledJob, SearchItem, SearchType,
    Status, WindDownKind,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

// ---- fakes ----

/// Git that knows nothing: every call answers with an empty output.
struct FakeGit;

impl GitRunner for FakeGit {
    fn run(&self, _call: GitCall) -> BoxFuture<'_, Result<GitOutput, GitError>> {
        Box::pin(async {
            Ok(GitOutput {
                exit_code: Some(0),
                stdout: Vec::new(),
                stderr: String::new(),
            })
        })
    }
}

/// The programs on the PC: an editor, a terminal and Claude, in a folder that does not exist.
struct FakeTools;

impl ToolEnv for FakeTools {
    fn path_dirs(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("C:\\fake")]
    }

    fn var(&self, _name: &str) -> Option<String> {
        None
    }

    fn exists(&self, path: &Path) -> bool {
        ["git.exe", "Code.exe", "wt.exe", "claude.exe"]
            .iter()
            .any(|name| path == Path::new("C:\\fake").join(name))
    }
}

struct NoSteam;

impl SteamRegistry for NoSteam {
    fn active_user(&self) -> Option<u32> {
        None
    }
    fn auto_login_user(&self) -> Option<String> {
        None
    }
    fn running_app_id(&self) -> Option<u32> {
        None
    }
}

/// "Processes" that end when the test says so.
#[derive(Default)]
struct FakeWaiter {
    waits: Mutex<HashMap<u32, oneshot::Sender<()>>>,
}

impl ProcessWaiter for FakeWaiter {
    fn wait_for(&self, pid: u32, _creation_time: u64) -> Option<BoxFuture<'static, ()>> {
        let (sender, receiver) = oneshot::channel();
        self.waits.lock().unwrap().insert(pid, sender);
        Some(Box::pin(async move {
            let _ = receiver.await;
        }))
    }
}

/// How long the PC has been idle, as the test says.
#[derive(Default)]
struct FakeIdle(AtomicU64);

impl MachineIdle for FakeIdle {
    fn idle_ms(&self) -> Option<u64> {
        Some(self.0.load(Ordering::SeqCst))
    }
}

/// An opener that answers the first calls like the fake and panics on the call with the number
/// `panic_on` (counting from 1): the one way to make a launch task die half way.
struct PanicOnCall {
    calls: AtomicU32,
    panic_on: u32,
}

impl Opener for PanicOnCall {
    fn open(&self, _request: OpenRequest) -> BoxFuture<'_, Result<Started, OpenError>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(call != self.panic_on, "the opener of this test panics");
        Box::pin(async move {
            Ok(Started {
                pid: Some(2000 + call),
                creation_time: Some(u64::from(call)),
                in_job: false,
            })
        })
    }
}

// ---- the world ----

const SESSION_A: &str = "aaaaaaaa-0000-0000-0000-000000000001";
const SESSION_B: &str = "bbbbbbbb-0000-0000-0000-000000000002";
const GAME: &str = "game-dir-fixture-quest";

/// What is made before the backend starts: the folders, the Claude registry and the fakes.
struct Prep {
    home: ClaudeHome,
    clock: Arc<FakeClock>,
    processes: Arc<FakeProcesses>,
    waiter: Arc<FakeWaiter>,
    probe: Arc<FakeProbe>,
    idle: Arc<FakeIdle>,
}

struct World {
    backend: TestBackend,
    home: ClaudeHome,
    opener: Arc<FakeOpener>,
    clock: Arc<FakeClock>,
    processes: Arc<FakeProcesses>,
    probe: Arc<FakeProbe>,
    idle: Arc<FakeIdle>,
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

fn fast_game_mode() -> Arc<Timings> {
    Arc::new(Timings {
        wait_limit: Duration::from_secs(5),
        review_abandon: Duration::from_secs(300),
        item_min_visible: Duration::from_millis(10),
        item_budget: Duration::from_secs(3),
        launch_budget: Duration::from_secs(3),
        closing: CloseTimings {
            polite_wait: Duration::from_millis(100),
            hidden_wait: Duration::from_millis(50),
            poll: Duration::from_millis(5),
        },
    })
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::File::create(path).unwrap().set_len(1024).unwrap();
}

impl Prep {
    /// The folders and the Claude registry, written before the backend starts.
    fn new() -> Self {
        let home = ClaudeHome::new();
        let temp = home.temp.path();
        // One project with a Git folder, one game folder with a program file.
        let alpha = temp.join("work").join("alpha");
        fs::create_dir_all(alpha.join(".git")).unwrap();
        fs::write(
            alpha.join(".git").join("HEAD"),
            "ref: refs/heads/main
",
        )
        .unwrap();
        touch(
            &temp
                .join("Games")
                .join("Fixture Quest")
                .join("Game")
                .join("FixtureQuest.exe"),
        );
        // Two Claude sessions: one inside the project folder, one elsewhere.
        let probe = Arc::new(FakeProbe::default());
        probe.add(4242, "claude.exe", 1);
        probe.add(4343, "claude.exe", 1);
        home.write_registry(
            4242,
            SESSION_A,
            json!({ "status": "busy", "cwd": alpha.join("src").to_string_lossy() }),
        );
        home.write_registry(4343, SESSION_B, json!({ "cwd": r"C:\elsewhere\other" }));
        Self {
            home,
            clock: Arc::new(FakeClock::new(SystemTime::now())),
            processes: Arc::new(FakeProcesses::default()),
            waiter: Arc::new(FakeWaiter::default()),
            probe,
            idle: Arc::new(FakeIdle::default()),
        }
    }

    async fn boot(
        self,
        opener: Arc<dyn Opener>,
        fake: Arc<FakeOpener>,
        timings: Option<Arc<Timings>>,
    ) -> World {
        let temp = self.home.temp.path().to_path_buf();
        let settings = format!(
            "schema_version = 1
             project_roots = ['{work}']
             game_folders = ['{games}']
             steam_path = '{steam}'
             epic_path = '{epic}'
             game_mode_countdown_seconds = 5
             monitor_claude = true
",
            work = temp.join("work").display(),
            games = temp.join("Games").display(),
            steam = temp.join("Steam").display(),
            epic = temp.join("Epic").display(),
        );
        let root = self.home.root();
        let mut builder = TestBackend::builder()
            .fixture("typical")
            .file("settings.toml", &settings)
            .config(move |config| config.claude_dir = Some(root.clone()))
            .clock(self.clock.clone())
            .opener(opener)
            .with::<dyn GitRunner>(Arc::new(FakeGit))
            .with::<dyn ToolEnv>(Arc::new(FakeTools))
            .with::<dyn WindowLister>(Arc::new(FakeWindows::default()))
            .with::<dyn ProcessLister>(self.processes.clone())
            .with::<dyn SteamRegistry>(Arc::new(NoSteam))
            .with::<dyn ProcessWaiter>(self.waiter.clone())
            .with::<dyn ProcessProbe>(self.probe.clone())
            .with::<dyn ExitSource>(Arc::new(NoExits))
            .with::<dyn MachineIdle>(self.idle.clone())
            .with::<dyn AppSource>(Arc::new(FakeAppSource::default()))
            .with::<Tuning>(tuning());
        if let Some(timings) = timings {
            builder = builder.with::<Timings>(timings);
        }
        World {
            backend: builder.start().await,
            home: self.home,
            opener: fake,
            clock: self.clock,
            processes: self.processes,
            probe: self.probe,
            idle: self.idle,
        }
    }
}

impl World {
    async fn start() -> Self {
        let fake = Arc::new(FakeOpener::default());
        Prep::new().boot(fake.clone(), fake, None).await
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> T {
        self.backend.get(path).await
    }

    async fn project_id(&self) -> String {
        let projects: Vec<Project> = self.get("/projects").await;
        projects
            .iter()
            .find(|p| p.name == "alpha")
            .expect("the fixture project")
            .id
            .clone()
    }

    async fn search(&self, query: &str, types: Option<&str>) -> Vec<SearchItem> {
        let types = types.map_or(String::new(), |t| format!("&types={t}"));
        self.get(&format!("/search?q={query}{types}")).await
    }

    async fn launch(&self, body: Value) -> (StatusCode, Value) {
        self.backend.send("POST", "/launches", Some(body)).await
    }

    async fn pin(&self, kind: &str, id: &str, pinned: bool) {
        let (status, body) = self
            .backend
            .send(
                "PUT",
                &format!("/pinned/{kind}/{id}"),
                Some(json!({ "pinned": pinned })),
            )
            .await;
        assert!(status.is_success(), "pin {kind} {id}: {status} {body}");
    }

    fn pins_file(&self) -> String {
        fs::read_to_string(self.backend.data_dir.join("pins.toml")).unwrap_or_default()
    }
}

/// Reads `launch` frames until the run with this id has ended. Returns every frame of the run.
async fn frames_until_end(stream: &mut StreamReader, id: &str) -> Vec<LaunchRun> {
    let mut frames = Vec::new();
    loop {
        let frame = stream.expect_topic("launch").await;
        let run: LaunchRun = serde_json::from_value(frame.payload).expect("a LaunchRun frame");
        if run.id != id {
            continue;
        }
        let ended = run.status != RunStatus::Running;
        frames.push(run);
        if ended {
            return frames;
        }
    }
}

async fn wait_for_end(backend: &TestBackend, id: &str) -> LaunchRun {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let run: LaunchRun = backend.get(&format!("/launches/{id}")).await;
        if run.status != RunStatus::Running {
            return run;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the run did not end"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn spawn_of(request: &OpenRequest) -> (&Path, &str) {
    match request {
        OpenRequest::Spawn { exe, args_raw, .. } => (exe, args_raw),
        other => panic!("not a direct start: {other:?}"),
    }
}

async fn games(world: &World) -> GameLibrary {
    world.get("/games").await
}

async fn game_launches(world: &World) -> i64 {
    world
        .backend
        .state
        .db
        .call(|c| {
            c.query_row(
                "SELECT COALESCE(SUM(launches), 0) FROM games_play",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap()
}

// ---- 1. search, pinned and recent for every type ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_finds_a_project_a_game_a_job_an_idle_job_and_a_session() {
    let w = World::start().await;
    let project = w.project_id().await;

    let found = w.search("alpha", Some("project")).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].r#type, found[0].id.as_str()),
        (SearchType::Project, project.as_str())
    );

    let found = w.search("quest", Some("game")).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].r#type, found[0].id.as_str()),
        (SearchType::Game, GAME)
    );

    // A job: the title is its name, the subtitle its schedule, the status "idle" before a run.
    let found = w.search("hello", Some("job")).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].r#type, found[0].id.as_str()),
        (SearchType::Job, "job-echo")
    );
    assert_eq!(found[0].title, "Say hello");
    assert_eq!(found[0].status, Some(Status::Idle));
    assert!(found[0].subtitle.is_some());

    // An idle job: the subtitle is its state in words.
    let found = w.search("burn", Some("idleJob")).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].r#type, found[0].id.as_str()),
        (SearchType::IdleJob, "idle-cpu-burner")
    );
    assert_eq!(found[0].subtitle.as_deref(), Some("Queued"));
    assert_eq!(found[0].status, Some(Status::Idle));

    // A session: the title is the folder it works in, the status is the session's own.
    let found = w.search("src", Some("session")).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].r#type, found[0].id.as_str()),
        (SearchType::Session, SESSION_A)
    );
    assert_eq!(found[0].title, "src");
    assert_eq!(found[0].status, Some(Status::Running));
    // ... and it is found by the folder of its project too (a keyword), but only when asked for.
    assert!(w.search("src", Some("job")).await.is_empty());

    // Without a filter every type takes part.
    let mut types = HashSet::new();
    for query in ["a", "s"] {
        types.extend(w.search(query, None).await.iter().map(|item| item.r#type));
    }
    for wanted in [
        SearchType::Project,
        SearchType::Game,
        SearchType::Job,
        SearchType::IdleJob,
        SearchType::Session,
    ] {
        assert!(
            types.contains(&wanted),
            "{wanted:?} is missing from {types:?}"
        );
    }
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_id_that_search_pinned_and_recent_return_exists_in_its_own_list() {
    let w = World::start().await;
    let project = w.project_id().await;
    // Something pinned and something recent, of the two types that the launcher does not own.
    w.pin("project", &project, true).await;
    w.pin("game", GAME, true).await;
    let mut stream = w.backend.stream().await;
    for target in [
        json!({ "type": "project", "id": project }),
        json!({ "type": "game", "id": GAME }),
    ] {
        let (status, run) = w.launch(target).await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        frames_until_end(&mut stream, run["id"].as_str().unwrap()).await;
    }

    let projects: Vec<Project> = w.get("/projects").await;
    let library = games(&w).await;
    let jobs: Vec<ScheduledJob> = w.get("/jobs").await;
    let idle: Vec<IdleJob> = w.get("/idle-jobs").await;
    let sessions: Vec<ClaudeSession> = w.get("/sessions").await;
    let known = |item: &SearchItem| match item.r#type {
        SearchType::Project => projects.iter().any(|p| p.id == item.id),
        SearchType::Game => library.games.iter().any(|g| g.id == item.id),
        SearchType::Job => jobs.iter().any(|j| j.id == item.id),
        SearchType::IdleJob => idle.iter().any(|j| j.id == item.id),
        SearchType::Session => sessions.iter().any(|s| s.id == item.id),
        // The app list of this world is empty and the pools name no real app.
        SearchType::App | SearchType::Pool => true,
    };

    let mut seen_types = HashSet::new();
    for query in [
        "a", "e", "o", "i", "s", "r", "t", "n", "al", "qu", "ho", "bu", "sr", "ba",
    ] {
        let found = w.search(query, None).await;
        assert!(found.iter().all(known), "search {query}: {found:?}");
        let mut ids = HashSet::new();
        assert!(
            found
                .iter()
                .all(|item| ids.insert((item.r#type, item.id.clone()))),
            "an item twice in {query}"
        );
        seen_types.extend(found.iter().map(|item| item.r#type));
    }
    assert!(seen_types.contains(&SearchType::Session) && seen_types.contains(&SearchType::Job));

    let pinned: Vec<SearchItem> = w.get("/pinned").await;
    let recent: Vec<SearchItem> = w.get("/recent").await;
    let pinned_ids: HashSet<&str> = pinned.iter().map(|i| i.id.as_str()).collect();
    assert!(
        pinned_ids.contains(project.as_str()) && pinned_ids.contains(GAME),
        "{pinned:?}"
    );
    let recent_ids: HashSet<&str> = recent.iter().map(|i| i.id.as_str()).collect();
    assert!(
        recent_ids.contains(project.as_str()) && recent_ids.contains(GAME),
        "{recent:?}"
    );
    assert!(pinned.iter().chain(&recent).all(known));
    w.backend.stop().await;
}

// ---- 2. pins ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pins_show_on_projects_and_games_and_hiding_a_game_removes_its_pin() {
    let w = World::start().await;
    let project = w.project_id().await;
    assert!(!w.get::<Vec<Project>>("/projects").await[0].pinned);

    w.pin("project", &project, true).await;
    w.pin("game", GAME, true).await;
    let projects: Vec<Project> = w.get("/projects").await;
    assert!(projects.iter().find(|p| p.id == project).unwrap().pinned);
    let library = games(&w).await;
    assert!(library.games.iter().find(|g| g.id == GAME).unwrap().pinned);
    // The typical fixture pins a pool of its own as well.
    let pinned: Vec<SearchItem> = w.get("/pinned").await;
    let of = |pinned: &[SearchItem], kind: SearchType| {
        pinned.iter().filter(|i| i.r#type == kind).count()
    };
    assert_eq!(of(&pinned, SearchType::Project), 1, "{pinned:?}");
    assert_eq!(of(&pinned, SearchType::Game), 1, "{pinned:?}");
    assert!(w.pins_file().contains(GAME));

    // Hiding the game takes the pin away: from the file, from the library and from the list.
    let (status, body) = w
        .backend
        .send(
            "PATCH",
            &format!("/games/{GAME}"),
            Some(json!({ "hidden": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!w.pins_file().contains(GAME), "{}", w.pins_file());
    let library = games(&w).await;
    let hidden = library.games.iter().find(|g| g.id == GAME).unwrap();
    assert!(hidden.hidden && !hidden.pinned);
    let pinned: Vec<SearchItem> = w.get("/pinned").await;
    assert_eq!(of(&pinned, SearchType::Game), 0, "{pinned:?}");
    assert_eq!(of(&pinned, SearchType::Project), 1, "{pinned:?}");
    // The project's pin is untouched.
    assert!(w.pins_file().contains(&project));

    // Showing it again does not bring the pin back.
    let (status, _) = w
        .backend
        .send(
            "PATCH",
            &format!("/games/{GAME}"),
            Some(json!({ "hidden": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !games(&w)
            .await
            .games
            .iter()
            .find(|g| g.id == GAME)
            .unwrap()
            .pinned
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_game_that_was_added_by_hand_removes_its_pin() {
    let w = World::start().await;
    let exe = w.home.temp.path().join("Hand").join("Hand.exe");
    touch(&exe);
    let (status, body) = w
        .backend
        .send(
            "POST",
            "/games",
            Some(json!({ "title": "Hand Made", "exePath": exe.to_string_lossy() })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();
    w.pin("game", &id, true).await;
    assert!(w.pins_file().contains(&id));

    let (status, _) = w
        .backend
        .send("DELETE", &format!("/games/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!w.pins_file().contains(&id), "{}", w.pins_file());
    w.backend.stop().await;
}

// ---- 3. launching a project and a game ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launching_a_project_runs_the_editor_then_one_terminal_tab_and_sets_last_opened_before_the_end()
 {
    let w = World::start().await;
    let project = w.project_id().await;
    let mut stream = w.backend.stream().await;
    assert!(
        w.get::<Vec<Project>>("/projects").await[0]
            .last_opened_at
            .is_none()
    );

    // A slow opener keeps the run alive long enough for a second request to find it.
    w.opener.delay_calls(Duration::from_millis(300));
    let (status, body) = w.launch(json!({ "type": "project", "id": project })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let first: LaunchRun = serde_json::from_value(body).unwrap();
    assert_eq!(first.status, RunStatus::Running);
    let (status, again) = w.launch(json!({ "type": "project", "id": project })).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a second request while it runs gets the same run"
    );
    assert_eq!(again["id"], first.id.as_str());

    let frames = frames_until_end(&mut stream, &first.id).await;
    // The frames come in order: a step never goes back, and only the last frame is final.
    let rank = |status: Status| match status {
        Status::Idle | Status::Waiting => 0,
        Status::Running => 1,
        Status::Success | Status::Failed => 2,
    };
    for pair in frames.windows(2) {
        for (before, after) in pair[0].steps.iter().zip(&pair[1].steps) {
            assert!(rank(before.status) <= rank(after.status), "{pair:?}");
        }
    }
    assert!(
        frames[..frames.len() - 1]
            .iter()
            .all(|f| f.status == RunStatus::Running)
    );
    let last = frames.last().unwrap();
    assert_eq!(last.status, RunStatus::Success);
    assert_eq!(
        last.steps
            .iter()
            .map(|s| s.label.as_str())
            .collect::<Vec<_>>(),
        ["VS Code", "Claude Code"]
    );
    assert!(last.steps.iter().all(|s| s.status == Status::Success));

    // At the final frame the history is already written: the screen refetches and sees it.
    let projects: Vec<Project> = w.get("/projects").await;
    assert!(
        projects
            .iter()
            .find(|p| p.id == project)
            .unwrap()
            .last_opened_at
            .is_some()
    );

    // The plan: the editor, then ONE terminal tab that starts Claude Code.
    let requests = w.opener.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    let (editor, _) = spawn_of(&requests[0]);
    assert_eq!(editor, Path::new("C:\\fake\\Code.exe"));
    let (terminal, line) = spawn_of(&requests[1]);
    assert_eq!(terminal, Path::new("C:\\fake\\wt.exe"));
    assert_eq!(line.matches("new-tab").count(), 1, "{line}");
    assert!(line.contains("claude.exe"), "{line}");

    // Ctrl+Enter: the editor only.
    w.opener.delay_calls(Duration::ZERO);
    let (status, body) = w
        .launch(json!({ "type": "project", "id": project, "mode": "editorOnly" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let run = wait_for_end(&w.backend, body["id"].as_str().unwrap()).await;
    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.steps[0].label, "VS Code");
    let requests = w.opener.requests();
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert_eq!(spawn_of(&requests[2]).0, Path::new("C:\\fake\\Code.exe"));
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launching_a_game_records_the_launch_and_starts_the_play_time() {
    let w = World::start().await;
    let mut stream = w.backend.stream().await;

    let (status, body) = w.launch(json!({ "type": "game", "id": GAME })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_owned();
    let frames = frames_until_end(&mut stream, &id).await;
    assert_eq!(frames.last().unwrap().status, RunStatus::Success);

    // The program is started directly, in its own folder.
    let requests = w.opener.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let OpenRequest::Spawn { exe, cwd, .. } = &requests[0] else {
        panic!("{:?}", requests[0]);
    };
    assert!(exe.ends_with("FixtureQuest.exe"));
    assert!(cwd.ends_with("Game"));

    // The launch is recorded before the final frame, and the play time is being counted.
    let quest = games(&w)
        .await
        .games
        .into_iter()
        .find(|g| g.id == GAME)
        .unwrap();
    assert!(
        quest.last_played_at.is_some(),
        "last played is set at the final frame"
    );
    assert!(quest.running, "the started program is watched");
    assert_eq!(game_launches(&w).await, 1);
    let sessions: i64 = w
        .backend
        .state
        .db
        .call(|c| c.query_row("SELECT count(*) FROM games_session", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(sessions, 1, "the play time has started");
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_launch_that_finds_the_game_already_watched_counts_no_launch() {
    let w = World::start().await;
    let mut stream = w.backend.stream().await;
    let (_, first) = w.launch(json!({ "type": "game", "id": GAME })).await;
    frames_until_end(&mut stream, first["id"].as_str().unwrap()).await;
    assert_eq!(game_launches(&w).await, 1);
    let played = games(&w)
        .await
        .games
        .into_iter()
        .find(|g| g.id == GAME)
        .unwrap()
        .last_played_at;

    // The game still runs (the waiter has not ended it). Another launch starts nothing ...
    w.clock.advance(Duration::from_secs(60));
    let (status, second) = w.launch(json!({ "type": "game", "id": GAME })).await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let run = wait_for_end(&w.backend, second["id"].as_str().unwrap()).await;
    assert_eq!(run.status, RunStatus::Success);
    assert_eq!(w.opener.requests().len(), 1, "no second start");
    // ... and does not count as one more launch, nor move "last played".
    assert_eq!(game_launches(&w).await, 1);
    let after = games(&w)
        .await
        .games
        .into_iter()
        .find(|g| g.id == GAME)
        .unwrap()
        .last_played_at;
    assert_eq!(after, played);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_launch_task_that_panics_still_tells_the_planner_what_was_opened() {
    // The editor starts, then the opener panics at the terminal step.
    let panicking = Arc::new(PanicOnCall {
        calls: AtomicU32::new(0),
        panic_on: 2,
    });
    let backend = Prep::new()
        .boot(panicking, Arc::new(FakeOpener::default()), None)
        .await;
    let project = backend.project_id().await;
    let (status, body) = backend
        .launch(json!({ "type": "project", "id": project }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let run = wait_for_end(&backend.backend, body["id"].as_str().unwrap()).await;
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(
        run.steps[0].status,
        Status::Success,
        "the editor was opened"
    );
    assert_eq!(run.steps[1].status, Status::Failed);
    // The project history is written by the planner's `finished` hook, which the recovery path
    // still has to call.
    let projects: Vec<Project> = backend.get("/projects").await;
    assert!(
        projects
            .iter()
            .find(|p| p.id == project)
            .unwrap()
            .last_opened_at
            .is_some(),
        "the planner was told about the run that panicked"
    );
    backend.backend.stop().await;
}

// ---- 4. the opener says whether the program is inside a job ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_handed_over_after_game_mode_reports_the_job_state_of_the_opener() {
    let w = World::start().await;
    let games_service = w.backend.state.games();
    // The first call has to build the library; the fake opener says "free".
    let free = games_service.launch_after_game_mode(GAME).await.unwrap();
    assert!(!free.in_job);
    w.opener.set_in_job(true);
    let inside = games_service.launch_after_game_mode(GAME).await.unwrap();
    assert!(inside.in_job, "the opener said the program is inside a job");
    assert_eq!(w.opener.requests().len(), 2);
    w.backend.stop().await;
}

// ---- 5. idle jobs and games ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_game_holds_an_idle_job_back() {
    let w = World::start().await;
    let image = w
        .home
        .temp
        .path()
        .join("Games")
        .join("Fixture Quest")
        .join("Game")
        .join("FixtureQuest.exe");
    w.processes
        .0
        .lock()
        .unwrap()
        .push(image.to_string_lossy().into_owned());
    // A refresh makes the games module look at the process list at once.
    let (status, _) = w.backend.send("POST", "/games/refresh", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(w.backend.state.games().any_game_running().await);

    // The PC has been quiet for a day and a half: the job is due, but a game runs.
    w.idle.0.store(1500 * 60_000, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let job = |jobs: Vec<IdleJob>| {
        jobs.into_iter()
            .find(|j| j.id == "idle-cpu-burner")
            .unwrap()
    };
    assert_eq!(job(w.get("/idle-jobs").await).state, IdleState::Queued);

    // The game ends: the same job starts, so it was the game that held it back.
    w.processes.0.lock().unwrap().clear();
    let (status, _) = w.backend.send("POST", "/games/refresh", None).await;
    assert_eq!(status, StatusCode::OK);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while job(w.get("/idle-jobs").await).state != IdleState::Running {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the idle job never started"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    w.backend.stop().await;
}

// ---- 6 and 7. game mode with its real neighbours, and the end of the stream ----

fn game_mode_frames(events: &[(String, Frame)]) -> Vec<GameModeState> {
    events
        .iter()
        .filter(|(_, frame)| frame.topic == "gameMode")
        .map(|(_, frame)| serde_json::from_value(frame.payload.clone()).expect("a state"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_mode_stops_a_real_job_lists_the_sessions_and_ends_with_done() {
    let opener = Arc::new(FakeOpener::default());
    opener.set_in_job(true);
    let mut w = Prep::new()
        .boot(opener.clone(), opener.clone(), Some(fast_game_mode()))
        .await;

    // A job runs (a short-lived `ping`, which game mode has to stop).
    let (status, run) = w
        .backend
        .send("POST", "/jobs/job-demo-wait/runs", None)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let run_id = run["id"].as_str().unwrap().to_owned();

    let mut stream = w.backend.stream().await;
    let (status, started) = w
        .backend
        .send(
            "POST",
            "/game-mode",
            Some(json!({ "thenLaunchGameId": GAME })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let state: GameModeState = serde_json::from_value(started).unwrap();
    assert_eq!(
        state.phase,
        GameModePhase::Review,
        "a running job asks for a choice"
    );
    // The two open Claude sessions are listed, not stopped.
    let sessions = state
        .items
        .iter()
        .find(|i| i.kind == WindDownKind::Session)
        .expect("a sessions row");
    assert_eq!(sessions.count, 2);
    assert_eq!(sessions.effect, rain_types::WindDownEffect::LeftRunning);

    for choice in ["stopAnyway", "closeNow"] {
        let (status, body) = w
            .backend
            .send(
                "POST",
                "/game-mode/choice",
                Some(json!({ "choice": choice })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let events = stream.rest().await;

    // `done` is the last topic frame, and only the lifecycle frame follows it. Nothing waited for
    // a pause: the stream wrote what was published before it ended.
    let states = game_mode_frames(&events);
    assert_eq!(states.last().unwrap().phase, GameModePhase::Done);
    let last_topic = events
        .iter()
        .rposition(|(_, f)| f.topic == "gameMode")
        .unwrap();
    let after: Vec<_> = events[last_topic + 1..]
        .iter()
        .map(|(_, f)| f.topic.as_str())
        .collect();
    assert_eq!(after, ["lifecycle"]);

    // The game was started through the one opener.
    let started_game = opener.requests();
    assert_eq!(started_game.len(), 1, "{started_game:?}");
    assert!(spawn_of(&started_game[0]).0.ends_with("FixtureQuest.exe"));
    // The sessions were not touched.
    assert!(w.probe.terminated.lock().unwrap().is_empty());

    // After the restart: the run was stopped on purpose (idle, not failed), the sessions still
    // live, and the recap says that the game may close with Rain.
    w.backend.restart_following_machine_file().await;
    let jobs: Vec<ScheduledJob> = w.get("/jobs").await;
    let ended = jobs
        .iter()
        .flat_map(|j| &j.recent_runs)
        .find(|r| r.id == run_id)
        .expect("the run is in the history");
    assert_eq!(ended.status, Status::Idle, "stopped on purpose: {ended:?}");
    let sessions: Vec<ClaudeSession> = w.get("/sessions").await;
    assert!(
        sessions.iter().filter(|s| s.alive).count() == 2,
        "{sessions:?}"
    );
    let off: GameModeState = w.get("/game-mode").await;
    let recap = off.last.expect("a recap");
    assert!(recap.complete);
    assert_eq!(recap.launched_title.as_deref(), Some("Fixture Quest"));
    assert_eq!(recap.launched_in_job, Some(true));
    let jobs_line = recap
        .items
        .iter()
        .find(|i| i.kind == WindDownKind::Job)
        .expect("a jobs line");
    assert_eq!(jobs_line.outcome, RecapOutcome::Stopped);
    assert_eq!(jobs_line.names, ["Demo wait"]);
    w.backend.stop().await;
}

// ---- 8. sessions with projects and jobs ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_in_a_project_folder_has_the_project_id() {
    let w = World::start().await;
    let project = w.project_id().await;
    let sessions: Vec<ClaudeSession> = w.get("/sessions").await;
    let inside = sessions.iter().find(|s| s.id == SESSION_A).unwrap();
    assert_eq!(inside.project_id.as_deref(), Some(project.as_str()));
    let outside = sessions.iter().find(|s| s.id == SESSION_B).unwrap();
    assert_eq!(outside.project_id, None);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_session_start_of_a_job_run_is_stored_with_the_run() {
    let w = World::start().await;
    let (status, run) = w
        .backend
        .send("POST", "/jobs/job-demo-wait/runs", None)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let run_id = run["id"].as_str().unwrap().to_owned();

    let stored = |id: String| {
        let db = w.backend.state.db.clone();
        async move {
            db.call(move |c| {
                c.query_row(
                    "SELECT claude_session_id FROM jobs_run WHERE id = ?1",
                    [id],
                    |r| r.get::<_, Option<String>>(0),
                )
            })
            .await
            .unwrap()
        }
    };
    assert_eq!(stored(run_id.clone()).await, None);

    // The hook of the run's own Claude program reports its session. Other events do not count.
    let env = json!({ "entrypoint": "cli", "runId": run_id });
    let post = |name: &str, session: &str| {
        let body = envelope("event", event(name, session, json!({})), env.clone());
        let backend = &w.backend;
        let name = name.to_owned();
        async move {
            let (status, body) = backend.send("POST", "/internal/hook", Some(body)).await;
            assert_eq!(status, StatusCode::NO_CONTENT, "{name}: {body}");
        }
    };
    post("UserPromptSubmit", "wrong-session").await;
    assert_eq!(stored(run_id.clone()).await, None);
    post("SessionStart", "claude-session-of-the-run").await;
    assert_eq!(
        stored(run_id.clone()).await.as_deref(),
        Some("claude-session-of-the-run")
    );
    // A run that nobody made changes nothing and is not an error.
    let body = envelope(
        "event",
        event("SessionStart", "other", json!({})),
        json!({ "runId": "run-19700101-000000-0000" }),
    );
    let (status, _) = w.backend.send("POST", "/internal/hook", Some(body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // A run's session is never shown in the list of sessions.
    let sessions: Vec<ClaudeSession> = w.get("/sessions").await;
    assert!(sessions.iter().all(|s| s.id != "claude-session-of-the-run"));
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_transcript_is_a_list_of_log_lines_like_the_log_of_a_job() {
    let prep = Prep::new();
    prep.home.write_transcript(
        &prep.home.temp.path().join("work").join("alpha").join("src").to_string_lossy(),
        SESSION_A,
        &[
            json!({ "type": "user", "timestamp": "2026-11-21T09:00:00.000Z", "message": { "content": "Hello there" } }),
            json!({ "type": "system", "subtype": "compact_boundary", "timestamp": "2026-11-21T09:00:04.000Z" }),
        ],
    );
    let fake = Arc::new(FakeOpener::default());
    let w = prep.boot(fake.clone(), fake, None).await;
    let lines: Vec<LogLine> = w.get(&format!("/sessions/{SESSION_A}/transcript")).await;
    assert_eq!(lines[0].level, LogLevel::Info);
    assert_eq!(lines[0].text, "You: Hello there");
    assert_eq!(lines[1].level, LogLevel::Warn);
    assert_eq!(lines[1].ts, "2026-11-21T09:00:04.000Z");
    w.backend.stop().await;
}

// ---- 9. the daily note through the one opener ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_daily_note_and_a_launch_go_through_the_same_opener() {
    let opener = Arc::new(FakeOpener::default());
    let detect = Arc::new(common::fakes::FakeDetect::default());
    let backend = TestBackend::builder()
        .fixture("life-basic")
        .clock(Arc::new(FakeClock::new(
            std::time::UNIX_EPOCH + Duration::from_secs(1_795_348_800),
        )))
        .opener(opener.clone())
        .with::<dyn AppSource>(Arc::new(FakeAppSource::from_fixture()))
        .with::<dyn rain_backend::modules::settings::Detect>(detect.clone())
        .start()
        .await;
    let vault = backend.data_dir.join("vault");
    *detect.vaults.lock().unwrap() = vec![common::fakes::FakeDetect::vault(
        "abc123",
        &vault.to_string_lossy(),
    )];
    for path in [
        "Z:\\elsewhere".to_owned(),
        vault.to_string_lossy().into_owned(),
    ] {
        let (status, _) = backend
            .send("PATCH", "/settings", Some(json!({ "vaultPath": path })))
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, body) = backend
        .send(
            "POST",
            "/launches",
            Some(json!({ "type": "app", "id": "app-fake-editor" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    wait_for_end(&backend, body["id"].as_str().unwrap()).await;
    let (status, body) = backend
        .send("POST", "/daily-notes/2026-11-22/open", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["opened"], true);

    let requests = opener.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(
        matches!(&requests[0], OpenRequest::ShellExec { file, .. } if file.starts_with("shell:AppsFolder"))
    );
    assert_eq!(
        requests[1],
        OpenRequest::ShellExec {
            file: "obsidian://open?vault=abc123&file=Daily%2F2026-11-22".into(),
            params: None,
            dir: None
        }
    );
    backend.stop().await;
}

// ---- 10. settings that call other modules ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_call_the_sessions_installer_and_life_for_the_note_format() {
    let home = ClaudeHome::without_registry();
    let forwarder = FakeForwarder::new(home.temp.path(), b"made-up bytes of the hook program");
    let root = home.root();
    let backend = TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .with::<dyn ForwarderProgram>(forwarder)
        .start()
        .await;
    assert!(!home.settings_file().exists());

    // The switch writes Rain's hooks into the throwaway Claude settings, through the installer.
    let (status, answer) = backend
        .send("PATCH", "/settings", Some(json!({ "monitorClaude": true })))
        .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let installed = fs::read_to_string(home.settings_file()).expect("the installer wrote the file");
    assert_eq!(installed.matches("--rain-managed=").count(), 12);
    let (status, _) = backend
        .send(
            "PATCH",
            "/settings",
            Some(json!({ "monitorClaude": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        fs::read_to_string(home.settings_file())
            .unwrap()
            .matches("--rain-managed=")
            .count(),
        0
    );

    // The note format is checked by the life module: a format with a time is refused, a plain
    // one is stored.
    let (status, body) = backend
        .send(
            "PATCH",
            "/settings",
            Some(json!({ "dailyNoteFormat": "YYYY-MM-DD HH:mm" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "badFormat");
    let (status, body) = backend
        .send(
            "PATCH",
            "/settings",
            Some(json!({ "dailyNoteFormat": "YYYY-MM-DD" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["dailyNoteFormat"], "YYYY-MM-DD");
    backend.stop().await;
}
