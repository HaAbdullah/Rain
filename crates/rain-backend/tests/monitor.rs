//! The switch "Monitor Claude sessions": the hook installer behind `PATCH /settings`, and what the
//! backend finds at start (section 03 6.9, stage 8b).
//!
//! ABSOLUTE RULE: nothing here touches the real Claude folder. Every test gives the backend a
//! throwaway folder through `Config.claude_dir`, and a made-up hook program.

#[path = "common/claude.rs"]
mod claude;
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use claude::{ClaudeHome, FakeForwarder, event_envelope};
use common::TestBackend;
use rain_backend::modules::sessions::ForwarderProgram;
use rain_types::{Health, SourceState};
use serde_json::{Value, json};

/// A settings file of a user, with other settings and a hook of their own, written the way Claude
/// Code writes it (two-space indent). A file in another layout is re-indented by an install, which
/// is a documented limit; the byte-for-byte round trip holds for this layout.
const USER_SETTINGS: &str = "{\n  \"theme\": \"dark\",\n  \"hooks\": {\n    \"PermissionRequest\": [\n      {\n        \"matcher\": \"\",\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"C:\\\\tools\\\\their-hook.exe\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";

struct Setup {
    backend: TestBackend,
    home: ClaudeHome,
    forwarder: Arc<FakeForwarder>,
}

async fn setup(settings_toml: Option<&str>, claude_file: Option<&str>) -> Setup {
    let home = ClaudeHome::without_registry();
    if let Some(text) = claude_file {
        std::fs::write(home.settings_file(), text).expect("a settings file");
    }
    let forwarder = FakeForwarder::new(home.temp.path(), b"made-up bytes of the hook program");
    let root = home.root();
    let mut builder = TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .with::<dyn ForwarderProgram>(forwarder.clone());
    if let Some(text) = settings_toml {
        builder = builder.file("settings.toml", text);
    }
    Setup {
        backend: builder.start().await,
        home,
        forwarder,
    }
}

async fn patch(backend: &TestBackend, body: Value) -> (StatusCode, Value) {
    backend.send("PATCH", "/settings", Some(body)).await
}

async fn settings(backend: &TestBackend) -> Value {
    backend.get("/settings").await
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("a readable file")
}

fn rain_entries(text: &str) -> usize {
    text.matches("--rain-managed=").count()
}

fn backups(backend: &TestBackend) -> Vec<PathBuf> {
    let dir = backend.local_dir.join("backups").join("claude-settings");
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.map(|e| e.expect("a folder entry").path()).collect(),
        Err(_) => Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_install_remove_in_temp() {
    let s = setup(None, Some(USER_SETTINGS)).await;
    let b = &s.backend;
    assert_eq!(settings(b).await["monitorClaudeState"], "off");

    // On: 12 entries, the forwarder copy, one backup of the original.
    let (status, answer) = patch(b, json!({ "monitorClaude": true })).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        (&answer["monitorClaude"], &answer["monitorClaudeState"]),
        (&json!(true), &json!("onInactive"))
    );
    let installed = read(&s.home.settings_file());
    assert_eq!(rain_entries(&installed), 12);
    assert!(
        installed.contains("their-hook.exe"),
        "the user's own hook stays"
    );
    assert!(
        installed.contains("--pipe="),
        "a backend that is not the default one tells the hook its pipe"
    );
    let forwarder = b.local_dir.join("bin").join("rain-hook.exe");
    assert_eq!(
        std::fs::read(&forwarder).unwrap(),
        b"made-up bytes of the hook program"
    );
    assert!(
        installed.contains(&forwarder.to_string_lossy().replace('\\', "\\\\")),
        "the entries point at the copy"
    );
    let saved = backups(b);
    assert_eq!(saved.len(), 1);
    assert_eq!(read(&saved[0]), USER_SETTINGS);
    assert!(read(&b.data_dir.join("settings.toml")).contains("monitor_claude = true"));

    // The first event makes the state "on".
    let (status, _) = b
        .send(
            "POST",
            "/internal/hook",
            Some(event_envelope("UserPromptSubmit", "s-1", json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(settings(b).await["monitorClaudeState"], "on");
    let health: Health = b.get("/health").await;
    assert_eq!(
        health
            .sources
            .iter()
            .find(|x| x.name == "claude-hooks")
            .unwrap()
            .state,
        SourceState::Ok
    );

    // Switching on again changes nothing (a repair that has nothing to repair).
    let before = read(&s.home.settings_file());
    let (status, _) = patch(b, json!({ "monitorClaude": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read(&s.home.settings_file()), before);

    // Off: the file is what it was, byte for byte. The program stays, so a running session that
    // loaded the hooks finds it.
    let (status, answer) = patch(b, json!({ "monitorClaude": false })).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["monitorClaudeState"], "off");
    assert_eq!(read(&s.home.settings_file()), USER_SETTINGS);
    assert!(forwarder.is_file());
    // With the switch off an event is ignored.
    let (status, _) = b
        .send(
            "POST",
            "/internal/hook",
            Some(event_envelope("UserPromptSubmit", "s-2", json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let list: Value = b.get("/sessions").await;
    assert!(list.as_array().unwrap().iter().all(|x| x["id"] != "s-2"));
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_replaces_a_forwarder_whose_content_differs() {
    let s = setup(None, Some("{}\n")).await;
    let b = &s.backend;
    patch(b, json!({ "monitorClaude": true })).await;
    let target = b.local_dir.join("bin").join("rain-hook.exe");
    std::fs::write(&s.forwarder.source, b"a newer program, which is longer").unwrap();
    // The state tells that the copy is out of date.
    assert_eq!(settings(b).await["monitorClaudeState"], "needsRepair");
    let (status, answer) = patch(b, json!({ "monitorClaude": true })).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"a newer program, which is longer"
    );
    assert_eq!(settings(b).await["monitorClaudeState"], "onInactive");
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_refusals_keep_the_file_and_the_switch() {
    let s = setup(None, None).await;
    let b = &s.backend;
    let cases = [
        (
            r#"{"disableAllHooks": true}"#,
            StatusCode::CONFLICT,
            "hooksDisabled",
            "Claude hooks are switched off in your Claude settings (disableAllHooks). Turn them on first.",
        ),
        (
            "{ not json",
            StatusCode::INTERNAL_SERVER_ERROR,
            "installFailed",
            "Your Claude settings file is not valid JSON. Rain did not change it.",
        ),
        (
            r#"{"hooks": 5}"#,
            StatusCode::INTERNAL_SERVER_ERROR,
            "installFailed",
            "Your Claude settings file has an unexpected shape in its hooks section. Rain did not change it.",
        ),
    ];
    for (file, expected_status, reason, message) in cases {
        std::fs::write(s.home.settings_file(), file).unwrap();
        let (status, answer) = patch(b, json!({ "monitorClaude": true })).await;
        assert_eq!(
            (status, &answer["reason"]),
            (expected_status, &json!(reason)),
            "{file} -> {answer}"
        );
        assert_eq!(answer["message"], message);
        assert_eq!(read(&s.home.settings_file()), file, "the file is untouched");
        assert_eq!(
            settings(b).await["monitorClaude"],
            false,
            "the switch keeps its old value"
        );
        assert!(
            !b.data_dir.join("settings.toml").exists(),
            "nothing was written to settings.toml"
        );
    }
    assert_eq!(rain_entries(&read(&s.home.settings_file())), 0);

    // A hook program that does not start.
    std::fs::write(s.home.settings_file(), "{}\n").unwrap();
    *s.forwarder.starts.lock().unwrap() = false;
    let (status, answer) = patch(b, json!({ "monitorClaude": true })).await;
    assert_eq!(
        (status, &answer["reason"]),
        (StatusCode::INTERNAL_SERVER_ERROR, &json!("hookMissing")),
        "{answer}"
    );
    assert_eq!(
        answer["message"],
        "The Rain hook program was not found or did not start."
    );
    assert_eq!(read(&s.home.settings_file()), "{}\n");
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_without_a_claude_folder_says_so() {
    // The harness gives every test a Claude folder that does not exist.
    let backend = TestBackend::start().await;
    let (status, answer) = patch(&backend, json!({ "monitorClaude": true })).await;
    assert_eq!(
        (status, &answer["reason"]),
        (StatusCode::SERVICE_UNAVAILABLE, &json!("noClaude")),
        "{answer}"
    );
    assert_eq!(answer["message"], "Claude Code was not found on this PC.");
    // Switching off has nothing to remove and is fine.
    let (status, _) = patch(&backend, json!({ "monitorClaude": false })).await;
    assert_eq!(status, StatusCode::OK);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_sandbox_refuses_real_folder() {
    // A folder that is neither inside the temporary folder nor in the data or local folder of the
    // run stands for the real one. It does not exist, and nothing may create it.
    let real_looking = PathBuf::from(r"Z:\no-such-drive\Claude-real");
    let target = real_looking.clone();
    let backend = TestBackend::builder()
        .file(
            "settings.toml",
            "schema_version = 1\nmonitor_claude = true\n",
        )
        .config(move |config| config.claude_dir = Some(target.clone()))
        .start()
        .await;
    for on in [true, false] {
        let (status, answer) = patch(&backend, json!({ "monitorClaude": on })).await;
        assert_eq!(
            (status, &answer["reason"]),
            (StatusCode::CONFLICT, &json!("sandbox")),
            "{answer}"
        );
        assert_eq!(
            answer["message"],
            "A development build never changes your real Claude settings."
        );
    }
    assert!(!real_looking.exists());
    assert!(backend.state.sessions().install_hooks().await.is_err());
    assert!(backend.state.sessions().remove_hooks().await.is_err());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_start_reconcile() {
    // Switch on, hooks missing: needs repair, and nothing is changed at start.
    let s = setup(
        Some("schema_version = 1\nmonitor_claude = true\n"),
        Some(USER_SETTINGS),
    )
    .await;
    assert_eq!(
        settings(&s.backend).await["monitorClaudeState"],
        "needsRepair"
    );
    assert_eq!(read(&s.home.settings_file()), USER_SETTINGS);
    let health: Health = s.backend.get("/health").await;
    let hooks = health
        .sources
        .iter()
        .find(|x| x.name == "claude-hooks")
        .expect("the source is reported");
    assert_eq!(hooks.state, SourceState::Failed);
    assert!(
        hooks
            .message
            .as_deref()
            .is_some_and(|m| m.contains("Repair")),
        "{hooks:?}"
    );

    // The repair is `install_hooks()` again; it ends ok although the stored key is still true.
    s.backend
        .state
        .sessions()
        .install_hooks()
        .await
        .expect("the repair worked");
    assert_eq!(rain_entries(&read(&s.home.settings_file())), 12);
    assert_eq!(
        settings(&s.backend).await["monitorClaudeState"],
        "onInactive"
    );
    // Removing ends the file as it was while the stored key is still true: needs repair again.
    s.backend
        .state
        .sessions()
        .remove_hooks()
        .await
        .expect("the removal worked");
    assert_eq!(read(&s.home.settings_file()), USER_SETTINGS);
    assert_eq!(
        settings(&s.backend).await["monitorClaudeState"],
        "needsRepair"
    );
    s.backend.stop().await;

    // Switch off with Rain's entries present (the owner edited settings.toml by hand): off, and
    // nothing is changed.
    let s = setup(
        Some("schema_version = 1\nmonitor_claude = false\n"),
        Some(USER_SETTINGS),
    )
    .await;
    s.backend.state.sessions().install_hooks().await.unwrap();
    let with_entries = read(&s.home.settings_file());
    assert_eq!(settings(&s.backend).await["monitorClaudeState"], "off");
    assert_eq!(read(&s.home.settings_file()), with_entries);
    s.backend.stop().await;

    // Switch on and hooks switched off by the user: blocked, which is shown as needs repair.
    let s = setup(
        Some("schema_version = 1\nmonitor_claude = true\n"),
        Some(r#"{"disableAllHooks": true}"#),
    )
    .await;
    assert_eq!(
        settings(&s.backend).await["monitorClaudeState"],
        "needsRepair"
    );
    assert_eq!(
        read(&s.home.settings_file()),
        r#"{"disableAllHooks": true}"#
    );
    s.backend.stop().await;
}
