//! The projects module on a real backend (feature section 02, tests of section 10.4).
//!
//! Git, the programs on the PC, the editor windows and the clock are fakes, and nothing is
//! started: the launch engine belongs to the launcher, so a launch is tested through the plan
//! that this module gives it (`LaunchPlanner`) and through `finished`.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use common::fakes::{FakeClock, FakeProcesses, FakeWindows};
use common::{StreamReader, TestBackend, TestBackendBuilder};
use futures_util::future::BoxFuture;
use hyper::StatusCode;
use rain_backend::core::launch_contract::{ItemSource, LaunchPlanner, PlannedStep, StepAction};
use rain_backend::modules::projects::{GitCall, GitError, GitOutput, GitRunner, ToolEnv};
use rain_backend::platform::process::ProcessLister;
use rain_backend::platform::windows::{WindowInfo, WindowLister};
use rain_types::{
    Health, LaunchMode, LaunchRun, LaunchStep, LaunchTarget, LaunchType, Project, RunStatus,
    SourceState, Status,
};

// ---- fakes ----

type Answer = Box<dyn Fn(&GitCall) -> Result<GitOutput, GitError> + Send + Sync>;

/// Git as a table: an answer for every call, a record of the calls, an optional delay.
struct FakeGit {
    answer: Mutex<Answer>,
    calls: Mutex<Vec<(String, &'static str)>>,
    delay: Mutex<Duration>,
    in_progress: AtomicUsize,
    most_at_once: AtomicUsize,
}

fn out(stdout: &str) -> Result<GitOutput, GitError> {
    Ok(GitOutput {
        exit_code: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: String::new(),
    })
}

impl FakeGit {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Box::new(|_| out(""))),
            calls: Mutex::new(Vec::new()),
            delay: Mutex::new(Duration::ZERO),
            in_progress: AtomicUsize::new(0),
            most_at_once: AtomicUsize::new(0),
        })
    }

    fn answer_with(
        &self,
        answer: impl Fn(&GitCall) -> Result<GitOutput, GitError> + Send + Sync + 'static,
    ) {
        *self.answer.lock().unwrap_or_else(PoisonError::into_inner) = Box::new(answer);
    }

    fn calls(&self) -> Vec<(String, &'static str)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// How many calls were made for the repository whose folder name is `name`.
    fn calls_for(&self, name: &str) -> usize {
        self.calls()
            .iter()
            .filter(|(repo, _)| repo.ends_with(&format!("\\{name}")))
            .count()
    }
}

impl GitRunner for FakeGit {
    fn run(&self, call: GitCall) -> BoxFuture<'_, Result<GitOutput, GitError>> {
        let sub = if call.args.iter().any(|a| a == "status") {
            "status"
        } else {
            "log"
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((call.repo.to_string_lossy().into_owned(), sub));
        let delay = *self.delay.lock().unwrap_or_else(PoisonError::into_inner);
        let answer = (self.answer.lock().unwrap_or_else(PoisonError::into_inner))(&call);
        Box::pin(async move {
            let now = self.in_progress.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_at_once.fetch_max(now, Ordering::SeqCst);
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            self.in_progress.fetch_sub(1, Ordering::SeqCst);
            answer
        })
    }
}

/// The programs on a PC that is a list of files.
struct FakeTools {
    files: Mutex<HashSet<PathBuf>>,
}

impl FakeTools {
    fn new(names: &[&str]) -> Arc<Self> {
        let files = names
            .iter()
            .map(|name| PathBuf::from("C:\\fake").join(name))
            .collect();
        Arc::new(Self {
            files: Mutex::new(files),
        })
    }

    fn everything() -> Arc<Self> {
        Self::new(&["git.exe", "Code.exe", "wt.exe", "claude.exe"])
    }
}

impl ToolEnv for FakeTools {
    fn path_dirs(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("C:\\fake")]
    }

    fn var(&self, _name: &str) -> Option<String> {
        None
    }

    fn exists(&self, path: &Path) -> bool {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(path)
    }
}

// ---- the world of a test ----

const ROOTS: &str = "project_roots = ['{TEMP}\\work\\a', '{TEMP}\\work\\b']\n";

struct World {
    backend: TestBackend,
    git: Arc<FakeGit>,
    windows: Arc<FakeWindows>,
    processes: Arc<FakeProcesses>,
    clock: Arc<FakeClock>,
    work: PathBuf,
}

struct Setup {
    settings: String,
    projects_toml: Option<String>,
    tools: Arc<FakeTools>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            settings: ROOTS.to_owned(),
            projects_toml: None,
            tools: FakeTools::everything(),
        }
    }
}

impl World {
    async fn start(setup: Setup) -> Self {
        let git = FakeGit::new();
        let windows = Arc::new(FakeWindows::default());
        let processes = Arc::new(FakeProcesses::default());
        let clock = Arc::new(FakeClock::new(SystemTime::now()));
        let mut builder: TestBackendBuilder = TestBackend::builder()
            .file("settings.toml", &setup.settings)
            .with::<dyn GitRunner>(git.clone())
            .with::<dyn ToolEnv>(setup.tools)
            .with::<dyn WindowLister>(windows.clone())
            .with::<dyn ProcessLister>(processes.clone())
            .clock(clock.clone());
        if let Some(text) = &setup.projects_toml {
            builder = builder.file("projects.toml", text);
        }
        let backend = builder.start().await;
        let work = backend
            .data_dir
            .parent()
            .expect("the data folder has a parent")
            .join("work");
        Self {
            backend,
            git,
            windows,
            processes,
            clock,
            work,
        }
    }

    /// A repository `name` in the root `root` (`a` or `b`), with this text in `HEAD`.
    fn repo(&self, root: &str, name: &str, head: &str) -> PathBuf {
        let folder = self.work.join(root).join(name);
        fs::create_dir_all(folder.join(".git")).expect("a repository folder");
        fs::write(folder.join(".git").join("HEAD"), head).expect("a HEAD file");
        folder
    }

    async fn projects(&self) -> Vec<Project> {
        self.backend.get("/projects").await
    }

    async fn project(&self, name: &str) -> Project {
        self.projects()
            .await
            .into_iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no project {name}"))
    }

    async fn health(&self) -> Health {
        self.backend.get("/health").await
    }

    fn planner(&self) -> Arc<rain_backend::modules::projects::Service> {
        self.backend.state.projects()
    }
}

fn main_branch() -> &'static str {
    "ref: refs/heads/main\n"
}

/// Reads frames of the topic until one satisfies `wanted`.
async fn frame_where(
    stream: &mut StreamReader,
    wanted: impl Fn(&[Project]) -> bool,
) -> Vec<Project> {
    loop {
        let frame = stream.expect_topic("projects").await;
        let list: Vec<Project> = serde_json::from_value(frame.payload).expect("a project list");
        if wanted(&list) {
            return list;
        }
    }
}

async fn open_stream(world: &World) -> StreamReader {
    let mut stream = world.backend.stream().await;
    assert_eq!(stream.expect_event().await.0, "hello");
    assert_eq!(stream.expect_event().await.0, "resync");
    stream
}

fn target(id: &str, mode: Option<LaunchMode>) -> LaunchTarget {
    LaunchTarget {
        r#type: LaunchType::Project,
        id: id.to_owned(),
        mode,
        args: None,
    }
}

fn plan(world: &World, id: &str, mode: Option<LaunchMode>) -> Vec<PlannedStep> {
    world
        .planner()
        .plan(&target(id, mode))
        .unwrap_or_else(|e| panic!("no plan: {}", e.body.message))
        .steps
}

fn spawn_parts(step: &PlannedStep) -> (&Path, &str) {
    match &step.action {
        StepAction::Spawn { exe, args_raw, .. } => (exe, args_raw),
        _ => panic!("step {} does not start a program", step.label),
    }
}

// ---- the list ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_list_has_the_repositories_directly_inside_the_roots() {
    let w = World::start(Setup {
        settings: "project_roots = ['{TEMP}\\work\\a', '{TEMP}\\work\\b', '{TEMP}\\work\\c']\n"
            .to_owned(),
        ..Setup::default()
    })
    .await;
    w.repo("a", "alpha", main_branch());
    w.repo("a", "beta", &"ab".repeat(20));
    // A worktree: `.git` is a file with a line that names the folder of the repository data.
    let store = w.work.join("store");
    fs::create_dir_all(&store).unwrap();
    fs::write(store.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
    let gamma = w.work.join("a").join("gamma");
    fs::create_dir_all(&gamma).unwrap();
    fs::write(gamma.join(".git"), format!("gitdir: {}\n", store.display())).unwrap();
    // Not projects: no `.git`, hidden, a loose file.
    fs::create_dir_all(w.work.join("a").join("delta")).unwrap();
    w.repo("a", ".hidden", main_branch());
    fs::write(w.work.join("a").join("loose.txt"), "x").unwrap();
    w.repo("b", "alpha", "ref: refs/heads/dev\n");

    let list = w.projects().await;

    let shown: Vec<(&str, &str)> = list
        .iter()
        .map(|p| (p.name.as_str(), p.branch.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("alpha", "main"),
            ("alpha", "dev"),
            ("beta", "abababa"),
            ("gamma", "topic")
        ]
    );
    // Two folders called alpha: the first keeps the plain id, the second gets the hash.
    assert_eq!(list[0].id, "proj-alpha");
    assert!(list[1].id.starts_with("proj-alpha-") && list[1].id.len() == "proj-alpha-".len() + 8);
    assert!(list.iter().all(|p| !p.missing && !p.open && !p.pinned));
    assert!(list.iter().all(|p| p.commits_14d.len() == 14));
    assert!(list[0].path.ends_with("work\\a\\alpha"));
    assert_eq!(list[0].environment.len(), 2);
    assert_eq!(list[0].environment[1].command.as_deref(), Some("claude"));

    // The root that does not exist is reported, and does not fail the call.
    let health = w.health().await;
    let source = health
        .sources
        .iter()
        .find(|s| s.name == "projects")
        .expect("a projects source");
    assert_eq!(source.state, SourceState::Failed);
    assert!(
        source
            .message
            .as_deref()
            .is_some_and(|m| m.starts_with("Project folder not found: "))
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_root_the_list_is_empty_and_the_source_is_off() {
    let w = World::start(Setup {
        settings: String::new(),
        ..Setup::default()
    })
    .await;
    assert!(w.projects().await.is_empty());
    let health = w.health().await;
    let source = health
        .sources
        .iter()
        .find(|s| s.name == "projects")
        .unwrap();
    assert_eq!(source.state, SourceState::Off);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_facts_are_cached_until_the_files_change() {
    let w = World::start(Setup::default()).await;
    let alpha = w.repo("a", "alpha", main_branch());
    w.repo("a", "beta", main_branch());
    w.git.answer_with(|call| {
        if call.args.iter().any(|a| a == "status") {
            out(" M a.txt\0")
        } else {
            out("")
        }
    });

    // The first list computes both facts of both repositories and waits for them.
    let first = w.projects().await;
    assert_eq!(first[0].dirty_files, Some(1));
    assert_eq!(w.git.calls().len(), 4);
    assert!(w.git.most_at_once.load(Ordering::SeqCst) <= 4);

    // The same files, within a minute: no git call at all.
    w.projects().await;
    assert_eq!(w.git.calls().len(), 4);

    // Someone commits in alpha: only alpha is asked again, in the background, and the topic
    // carries the new value.
    let mut stream = open_stream(&w).await;
    w.git.answer_with(|call| {
        if call.args.iter().any(|a| a == "status") {
            out(" M a.txt\0 M b.txt\0")
        } else {
            out("")
        }
    });
    let logs = alpha.join(".git").join("logs");
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join("HEAD"), "a new commit").unwrap();
    let answered = w.projects().await;
    assert_eq!(
        answered[0].dirty_files,
        Some(1),
        "the old value is still shown"
    );
    let pushed = frame_where(&mut stream, |list| {
        list.iter()
            .any(|p| p.name == "alpha" && p.dirty_files == Some(2))
    })
    .await;
    assert_eq!(
        pushed
            .iter()
            .find(|p| p.name == "beta")
            .unwrap()
            .dirty_files,
        Some(1)
    );
    assert_eq!(w.git.calls_for("alpha"), 4);
    assert_eq!(w.git.calls_for("beta"), 2);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_repository_does_not_hold_the_first_answer_longer_than_the_budget() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.repo("a", "beta", main_branch());
    *w.git.delay.lock().unwrap() = Duration::from_millis(3500);
    w.git.answer_with(|_| out(""));
    let mut stream = open_stream(&w).await;

    let started = std::time::Instant::now();
    let list = w.projects().await;
    let waited = started.elapsed();

    assert!(waited < Duration::from_secs(7), "{waited:?}");
    assert!(waited >= Duration::from_secs(5), "{waited:?}");
    assert!(list.iter().all(|p| p.dirty_files.is_none()));
    assert!(list.iter().all(|p| p.commits_14d == vec![0; 14]));
    let later = frame_where(&mut stream, |list| {
        !list.is_empty() && list.iter().all(|p| p.dirty_files == Some(0))
    })
    .await;
    assert_eq!(later.len(), 2);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_most_four_git_processes_run_at_once() {
    let w = World::start(Setup::default()).await;
    for name in ["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"] {
        w.repo("a", name, main_branch());
    }
    *w.git.delay.lock().unwrap() = Duration::from_millis(60);
    w.projects().await;
    assert_eq!(w.git.calls().len(), 16);
    let most = w.git.most_at_once.load(Ordering::SeqCst);
    assert!((2..=4).contains(&most), "{most} at once");
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_git_the_branch_shows_and_the_counts_do_not() {
    let w = World::start(Setup {
        tools: FakeTools::new(&["Code.exe", "wt.exe", "claude.exe"]),
        ..Setup::default()
    })
    .await;
    w.repo("a", "alpha", main_branch());
    let list = w.projects().await;
    assert_eq!(list[0].branch, "main");
    assert_eq!(list[0].dirty_files, None);
    assert_eq!(list[0].commits_14d, vec![0; 14]);
    assert!(w.git.calls().is_empty());
    let health = w.health().await;
    let git = health.sources.iter().find(|s| s.name == "git").unwrap();
    assert_eq!(git.state, SourceState::Off);
    assert_eq!(
        git.message.as_deref(),
        Some("git was not found. Showing the branch only")
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_git_leaves_the_facts_absent_and_is_not_asked_again_at_once() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.git.answer_with(|_| {
        Ok(GitOutput {
            exit_code: Some(128),
            stdout: Vec::new(),
            stderr: "fatal: detected dubious ownership in repository".into(),
        })
    });
    let list = w.projects().await;
    assert_eq!(list[0].dirty_files, None);
    assert_eq!(list[0].branch, "main");
    let calls = w.git.calls().len();
    w.projects().await;
    assert_eq!(
        w.git.calls().len(),
        calls,
        "a failure is not retried within a minute"
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_commit_strip_and_the_score_follow_the_log() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "busy", main_branch());
    w.repo("a", "quiet", main_branch());
    let now = w.clock_unix();
    w.git.answer_with(move |call| {
        let busy = call.repo.to_string_lossy().ends_with("\\busy");
        if call.args.iter().any(|a| a == "status") {
            out("")
        } else if busy {
            out(&format!("{}\n{}\n{}\n", now, now, now - 86_400))
        } else {
            out("")
        }
    });
    let list = w.projects().await;
    assert_eq!(list[0].name, "busy", "the most active project comes first");
    assert_eq!(list[0].commits_14d[13], 2);
    assert_eq!(list[0].commits_14d[12], 1);
    // 2 commits today (weight 14/14) and 1 yesterday (13/14): round(2 + 0.93) = 3.
    assert_eq!(list[0].activity_score, 3);
    assert_eq!(list[1].activity_score, 0);
    assert!(list.iter().all(|p| p.last_opened_at.is_none()));
    w.backend.stop().await;
}

impl World {
    fn clock_unix(&self) -> i64 {
        use rain_backend::core::clock::Clock;
        self.clock
            .now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_secs() as i64
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_vanished_project_is_missing_and_then_forgotten() {
    let w = World::start(Setup::default()).await;
    let folder = w.repo("a", "alpha", main_branch());
    w.repo("a", "beta", main_branch());
    let before = w.project("alpha").await;

    fs::remove_dir_all(&folder).unwrap();
    let list = w.projects().await;
    let gone = list.iter().find(|p| p.name == "alpha").unwrap();
    assert!(gone.missing);
    assert_eq!(gone.id, before.id);
    assert_eq!(gone.branch, "");
    assert_eq!(gone.dirty_files, None);
    assert!(!gone.open);
    // It is still known to search, so a pin of it stays; but nobody finds it by typing.
    let service = w.planner();
    let source: &dyn ItemSource = service.as_ref();
    assert!(source.item_for(&gone.id).is_some());
    assert!(
        source
            .search_entries()
            .iter()
            .all(|(item, _)| item.id != gone.id)
    );

    w.clock.advance(Duration::from_secs(31 * 86_400));
    let later = w.projects().await;
    assert!(later.iter().all(|p| p.name != "alpha"));
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forget_time_of_zero_never_lists_vanished_projects() {
    let w = World::start(Setup {
        projects_toml: Some("[defaults]\nforget_missing_days = 0\n".to_owned()),
        ..Setup::default()
    })
    .await;
    let folder = w.repo("a", "alpha", main_branch());
    w.project("alpha").await;
    fs::remove_dir_all(&folder).unwrap();
    assert!(w.projects().await.is_empty());
    w.backend.stop().await;
}

// ---- search, recent, ids ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_recent_and_the_project_of_a_path_come_from_the_list() {
    let w = World::start(Setup::default()).await;
    let alpha = w.repo("a", "alpha", main_branch());
    w.repo("a", "beta", main_branch());
    let list = w.projects().await;
    let alpha_id = list.iter().find(|p| p.name == "alpha").unwrap().id.clone();
    let service = w.planner();

    let entries = service.search_entries();
    assert_eq!(entries.len(), 2);
    let (item, keywords) = entries.iter().find(|(i, _)| i.id == alpha_id).unwrap();
    assert_eq!(item.title, "alpha");
    assert_eq!(item.score, 0);
    assert_eq!(keywords, &["main".to_owned()]);
    assert!(service.recent().is_empty(), "nothing was opened yet");
    assert!(service.item_for("proj-nothing").is_none());

    let inside = alpha.join("src").join("deep");
    assert_eq!(
        service.project_id_for_path(&inside.to_string_lossy().to_lowercase()),
        Some(alpha_id.clone())
    );
    assert_eq!(
        service.project_id_for_path(&alpha.to_string_lossy()),
        Some(alpha_id)
    );
    assert_eq!(
        service.project_id_for_path(&format!("{}-old", alpha.display())),
        None,
        "a folder with a longer name is not inside alpha"
    );
    w.backend.stop().await;
}

// ---- open detection ----

fn window(title: &str) -> WindowInfo {
    WindowInfo {
        pid: 7,
        title: title.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_is_open_when_its_editor_has_a_window_with_its_name() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.repo("a", "beta", main_branch());
    assert!(w.projects().await.iter().all(|p| !p.open));

    w.windows
        .0
        .lock()
        .unwrap()
        .push(window("main.rs - alpha - Visual Studio Code"));
    // The editor does not run: the window alone does not count.
    // (The list of windows is reused for two seconds, so wait it out.)
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert!(w.projects().await.iter().all(|p| !p.open));

    w.processes
        .0
        .lock()
        .unwrap()
        .push("C:\\Apps\\Code\\Code.exe".to_owned());
    tokio::time::sleep(Duration::from_millis(2100)).await;
    let list = w.projects().await;
    assert!(list.iter().find(|p| p.name == "alpha").unwrap().open);
    assert!(!list.iter().find(|p| p.name == "beta").unwrap().open);
    w.backend.stop().await;
}

// ---- the plan to open a project ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_default_plan_is_the_editor_and_one_terminal_tab_that_runs_claude() {
    let w = World::start(Setup::default()).await;
    let folder = w.repo("a", "alpha", main_branch());
    let id = w.project("alpha").await.id;
    let folder = folder.to_string_lossy().into_owned();

    let steps = plan(&w, &id, None);

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].label, "VS Code");
    let (editor, editor_args) = spawn_parts(&steps[0]);
    assert_eq!(editor, Path::new("C:\\fake\\Code.exe"));
    assert!(editor_args.contains(&folder), "{editor_args}");
    assert_eq!(steps[1].label, "Claude Code");
    let (terminal, line) = spawn_parts(&steps[1]);
    assert_eq!(terminal, Path::new("C:\\fake\\wt.exe"));
    assert!(line.starts_with("-w 0 new-tab -d "), "{line}");
    assert_eq!(line.matches("new-tab").count(), 1, "ONE tab: {line}");
    assert!(line.contains("--title alpha"), "{line}");
    assert!(
        line.ends_with("cmd.exe /d /k \"\"C:\\fake\\claude.exe\"\""),
        "{line}"
    );
    // The fallback gets the same text as the direct start.
    let fallback = steps[1].shell_fallback.as_ref().unwrap();
    assert_eq!(fallback.params.as_deref(), Some(line));
    assert_eq!(fallback.dir.as_deref(), Some(folder.as_str()));
    assert!(
        steps
            .iter()
            .all(|s| s.preset_error.is_none() && !s.stops_run)
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_modes_choose_the_steps() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    let id = w.project("alpha").await.id;

    // Ctrl+Enter: the editor only.
    let editor_only = plan(&w, &id, Some(LaunchMode::EditorOnly));
    assert_eq!(editor_only.len(), 1);
    assert_eq!(editor_only[0].label, "VS Code");

    // "Terminal here" works with the default list, which has no terminal step: one tab, no editor,
    // no Claude.
    let terminal = plan(&w, &id, Some(LaunchMode::Terminal));
    assert_eq!(terminal.len(), 1);
    let (exe, line) = spawn_parts(&terminal[0]);
    assert_eq!(exe, Path::new("C:\\fake\\wt.exe"));
    assert_eq!(line.matches("new-tab").count(), 1);
    assert!(!line.contains("cmd.exe"), "{line}");

    // "New Claude session here": one tab that runs claude.
    let claude = plan(&w, &id, Some(LaunchMode::Claude));
    assert_eq!(claude.len(), 1);
    let (_, line) = spawn_parts(&claude[0]);
    assert_eq!(line.matches("new-tab").count(), 1);
    assert!(line.contains("claude.exe"), "{line}");
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_editor_only_plan_needs_an_editor_step() {
    let w = World::start(Setup {
        projects_toml: Some("[defaults]\nsteps = [\"terminal\"]\n".to_owned()),
        ..Setup::default()
    })
    .await;
    w.repo("a", "alpha", main_branch());
    let id = w.project("alpha").await.id;
    let error = w
        .planner()
        .plan(&target(&id, Some(LaunchMode::EditorOnly)))
        .err()
        .expect("no editor step");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.body.reason.as_deref(), Some("noSteps"));
    assert_eq!(error.body.message, "This project has no editor step");
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_project_is_not_found() {
    let w = World::start(Setup::default()).await;
    let error = w
        .planner()
        .plan(&target("proj-nothing", None))
        .err()
        .expect("unknown id");
    assert_eq!(error.status, StatusCode::NOT_FOUND);
    assert_eq!(error.body.reason.as_deref(), Some("unknownId"));
    assert_eq!(error.body.message, "Project not found");
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_project_brings_its_editor_forward_and_starts_nothing_else() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.windows
        .0
        .lock()
        .unwrap()
        .push(window("main.rs - alpha - Visual Studio Code"));
    w.processes
        .0
        .lock()
        .unwrap()
        .push("C:\\Apps\\Code\\Code.exe".to_owned());
    let project = w.project("alpha").await;
    assert!(project.open);

    let steps = plan(&w, &project.id, None);
    assert!(matches!(steps[0].action, StepAction::Spawn { .. }));
    assert!(matches!(steps[1].action, StepAction::Nothing));
    assert!(steps[1].preset_error.is_none());

    // The other modes always start their steps.
    let terminal = plan(&w, &project.id, Some(LaunchMode::Terminal));
    assert!(matches!(terminal[0].action, StepAction::Spawn { .. }));
    w.backend.stop().await;
}

/// The list of an earlier moment said "open"; the editor was closed since. Enter must not take
/// the old answer for the truth (it would skip the Claude tab).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_plan_looks_at_the_editor_windows_again_when_the_list_is_old() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.windows
        .0
        .lock()
        .unwrap()
        .push(window("main.rs - alpha - Visual Studio Code"));
    w.processes
        .0
        .lock()
        .unwrap()
        .push("C:\\Apps\\Code\\Code.exe".to_owned());
    let project = w.project("alpha").await;
    assert!(project.open);

    // The editor window is closed; the list of windows is older than two seconds.
    w.windows.0.lock().unwrap().clear();
    tokio::time::sleep(Duration::from_millis(2100)).await;
    let steps = plan(&w, &project.id, None);
    assert!(
        matches!(steps[1].action, StepAction::Spawn { .. }),
        "the terminal tab must start: the project is not open any more"
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_that_is_gone_is_one_failing_step() {
    let w = World::start(Setup::default()).await;
    let folder = w.repo("a", "alpha", main_branch());
    let id = w.project("alpha").await.id;
    fs::remove_dir_all(&folder).unwrap();
    w.projects().await; // the list notices

    let steps = plan(&w, &id, None);
    assert_eq!(steps.len(), 1);
    let message = steps[0].preset_error.as_deref().unwrap();
    assert!(message.starts_with("Folder not found: "), "{message}");
    assert!(steps[0].stops_run);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_program_fails_only_its_own_step() {
    let w = World::start(Setup {
        tools: FakeTools::new(&["git.exe", "wt.exe"]),
        ..Setup::default()
    })
    .await;
    w.repo("a", "alpha", main_branch());
    let id = w.project("alpha").await.id;
    let steps = plan(&w, &id, None);
    assert_eq!(
        steps[0].preset_error.as_deref(),
        Some("VS Code was not found. Set its path in projects.toml")
    );
    assert_eq!(
        steps[1].preset_error.as_deref(),
        Some("Claude Code was not found. Set its path in projects.toml")
    );
    assert!(steps.iter().all(|s| !s.stops_run));
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_program_path_in_projects_toml_that_does_not_exist_is_an_error_without_a_fall_through() {
    let w = World::start(Setup {
        projects_toml: Some("[tools]\neditor = 'D:\\nowhere\\Code.exe'\n".to_owned()),
        ..Setup::default()
    })
    .await;
    w.repo("a", "alpha", main_branch());
    let id = w.project("alpha").await.id;
    let steps = plan(&w, &id, None);
    assert_eq!(
        steps[0].preset_error.as_deref(),
        Some("Configured program not found: D:\\nowhere\\Code.exe")
    );
    assert!(
        steps[1].preset_error.is_none(),
        "the terminal step still runs"
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_settings_and_named_steps_come_from_projects_toml() {
    let toml = r#"
[terminal]
window = "new"
profile = "Command Prompt"

[[step]]
name = "dev-server"
kind = "terminal"
label = "Dev server"
command = "echo hello; echo there"

[[step]]
name = "preview"
kind = "browser"
url = "http://localhost:5173"
delay_ms = 3000

[[project]]
path = '{TEMP}\work\a\alpha'
steps = ["editor", "dev-server", "preview", "claude"]
"#;
    let w = World::start(Setup {
        projects_toml: Some(toml.to_owned()),
        ..Setup::default()
    })
    .await;
    w.repo("a", "alpha", main_branch());
    let project = w.project("alpha").await;
    let kinds: Vec<_> = project
        .environment
        .iter()
        .map(|s| s.label.as_str())
        .collect();
    assert_eq!(kinds, ["VS Code", "Dev server", "Browser", "Claude Code"]);
    assert_eq!(
        project.environment[2].url.as_deref(),
        Some("http://localhost:5173")
    );

    let steps = plan(&w, &project.id, None);
    assert_eq!(steps.len(), 4);
    let (_, line) = spawn_parts(&steps[1]);
    assert!(
        line.starts_with("-w new new-tab -p \"Command Prompt\" -d "),
        "{line}"
    );
    assert!(
        line.ends_with("cmd.exe /d /k \"echo hello\\; echo there\""),
        "{line}"
    );
    assert_eq!(steps[2].delay_ms, 3000);
    assert!(
        matches!(&steps[2].action, StepAction::ShellExec { file, .. } if file == "http://localhost:5173")
    );
    w.backend.stop().await;
}

// ---- game mode ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_mode_refuses_refresh_and_launch_but_not_reading() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    let id = w.project("alpha").await.id;
    let calls = w.git.calls().len();

    w.backend.state.set_game_mode_active(true);
    let (status, body) = w.backend.send("POST", "/projects/refresh", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "gameMode");
    let error = w.planner().plan(&target(&id, None)).err().expect("refused");
    assert_eq!(error.body.reason.as_deref(), Some("gameMode"));

    // Reading still works, from the cache, and starts no git process.
    let list = w.projects().await;
    assert_eq!(list.len(), 1);
    assert_eq!(w.git.calls().len(), calls);
    w.backend.state.set_game_mode_active(false);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_asks_git_again_for_every_project() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.projects().await;
    let before = w.git.calls().len();
    w.git.answer_with(|call| {
        if call.args.iter().any(|a| a == "status") {
            out(" M a\0 M b\0 M c\0")
        } else {
            out("")
        }
    });
    let (status, body) = w.backend.send("POST", "/projects/refresh", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body[0]["dirtyFiles"], 3);
    assert_eq!(w.git.calls().len(), before + 2);
    w.backend.stop().await;
}

// ---- after an open ----

fn run_with(id: &str, statuses: &[Status]) -> LaunchRun {
    LaunchRun {
        id: "run-1".to_owned(),
        target: target(id, None),
        status: RunStatus::Success,
        steps: statuses
            .iter()
            .map(|status| LaunchStep {
                label: "step".to_owned(),
                icon: rain_types::IconRef {
                    kind: rain_types::IconKind::Letter,
                    value: "s".to_owned(),
                    tint: None,
                    fallback: None,
                },
                status: *status,
                error: None,
            })
            .collect(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_run_with_a_working_step_records_the_open_and_publishes() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.repo("a", "beta", main_branch());
    let id = w.project("alpha").await.id;
    let mut stream = open_stream(&w).await;

    // A run in which nothing worked records nothing.
    w.planner()
        .finished(&run_with(&id, &[Status::Failed, Status::Failed]))
        .await;
    assert!(w.project("alpha").await.last_opened_at.is_none());

    w.planner()
        .finished(&run_with(&id, &[Status::Failed, Status::Success]))
        .await;
    let pushed = frame_where(&mut stream, |list| {
        list.iter()
            .any(|p| p.name == "alpha" && p.last_opened_at.is_some())
    })
    .await;
    assert_eq!(
        pushed[0].name, "alpha",
        "the opened project is listed first"
    );

    let list = w.projects().await;
    assert!(
        list.iter()
            .find(|p| p.name == "alpha")
            .unwrap()
            .last_opened_at
            .is_some()
    );
    assert!(
        list.iter()
            .find(|p| p.name == "beta")
            .unwrap()
            .last_opened_at
            .is_none()
    );
    let recent = w.planner().recent();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].1.id, id);
    w.backend.stop().await;
}

// ---- projects.toml ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_example_file_is_written_once_and_not_rewritten() {
    let mut w = World::start(Setup::default()).await;
    let path = w.backend.data_dir.join("projects.toml");
    let text = fs::read_to_string(&path).expect("projects.toml was created");
    assert!(text.starts_with("schema_version = 1"));
    assert!(
        text.lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count()
            == 1,
        "only the version is more than a comment"
    );

    fs::write(&path, "schema_version = 1\n# mine\n").unwrap();
    w.backend.restart().await;
    w.projects().await;
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "schema_version = 1\n# mine\n"
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_of_projects_toml_is_published_and_a_broken_file_keeps_the_last_copy() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    assert_eq!(w.project("alpha").await.environment.len(), 2);
    let mut stream = open_stream(&w).await;
    let path = w.backend.data_dir.join("projects.toml");

    fs::write(
        &path,
        "schema_version = 1\n[defaults]\nsteps = [\"editor\", \"terminal\", \"claude\"]\n",
    )
    .unwrap();
    let pushed = frame_where(&mut stream, |list| {
        list.iter().any(|p| p.environment.len() == 3)
    })
    .await;
    assert_eq!(pushed[0].environment[1].label, "Terminal");

    // A syntax error: the last good steps stay and the problem is in /health.
    fs::write(&path, "schema_version = 1\n[defaults\nsteps = \n").unwrap();
    let mut issue = false;
    for _ in 0..50 {
        let health = w.health().await;
        if health
            .config_issues
            .iter()
            .any(|i| i.file == "projects.toml")
        {
            issue = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(issue, "the broken file shows in /health");
    assert_eq!(w.project("alpha").await.environment.len(), 3);
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_of_the_project_folders_is_published() {
    let w = World::start(Setup::default()).await;
    w.repo("a", "alpha", main_branch());
    w.repo("b", "omega", main_branch());
    assert_eq!(w.projects().await.len(), 2);
    let mut stream = open_stream(&w).await;

    let only_b = format!("{}\\work\\b", w.work.parent().unwrap().display());
    let (status, body) = w
        .backend
        .send(
            "PATCH",
            "/settings",
            Some(serde_json::json!({ "projectRoots": [only_b] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pushed = frame_where(&mut stream, |list| list.len() == 1).await;
    assert_eq!(pushed[0].name, "omega");
    w.backend.stop().await;
}
