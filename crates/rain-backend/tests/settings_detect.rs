//! What Rain detects on the PC, what the other modules read as `Resolved`, and the settings of
//! the later stages that are stored from stage 2 on. Feature section 06 (6.1, 6.2, table V).
//!
//! The PC is a `FakeDetect`; no test reads the real Obsidian, Steam or Epic.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::TestBackend;
use common::fakes::{FakeClock, FakeDetect};
use hyper::StatusCode;
use rain_backend::modules::settings::{Detect, NoteSettings};
use serde_json::{Value, json};

async fn patch(backend: &TestBackend, body: Value) -> (StatusCode, Value) {
    backend.send("PATCH", "/settings", Some(body)).await
}

fn settings_text(backend: &TestBackend) -> String {
    std::fs::read_to_string(backend.data_dir.join("settings.toml"))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

/// A backend on a PC that has one vault, Steam and Epic.
async fn detecting_backend(detect: &Arc<FakeDetect>) -> TestBackend {
    TestBackend::builder()
        .with::<dyn Detect>(detect.clone())
        .start()
        .await
}

fn pc_with_one_vault() -> Arc<FakeDetect> {
    let detect = Arc::new(FakeDetect::with_vaults(vec![FakeDetect::vault(
        "abc",
        r"C:\Fake Notes",
    )]));
    *detect.steam.lock().expect("lock") = Some(PathBuf::from(r"C:\Fake Steam"));
    *detect.epic.lock().expect("lock") = Some(PathBuf::from(r"C:\Fake Epic"));
    detect.notes.lock().expect("lock").push((
        PathBuf::from(r"C:\Fake Notes"),
        NoteSettings {
            format: Some("DD.MM.YYYY".into()),
            folder: Some("Dailys".into()),
        },
    ));
    detect
}

// ---- detection ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_only_vault_steam_epic_and_the_daily_note_settings_are_shown_without_being_stored() {
    let backend = detecting_backend(&pc_with_one_vault()).await;
    let settings: Value = backend.get("/settings").await;

    assert_eq!(settings["vaultPath"], r"C:\Fake Notes");
    assert_eq!(settings["dailyNoteFormat"], "DD.MM.YYYY");
    assert_eq!(settings["dailyNoteFolder"], "Dailys");
    assert_eq!(settings["steamPath"], r"C:\Fake Steam");
    assert_eq!(settings["epicPath"], r"C:\Fake Epic");
    assert!(
        !backend.data_dir.join("settings.toml").exists(),
        "detected values are not written"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn several_vaults_and_no_choice_mean_no_vault_and_a_choice_reads_that_vault() {
    let detect = Arc::new(FakeDetect::with_vaults(vec![
        FakeDetect::vault("one", r"C:\One"),
        FakeDetect::vault("two", r"C:\Two"),
    ]));
    detect.notes.lock().unwrap().push((
        PathBuf::from(r"C:\Two"),
        NoteSettings {
            format: None,
            folder: Some("Journal".into()),
        },
    ));
    let backend = detecting_backend(&detect).await;

    let settings: Value = backend.get("/settings").await;
    assert_eq!(settings["vaultPath"], "");
    assert_eq!(settings["dailyNoteFolder"], "");
    assert_eq!(backend.state.settings().resolved().vault, None);
    assert_eq!(backend.state.settings().vault_id(), None);

    let (_, chosen) = patch(&backend, json!({ "vaultPath": r"C:\Two" })).await;
    assert_eq!(chosen["vaultPath"], r"C:\Two");
    assert_eq!(
        chosen["dailyNoteFolder"], "Journal",
        "the notes settings of the chosen vault"
    );
    assert_eq!(backend.state.settings().vault_id().as_deref(), Some("two"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_override_beats_detection_and_clearing_it_brings_detection_back_and_removes_the_key() {
    let backend = detecting_backend(&pc_with_one_vault()).await;

    let (_, answer) = patch(&backend, json!({ "vaultPath": r"D:\My Vault" })).await;
    assert_eq!(answer["vaultPath"], r"D:\My Vault");
    assert_eq!(
        answer["dailyNoteFormat"], "YYYY-MM-DD",
        "no notes settings for another vault"
    );
    assert!(settings_text(&backend).contains("vault_path = 'D:\\My Vault'"));

    let (_, answer) = patch(&backend, json!({ "vaultPath": "" })).await;
    assert_eq!(answer["vaultPath"], r"C:\Fake Notes");
    assert_eq!(answer["dailyNoteFormat"], "DD.MM.YYYY");
    let keys: Vec<String> = settings_text(&backend)
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(str::to_owned)
        .collect();
    assert_eq!(keys, ["schema_version = 1"], "only the version is left");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_override_of_the_note_format_and_folder_beats_what_the_vault_says() {
    let backend = detecting_backend(&pc_with_one_vault()).await;
    let (_, answer) = patch(&backend, json!({ "dailyNoteFormat": "YYYY/MM/DD" })).await;
    assert_eq!(answer["dailyNoteFormat"], "YYYY/MM/DD");
    let (_, answer) = patch(&backend, json!({ "dailyNoteFolder": "/My Days/" })).await;
    assert_eq!(answer["dailyNoteFolder"], "My Days");
    assert!(settings_text(&backend).contains("daily_note_folder = 'My Days'"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_detected_value_that_appears_later_shows_after_thirty_seconds_or_a_patch() {
    let detect = Arc::new(FakeDetect::default());
    let clock = Arc::new(FakeClock::new(SystemTime::now()));
    let backend = TestBackend::builder()
        .with::<dyn Detect>(detect.clone())
        .clock(clock.clone())
        .start()
        .await;
    let settings: Value = backend.get("/settings").await;
    assert_eq!(settings["steamPath"], "");

    *detect.steam.lock().expect("lock") = Some(PathBuf::from(r"C:\Fake Steam"));
    let cached: Value = backend.get("/settings").await;
    assert_eq!(cached["steamPath"], "", "the answer is kept for 30 seconds");

    clock.advance(Duration::from_secs(31));
    let fresh: Value = backend.get("/settings").await;
    assert_eq!(fresh["steamPath"], r"C:\Fake Steam");
    backend.stop().await;
}

// ---- Resolved ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolved_has_normalised_folders_and_fires_when_the_file_changes() {
    let backend = TestBackend::start().await;
    let settings = backend.state.settings();
    let mut watch = settings.watch();
    watch.borrow_and_update();

    patch(
        &backend,
        json!({ "projectRoots": ["D:/Code/", r"d:\code", r"relative\x", r#""C:\Fake\Library""#] }),
    )
    .await;

    tokio::time::timeout(Duration::from_secs(5), watch.changed())
        .await
        .expect("the watch fired")
        .unwrap();
    let resolved = settings.resolved();
    assert_eq!(
        resolved.project_roots,
        [PathBuf::from(r"D:\Code"), PathBuf::from(r"C:\Fake\Library")],
        "a relative root is dropped from Resolved"
    );
    // The API value keeps what the user typed (minus the duplicate), so the user sees the entry.
    let shown: Value = backend.get("/settings").await;
    assert_eq!(
        shown["projectRoots"],
        json!(["D:/Code/", r"relative\x", r#""C:\Fake\Library""#])
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_changes_resolved_too() {
    let backend = TestBackend::with_fixture("typical").await;
    let settings = backend.state.settings();
    let mut watch = settings.watch();
    watch.borrow_and_update();

    std::fs::write(
        backend.data_dir.join("settings.toml"),
        "schema_version = 1\ngame_folders = ['E:\\Games']\ngame_mode_countdown_seconds = 5\n",
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), watch.changed())
        .await
        .expect("the watch fired after the hand edit")
        .unwrap();

    let resolved = settings.resolved();
    assert_eq!(resolved.game_folders, [PathBuf::from(r"E:\Games")]);
    assert_eq!(resolved.countdown_seconds, 5);
    backend.stop().await;
}

// ---- the settings of the later stages ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_folders_and_the_note_folder_are_stored_with_their_keys() {
    let backend = TestBackend::start().await;
    let (status, answer) = patch(
        &backend,
        json!({ "gameFolders": [r"C:\Games", r"c:/games/", r"D:\Other"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["gameFolders"], json!([r"C:\Games", r"D:\Other"]));
    assert!(settings_text(&backend).contains(r"game_folders = ['C:\Games', 'D:\Other']"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rules_of_table_v_for_the_new_fields() {
    let backend = TestBackend::start().await;
    let cases = [
        (json!({ "dailyNoteFolder": r"C:\Notes" }), "badFolder"),
        (json!({ "dailyNoteFolder": "../outside" }), "badFolder"),
        (json!({ "dailyNoteFolder": "x".repeat(129) }), "badFolder"),
        (json!({ "dailyNoteFolder": "a\nb" }), "badText"),
        (
            json!({ "gameFolders": (0..33).map(|n| format!(r"D:\Games{n}")).collect::<Vec<_>>() }),
            "tooMany",
        ),
        (json!({ "gameFolders": ["a\nb"] }), "badText"),
        (json!({ "dataDir": "x".repeat(1025) }), "tooLong"),
    ];
    for (body, reason) in cases {
        let (status, answer) = patch(&backend, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {answer}");
        assert_eq!(answer["reason"], reason, "{body}");
    }
    assert!(
        !backend.data_dir.join("settings.toml").exists(),
        "nothing was written"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_claude_switch_is_read_from_the_file_and_changing_it_waits_for_its_stage() {
    let backend = TestBackend::with_fixture("typical").await;
    let settings: Value = backend.get("/settings").await;
    assert_eq!(
        settings["monitorClaude"], true,
        "the fixture has monitor_claude = true"
    );
    assert_eq!(
        settings["monitorClaudeState"], "off",
        "the state is computed by section 03"
    );

    for on in [true, false] {
        let (status, answer) = patch(&backend, json!({ "monitorClaude": on })).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
        assert_eq!(answer["reason"], "notBuilt");
    }
    // Nothing was written: the file still says what the fixture said.
    assert!(settings_text(&backend).contains("monitor_claude = true"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_is_refused_as_a_whole_while_the_file_is_broken_even_for_an_effect() {
    let backend = TestBackend::with_fixture("broken").await;
    for body in [
        json!({ "hideOnBlur": false }),
        json!({ "dataDir": "" }),
        json!({ "monitorClaude": true }),
    ] {
        let (status, answer) = patch(&backend, body.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body} -> {answer}");
        assert_eq!(answer["reason"], "configBroken");
    }
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_machine_file_that_cannot_be_used_is_an_issue_in_the_health_report() {
    let backend = TestBackend::builder()
        .config(|config| {
            config.startup_issues.push(rain_types::ConfigIssue {
                file: "machine.toml".into(),
                line: None,
                message:
                    "The data folder D:\\Gone could not be used. Rain uses the default folder."
                        .into(),
            });
        })
        .start()
        .await;
    let health: rain_types::Health = backend.get("/health").await;
    assert_eq!(health.config_issues.len(), 1);
    assert_eq!(health.config_issues[0].file, "machine.toml");
    backend.stop().await;
}
