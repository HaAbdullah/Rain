//! The hook path (section 03, stage 8b): `POST /internal/hook`, the session book fed by events, and
//! permission requests that are held for the hook program and answered through
//! `POST /sessions/{id}/permission`. The hook program itself is tested in `hook_process.rs`.
//!
//! Every test has the switch "Monitor Claude sessions" on through `settings.toml`, a throwaway Claude
//! folder and a fake process table.

#[path = "common/claude.rs"]
mod claude;
mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use claude::{ClaudeHome, FakeProbe, NoExits, event_envelope, permission_envelope};
use common::{StreamReader, TestBackend};
use rain_backend::modules::sessions::{ExitSource, HookHold, ProcessProbe};
use rain_types::{ClaudeSession, PermissionRequest, SessionEventKind, SessionHost, Status};
use serde_json::{Value, json};

const S1: &str = "11111111-0000-0000-0000-000000000001";
const S2: &str = "22222222-0000-0000-0000-000000000002";
const HOOK: &str = "/internal/hook";

struct Setup {
    backend: TestBackend,
    probe: Arc<FakeProbe>,
    // Kept so that the folder lives as long as the backend.
    _home: ClaudeHome,
}

async fn setup_with(hold: Duration, monitor: bool) -> Setup {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    probe.add(9001, "claude.exe", 1);
    probe.add(9002, "claude.exe", 1);
    let root = home.root();
    let settings = if monitor {
        "schema_version = 1\nmonitor_claude = true\n"
    } else {
        "schema_version = 1\n"
    };
    let backend = TestBackend::builder()
        .file("settings.toml", settings)
        .config(move |config| config.claude_dir = Some(root.clone()))
        .with::<dyn ProcessProbe>(probe.clone())
        .with::<dyn ExitSource>(Arc::new(NoExits))
        .with::<HookHold>(Arc::new(HookHold(hold)))
        .start()
        .await;
    Setup {
        backend,
        probe,
        _home: home,
    }
}

async fn setup() -> Setup {
    setup_with(Duration::from_secs(5), true).await
}

async fn post_event(backend: &TestBackend, name: &str, session: &str, extra: Value) -> StatusCode {
    let (status, body) = backend
        .send("POST", HOOK, Some(event_envelope(name, session, extra)))
        .await;
    assert!(status == StatusCode::NO_CONTENT, "{name}: {status} {body}");
    status
}

async fn list(backend: &TestBackend) -> Vec<ClaudeSession> {
    backend.get("/sessions").await
}

fn session<'a>(list: &'a [ClaudeSession], id: &str) -> &'a ClaudeSession {
    list.iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("session {id} is not listed: {list:?}"))
}

/// Waits on the stream until a session has an open request, and returns it.
async fn next_request(stream: &mut StreamReader, answerable: bool) -> (String, PermissionRequest) {
    loop {
        let frame = stream.expect_topic("sessions").await;
        let list: Vec<ClaudeSession> =
            serde_json::from_value(frame.payload).expect("a session list");
        for s in list {
            if let Some(request) = s.request
                && request.answerable == answerable
            {
                return (s.id, request);
            }
        }
    }
}

async fn answer(
    backend: &TestBackend,
    session: &str,
    request_id: &str,
    answer: &str,
) -> (StatusCode, Value) {
    backend
        .send(
            "POST",
            &format!("/sessions/{session}/permission"),
            Some(json!({ "requestId": request_id, "answer": answer })),
        )
        .await
}

/// The decision JSON that Claude Code expects for each answer (section 03 6.5.5).
fn decision(body: Value) -> Value {
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": body } })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_event_flow() {
    let t = setup().await;
    let b = &t.backend;
    let env = |name: &str, extra: Value| {
        let mut envelope = event_envelope(name, S1, extra);
        envelope["env"]["claudePid"] = json!(9001);
        envelope
    };
    let send = |envelope: Value| async move {
        let (status, body) = b.send("POST", HOOK, Some(envelope)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    };

    send(env("SessionStart", json!({ "model": "claude-sonnet-4-5-20250929", "session_title": "Parser work", "source": "startup" }))).await;
    let l = list(b).await;
    let s = session(&l, S1);
    assert_eq!(
        (s.status, s.alive, s.host),
        (Status::Idle, true, SessionHost::Terminal)
    );
    assert_eq!(
        (s.title.as_str(), s.model.as_str(), s.cwd.as_str()),
        ("Parser work", "sonnet 4.5", r"C:\work\demo")
    );

    send(env(
        "UserPromptSubmit",
        json!({ "prompt": "Fix the parser\nplease" }),
    ))
    .await;
    send(env(
        "PreToolUse",
        json!({ "tool_name": "Bash", "tool_input": { "command": "cargo test" } }),
    ))
    .await;
    let l = list(b).await;
    let s = session(&l, S1);
    assert_eq!(s.status, Status::Running);
    let recent: Vec<_> = s.recent.iter().map(|e| (e.kind, e.text.as_str())).collect();
    assert_eq!(
        recent,
        [
            (SessionEventKind::Tool, "Bash: cargo test"),
            (SessionEventKind::Prompt, "Fix the parser")
        ]
    );

    send(env(
        "Stop",
        json!({ "last_assistant_message": "All done." }),
    ))
    .await;
    let l = list(b).await;
    let s = session(&l, S1);
    assert_eq!(
        (s.status, s.last_message.as_str()),
        (Status::Idle, "All done.")
    );

    // `/clear`: the process lives on and a new session id follows, so the record is dropped.
    send(env("SessionEnd", json!({ "reason": "clear" }))).await;
    assert!(list(b).await.is_empty());

    // A normal end is a session that is done.
    send(env(
        "SessionStart",
        json!({ "model": "claude-opus-4-1-20250805" }),
    ))
    .await;
    send(env("SessionEnd", json!({ "reason": "other" }))).await;
    let l = list(b).await;
    let s = session(&l, S1);
    assert_eq!((s.status, s.alive), (Status::Success, false));
    assert_eq!(s.model, "opus 4.1");

    // An event for a session that Rain never saw start creates it.
    post_event(
        b,
        "PostToolUse",
        S2,
        json!({ "tool_name": "Edit", "tool_input": { "file_path": r"C:\work\demo\a.rs" } }),
    )
    .await;
    let l = list(b).await;
    let s = session(&l, S2);
    assert_eq!(s.status, Status::Running);
    assert_eq!(s.recent[0].text, "a.rs");
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_a_process_that_dies_ends_a_session_that_only_hooks_knew() {
    let t = setup().await;
    let mut envelope = event_envelope("UserPromptSubmit", S1, json!({}));
    envelope["env"]["claudePid"] = json!(9001);
    let (status, _) = t.backend.send("POST", HOOK, Some(envelope)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(session(&list(&t.backend).await, S1).status, Status::Running);
    t.probe.end(9001);
    let l = list(&t.backend).await;
    let s = session(&l, S1);
    assert_eq!(
        (s.status, s.alive),
        (Status::Failed, false),
        "it was working"
    );
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_with_the_switch_off_or_in_game_mode_every_event_is_ignored() {
    let off = setup_with(Duration::from_secs(5), false).await;
    post_event(&off.backend, "UserPromptSubmit", S1, json!({})).await;
    let (status, _) = off
        .backend
        .send(
            "POST",
            HOOK,
            Some(permission_envelope(
                S1,
                "Bash",
                json!({ "command": "ls" }),
                json!({}),
            )),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "no hold when the switch is off"
    );
    assert!(list(&off.backend).await.is_empty());
    off.backend.stop().await;

    let on = setup().await;
    on.backend.state.set_game_mode_active(true);
    post_event(&on.backend, "UserPromptSubmit", S1, json!({})).await;
    assert!(
        list(&on.backend).await.is_empty(),
        "game mode: nothing is recorded"
    );
    on.backend.state.set_game_mode_active(false);
    post_event(&on.backend, "UserPromptSubmit", S1, json!({})).await;
    assert_eq!(list(&on.backend).await.len(), 1);
    on.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_a_bad_envelope_is_refused_with_its_own_reason() {
    let t = setup().await;
    let bad = [
        json!({ "v": 2, "mode": "event", "event": {} }),
        json!({ "v": 1, "mode": "weird", "event": {} }),
        json!({ "v": 1, "mode": "event" }),
        json!({ "v": 1, "mode": "event", "event": [1] }),
    ];
    for body in bad {
        let (status, answer) = t.backend.send("POST", HOOK, Some(body.clone())).await;
        assert_eq!(
            (status, &answer["reason"]),
            (StatusCode::BAD_REQUEST, &json!("badEnvelope")),
            "{body} -> {answer}"
        );
    }
    let (status, answer) = t
        .backend
        .send_bytes("POST", HOOK, Some("not json at all".into()))
        .await;
    assert_eq!(
        (status, &answer["reason"]),
        (StatusCode::BAD_REQUEST, &json!("badEnvelope")),
        "{answer}"
    );
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_a_large_write_is_accepted_up_to_four_mebibytes() {
    let mut t = setup().await;
    // Over the 1 MiB of every other route, under the 4 MiB of this one.
    let content = "x".repeat(2 * 1024 * 1024);
    post_event(
        &t.backend,
        "PreToolUse",
        S1,
        json!({ "tool_name": "Write", "tool_input": { "file_path": "a.txt", "content": content } }),
    )
    .await;
    // Over 4 MiB the route refuses. The server may answer before the client has written it all, so
    // the client sees either the refusal or a closed connection; never a success.
    let huge = "x".repeat(5 * 1024 * 1024);
    let body = event_envelope(
        "PreToolUse",
        S1,
        json!({ "tool_input": { "content": huge } }),
    )
    .to_string();
    let answered = t
        .backend
        .client
        .request(
            hyper::Method::POST,
            "/api/v1/internal/hook",
            Some(body.into()),
            &[],
        )
        .await;
    if let Ok(raw) = answered {
        assert_eq!(raw.status, StatusCode::BAD_REQUEST);
    }
    // The refused connection was cut by the server. The pooled client may still hold it, so the
    // test continues on a fresh client (a real client would retry).
    t.backend.client = rain_pipe::PipeClient::new(&t.backend.pipe_name);
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_owned_events_ignored() {
    let t = setup().await;
    let mut started = event_envelope("SessionStart", S1, json!({}));
    started["env"]["runId"] = json!("run-20261121-060000-ab12");
    let (status, _) = t.backend.send("POST", HOOK, Some(started)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let mut permission = permission_envelope(S1, "Bash", json!({ "command": "ls" }), json!({}));
    permission["env"]["runId"] = json!("run-20261121-060000-ab12");
    let started_at = std::time::Instant::now();
    let (status, _) = t.backend.send("POST", HOOK, Some(permission)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "never held, never shown");
    assert!(started_at.elapsed() < Duration::from_secs(2));
    assert!(
        list(&t.backend).await.is_empty(),
        "a run of Rain's own job is never a session"
    );
    assert_eq!(t.backend.state.sessions().open_user_sessions(), 0);
    t.backend.stop().await;
}

/// Posts a permission request and answers it from the other side, as the UI would.
async fn round_trip(
    t: &Setup,
    stream: &mut StreamReader,
    envelope: Value,
    choice: &str,
) -> (StatusCode, Value, StatusCode, Value) {
    let held = t.backend.send("POST", HOOK, Some(envelope));
    let driver = async {
        let (session, request) = next_request(stream, true).await;
        answer(&t.backend, &session, &request.id, choice).await
    };
    let ((hook_status, hook_body), (answer_status, answer_body)) = tokio::join!(held, driver);
    (hook_status, hook_body, answer_status, answer_body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_roundtrip() {
    let t = setup().await;
    let mut stream = t.backend.stream().await;

    // Once.
    let envelope = permission_envelope(S1, "Bash", json!({ "command": "npm install" }), json!({}));
    let held = t.backend.send("POST", HOOK, Some(envelope));
    let driver = async {
        let (session, request) = next_request(&mut stream, true).await;
        assert_eq!(
            (
                session.as_str(),
                request.tool.as_str(),
                request.summary.as_str()
            ),
            (S1, "Bash", "npm install")
        );
        assert_eq!(request.detail, "npm install");
        assert!(
            request.id.starts_with("req-") && request.id.len() == 12,
            "{}",
            request.id
        );
        // While it is open the session is waiting.
        let l = list(&t.backend).await;
        assert_eq!(session_status(&l, S1), Status::Waiting);
        answer(&t.backend, &session, &request.id, "once").await
    };
    let ((hook_status, hook_body), (answer_status, answer_body)) = tokio::join!(held, driver);
    assert_eq!(hook_status, StatusCode::OK, "{hook_body}");
    assert_eq!(
        hook_body,
        decision(json!({ "behavior": "allow" })),
        "the exact JSON that Claude Code expects"
    );
    assert_eq!(
        (answer_status, answer_body),
        (StatusCode::OK, json!({ "applied": "once" }))
    );
    let l = list(&t.backend).await;
    let s = session(&l, S1);
    assert_eq!(s.status, Status::Running, "the session works again");
    assert!(s.request.is_none());
    assert_eq!(s.recent[0].text, "Allowed: Bash: npm install");

    // Always: a rule for this session only.
    let envelope = permission_envelope(S1, "Bash", json!({ "command": "npm test" }), json!({}));
    let (hs, hb, as_, ab) = round_trip(&t, &mut stream, envelope, "always").await;
    assert_eq!(hs, StatusCode::OK);
    assert_eq!(
        hb,
        decision(json!({ "behavior": "allow", "updatedPermissions": [{
            "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }],
            "behavior": "allow", "destination": "session" }] }))
    );
    assert_eq!(
        (as_, ab),
        (
            StatusCode::OK,
            json!({ "applied": "always", "ruleText": "Bash(npm test)" })
        )
    );

    // Always for an edit degrades to once, and says so.
    let envelope = permission_envelope(
        S1,
        "Edit",
        json!({ "file_path": r"C:\work\demo\a.rs", "old_string": "a", "new_string": "b" }),
        json!({}),
    );
    let (hs, hb, as_, ab) = round_trip(&t, &mut stream, envelope, "always").await;
    assert_eq!(
        (hs, hb),
        (StatusCode::OK, decision(json!({ "behavior": "allow" })))
    );
    assert_eq!((as_, ab), (StatusCode::OK, json!({ "applied": "once" })));

    // Always for a command with a star degrades to once too: a star is a wildcard in a rule.
    let envelope = permission_envelope(S1, "Bash", json!({ "command": "rm *.tmp" }), json!({}));
    let (_, hb, _, ab) = round_trip(&t, &mut stream, envelope, "always").await;
    assert_eq!(hb, decision(json!({ "behavior": "allow" })));
    assert_eq!(ab, json!({ "applied": "once" }));

    // Always with a suggestion of Claude Code: its rule, with the destination forced to this session.
    let suggestions = json!({ "permission_suggestions": [{ "type": "addRules", "behavior": "allow", "destination": "projectSettings",
        "rules": [{ "toolName": "mcp__files__list" }] }] });
    let envelope = permission_envelope(S1, "mcp__files__list", json!({ "path": "." }), suggestions);
    let (_, hb, _, ab) = round_trip(&t, &mut stream, envelope, "always").await;
    assert_eq!(
        hb["hookSpecificOutput"]["decision"]["updatedPermissions"][0]["destination"],
        "session"
    );
    assert_eq!(
        ab,
        json!({ "applied": "always", "ruleText": "mcp__files__list" })
    );

    // Deny.
    let envelope = permission_envelope(S1, "Bash", json!({ "command": "rm -rf build" }), json!({}));
    let (hs, hb, as_, ab) = round_trip(&t, &mut stream, envelope, "deny").await;
    assert_eq!(hs, StatusCode::OK);
    assert_eq!(
        hb,
        decision(json!({ "behavior": "deny", "message": "Denied from Rain", "interrupt": false }))
    );
    assert_eq!((as_, ab), (StatusCode::OK, json!({ "applied": "deny" })));
    assert_eq!(
        session(&list(&t.backend).await, S1).recent[0].text,
        "Denied: Bash: rm -rf build"
    );
    t.backend.stop().await;
}

fn session_status(list: &[ClaudeSession], id: &str) -> Status {
    session(list, id).status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_a_second_answer_is_too_late() {
    let t = setup().await;
    let mut stream = t.backend.stream().await;
    let envelope = permission_envelope(S1, "Bash", json!({ "command": "ls" }), json!({}));
    let held = t.backend.send("POST", HOOK, Some(envelope));
    let driver = async {
        let (session, request) = next_request(&mut stream, true).await;
        let first = answer(&t.backend, &session, &request.id, "once").await;
        let second = answer(&t.backend, &session, &request.id, "deny").await;
        (first, second)
    };
    let (_, ((first, _), (second, body))) = tokio::join!(held, driver);
    assert_eq!(first, StatusCode::OK);
    assert_eq!(second, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "requestExpired");
    assert_eq!(
        body["message"],
        "That request is no longer open. Answer it in the terminal."
    );
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_expiry() {
    let t = setup_with(Duration::from_millis(400), true).await;
    // A request is only held while a client listens (nobody could answer it otherwise).
    let _stream = t.backend.stream().await;
    let started = std::time::Instant::now();
    let (status, body) = t
        .backend
        .send(
            "POST",
            HOOK,
            Some(permission_envelope(
                S1,
                "Bash",
                json!({ "command": "ls" }),
                json!({}),
            )),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "after the hold the hook program is told nothing: {body}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "the request was held"
    );

    // The request is still open in Claude Code, so the session still waits, but Rain cannot answer.
    let l = list(&t.backend).await;
    let s = session(&l, S1);
    assert_eq!(s.status, Status::Waiting);
    let request = s.request.clone().expect("the request is still shown");
    assert!(!request.answerable);
    let (status, body) = answer(&t.backend, S1, &request.id, "once").await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::CONFLICT, &json!("requestExpired")),
        "{body}"
    );
    assert_eq!(
        body["message"],
        "That request is no longer open. Answer it in the terminal."
    );

    // The tool ran after the user answered in the terminal: the matching event clears it.
    post_event(
        &t.backend,
        "PostToolUse",
        S1,
        json!({ "tool_name": "Bash", "tool_input": { "command": "ls" } }),
    )
    .await;
    let l = list(&t.backend).await;
    assert_eq!(session_status(&l, S1), Status::Running);
    assert!(session(&l, S1).request.is_none());
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_dropped_forwarder() {
    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::ClientOptions;

    let t = setup_with(Duration::from_secs(10), true).await;
    // A request is only held while a client listens (nobody could answer it otherwise).
    let _stream = t.backend.stream().await;
    let body = permission_envelope(S1, "Bash", json!({ "command": "ls" }), json!({})).to_string();
    let request = format!(
        "POST /api/v1/internal/hook HTTP/1.1\r\nHost: rain\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut pipe = ClientOptions::new()
        .open(common::path_of(&t.backend.pipe_name))
        .expect("the pipe opened");
    pipe.write_all(request.as_bytes()).await.unwrap();
    pipe.flush().await.unwrap();

    // Wait until the request is shown, then close the connection like a killed forwarder.
    let mut shown = false;
    for _ in 0..100 {
        let l = list(&t.backend).await;
        if l.iter()
            .any(|s| s.request.as_ref().is_some_and(|r| r.answerable))
        {
            shown = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(shown, "the request appeared");
    drop(pipe);

    let mut closed = false;
    for _ in 0..100 {
        let l = list(&t.backend).await;
        if l.iter()
            .any(|s| s.request.as_ref().is_some_and(|r| !r.answerable))
        {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(
        closed,
        "a request whose forwarder went away is no longer answerable, well before the hold ends"
    );
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_clearing_events() {
    let t = setup().await;
    let mut stream = t.backend.stream().await;
    let first = t.backend.send(
        "POST",
        HOOK,
        Some(permission_envelope(
            S1,
            "Bash",
            json!({ "command": "a", "timeout": 5 }),
            json!({}),
        )),
    );
    let second = async {
        // The second request arrives while the first is open (a parallel tool call).
        tokio::time::sleep(Duration::from_millis(150)).await;
        t.backend
            .send(
                "POST",
                HOOK,
                Some(permission_envelope(
                    S1,
                    "Bash",
                    json!({ "command": "b" }),
                    json!({}),
                )),
            )
            .await
    };
    let driver = async {
        let (_, shown) = next_request(&mut stream, true).await;
        assert_eq!(shown.summary, "a");
        tokio::time::sleep(Duration::from_millis(500)).await;
        let l = list(&t.backend).await;
        assert_eq!(
            session(&l, S1).request.as_ref().unwrap().id,
            shown.id,
            "the first one stays the shown one"
        );

        // A finished tool call with another input clears nothing.
        post_event(
            &t.backend,
            "PostToolUse",
            S1,
            json!({ "tool_name": "Bash", "tool_input": { "command": "zzz" } }),
        )
        .await;
        assert_eq!(
            session(&list(&t.backend).await, S1)
                .request
                .as_ref()
                .unwrap()
                .id,
            shown.id
        );

        // After an answer the next request shows at once, and the status stays waiting.
        let (status, _) = answer(&t.backend, S1, &shown.id, "once").await;
        assert_eq!(status, StatusCode::OK);
        let l = list(&t.backend).await;
        let next = session(&l, S1)
            .request
            .clone()
            .expect("the second request shows at once");
        assert_eq!(
            (next.summary.as_str(), session_status(&l, S1)),
            ("b", Status::Waiting)
        );

        // The tool call of the second one finished: it clears, and the key order does not matter.
        post_event(
            &t.backend,
            "PostToolUse",
            S1,
            json!({ "tool_name": "Bash", "tool_input": { "command": "b" } }),
        )
        .await;
        let l = list(&t.backend).await;
        assert!(session(&l, S1).request.is_none());
        assert_eq!(session_status(&l, S1), Status::Running);
    };
    let ((s1, _), (s2, _), ()) = tokio::join!(first, second, driver);
    assert_eq!(s1, StatusCode::OK, "the first was answered");
    assert_eq!(
        s2,
        StatusCode::NO_CONTENT,
        "the second was cleared by an event: Claude Code moved on"
    );
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_the_key_order_of_the_input_does_not_matter_and_stop_clears_everything() {
    let t = setup().await;
    let mut stream = t.backend.stream().await;
    let held = t.backend.send(
        "POST",
        HOOK,
        Some(permission_envelope(
            S1,
            "Bash",
            json!({ "command": "a", "timeout": 5 }),
            json!({}),
        )),
    );
    let driver = async {
        next_request(&mut stream, true).await;
        post_event(
            &t.backend,
            "PostToolUse",
            S1,
            json!({ "tool_name": "Bash", "tool_input": { "timeout": 5, "command": "a" } }),
        )
        .await;
        assert!(
            session(&list(&t.backend).await, S1).request.is_none(),
            "cleared with the keys in another order"
        );
    };
    let ((status, _), ()) = tokio::join!(held, driver);
    assert_eq!(status, StatusCode::NO_CONTENT);

    let held = t.backend.send(
        "POST",
        HOOK,
        Some(permission_envelope(
            S1,
            "Edit",
            json!({ "file_path": "a.rs" }),
            json!({}),
        )),
    );
    let driver = async {
        next_request(&mut stream, true).await;
        post_event(&t.backend, "Stop", S1, json!({})).await;
        let l = list(&t.backend).await;
        assert!(session(&l, S1).request.is_none());
        assert_eq!(session_status(&l, S1), Status::Idle);
    };
    let ((status, _), ()) = tokio::join!(held, driver);
    assert_eq!(status, StatusCode::NO_CONTENT);
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_askuser_not_answerable() {
    let t = setup().await;
    let started = std::time::Instant::now();
    let (status, _) = t
        .backend
        .send(
            "POST",
            HOOK,
            Some(permission_envelope(
                S1,
                "AskUserQuestion",
                json!({ "questions": [] }),
                json!({}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(started.elapsed() < Duration::from_secs(2), "not held");
    let l = list(&t.backend).await;
    let s = session(&l, S1);
    assert_eq!(s.status, Status::Waiting);
    let request = s.request.clone().unwrap();
    assert_eq!(
        (
            request.answerable,
            request.summary.as_str(),
            request.detail.as_str()
        ),
        (false, "AskUserQuestion", "")
    );
    let (status, body) = answer(&t.backend, S1, &request.id, "once").await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::CONFLICT, &json!("requestExpired"))
    );
    t.backend.stop().await;
}

/// With no client on the stream nobody can answer, so the hook must not be held: Claude Code's own
/// prompt would only appear after the whole wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_without_a_listening_client_is_not_held() {
    let t = setup().await;
    let started = std::time::Instant::now();
    let (status, _) = t
        .backend
        .send(
            "POST",
            HOOK,
            Some(permission_envelope(
                S1,
                "Bash",
                json!({ "command": "ls" }),
                json!({}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(started.elapsed() < Duration::from_secs(2), "not held");
    let l = list(&t.backend).await;
    let request = session(&l, S1).request.clone().expect("the session waits");
    assert!(!request.answerable, "Rain cannot answer it");
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_wrong_id_and_monitoring_off() {
    let t = setup().await;
    let mut stream = t.backend.stream().await;
    let held = t.backend.send(
        "POST",
        HOOK,
        Some(permission_envelope(
            S1,
            "Bash",
            json!({ "command": "ls" }),
            json!({}),
        )),
    );
    let driver = async {
        let (session, request) = next_request(&mut stream, true).await;
        // An id that is not the one shown is never approved blind.
        let (status, body) = answer(&t.backend, &session, "req-deadbeef", "once").await;
        assert_eq!(
            (status, &body["reason"]),
            (StatusCode::CONFLICT, &json!("requestExpired")),
            "{body}"
        );
        let (status, body) = answer(&t.backend, "no-such-session", &request.id, "once").await;
        assert_eq!(
            (status, &body["reason"]),
            (StatusCode::NOT_FOUND, &json!("unknownSession")),
            "{body}"
        );
        // The real one still works.
        answer(&t.backend, &session, &request.id, "deny").await
    };
    let ((hook_status, _), (answer_status, _)) = tokio::join!(held, driver);
    assert_eq!(
        (hook_status, answer_status),
        (StatusCode::OK, StatusCode::OK)
    );
    t.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_shutdown_does_not_wait_for_a_held_request() {
    // The backend stops while a forwarder waits: the connection ends and nothing hangs.
    let t = setup().await;
    let body = permission_envelope(S1, "Bash", json!({ "command": "ls" }), json!({})).to_string();
    let held = t.backend.client.request(
        hyper::Method::POST,
        "/api/v1/internal/hook",
        Some(body.into()),
        &[],
    );
    let stopper = async {
        for _ in 0..100 {
            if list(&t.backend).await.iter().any(|s| s.request.is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        t.backend.request_stop("stop").await;
    };
    let (answered, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(held, stopper)
    })
    .await
    .expect("the held request did not block the stop");
    // Either the connection was cut or an empty answer came: never a decision.
    if let Ok(raw) = answered {
        assert_ne!(raw.status, StatusCode::OK);
    }
    t.backend.wait_stopped().await;
}
