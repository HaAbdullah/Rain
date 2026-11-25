//! The scheduled tasks of the Claude desktop app in the Jobs list (section 03, read only). The
//! app's files are a throwaway copy inside a throwaway Claude folder, handed to the backend
//! through `Config.claude_dir`; every name and text here is made up.

mod common;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use common::TestBackend;
use hyper::StatusCode;
use rain_types::{JobSource, ScheduledJob, Status};
use serde_json::{Value, json};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a fixture step")
        .as_millis() as u64
}

const HOUR: u64 = 3_600_000;

fn app_folder(root: &Path) -> std::path::PathBuf {
    root.join("desktop-app")
        .join("claude-code-sessions")
        .join("account-1")
        .join("org-1")
}

/// Writes the files of the app below `<root>/desktop-app`.
fn write_app(root: &Path, tasks: Value, sessions: &[(&str, Value)]) {
    let folder = app_folder(root);
    std::fs::create_dir_all(&folder).expect("a fixture step");
    std::fs::write(
        folder.join("scheduled-tasks.json"),
        serde_json::to_string(&json!({ "scheduledTasks": tasks })).expect("a fixture step"),
    )
    .expect("a fixture step");
    for (name, body) in sessions {
        std::fs::write(
            folder.join(format!("local_{name}.json")),
            serde_json::to_string(body).expect("a fixture step"),
        )
        .expect("a fixture step");
    }
}

fn session(
    task: Option<&str>,
    id: &str,
    started: u64,
    category: Option<&str>,
    detail: &str,
) -> Value {
    let mut body = json!({
        "sessionId": format!("local_{id}"),
        "createdAt": started,
        "lastActivityAt": started + 60_000,
        "lastTurnEdgeAt": started + 90_000,
        "completedTurns": 1,
        "title": "Made up title",
        "someOtherKey": { "nested": [1, 2, 3] },
    });
    if let Some(task) = task {
        body["scheduledTaskId"] = json!(task);
    }
    if let Some(category) = category {
        body["postTurnSummary"] = json!({
            "status_category": category,
            "status_detail": detail,
            "needs_action": "none",
        });
    }
    body
}

async fn start_with(files: impl FnOnce(&Path), jobs_toml: Option<&str>) -> TestBackend {
    let dir = tempfile::tempdir().expect("a fixture step");
    let root = dir.path().join("claude-home");
    files(&root);
    let mut builder =
        TestBackend::builder().config(move |config| config.claude_dir = Some(root.clone()));
    if let Some(text) = jobs_toml {
        builder = builder.file("jobs.toml", text);
    }
    let backend = builder.start().await;
    // The folder lives as long as the test program; the system cleans the temporary folder.
    std::mem::forget(dir);
    backend
}

fn find<'a>(list: &'a [ScheduledJob], id: &str) -> &'a ScheduledJob {
    list.iter()
        .find(|job| job.id == id)
        .unwrap_or_else(|| panic!("no entry {id}"))
}

const MINE: &str = "schema_version = 1\n[[job]]\nid = 'job-mine'\nname = 'Mine'\nkind = 'cron'\nschedule = '0 1 * * *'\ncommand = 'cmd'\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tasks_of_the_app_are_listed_next_to_the_jobs_of_rain() {
    let now = now_ms();
    let backend = start_with(
        |root| {
            write_app(
                root,
                json!([
                    { "id": "morning-run", "displayName": "Morning run", "cronExpression": "0 6 * * *", "enabled": true },
                    { "id": "radar", "displayName": "Radar", "cronExpression": "0 7 * * *", "enabled": false },
                    { "id": "no-switch", "displayName": "No switch", "cronExpression": "0 8 * * *" },
                    { "displayName": "No id", "cronExpression": "0 9 * * *", "enabled": true },
                ]),
                &[
                    ("a", session(Some("morning-run"), "a", now - 50 * HOUR, Some("completed"), "Two fits found")),
                    ("b", session(Some("morning-run"), "b", now - 26 * HOUR, Some("blocked"), "Login   wall\nstopped the run")),
                    ("c", session(Some("morning-run"), "c", now - 2 * HOUR, Some("completed"), "All good")),
                    ("d", session(Some("radar"), "d", now - 3 * HOUR, None, "")),
                    ("e", session(None, "e", now - HOUR, Some("completed"), "Not a task")),
                    ("f", session(Some("morning-run"), "f", now - 80 * HOUR, Some("somethingNew"), "")),
                ],
            );
            // A file that is not JSON is skipped, not an error.
            std::fs::write(app_folder(root).join("local_broken.json"), "{ not json").expect("a fixture step");
        },
        Some(MINE),
    )
    .await;

    let list: Vec<ScheduledJob> = backend.get("/jobs").await;
    let ids: Vec<_> = list.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(
        ids,
        ["job-mine", "claude-app:morning-run", "claude-app:radar"]
    );
    assert_eq!(list[0].source, JobSource::Rain);

    let morning = find(&list, "claude-app:morning-run");
    assert_eq!(morning.source, JobSource::ClaudeApp);
    assert_eq!(morning.name, "Morning run");
    assert_eq!(morning.cron, "0 6 * * *");
    assert_eq!(morning.schedule_label, "Every day at 06:00");
    assert!(morning.enabled && morning.next_run_at.is_some());
    let statuses: Vec<_> = morning.recent_runs.iter().map(|run| run.status).collect();
    assert_eq!(
        statuses,
        [
            Status::Success,
            Status::Failed,
            Status::Success,
            Status::Idle
        ],
        "newest first; a blocked run is a problem; an unknown category is neutral"
    );
    assert_eq!(morning.recent_runs[0].outcome.as_deref(), Some("All good"));
    assert_eq!(
        morning.recent_runs[1].outcome.as_deref(),
        Some("Login wall stopped the run")
    );
    assert_eq!(morning.recent_runs[0].duration_ms, Some(90_000));
    assert_eq!(morning.recent_runs[0].id, "local_c");
    assert!(!morning.running);

    let radar = find(&list, "claude-app:radar");
    assert!(!radar.enabled && radar.next_run_at.is_none());
    assert_eq!(
        radar.recent_runs.len(),
        1,
        "only the sessions of the task count"
    );
    assert_eq!(
        radar.recent_runs[0].status,
        Status::Idle,
        "without a summary and a while old: ended, outcome unknown"
    );
    assert!(radar.recent_runs[0].duration_ms.is_none());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_without_a_summary_that_is_still_active_is_running() {
    let now = now_ms();
    let backend = start_with(
        |root| {
            let mut body = session(Some("t"), "live", now - 5 * 60_000, None, "");
            body["lastActivityAt"] = json!(now - 60_000);
            write_app(
                root,
                json!([{ "id": "t", "displayName": "T", "cronExpression": "0 6 * * *", "enabled": true }]),
                &[("live", body)],
            );
        },
        None,
    )
    .await;
    let list: Vec<ScheduledJob> = backend.get("/jobs").await;
    let task = find(&list, "claude-app:t");
    assert!(task.running);
    assert_eq!(task.recent_runs[0].status, Status::Running);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rain_cannot_switch_or_start_a_task_of_the_app() {
    let backend = start_with(
        |root| {
            write_app(
                root,
                json!([{ "id": "t", "displayName": "T", "cronExpression": "0 6 * * *", "enabled": true }]),
                &[],
            );
        },
        None,
    )
    .await;
    let (status, body) = backend
        .send(
            "PATCH",
            "/jobs/claude-app:t",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "claudeAppJob");
    let (status, _) = backend.send("POST", "/jobs/claude-app:t/runs", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let list: Vec<ScheduledJob> = backend.get("/jobs").await;
    assert!(find(&list, "claude-app:t").enabled, "nothing changed");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_app_the_list_is_only_the_jobs_of_rain() {
    let backend = start_with(|_| {}, Some(MINE)).await;
    let list: Vec<ScheduledJob> = backend.get("/jobs").await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].source, JobSource::Rain);
    backend.stop().await;
}
