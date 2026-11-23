//! The sessions module without hooks (section 03, stage 8a): the registry, the transcript, liveness,
//! focus and stop. Every test uses a throwaway Claude folder (`Config.claude_dir`) and a fake of
//! the process table, so nothing reads the real Claude folder or touches a real process, except
//! the one test that says so and starts its own harmless child processes.

#[path = "common/claude.rs"]
mod claude;
mod common;

use std::os::windows::process::CommandExt;
use std::sync::Arc;

use axum::http::StatusCode;
use claude::{ClaudeHome, FakeDesktop, FakeProbe, ManualExits, NoExits};
use common::TestBackend;
use rain_backend::modules::sessions::{ExitSource, ProcessProbe, WindowApi};
use rain_backend::platform::windows_focus::FocusWindow;
use rain_types::{ClaudeSession, Health, SessionHost, SourceState, Status};
use serde_json::{Value, json};

const ID_A: &str = "aaaaaaaa-0000-0000-0000-000000000001";
const ID_B: &str = "bbbbbbbb-0000-0000-0000-000000000002";

async fn start(home: &ClaudeHome, probe: &Arc<FakeProbe>) -> TestBackend {
    let root = home.root();
    TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .with::<dyn ProcessProbe>(probe.clone())
        .with::<dyn ExitSource>(Arc::new(NoExits))
        .start()
        .await
}

async fn sessions(backend: &TestBackend) -> Vec<ClaudeSession> {
    backend.get("/sessions").await
}

fn find<'a>(list: &'a [ClaudeSession], id: &str) -> &'a ClaudeSession {
    list.iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("session {id} is not listed"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_registry_discovery() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    for pid in [4242, 4343, 4646] {
        probe.add(pid, "claude.exe", 1);
    }
    home.write_registry(
        4242,
        ID_A,
        json!({ "status": "busy", "name": "My own name", "nameSource": "user" }),
    );
    home.write_registry(4343, ID_B, json!({ "status": "somethingNew", "entrypoint": "claude-desktop", "cwd": r"C:\work\other" }));
    // A process that does not run: the file stays behind until the next start of `claude`.
    home.write_registry(4747, "gone-session", json!({}));
    // The same pid with a start time that is not the process's: a reused pid.
    home.write_registry(
        4646,
        "reused-session",
        json!({ "procStart": (FakeProbe::start_of(4646) + 20_000_000).to_string() }),
    );
    let sessions_dir = home.root().join("sessions");
    std::fs::write(sessions_dir.join("4545.json"), "{ broken").unwrap();
    // Key files are never opened; this one holds a canary that must never show up anywhere.
    std::fs::write(sessions_dir.join("4242.abcdef0123456789.key"), "CANARY-KEY").unwrap();

    let backend = start(&home, &probe).await;
    let list = sessions(&backend).await;
    assert_eq!(list.len(), 2, "{list:?}");

    let a = find(&list, ID_A);
    assert_eq!(
        (a.status, a.alive, a.host),
        (Status::Running, true, SessionHost::Terminal)
    );
    assert_eq!(a.title, "My own name");
    assert_eq!(a.cwd, r"C:\work\demo");
    assert_eq!(a.started_at, "2026-09-21T14:13:20.000Z");
    assert_eq!(a.model, "unknown");
    assert!(a.request.is_none());

    let b = find(&list, ID_B);
    assert_eq!(b.status, Status::Idle, "an unknown status text is idle");
    assert_eq!(b.host, SessionHost::Desktop);
    assert_eq!(
        b.title, "Demo session",
        "a derived name is used when nothing better exists"
    );

    assert!(!serde_json::to_string(&list).unwrap().contains("CANARY"));
    let health: Health = backend.get("/health").await;
    let source = health
        .sources
        .iter()
        .find(|s| s.name == "claude-registry")
        .expect("the source is reported");
    assert_eq!(source.state, SourceState::Ok);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_without_a_claude_folder_are_an_empty_list_and_an_off_source() {
    // The harness gives every test a Claude folder that does not exist.
    let backend = TestBackend::start().await;
    assert!(sessions(&backend).await.is_empty());
    let health: Health = backend.get("/health").await;
    let source = health
        .sources
        .iter()
        .find(|s| s.name == "claude-registry")
        .expect("the source is reported");
    assert_eq!(source.state, SourceState::Off);
    assert!(
        source
            .message
            .as_deref()
            .is_some_and(|m| m.contains("has not run"))
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_files_that_are_not_understood_make_the_source_failed_and_the_list_empty() {
    let home = ClaudeHome::new();
    std::fs::write(
        home.root().join("sessions").join("1234.json"),
        "{\"format\": \"from the future\"}",
    )
    .unwrap();
    let probe = Arc::new(FakeProbe::default());
    let backend = start(&home, &probe).await;
    assert!(sessions(&backend).await.is_empty());
    let health: Health = backend.get("/health").await;
    let source = health
        .sources
        .iter()
        .find(|s| s.name == "claude-registry")
        .expect("the source is reported");
    assert_eq!(source.state, SourceState::Failed);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_death_and_ended_survive_a_restart() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    probe.add(5001, "claude.exe", 1);
    probe.add(5002, "claude.exe", 1);
    home.write_registry(5001, ID_A, json!({ "status": "busy" }));
    home.write_registry(5002, ID_B, json!({ "status": "idle" }));
    let mut backend = start(&home, &probe).await;
    let list = sessions(&backend).await;
    assert_eq!(find(&list, ID_A).status, Status::Running);
    assert_eq!(find(&list, ID_B).status, Status::Idle);

    // Both processes end without a hook. One was working, one was idle.
    probe.end(5001);
    probe.end(5002);
    let list = sessions(&backend).await;
    let (a, b) = (find(&list, ID_A), find(&list, ID_B));
    assert_eq!(
        (a.status, a.alive),
        (Status::Failed, false),
        "killed while working"
    );
    assert_eq!(
        (b.status, b.alive),
        (Status::Success, false),
        "closed while idle"
    );

    backend.restart().await;
    let list = sessions(&backend).await;
    assert_eq!(
        find(&list, ID_A).status,
        Status::Failed,
        "the ended sessions are remembered"
    );
    assert_eq!(find(&list, ID_B).status, Status::Success);
    assert!(list.iter().all(|s| !s.alive));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_exit_watch_ends_a_session_at_once_and_the_topic_says_so() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    probe.add(5101, "claude.exe", 1);
    home.write_registry(5101, ID_A, json!({ "status": "idle" }));
    let exits = Arc::new(ManualExits::default());
    let root = home.root();
    let backend = TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .with::<dyn ProcessProbe>(probe.clone())
        .with::<dyn ExitSource>(exits.clone())
        .start()
        .await;
    assert_eq!(
        exits.watched(),
        [5101],
        "a watch exists for every live session with a process"
    );

    let mut stream = backend.stream().await;
    probe.end(5101);
    assert!(exits.fire(5101));
    let frame = stream.expect_topic("sessions").await;
    let shown: Vec<ClaudeSession> = serde_json::from_value(frame.payload).unwrap();
    assert_eq!(find(&shown, ID_A).status, Status::Success);
    backend.stop().await;
}

fn transcript_lines() -> Vec<Value> {
    vec![
        json!({ "type": "api-request-blob", "blob": "CANARY-BLOB" }),
        json!({ "type": "session-owner", "owner": "CANARY-OWNER" }),
        json!({ "type": "user", "timestamp": "2026-11-21T09:00:00.000Z", "gitBranch": "main",
                "message": { "role": "user", "content": "Please fix the parser" } }),
        json!({ "type": "assistant", "timestamp": "2026-11-21T09:00:01.000Z", "message": {
            "id": "m1", "model": "claude-sonnet-4-5-20250929",
            "content": [
                { "type": "thinking", "thinking": "CANARY-THINKING" },
                { "type": "text", "text": "I will run the tests." },
                { "type": "tool_use", "name": "Bash", "input": { "command": "cargo test" } } ],
            "usage": { "input_tokens": 10, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 0, "output_tokens": 20 } } }),
        json!({ "type": "user", "timestamp": "2026-11-21T09:00:02.000Z", "message": { "content": [
            { "type": "tool_result", "is_error": false, "content": "CANARY-FILE" },
            { "type": "tool_result", "is_error": true, "content": "test failed\nsecond line" } ] } }),
        json!({ "type": "custom-title", "customTitle": "Parser fix" }),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_transcript_route() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    probe.add(5201, "claude.exe", 1);
    probe.add(5202, "claude.exe", 1);
    probe.add(5203, "claude.exe", 1);
    home.write_registry(5201, ID_A, json!({ "status": "idle", "name": "" }));
    home.write_transcript(r"C:\work\demo", ID_A, &transcript_lines());
    home.write_registry(5202, ID_B, json!({ "cwd": r"C:\work\nofile" }));
    // A long one.
    let long: Vec<Value> = (0..1100)
        .map(|n| json!({ "type": "user", "timestamp": "2026-11-21T09:00:00.000Z", "message": { "content": format!("line {n}") } }))
        .collect();
    home.write_registry(
        5203,
        "cccccccc-0000-0000-0000-000000000003",
        json!({ "cwd": r"C:\work\long" }),
    );
    home.write_transcript(
        r"C:\work\long",
        "cccccccc-0000-0000-0000-000000000003",
        &long,
    );
    let backend = start(&home, &probe).await;
    sessions(&backend).await;

    let (status, body) = backend
        .send("GET", &format!("/sessions/{ID_A}/transcript"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let lines = body.as_array().unwrap();
    let texts: Vec<&str> = lines.iter().map(|l| l["text"].as_str().unwrap()).collect();
    assert_eq!(
        texts,
        [
            "You: Please fix the parser",
            "Claude: I will run the tests.",
            "Bash: cargo test",
            "  test failed",
            "Waiting for your next prompt"
        ]
    );
    assert_eq!(lines[3]["level"], "error");
    assert_eq!(lines[0]["ts"], "2026-11-21T09:00:00.000Z");
    assert!(
        !body.to_string().contains("CANARY"),
        "lines that are not part of the talk are never shown"
    );

    // The facts of the same file reach the list once the tail was read (the title of a session
    // comes from the registry or the transcript).
    let mut shown = None;
    for _ in 0..100 {
        let list = sessions(&backend).await;
        let a = find(&list, ID_A).clone();
        if a.tokens > 0 {
            shown = Some(a);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let a = shown.expect("the transcript tail was read");
    assert_eq!(
        (a.tokens, a.model.as_str(), a.branch.as_deref()),
        (130, "sonnet 4.5", Some("main"))
    );
    assert_eq!(a.title, "Parser fix");
    assert_eq!(
        a.recent.first().map(|e| e.text.as_str()),
        Some("Bash: cargo test")
    );

    let (status, body) = backend
        .send("GET", &format!("/sessions/{ID_B}/transcript"), None)
        .await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::NOT_FOUND, &json!("noTranscript")),
        "{body}"
    );
    let (status, body) = backend
        .send("GET", "/sessions/nobody/transcript", None)
        .await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::NOT_FOUND, &json!("unknownSession")),
        "{body}"
    );

    let (_, body) = backend
        .send(
            "GET",
            "/sessions/cccccccc-0000-0000-0000-000000000003/transcript",
            None,
        )
        .await;
    let lines = body.as_array().unwrap();
    assert_eq!(
        lines.len(),
        1000 + 2,
        "the cap, the note and the status line"
    );
    assert_eq!(lines[0]["level"], "warn");
    assert_eq!(lines[0]["text"], "100 earlier lines are not shown");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_stop_rules() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    // A desktop session, a session whose pid is not Claude, a terminal session with a child.
    probe.add(6001, "claude.exe", 1);
    probe.add(6002, "notepad.exe", 1);
    probe.add(6003, "claude.exe", 1);
    probe.add(6004, "node.exe", 6003);
    home.write_registry(6001, "desktop-1", json!({ "entrypoint": "claude-desktop" }));
    home.write_registry(6002, "notclaude-1", json!({}));
    home.write_registry(6003, "terminal-1", json!({ "status": "busy" }));
    let backend = start(&home, &probe).await;
    sessions(&backend).await;

    let (status, body) = backend.send("POST", "/sessions/desktop-1/stop", None).await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::CONFLICT, &json!("notStoppable")),
        "{body}"
    );
    assert_eq!(
        body["message"],
        "This session runs inside the Claude desktop app. Close it there."
    );
    let (status, body) = backend
        .send("POST", "/sessions/notclaude-1/stop", None)
        .await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::CONFLICT, &json!("notStoppable")),
        "{body}"
    );
    assert_eq!(
        body["message"],
        "That process is not Claude. Nothing was stopped."
    );
    let (status, _) = backend.send("POST", "/sessions/nobody/stop", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        probe.terminated.lock().unwrap().is_empty(),
        "nothing was ended by the refusals"
    );

    let (status, body) = backend
        .send("POST", "/sessions/terminal-1/stop", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        *probe.terminated.lock().unwrap(),
        [6003, 6004],
        "the session first, then its child"
    );
    let list = sessions(&backend).await;
    let stopped = find(&list, "terminal-1");
    assert_eq!(
        (stopped.status, stopped.alive),
        (Status::Success, false),
        "an end that Rain chose is a normal end"
    );
    assert_eq!(stopped.recent[0].text, "Stopped from Rain");

    // An ended session answers 204 again.
    let (status, _) = backend
        .send("POST", "/sessions/terminal-1/stop", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    backend.stop().await;
}

/// The one test that starts real processes: a copy of `cmd.exe` named `claude.exe` that runs a
/// `ping` as its child. Both must end, and nothing else may.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_stop_ends_a_real_process_tree() {
    let home = ClaudeHome::new();
    let tools = tempfile::tempdir().unwrap();
    let system = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .expect("SystemRoot is set");
    let fake_claude = tools.path().join("claude.exe");
    std::fs::copy(system.join("System32").join("cmd.exe"), &fake_claude).unwrap();
    // clippy.toml forbids `Command::new` in the backend; this is a test.
    #[allow(clippy::disallowed_methods)]
    let mut child = std::process::Command::new(&fake_claude)
        .args(["/c", "ping", "-n", "60", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        // No console window may flash up when the test run has no console of its own.
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .expect("the fake claude started");
    let pid = child.id();
    // Give cmd a moment to start its child.
    let mut grandchild = None;
    for _ in 0..100 {
        grandchild = rain_backend::platform::process::list_processes()
            .into_iter()
            .find(|p| p.parent_pid == pid)
            .map(|p| p.pid);
        if grandchild.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let grandchild = grandchild.expect("the fake claude started a child");
    let start =
        rain_backend::platform::process::creation_time(pid).expect("the process can be asked");
    home.write_registry(
        pid,
        "real-1",
        json!({ "status": "busy", "procStart": start.to_string() }),
    );

    let root = home.root();
    let backend = TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .start()
        .await;
    let list = sessions(&backend).await;
    assert_eq!(find(&list, "real-1").status, Status::Running);

    let (status, body) = backend.send("POST", "/sessions/real-1/stop", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    // Windows ends a process a moment after `TerminateProcess` returns, so the check waits (at
    // most 3 seconds) for the end instead of asking once.
    for _ in 0..60 {
        if !rain_backend::platform::process::pid_alive(pid)
            && !rain_backend::platform::process::pid_alive(grandchild)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        !rain_backend::platform::process::pid_alive(pid),
        "the session's process ended"
    );
    assert!(
        !rain_backend::platform::process::pid_alive(grandchild),
        "its child ended too"
    );
    let list = sessions(&backend).await;
    assert_eq!(find(&list, "real-1").status, Status::Success);

    let _ = child.wait();
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_focus_choice() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    // A terminal session inside a terminal program, a desktop session inside the app, a session
    // whose chain has no window, and a session that ended.
    probe.add(7000, "windowsterminal.exe", 0);
    probe.add(7001, "claude.exe", 7000);
    probe.add(7100, "claude-app.exe", 0);
    probe.add(7101, "claude.exe", 7100);
    probe.add(7201, "claude.exe", 0);
    probe.add(7301, "claude.exe", 7000);
    home.write_registry(7001, "term", json!({ "cwd": r"C:\work\demo" }));
    home.write_registry(7101, "desk", json!({ "entrypoint": "claude-desktop" }));
    home.write_registry(7201, "nowindow", json!({}));
    home.write_registry(7301, "done", json!({}));
    let desktop = Arc::new(FakeDesktop::new(vec![
        FocusWindow {
            hwnd: 1,
            pid: 9999,
            title: "Some other program".into(),
        },
        FocusWindow {
            hwnd: 11,
            pid: 7000,
            title: "pwsh - demo".into(),
        },
        FocusWindow {
            hwnd: 21,
            pid: 7100,
            title: "Claude".into(),
        },
    ]));
    let root = home.root();
    let backend = TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .with::<dyn ProcessProbe>(probe.clone())
        .with::<dyn ExitSource>(Arc::new(NoExits))
        .with::<dyn WindowApi>(desktop.clone())
        .start()
        .await;
    sessions(&backend).await;
    probe.end(7301);

    let focus = |id: &'static str| {
        let backend = &backend;
        async move {
            let (status, body) = backend
                .send("POST", &format!("/sessions/{id}/focus"), None)
                .await;
            assert_eq!(status, StatusCode::OK, "{id}: {body}");
            body["focused"].as_str().unwrap().to_owned()
        }
    };
    assert_eq!(focus("term").await, "window");
    assert_eq!(*desktop.brought.lock().unwrap(), [11]);
    assert_eq!(focus("desk").await, "app");
    assert_eq!(*desktop.brought.lock().unwrap(), [11, 21]);
    assert_eq!(focus("nowindow").await, "none");
    assert_eq!(
        focus("done").await,
        "none",
        "an ended session has nothing to focus"
    );

    // Windows refuses: the taskbar button flashes and the answer is none, not an error.
    *desktop.works.lock().unwrap() = false;
    assert_eq!(focus("term").await, "none");
    assert_eq!(*desktop.flashed.lock().unwrap(), [11]);

    let (status, body) = backend.send("POST", "/sessions/nobody/focus", None).await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::NOT_FOUND, &json!("unknownSession")),
        "{body}"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_answering_needs_an_open_request() {
    // Without hooks no session has a request: every answer is "no longer open", or "monitoring
    // is off" while the switch is off.
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    probe.add(8001, "claude.exe", 1);
    home.write_registry(8001, ID_A, json!({}));
    let backend = start(&home, &probe).await;
    sessions(&backend).await;
    let body = json!({ "requestId": "req-00000000", "answer": "once" });
    let (status, answer) = backend
        .send(
            "POST",
            &format!("/sessions/{ID_A}/permission"),
            Some(body.clone()),
        )
        .await;
    assert_eq!(
        (status, &answer["reason"]),
        (StatusCode::CONFLICT, &json!("monitoringOff")),
        "{answer}"
    );
    assert_eq!(
        answer["message"],
        "Monitoring is off. Turn on Monitor Claude sessions in Settings."
    );

    let (status, answer) = backend
        .send(
            "POST",
            &format!("/sessions/{ID_A}/permission"),
            Some(json!({ "requestId": "r", "answer": "maybe" })),
        )
        .await;
    assert_eq!(
        (status, &answer["code"]),
        (StatusCode::BAD_REQUEST, &json!("invalid")),
        "{answer}"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_registry_watch_finds_a_file_that_appears_later() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    probe.add(4801, "claude.exe", 1);
    let backend = start(&home, &probe).await;
    assert!(sessions(&backend).await.is_empty());

    // A new session starts: its registry file appears. No timer reads it: the folder watch does.
    home.write_registry(4801, ID_A, json!({ "status": "busy" }));
    let mut found = false;
    for _ in 0..100 {
        if sessions(&backend).await.iter().any(|s| s.id == ID_A) {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(found, "the watcher read the new registry file");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_a_registry_folder_that_appears_after_the_start_is_read() {
    // Claude Code has not run when Rain starts: there is a Claude folder but no registry folder.
    let home = ClaudeHome::without_registry();
    let probe = Arc::new(FakeProbe::default());
    probe.add(4901, "claude.exe", 1);
    let backend = start(&home, &probe).await;
    assert!(sessions(&backend).await.is_empty());

    std::fs::create_dir_all(home.root().join("sessions")).unwrap();
    home.write_registry(4901, ID_A, json!({}));
    let list = sessions(&backend).await;
    assert!(list.iter().any(|s| s.id == ID_A), "{list:?}");
    let health: Health = backend.get("/health").await;
    let source = health
        .sources
        .iter()
        .find(|s| s.name == "claude-registry")
        .unwrap();
    assert_eq!(source.state, SourceState::Ok);
    backend.stop().await;
}
