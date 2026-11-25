//! Launch layouts of projects on a real backend (feature section 02 6.17).
//!
//! The programs on the PC, the opener and the desktop (windows and monitors) are fakes, so
//! nothing is started and no window is moved. The one test that opens a real window of its own is
//! in `platform/windows_place.rs`.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use common::fakes::FakeOpener;
use common::{TestBackend, TestBackendBuilder};
use hyper::StatusCode;
use rain_backend::core::launch_contract::{LaunchPlanner, PlannedStep, StepAction};
use rain_backend::core::placement::{Desktop, MonitorInfo, Placer, Rect, SeenWindow};
use rain_backend::modules::projects::ToolEnv;
use rain_types::{LaunchRun, LaunchTarget, LaunchType, ProjectLayout, RunStatus, Status};
use serde_json::{Value, json};

// ---- fakes ----

struct FakeTools;

impl ToolEnv for FakeTools {
    fn path_dirs(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("C:\\fake")]
    }

    fn var(&self, _name: &str) -> Option<String> {
        None
    }

    fn exists(&self, path: &Path) -> bool {
        ["Code.exe", "wt.exe", "claude.exe"]
            .iter()
            .any(|name| path == PathBuf::from("C:\\fake").join(name))
    }
}

/// One monitor of 1920 x 1040 (the taskbar is below). Windows show up at the call of `windows()`
/// that the test names.
struct FakeDesktop {
    windows: Mutex<Vec<SeenWindow>>,
    appear: Mutex<Vec<(usize, SeenWindow)>>,
    calls: Mutex<usize>,
    placed: Mutex<Vec<(isize, Rect)>>,
}

impl FakeDesktop {
    fn new(windows: Vec<SeenWindow>, appear: Vec<(usize, SeenWindow)>) -> Arc<Self> {
        Arc::new(Self {
            windows: Mutex::new(windows),
            appear: Mutex::new(appear),
            calls: Mutex::new(0),
            placed: Mutex::new(Vec::new()),
        })
    }

    fn placed(&self) -> Vec<(isize, Rect)> {
        self.placed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Desktop for FakeDesktop {
    fn windows(&self) -> Vec<SeenWindow> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        *calls += 1;
        let mut appear = self.appear.lock().unwrap_or_else(PoisonError::into_inner);
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let due: Vec<SeenWindow> = appear
            .iter()
            .filter(|(at, _)| *at <= *calls)
            .map(|(_, window)| window.clone())
            .collect();
        appear.retain(|(at, _)| *at > *calls);
        for window in due {
            windows.insert(0, window);
        }
        windows.clone()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        vec![MonitorInfo {
            work: Rect::new(0, 0, 1920, 1040),
            primary: true,
            dpi: 96,
        }]
    }

    fn anchor(&self) -> Option<(i32, i32)> {
        Some((100, 100))
    }

    fn place(&self, hwnd: isize, visible: Rect) -> Result<(), String> {
        self.placed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((hwnd, visible));
        Ok(())
    }

    fn visible_frame(&self, hwnd: isize) -> Option<Rect> {
        self.placed()
            .iter()
            .rev()
            .find(|(placed, _)| *placed == hwnd)
            .map(|(_, rect)| *rect)
    }
}

fn window(hwnd: isize, pid: u32, image: &str, title: &str) -> SeenWindow {
    SeenWindow {
        hwnd,
        pid,
        image: image.to_owned(),
        title: title.to_owned(),
    }
}

// ---- the world of a test ----

struct World {
    backend: TestBackend,
    opener: Arc<FakeOpener>,
    desktop: Arc<FakeDesktop>,
}

async fn start(projects_toml: &str, desktop: Arc<FakeDesktop>) -> World {
    let opener = Arc::new(FakeOpener::default());
    let placer = Placer::with_timing(
        desktop.clone(),
        Duration::from_millis(400),
        Duration::from_millis(5),
        Duration::from_millis(5),
    );
    let builder: TestBackendBuilder = TestBackend::builder()
        .file("settings.toml", "project_roots = ['{TEMP}\\work\\a']\n")
        .file("projects.toml", projects_toml)
        .with::<dyn ToolEnv>(Arc::new(FakeTools))
        .with::<Placer>(Arc::new(placer))
        .opener(opener.clone());
    let backend = builder.start().await;
    let folder = backend
        .data_dir
        .parent()
        .expect("the data folder has a parent")
        .join("work")
        .join("a")
        .join("alpha");
    fs::create_dir_all(folder.join(".git")).expect("a repository folder");
    fs::write(folder.join(".git").join("HEAD"), "ref: refs/heads/main\n").expect("a HEAD file");
    World {
        backend,
        opener,
        desktop,
    }
}

async fn alpha_id(world: &World) -> String {
    let list: Vec<rain_types::Project> = world.backend.get("/projects").await;
    list.into_iter()
        .find(|p| p.name == "alpha")
        .expect("the project alpha")
        .id
}

fn plan(world: &World, id: &str) -> Vec<PlannedStep> {
    world
        .backend
        .state
        .projects()
        .plan(&LaunchTarget {
            r#type: LaunchType::Project,
            id: id.to_owned(),
            mode: None,
            args: None,
        })
        .unwrap_or_else(|e| panic!("no plan: {}", e.body.message))
        .steps
}

const LAYOUT: &str = r#"
[defaults]
layout = [
  { step = "editor", zone = "right-half" },
  { step = "terminal", zone = "left-half" },
  { step = "claude", zone = "left-half" },
  { folder = true, zone = "bottom-left" },
]
"#;

// ---- the plan ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_default_layout_makes_placed_steps_and_one_terminal_window_for_a_zone() {
    let w = start(LAYOUT, FakeDesktop::new(Vec::new(), Vec::new())).await;
    let id = alpha_id(&w).await;
    let steps = plan(&w, &id);
    assert_eq!(steps.len(), 3, "the terminal and Claude share one call");
    assert_eq!(steps[1].label, "Terminal + Claude Code");
    let StepAction::Spawn { args_raw, .. } = &steps[1].action else {
        panic!("a terminal call");
    };
    assert!(args_raw.starts_with("-w new new-tab"), "{args_raw}");
    assert_eq!(args_raw.matches("new-tab").count(), 2);
    let editor = steps[0].placement.as_ref().expect("the editor is placed");
    assert_eq!(editor.zone, rain_types::LayoutZone::RightHalf);
    assert!(editor.find.allow_existing);
    assert!(steps[2].placement.as_ref().is_some());
    let environment = &w.backend.get::<Vec<rain_types::Project>>("/projects").await[0].environment;
    assert_eq!(environment.len(), 4, "the environment follows the layout");
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_layout_nothing_is_placed() {
    let w = start("", FakeDesktop::new(Vec::new(), Vec::new())).await;
    let id = alpha_id(&w).await;
    let steps = plan(&w, &id);
    assert_eq!(steps.len(), 2);
    assert!(steps.iter().all(|step| step.placement.is_none()));
    let layout: ProjectLayout = w.backend.get(&format!("/projects/{id}/layout")).await;
    assert!(layout.items.is_empty());
    assert_eq!(layout.scope, rain_types::LayoutScope::None);
    assert_eq!(layout.monitors, 1);
    assert_eq!(layout.steps[0].name, "editor");
    w.backend.stop().await;
}

// ---- saving a layout ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saving_a_layout_writes_projects_toml_and_keeps_the_comments() {
    let hand_written = "schema_version = 1\n\n# my notes\n[[step]]\nname = \"dev\"   # mine\nkind = \"terminal\"\n";
    let w = start(hand_written, FakeDesktop::new(Vec::new(), Vec::new())).await;
    let id = alpha_id(&w).await;
    let body = json!({
        "items": [
            { "what": { "kind": "step", "name": "editor" }, "zone": "left-two-thirds", "monitor": 1 },
            { "what": { "kind": "step", "name": "dev" }, "zone": "right-third" },
            { "what": { "kind": "folder" }, "zone": "free" },
        ],
        "all": false,
    });
    let (status, answer) = w
        .backend
        .send("PUT", &format!("/projects/{id}/layout"), Some(body))
        .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["scope"], "project");
    assert_eq!(answer["items"].as_array().map(Vec::len), Some(3));

    let text = fs::read_to_string(w.backend.data_dir.join("projects.toml")).unwrap();
    assert!(text.contains("# my notes"), "{text}");
    assert!(text.contains("name = \"dev\"   # mine"), "{text}");
    assert!(
        text.contains("zone = \"left-two-thirds\", monitor = 1"),
        "{text}"
    );
    assert!(text.contains("[[project]]"), "{text}");

    // The environment of the project shows the layout's items.
    let list: Vec<rain_types::Project> = w.backend.get("/projects").await;
    assert_eq!(list[0].environment.len(), 3);

    // The same save for all projects goes under [defaults]; an empty list removes it.
    let all = json!({ "items": [{ "what": { "kind": "step", "name": "claude" }, "zone": "full" }], "all": true });
    let (status, _) = w
        .backend
        .send("PUT", &format!("/projects/{id}/layout"), Some(all))
        .await;
    assert_eq!(status, StatusCode::OK);
    let text = fs::read_to_string(w.backend.data_dir.join("projects.toml")).unwrap();
    assert!(text.contains("[defaults]"), "{text}");
    let none = json!({ "items": [], "all": true });
    w.backend
        .send("PUT", &format!("/projects/{id}/layout"), Some(none))
        .await;
    let text = fs::read_to_string(w.backend.data_dir.join("projects.toml")).unwrap();
    assert_eq!(
        text.matches("layout = [").count(),
        1,
        "only the project's own: {text}"
    );
    w.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layout_with_an_unknown_step_or_app_or_too_many_items_is_refused() {
    let w = start("", FakeDesktop::new(Vec::new(), Vec::new())).await;
    let id = alpha_id(&w).await;
    let path = format!("/projects/{id}/layout");
    let bad_step = json!({ "items": [{ "what": { "kind": "step", "name": "nope" }, "zone": "full" }], "all": false });
    let (status, body) = w.backend.send("PUT", &path, Some(bad_step)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "unknownStep");
    let bad_app = json!({ "items": [{ "what": { "kind": "app", "id": "app-gone" }, "zone": "full" }], "all": false });
    let (_, body) = w.backend.send("PUT", &path, Some(bad_app)).await;
    assert_eq!(body["reason"], "unknownApp");
    let many: Vec<Value> = (0..13)
        .map(|_| json!({ "what": { "kind": "step", "name": "editor" }, "zone": "full" }))
        .collect();
    let (_, body) = w
        .backend
        .send("PUT", &path, Some(json!({ "items": many, "all": false })))
        .await;
    assert_eq!(body["reason"], "tooManyItems");
    let (status, _) = w
        .backend
        .send("GET", "/projects/proj-nothing/layout", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Nothing was written.
    let text = fs::read_to_string(w.backend.data_dir.join("projects.toml")).unwrap_or_default();
    assert!(!text.contains("layout = ["));
    w.backend.stop().await;
}

// ---- a launch ----

async fn finished(world: &World, id: &str) -> LaunchRun {
    let (status, body) = world
        .backend
        .send(
            "POST",
            "/launches",
            Some(json!({ "type": "project", "id": id })),
        )
        .await;
    assert!(status.is_success(), "{body}");
    let run: LaunchRun = serde_json::from_value(body).expect("a launch run");
    for _ in 0..100 {
        let run: LaunchRun = world.backend.get(&format!("/launches/{}", run.id)).await;
        if run.status != RunStatus::Running {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the launch did not end");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_launch_moves_the_windows_it_made_and_reports_the_one_that_never_showed() {
    // The user's own terminal window (5) is open already. Call 1 of `windows()` is the snapshot
    // before the editor, call 2 the first look for its window; the terminal window shows at call
    // 4, and the folder window never does.
    let desktop = FakeDesktop::new(
        vec![window(5, 9, "windowsterminal.exe", "my shell")],
        vec![
            (2, window(11, 700, "code.exe", "alpha - Visual Studio Code")),
            (4, window(12, 9, "windowsterminal.exe", "alpha")),
        ],
    );
    let w = start(LAYOUT, desktop).await;
    let id = alpha_id(&w).await;
    let run = finished(&w, &id).await;

    assert_eq!(run.steps.len(), 3);
    assert!(run.steps.iter().all(|step| step.status == Status::Success));
    assert_eq!(run.steps[0].note, None);
    assert_eq!(run.steps[1].note, None);
    assert_eq!(
        run.steps[2].note.as_deref(),
        Some("Its window did not show up, so it was left where it is")
    );
    assert_eq!(w.opener.requests().len(), 3);
    let placed = w.desktop.placed();
    assert_eq!(
        placed,
        vec![
            (11, Rect::new(960, 0, 1920, 1040)),
            (12, Rect::new(0, 0, 960, 1040)),
        ],
        "the editor goes right, the new terminal left, and the user's terminal is not touched"
    );
    let unique: HashSet<isize> = placed.iter().map(|(hwnd, _)| *hwnd).collect();
    assert!(!unique.contains(&5));
    w.backend.stop().await;
}
