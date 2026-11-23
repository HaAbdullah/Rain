//! `GET /settings`, `PATCH /settings` and `PUT /internal/shell`, on a real backend with a real
//! data folder (foundation 14.1, stage 1b; feature section 06).

mod common;

use std::fs;

use common::{StreamReader, TestBackend};
use hyper::StatusCode;
use rain_types::{Health, Settings};
use serde_json::{Value, json};

/// What a backend answers when the data folder has no `settings.toml` and the PC has nothing
/// that Rain detects (the harness's `FakeDetect`).
fn default_settings(backend: &TestBackend) -> Value {
    json!({
        "hotkey": "Ctrl+Backquote",
        "hotkeyAvailable": true,
        "hideOnBlur": true,
        "resetAfterSeconds": 60,
        "startWithWindows": false,
        "dataDir": backend.data_dir.to_string_lossy(),
        "vaultPath": "",
        "dailyNoteFormat": "YYYY-MM-DD",
        "dailyNoteFolder": "",
        "projectRoots": [],
        "gameFolders": [],
        "steamEnabled": true,
        "steamPath": "",
        "epicEnabled": true,
        "epicPath": "",
        "calendarConnected": false,
        "monitorClaude": false,
        "monitorClaudeState": "off",
        "gameModeCountdownSeconds": 10
    })
}

fn settings_path(backend: &TestBackend) -> std::path::PathBuf {
    backend.data_dir.join("settings.toml")
}

/// The text of `settings.toml` with LF line ends, whatever the checkout turned the fixture into.
fn settings_file(backend: &TestBackend) -> String {
    fs::read_to_string(settings_path(backend))
        .expect("settings.toml exists")
        .replace("\r\n", "\n")
}

async fn get_settings(backend: &TestBackend) -> Value {
    let (status, body) = backend.send("GET", "/settings", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn patch(backend: &TestBackend, body: Value) -> (StatusCode, Value) {
    backend.send("PATCH", "/settings", Some(body)).await
}

/// Reads the two frames that start every stream.
async fn open_stream(backend: &TestBackend) -> StreamReader {
    let mut stream = backend.stream().await;
    assert_eq!(stream.expect_event().await.0, "hello");
    assert_eq!(stream.expect_event().await.0, "resync");
    stream
}

#[track_caller]
fn assert_refused(
    (status, body): (StatusCode, Value),
    expected_status: StatusCode,
    code: &str,
    reason: &str,
) {
    assert_eq!(status, expected_status, "body: {body}");
    assert_eq!(body["code"], code, "body: {body}");
    assert_eq!(body["reason"], reason, "body: {body}");
    assert_eq!(body["source"], "settings", "body: {body}");
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
}

// ---- reading ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_file_the_settings_are_the_defaults_and_nothing_is_written() {
    let backend = TestBackend::start().await;

    assert_eq!(get_settings(&backend).await, default_settings(&backend));

    assert!(
        backend.data_dir.is_dir(),
        "the data folder was made at start"
    );
    assert!(!settings_path(&backend).exists(), "reading writes nothing");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_answer_has_the_type_of_the_contract() {
    let backend = TestBackend::start().await;
    let settings: Settings = backend.get("/settings").await;
    assert_eq!(settings.hotkey, "Ctrl+Backquote");
    assert!(settings.hotkey_available && !settings.calendar_connected);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fixture_file_is_read_and_unknown_keys_are_ignored() {
    let backend = TestBackend::with_fixture("typical").await;
    let settings = get_settings(&backend).await;

    assert_eq!(settings["hideOnBlur"], false);
    assert_eq!(settings["resetAfterSeconds"], 300);
    assert_eq!(
        settings["projectRoots"],
        json!([r"D:\Projects", r"D:\Work"])
    );
    assert_eq!(settings["steamEnabled"], false);
    assert_eq!(settings["gameModeCountdownSeconds"], 20);
    assert_eq!(
        settings["epicEnabled"], true,
        "a missing key is its default"
    );
    backend.stop().await;
}

// ---- changing ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_is_answered_with_the_new_state_and_can_be_read_back() {
    let backend = TestBackend::start().await;

    let (status, answer) = patch(
        &backend,
        json!({
            "hideOnBlur": false,
            "resetAfterSeconds": 300,
            "projectRoots": [r"D:\Code", r"D:\Work"],
            "vaultPath": r"D:\Vault",
            "dailyNoteFormat": "YYYY/MM/DD",
            "gameModeCountdownSeconds": 30
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    let mut expected = default_settings(&backend);
    expected["hideOnBlur"] = json!(false);
    expected["resetAfterSeconds"] = json!(300);
    expected["projectRoots"] = json!([r"D:\Code", r"D:\Work"]);
    expected["vaultPath"] = json!(r"D:\Vault");
    expected["dailyNoteFormat"] = json!("YYYY/MM/DD");
    expected["gameModeCountdownSeconds"] = json!(30);
    assert_eq!(answer, expected);
    assert_eq!(
        get_settings(&backend).await,
        expected,
        "the same when read back"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_write_creates_a_file_with_a_header_and_readable_paths() {
    let backend = TestBackend::start().await;
    patch(
        &backend,
        json!({ "projectRoots": [r"D:\Code"], "resetAfterSeconds": 30 }),
    )
    .await;

    let text = settings_file(&backend);
    assert!(text.starts_with("# Rain settings.\n"), "{text}");
    assert!(
        text.ends_with(
            "schema_version = 1\nreset_after_seconds = 30\nproject_roots = ['D:\\Code']\n"
        ),
        "{text}"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_changes_only_its_keys_and_keeps_the_comments_of_the_user() {
    let backend = TestBackend::with_fixture("typical").await;

    patch(
        &backend,
        json!({ "resetAfterSeconds": 60, "epicEnabled": false }),
    )
    .await;

    let text = settings_file(&backend);
    assert!(text.contains("# a comment above a key\nhide_on_blur = false          # a trailing comment that must survive\n"));
    assert!(text.contains("reset_after_seconds = 60\n"));
    assert!(
        text.contains("monitor_claude = true"),
        "a key of a later stage stays"
    );
    assert!(text.ends_with("epic_enabled = false\n"));
    assert!(text.starts_with("# Rain settings (made-up values for the tests).\n"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_text_clears_an_override_and_removes_its_key() {
    let backend = TestBackend::start().await;
    patch(
        &backend,
        json!({ "vaultPath": r"D:\Vault", "steamPath": r"D:\Steam" }),
    )
    .await;
    assert!(settings_file(&backend).contains("vault_path = 'D:\\Vault'"));

    let (_, answer) = patch(&backend, json!({ "vaultPath": "" })).await;

    assert_eq!(answer["vaultPath"], "");
    assert_eq!(answer["steamPath"], r"D:\Steam");
    let text = settings_file(&backend);
    // The header comment explains the key, so only the lines that are not comments count.
    let keys: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(keys, ["schema_version = 1", "steam_path = 'D:\\Steam'"]);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_that_changes_nothing_writes_nothing() {
    let backend = TestBackend::with_fixture("typical").await;
    let before = fs::read_to_string(settings_path(&backend)).unwrap();
    let modified = fs::metadata(settings_path(&backend))
        .unwrap()
        .modified()
        .unwrap();

    let (status, answer) = patch(&backend, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["resetAfterSeconds"], 300);
    let (status, _) = patch(
        &backend,
        json!({ "resetAfterSeconds": 300, "hideOnBlur": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(fs::read_to_string(settings_path(&backend)).unwrap(), before);
    assert_eq!(
        fs::metadata(settings_path(&backend))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_list_drops_empty_entries_and_duplicates() {
    let backend = TestBackend::start().await;
    let (_, answer) = patch(
        &backend,
        json!({ "projectRoots": ["D:/Code/", "", "  ", r"d:\code", r"D:\Work"] }),
    )
    .await;
    assert_eq!(answer["projectRoots"], json!(["D:/Code/", r"D:\Work"]));
    backend.stop().await;
}

// ---- persistence ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_survives_a_restart_of_the_backend() {
    let mut backend = TestBackend::start().await;
    let first_start = backend.state.start_id.clone();
    patch(
        &backend,
        json!({ "resetAfterSeconds": 300, "projectRoots": [r"D:\Code"] }),
    )
    .await;

    backend.restart().await;

    assert_ne!(
        backend.state.start_id, first_start,
        "a new backend process start"
    );
    let settings = get_settings(&backend).await;
    assert_eq!(settings["resetAfterSeconds"], 300);
    assert_eq!(settings["projectRoots"], json!([r"D:\Code"]));
    backend.stop().await;
}

// ---- refused patches ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_the_types_refuse_is_invalid_with_reason_bad_body() {
    let backend = TestBackend::start().await;
    let bodies = [
        json!({ "resetAfterSeconds": 45 }),
        json!({ "gameModeCountdownSeconds": 15 }),
        json!({ "hideOnBlur": "yes" }),
        json!({ "noSuchSetting": 1 }),
        json!({ "hotkey": "Ctrl+K" }),
        json!({ "hotkeyAvailable": false }),
        json!({ "calendarConnected": true }),
    ];
    for body in bodies {
        let (status, answer) = patch(&backend, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer["code"], "invalid", "{body}");
        assert_eq!(answer["reason"], "badBody", "{body}");
        assert!(
            answer["message"]
                .as_str()
                .unwrap()
                .starts_with("The request was not valid: "),
            "{answer}"
        );
    }
    let broken_json = backend
        .send_bytes("PATCH", "/settings", Some("{ not json".into()))
        .await;
    assert_eq!(broken_json.0, StatusCode::BAD_REQUEST);
    assert_eq!(broken_json.1["reason"], "badBody");
    assert!(
        !settings_path(&backend).exists(),
        "a refused patch writes nothing"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rules_of_the_values_each_have_their_error() {
    let backend = TestBackend::start().await;
    let many_folders: Vec<String> = (0..33).map(|n| format!(r"D:\Code{n}")).collect();
    let cases = [
        (json!({ "vaultPath": "x".repeat(1025) }), "tooLong"),
        (json!({ "projectRoots": ["x".repeat(1025)] }), "tooLong"),
        (json!({ "steamPath": "a\nb" }), "badText"),
        (json!({ "epicPath": "a\u{1}b" }), "badText"),
        (json!({ "projectRoots": many_folders }), "tooMany"),
        (json!({ "dailyNoteFormat": "YYYY:MM" }), "badFormat"),
        (json!({ "dailyNoteFormat": "Y".repeat(65) }), "badFormat"),
        (
            json!({ "startWithWindows": true, "hideOnBlur": false }),
            "onePerRequest",
        ),
    ];
    for (body, reason) in cases {
        let answer = patch(&backend, body.clone()).await;
        assert_refused(answer, StatusCode::BAD_REQUEST, "invalid", reason);
    }
    assert!(
        !settings_path(&backend).exists(),
        "refused patches write nothing"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_with_windows_is_refused_in_a_sandbox_run() {
    let backend = TestBackend::start().await;
    for value in [true, false] {
        let answer = patch(&backend, json!({ "startWithWindows": value })).await;
        // `lifecycle` raises it, so the source is not `settings`.
        let (status, body) = answer;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["reason"], "sandbox", "{body}");
    }
    assert_eq!(get_settings(&backend).await["startWithWindows"], false);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_is_one_unit_when_one_of_its_values_is_refused() {
    let backend = TestBackend::start().await;
    let answer = patch(
        &backend,
        json!({ "hideOnBlur": false, "vaultPath": "a\nb" }),
    )
    .await;
    assert_refused(answer, StatusCode::BAD_REQUEST, "invalid", "badText");
    assert_eq!(get_settings(&backend).await["hideOnBlur"], true);
    backend.stop().await;
}

// ---- the file edited by hand ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_is_picked_up_by_the_next_read() {
    let backend = TestBackend::with_fixture("typical").await;
    assert_eq!(get_settings(&backend).await["resetAfterSeconds"], 300);

    let text = fs::read_to_string(settings_path(&backend)).unwrap();
    // The new text is longer, so its length differs even when the clock has not moved on.
    let edited = text.replace(
        "reset_after_seconds = 300",
        "reset_after_seconds = 30\nsteam_path = 'D:\\Steam'",
    );
    fs::write(settings_path(&backend), edited).unwrap();

    let settings = get_settings(&backend).await;
    assert_eq!(settings["resetAfterSeconds"], 30);
    assert_eq!(settings["steamPath"], r"D:\Steam");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_between_two_patches_is_not_overwritten() {
    let backend = TestBackend::start().await;
    patch(&backend, json!({ "hideOnBlur": false })).await;

    let text = fs::read_to_string(settings_path(&backend)).unwrap();
    fs::write(
        settings_path(&backend),
        format!("{text}# my note\nepic_path = 'D:\\Epic'\n"),
    )
    .unwrap();
    let (_, answer) = patch(&backend, json!({ "steamEnabled": false })).await;

    assert_eq!(
        answer["epicPath"], r"D:\Epic",
        "the edit of the user was kept"
    );
    let text = settings_file(&backend);
    assert!(
        text.contains("# my note\nepic_path = 'D:\\Epic'\n"),
        "{text}"
    );
    assert!(text.contains("steam_enabled = false"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_that_breaks_the_file_keeps_the_last_good_settings_and_blocks_patches() {
    let backend = TestBackend::start().await;
    patch(&backend, json!({ "resetAfterSeconds": 300 })).await;
    let good = settings_file(&backend);

    fs::write(
        settings_path(&backend),
        format!("{good}steam_enabled = [\n"),
    )
    .unwrap();

    assert_eq!(get_settings(&backend).await["resetAfterSeconds"], 300);
    let health: Health = backend.get("/health").await;
    assert_eq!(health.config_issues.len(), 1);
    assert_eq!(health.config_issues[0].file, "settings.toml");
    let line = health.config_issues[0].line.expect("the line of the error");
    assert_eq!(line as usize, good.lines().count() + 1);

    let answer = patch(&backend, json!({ "hideOnBlur": false })).await;
    assert_refused(
        answer.clone(),
        StatusCode::CONFLICT,
        "conflict",
        "configBroken",
    );
    assert_eq!(
        answer.1["message"],
        format!("settings.toml has an error on line {line}. Fix the file, then try again.")
    );
    assert!(
        fs::read_to_string(settings_path(&backend))
            .unwrap()
            .contains("steam_enabled = [\n")
    );

    // The user fixes the file: the issue goes and a patch works again.
    fs::write(settings_path(&backend), format!("{good}# fixed\n")).unwrap();
    assert_eq!(get_settings(&backend).await["resetAfterSeconds"], 300);
    assert!(
        backend
            .get::<Health>("/health")
            .await
            .config_issues
            .is_empty()
    );
    assert_eq!(
        patch(&backend, json!({ "hideOnBlur": false })).await.0,
        StatusCode::OK
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_value_outside_a_number_union_in_the_file_is_an_issue_with_its_line() {
    let backend = TestBackend::start().await;
    fs::write(
        settings_path(&backend),
        "schema_version = 1\nreset_after_seconds = 45\n",
    )
    .unwrap();

    assert_eq!(
        get_settings(&backend).await["resetAfterSeconds"],
        60,
        "defaults"
    );
    let health: Health = backend.get("/health").await;
    assert_eq!(health.config_issues[0].line, Some(2));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_from_a_newer_rain_is_read_but_never_written() {
    let backend = TestBackend::start().await;
    let text = "schema_version = 2\nreset_after_seconds = 30\n";
    fs::write(settings_path(&backend), text).unwrap();

    let answer = patch(&backend, json!({ "hideOnBlur": false })).await;
    assert_refused(answer, StatusCode::CONFLICT, "conflict", "configNewer");
    assert_eq!(fs::read_to_string(settings_path(&backend)).unwrap(), text);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_file_at_start_means_defaults_and_an_issue() {
    let backend = TestBackend::with_fixture("broken").await;

    assert_eq!(get_settings(&backend).await, default_settings(&backend));
    let health: Health = backend.get("/health").await;
    assert_eq!(health.config_issues.len(), 1);
    assert_eq!(health.config_issues[0].file, "settings.toml");
    assert_eq!(health.config_issues[0].line, Some(3));

    let answer = patch(&backend, json!({ "hideOnBlur": false })).await;
    assert_refused(answer, StatusCode::CONFLICT, "conflict", "configBroken");
    backend.stop().await;
}

// ---- what the shell tells the backend ----

fn shell_facts(hotkey_registered: bool) -> Value {
    json!({
        "hotkeyRegistered": hotkey_registered,
        "shellExe": "rain.exe",
        "shellVersion": "0.1.0",
        "windowVisible": false
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hotkey_is_available_until_a_shell_says_it_is_not() {
    let backend = TestBackend::start().await;
    assert_eq!(get_settings(&backend).await["hotkeyAvailable"], true);
    assert!(!backend.get::<Health>("/health").await.shell.reported);

    let (status, _) = backend
        .send("PUT", "/internal/shell", Some(shell_facts(false)))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(get_settings(&backend).await["hotkeyAvailable"], false);
    let shell = backend.get::<Health>("/health").await.shell;
    assert!(shell.reported);
    assert_eq!(shell.hotkey_registered, Some(false));

    backend
        .send("PUT", "/internal/shell", Some(shell_facts(true)))
        .await;
    assert_eq!(get_settings(&backend).await["hotkeyAvailable"], true);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_shell_facts_are_not_a_setting_and_are_not_written_to_the_file() {
    let backend = TestBackend::start().await;
    backend
        .send("PUT", "/internal/shell", Some(shell_facts(false)))
        .await;
    assert!(!settings_path(&backend).exists());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_shell_report_is_invalid() {
    let backend = TestBackend::start().await;
    for body in [
        json!({ "hotkeyRegistered": "yes", "shellExe": "x", "shellVersion": "1", "windowVisible": false }),
        json!({ "shellExe": "x" }),
        json!({ "hotkeyRegistered": true, "shellExe": "x", "shellVersion": "1", "windowVisible": false, "extra": 1 }),
    ] {
        let (status, answer) = backend
            .send("PUT", "/internal/shell", Some(body.clone()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer["reason"], "badBody");
    }
    assert!(!backend.get::<Health>("/health").await.shell.reported);
    backend.stop().await;
}

// ---- the stream ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_is_published_on_the_topic_settings() {
    let backend = TestBackend::start().await;
    let mut stream = open_stream(&backend).await;

    let (_, answer) = patch(&backend, json!({ "resetAfterSeconds": 300 })).await;

    let frame = stream.expect_topic("settings").await;
    assert_eq!(frame.payload, answer, "the frame is the whole new state");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_that_changes_nothing_publishes_nothing() {
    let backend = TestBackend::start().await;
    patch(&backend, json!({ "resetAfterSeconds": 300 })).await;
    let mut stream = open_stream(&backend).await;

    patch(&backend, json!({ "resetAfterSeconds": 300 })).await; // nothing changes
    patch(&backend, json!({ "resetAfterSeconds": 30 })).await;

    let frame = stream.expect_topic("settings").await;
    assert_eq!(
        frame.payload["resetAfterSeconds"], 30,
        "the first frame is the real change"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_that_a_read_takes_in_is_published_too() {
    let backend = TestBackend::start().await;
    patch(&backend, json!({ "resetAfterSeconds": 300 })).await;
    let mut stream = open_stream(&backend).await;

    let text = fs::read_to_string(settings_path(&backend)).unwrap();
    fs::write(
        settings_path(&backend),
        format!("{text}epic_enabled = false\n"),
    )
    .unwrap();
    get_settings(&backend).await;

    let frame = stream.expect_topic("settings").await;
    assert_eq!(frame.payload["epicEnabled"], false);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_shell_report_is_published_on_the_topics_health_and_settings() {
    let backend = TestBackend::start().await;
    let mut stream = open_stream(&backend).await;

    backend
        .send("PUT", "/internal/shell", Some(shell_facts(false)))
        .await;

    let health = stream.expect_topic("health").await;
    assert_eq!(
        health.payload["shell"],
        json!({ "reported": true, "hotkeyRegistered": false })
    );
    let settings = stream.expect_topic("settings").await;
    assert_eq!(settings.payload["hotkeyAvailable"], false);

    // The same report again is no news; a report that changes the hotkey is.
    backend
        .send("PUT", "/internal/shell", Some(shell_facts(false)))
        .await;
    backend
        .send("PUT", "/internal/shell", Some(shell_facts(true)))
        .await;
    // The hotkey source of `GET /health` changes too, so more than one health frame arrives.
    loop {
        let health = stream.expect_topic("health").await;
        let registered = health.payload["shell"]["hotkeyRegistered"] == true;
        let hotkey_source_left = health.payload["sources"]
            .as_array()
            .is_some_and(|sources| sources.iter().any(|source| source["name"] == "hotkey"));
        if registered && !hotkey_source_left {
            break; // the hotkey source is gone again
        }
    }
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shell_report_that_does_not_touch_the_hotkey_publishes_no_settings() {
    let backend = TestBackend::start().await;
    backend
        .send("PUT", "/internal/shell", Some(shell_facts(true)))
        .await;
    let mut stream = open_stream(&backend).await;

    let mut facts = shell_facts(true);
    facts["windowVisible"] = json!(true);
    backend.send("PUT", "/internal/shell", Some(facts)).await;
    patch(&backend, json!({ "resetAfterSeconds": 30 })).await;

    let frame = stream.expect_topic("settings").await;
    assert_eq!(
        frame.payload["resetAfterSeconds"], 30,
        "no settings frame came before the patch"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_problem_found_in_the_file_is_published_on_the_topic_health() {
    let backend = TestBackend::start().await;
    let mut stream = open_stream(&backend).await;

    fs::write(settings_path(&backend), "hide_on_blur = [\n").unwrap();
    get_settings(&backend).await;

    let frame = stream.expect_topic("health").await;
    assert_eq!(frame.payload["configIssues"][0]["file"], "settings.toml");
    assert_eq!(frame.payload["configIssues"][0]["line"], 1);
    backend.stop().await;
}
