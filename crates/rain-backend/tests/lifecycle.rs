//! Start with Windows, the data folder change, the hotkey in `GET /health` and the rules for
//! stopping the backend. Feature section 06 (6.3, 6.4, 6.5, 6.9); foundation 2.5 and 2.6.

mod common;

use std::sync::Arc;

use common::fakes::FakeRunKey;
use common::{StreamReader, TestBackend};
use hyper::StatusCode;
use rain_backend::platform::registry::{RunKey, RunState};
use rain_types::Health;
use serde_json::{Value, json};

fn shell_facts(exe: &str, hotkey_registered: bool) -> Value {
    json!({
        "hotkeyRegistered": hotkey_registered,
        "shellExe": exe,
        "shellVersion": "0.1.0",
        "windowVisible": false
    })
}

async fn patch(backend: &TestBackend, body: Value) -> (StatusCode, Value) {
    backend.send("PATCH", "/settings", Some(body)).await
}

/// A real-mode backend whose autostart entry is the fake `key`.
async fn real_mode_backend(key: &Arc<FakeRunKey>) -> TestBackend {
    TestBackend::builder()
        .real_mode()
        .with::<dyn RunKey>(key.clone())
        .start()
        .await
}

async fn open_stream(backend: &TestBackend) -> StreamReader {
    let mut stream = backend.stream().await;
    assert_eq!(stream.expect_event().await.0, "hello");
    assert_eq!(stream.expect_event().await.0, "resync");
    stream
}

// ---- start with Windows ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_on_writes_the_exact_command_and_switching_off_removes_it() {
    let key = Arc::new(FakeRunKey::default());
    let backend = real_mode_backend(&key).await;
    backend
        .send(
            "PUT",
            "/internal/shell",
            Some(shell_facts(r"C:\Fake Folder\rain.exe", true)),
        )
        .await;

    let (status, body) = patch(&backend, json!({ "startWithWindows": true })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["startWithWindows"], true);
    assert_eq!(
        key.writes.lock().unwrap().as_slice(),
        [r#""C:\Fake Folder\rain.exe" --hidden"#]
    );

    let (_, body) = patch(&backend, json!({ "startWithWindows": false })).await;
    assert_eq!(body["startWithWindows"], false);
    assert_eq!(key.state.lock().unwrap().command, None);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_entry_that_windows_disabled_reads_as_off_and_switching_on_clears_the_mark() {
    let key = Arc::new(FakeRunKey::default());
    *key.state.lock().unwrap() = RunState {
        command: Some(r#""C:\Fake\rain.exe" --hidden"#.into()),
        disabled_by_windows: true,
    };
    let backend = real_mode_backend(&key).await;
    backend
        .send(
            "PUT",
            "/internal/shell",
            Some(shell_facts(r"C:\Fake\rain.exe", true)),
        )
        .await;

    let settings: Value = backend.get("/settings").await;
    assert_eq!(settings["startWithWindows"], false);

    let (_, body) = patch(&backend, json!({ "startWithWindows": true })).await;
    assert_eq!(body["startWithWindows"], true);
    assert!(!key.state.lock().unwrap().disabled_by_windows);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_on_without_a_reported_shell_is_a_conflict_and_a_failing_registry_is_an_error() {
    let key = Arc::new(FakeRunKey::default());
    let backend = real_mode_backend(&key).await;

    let (status, body) = patch(&backend, json!({ "startWithWindows": true })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "noShell");
    assert_eq!(body["message"], "Open the Rain app once, then try again.");

    backend
        .send(
            "PUT",
            "/internal/shell",
            Some(shell_facts(r"C:\Fake\rain.exe", true)),
        )
        .await;
    *key.fail.lock().unwrap() = true;
    let (status, body) = patch(&backend, json!({ "startWithWindows": true })).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["reason"], "registry");
    assert_eq!(body["message"], "Could not change start with Windows.");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_change_is_published_on_the_topic_settings() {
    let key = Arc::new(FakeRunKey::default());
    let backend = real_mode_backend(&key).await;
    backend
        .send(
            "PUT",
            "/internal/shell",
            Some(shell_facts(r"C:\Fake\rain.exe", true)),
        )
        .await;
    let mut stream = open_stream(&backend).await;

    patch(&backend, json!({ "startWithWindows": true })).await;
    let frame = stream.expect_topic("settings").await;
    assert_eq!(frame.payload["startWithWindows"], true);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn when_the_shell_moves_the_entry_is_rewritten_and_when_there_is_none_nothing_is_written() {
    let key = Arc::new(FakeRunKey::default());
    let backend = real_mode_backend(&key).await;

    // No entry: the user has not opted in, so a report writes nothing.
    backend
        .send(
            "PUT",
            "/internal/shell",
            Some(shell_facts(r"C:\Old\rain.exe", true)),
        )
        .await;
    assert!(key.writes.lock().unwrap().is_empty());

    patch(&backend, json!({ "startWithWindows": true })).await;
    backend
        .send(
            "PUT",
            "/internal/shell",
            Some(shell_facts(r"D:\New\rain.exe", true)),
        )
        .await;
    assert_eq!(
        key.state.lock().unwrap().command.as_deref(),
        Some(r#""D:\New\rain.exe" --hidden"#)
    );
    backend.stop().await;
}

// ---- the hotkey ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hotkey_that_did_not_register_is_a_failed_source_in_the_health_report_until_it_does() {
    let backend = TestBackend::start().await;

    let mut facts = shell_facts(r"C:\Fake\rain.exe", false);
    facts["hotkeyError"] = json!("The hotkey is already in use");
    backend.send("PUT", "/internal/shell", Some(facts)).await;
    let health: Health = backend.get("/health").await;
    // Other modules report their own sources at start (the Claude registry, for one), so the
    // hotkey source is looked up by name.
    let hotkey = health
        .sources
        .iter()
        .find(|source| source.name == "hotkey")
        .expect("the hotkey source is reported");
    assert_eq!(
        hotkey.message.as_deref(),
        Some("The hotkey is already in use")
    );

    backend
        .send(
            "PUT",
            "/internal/shell",
            Some(shell_facts(r"C:\Fake\rain.exe", true)),
        )
        .await;
    let health: Health = backend.get("/health").await;
    assert!(health.sources.iter().all(|source| source.name != "hotkey"));
    backend.stop().await;
}

// ---- the data folder ----

fn machine_file(backend: &TestBackend) -> std::path::PathBuf {
    backend.data_dir.with_file_name("machine.toml")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_data_folder_is_chosen_copied_into_remembered_and_read_after_the_restart() {
    let mut backend = TestBackend::with_fixture("typical").await;
    let mut stream = open_stream(&backend).await;
    let new_folder = backend.data_dir.with_file_name("moved data");

    let (status, body) = patch(&backend, json!({ "dataDir": new_folder.to_string_lossy() })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["dataDir"], new_folder.to_string_lossy().as_ref());

    // The old files were copied, and the old folder was not touched.
    assert!(new_folder.join("settings.toml").is_file());
    assert!(backend.data_dir.join("settings.toml").is_file());
    let machine = std::fs::read_to_string(machine_file(&backend)).unwrap();
    assert!(machine.contains("moved data"), "{machine}");

    // After a short delay the backend stops with the reason `restart`.
    let rest = stream.rest().await;
    let (event, frame) = rest
        .last()
        .expect("the stream ended with a lifecycle frame");
    assert_eq!(event, "lifecycle");
    assert_eq!(
        frame.payload,
        json!({ "kind": "stopping", "reason": "restart" })
    );

    backend.restart_following_machine_file().await;
    let settings: Value = backend.get("/settings").await;
    assert_eq!(settings["dataDir"], new_folder.to_string_lossy().as_ref());
    assert_eq!(
        settings["resetAfterSeconds"], 300,
        "the copied settings are read"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_data_folder_that_overlaps_the_current_one_is_refused_and_nothing_is_written() {
    let backend = TestBackend::with_fixture("typical").await;
    let inside = backend.data_dir.join("inner");
    for folder in [backend.data_dir.clone(), inside] {
        let (status, body) = patch(&backend, json!({ "dataDir": folder.to_string_lossy() })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["reason"], "nested");
    }
    let (status, body) = patch(&backend, json!({ "dataDir": "relative" })).await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::BAD_REQUEST, &json!("badPath"))
    );
    assert!(!machine_file(&backend).exists());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_data_folder_is_not_changed_while_the_environment_decides_work_runs_or_game_mode_runs()
{
    let env_backend = TestBackend::builder()
        .config(|config| config.data_dir_from_env = true)
        .start()
        .await;
    let target = env_backend.data_dir.with_file_name("elsewhere");
    let (status, body) = patch(&env_backend, json!({ "dataDir": target.to_string_lossy() })).await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::CONFLICT, &json!("environment"))
    );
    env_backend.stop().await;

    let backend = TestBackend::start().await;
    let target = backend.data_dir.with_file_name("elsewhere");
    let body = json!({ "dataDir": target.to_string_lossy() });

    let busy = backend.state.activity.guard();
    let (status, answer) = patch(&backend, body.clone()).await;
    assert_eq!(
        (status, &answer["reason"]),
        (StatusCode::CONFLICT, &json!("busy"))
    );
    drop(busy);

    backend.state.set_game_mode_active(true);
    let (status, answer) = patch(&backend, body).await;
    assert_eq!(
        (status, &answer["reason"]),
        (StatusCode::CONFLICT, &json!("gameMode"))
    );
    backend.state.set_game_mode_active(false);
    assert!(!machine_file(&backend).exists());
    backend.stop().await;
}

// ---- the rules for stopping ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upgrade_is_refused_while_work_runs_and_the_backend_keeps_running() {
    let backend = TestBackend::start().await;
    let busy = backend.state.activity.guard();

    let (status, body) = backend
        .send(
            "POST",
            "/internal/shutdown",
            Some(json!({ "reason": "upgrade" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "busy");
    let ping: Value = backend.get("/ping").await;
    assert_eq!(ping["busy"], true, "still answering");
    assert!(!backend.state.shutdown.shutting_down());

    drop(busy);
    backend.request_stop("upgrade").await;
    backend.wait_stopped().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_and_the_reasons_stop_and_game_mode_end_a_busy_backend() {
    for body in [
        json!({ "reason": "restart", "force": true }),
        json!({ "reason": "stop" }),
        json!({ "reason": "gameMode" }),
    ] {
        let backend = TestBackend::start().await;
        let _busy = backend.state.activity.guard();
        let (status, answer) = backend
            .send("POST", "/internal/shutdown", Some(body.clone()))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body} -> {answer}");
        backend.wait_stopped().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_is_refused_while_game_mode_runs() {
    let backend = TestBackend::start().await;
    backend.state.set_game_mode_active(true);
    let (status, body) = backend
        .send(
            "POST",
            "/internal/shutdown",
            Some(json!({ "reason": "restart" })),
        )
        .await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::CONFLICT, &json!("gameMode"))
    );
    backend.state.set_game_mode_active(false);
    backend.stop().await;
}
