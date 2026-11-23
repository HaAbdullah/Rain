//! The real `rain-hook.exe` against the backend and against small fake pipe servers (section 03
//! 6.2.1 and 10.2). It must fail open in every branch: exit code 0 and an empty standard output,
//! except when Rain answered a permission request.
//!
//! The program is built by `cargo build -p rain-hook` when it is not there yet. The hook honours
//! `RAIN_HOOK_WAIT_MS`, which can only shorten its wait, so no test waits 20 seconds.

#[path = "common/claude.rs"]
mod claude;
mod common;

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use claude::{ClaudeHome, FakeProbe, NoExits};
use common::{PATIENCE, TestBackend, path_of, unique_pipe_name};
use rain_backend::modules::sessions::{ExitSource, ProcessProbe};
use rain_types::ClaudeSession;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ServerOptions;

/// `CREATE_NO_WINDOW` of Windows: a child console program gets no window of its own.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The program next to the test program's `deps` folder, built when missing.
fn hook_exe() -> PathBuf {
    // The test program may sit deep inside the profile folder (`targetdebugdeps` or below
    // `targetdebuguild`), so the profile folder is the nearest parent named `debug` or
    // `release`.
    let test_exe = std::env::current_exe().expect("the path of the test program");
    let profile = test_exe
        .ancestors()
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| name == "debug" || name == "release")
        })
        .expect("the profile folder of the target folder")
        .to_path_buf();
    let exe = profile.join("rain-hook.exe");
    if !exe.is_file() {
        // clippy.toml forbids `Command::new` in the backend; this is a test.
        #[allow(clippy::disallowed_methods)]
        let built = Command::new(env!("CARGO"))
            .args(["build", "-p", "rain-hook"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("cargo started");
        assert!(built.success(), "cargo build -p rain-hook failed");
    }
    assert!(exe.is_file(), "{} was not built", exe.display());
    exe
}

struct Ran {
    code: Option<i32>,
    stdout: String,
    elapsed: Duration,
}

/// Runs the hook with `input` on its standard input. Blocking.
fn run_hook(args: &[&str], input: &[u8], env: &[(&str, &str)]) -> Ran {
    #[allow(clippy::disallowed_methods)]
    let mut command = Command::new(hook_exe());
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // No console window may flash up when the test run has no console of its own.
        .creation_flags(CREATE_NO_WINDOW)
        // Nothing of the real environment may steer the hook to a real backend.
        .env_remove("RAIN_PIPE")
        .env_remove("RAIN_RUN_ID")
        .env_remove("RAIN_JOB_ID")
        .env_remove("CLAUDE_PID")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env_remove("RAIN_HOOK_WAIT_MS");
    for (key, value) in env {
        command.env(key, value);
    }
    let started = Instant::now();
    let mut child = command.spawn().expect("the hook started");
    let mut stdin = child.stdin.take().expect("a piped stdin");
    // A program that stops reading early (too large an input) closes the pipe: not an error here.
    let _ = stdin.write_all(input);
    drop(stdin);
    let output = child.wait_with_output().expect("the hook ended");
    Ran {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        elapsed: started.elapsed(),
    }
}

const EVENT: &str = r#"{"session_id":"hook-test-1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"npm install"},"cwd":"C:\\work\\demo"}"#;

#[test]
fn hook_process_selftest_and_unusable_input_print_nothing_and_exit_zero() {
    let ran = run_hook(&["selftest"], b"", &[]);
    assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""));
    for (args, input) in [
        (&["permission"][..], &b"hello, not json"[..]),
        (&["event"], b""),
        (&["permission"], b"[1, 2]"),
        (&["unknown-mode"], EVENT.as_bytes()),
        (&[], EVENT.as_bytes()),
    ] {
        let ran = run_hook(
            args,
            input,
            &[("RAIN_PIPE", "rain-test-no-such-pipe-anywhere")],
        );
        assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""), "{args:?}");
    }
}

#[test]
fn hook_process_with_no_backend_prints_nothing_and_is_quick() {
    let pipe = unique_pipe_name();
    let ran = run_hook(
        &["permission", "--rain-managed=abc123"],
        EVENT.as_bytes(),
        &[("RAIN_PIPE", &pipe)],
    );
    assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""));
    assert!(
        ran.elapsed < Duration::from_millis(1000),
        "Rain is not running: {:?}",
        ran.elapsed
    );
    // A pipe argument that is not a pipe name does nothing, too.
    let ran = run_hook(&["event", "--pipe=not a pipe name"], EVENT.as_bytes(), &[]);
    assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""));
}

/// A pipe server that reads one request and answers as told. Returns the request it got.
async fn fake_server(pipe: &str, reply: Option<&'static str>) -> tokio::task::JoinHandle<String> {
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(path_of(pipe))
        .expect("the fake pipe was created");
    tokio::spawn(async move {
        server.connect().await.expect("the hook connected");
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = server.read(&mut chunk).await.unwrap_or(0);
            request.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&request).into_owned();
            let complete = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|n| n.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                body.len() >= length
            });
            if n == 0 || complete {
                break;
            }
        }
        match reply {
            Some(text) => {
                let _ = server.write_all(text.as_bytes()).await;
                let _ = server.flush().await;
                let _ = server.disconnect();
            }
            // Never answers: the hook has to give up by itself.
            None => tokio::time::sleep(Duration::from_secs(5)).await,
        }
        String::from_utf8_lossy(&request).into_owned()
    })
}

fn blocking_hook(
    args: Vec<String>,
    input: Vec<u8>,
    env: Vec<(String, String)>,
) -> tokio::task::JoinHandle<Ran> {
    tokio::task::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        run_hook(&args, &input, &env)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_process_sends_the_envelope_and_prints_nothing_for_a_204() {
    let pipe = unique_pipe_name();
    let server = fake_server(&pipe, Some("HTTP/1.1 204 No Content\r\n\r\n")).await;
    let ran = blocking_hook(
        vec!["event".into(), "--rain-managed=abc123".into()],
        EVENT.as_bytes().to_vec(),
        vec![
            ("RAIN_PIPE".into(), pipe.clone()),
            ("RAIN_RUN_ID".into(), "run-20261121-060000-ab12".into()),
            ("CLAUDE_PID".into(), "4242".into()),
            ("CLAUDE_CODE_ENTRYPOINT".into(), "cli".into()),
        ],
    )
    .await
    .unwrap();
    assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""));
    let request = server.await.unwrap();
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(
        head.starts_with("POST /api/v1/internal/hook HTTP/1.1"),
        "{head}"
    );
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["v"], 1);
    assert_eq!(body["mode"], "event");
    assert_eq!(
        body["env"],
        json!({ "runId": "run-20261121-060000-ab12", "entrypoint": "cli", "claudePid": 4242 })
    );
    assert_eq!(
        body["event"]["tool_input"]["command"], "npm install",
        "the input of Claude Code is passed on unchanged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_process_prints_nothing_for_an_error_status_and_for_a_body_that_is_not_json() {
    for reply in [
        "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 2\r\n\r\n{}",
        "HTTP/1.1 400 Bad Request\r\ncontent-length: 7\r\n\r\n{\"a\":1}",
        "HTTP/1.1 200 OK\r\ncontent-length: 4\r\n\r\nnope",
        "garbage that is not http",
    ] {
        let pipe = unique_pipe_name();
        let server = fake_server(&pipe, Some(reply)).await;
        let ran = blocking_hook(
            vec!["permission".into()],
            EVENT.as_bytes().to_vec(),
            vec![("RAIN_PIPE".into(), pipe)],
        )
        .await
        .unwrap();
        assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""), "{reply}");
        server.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_process_gives_up_when_the_server_never_answers() {
    let pipe = unique_pipe_name();
    let _server = fake_server(&pipe, None).await;
    let ran = blocking_hook(
        vec!["permission".into()],
        EVENT.as_bytes().to_vec(),
        vec![
            ("RAIN_PIPE".into(), pipe),
            ("RAIN_HOOK_WAIT_MS".into(), "300".into()),
        ],
    )
    .await
    .unwrap();
    assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""));
    assert!(
        ran.elapsed >= Duration::from_millis(250),
        "it waited: {:?}",
        ran.elapsed
    );
    assert!(
        ran.elapsed < Duration::from_secs(4),
        "and then gave up: {:?}",
        ran.elapsed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_process_a_too_large_input_is_not_forwarded() {
    let pipe = unique_pipe_name();
    // No server: if the program tried to forward, it would find no pipe. The point is the exit.
    let input = format!("{{\"x\":\"{}\"}}", "y".repeat(4 * 1024 * 1024));
    let ran = blocking_hook(
        vec!["permission".into()],
        input.into_bytes(),
        vec![("RAIN_PIPE".into(), pipe)],
    )
    .await
    .unwrap();
    assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""));
}

/// The real hook against the real backend: it prints exactly the JSON that Rain answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_process_prints_the_allow_json_that_the_backend_answers() {
    let home = ClaudeHome::new();
    let probe = Arc::new(FakeProbe::default());
    let root = home.root();
    let backend = TestBackend::builder()
        .file(
            "settings.toml",
            "schema_version = 1\nmonitor_claude = true\n",
        )
        .config(move |config| config.claude_dir = Some(root.clone()))
        .with::<dyn ProcessProbe>(probe)
        .with::<dyn ExitSource>(Arc::new(NoExits))
        .start()
        .await;
    let mut stream = backend.stream().await;
    let hook = blocking_hook(
        vec!["permission".into(), format!("--pipe={}", backend.pipe_name)],
        EVENT.as_bytes().to_vec(),
        vec![],
    );
    let answerer = async {
        let (session, request) = loop {
            let frame = tokio::time::timeout(PATIENCE, stream.expect_topic("sessions"))
                .await
                .expect("a frame");
            let list: Vec<ClaudeSession> = serde_json::from_value(frame.payload).unwrap();
            if let Some(found) = list.into_iter().find_map(|s| s.request.map(|r| (s.id, r))) {
                break found;
            }
        };
        assert_eq!(
            (session.as_str(), request.summary.as_str()),
            ("hook-test-1", "npm install")
        );
        backend
            .send(
                "POST",
                &format!("/sessions/{session}/permission"),
                Some(json!({ "requestId": request.id, "answer": "once" })),
            )
            .await
    };
    let (ran, (status, _)) = tokio::join!(hook, answerer);
    let ran = ran.unwrap();
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(ran.code, Some(0));
    let printed: Value = serde_json::from_str(&ran.stdout).expect("the hook printed JSON");
    assert_eq!(
        printed,
        json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } } })
    );
    backend.stop().await;
}

/// Median wall time of `rain-hook.exe event` with no backend: under 50 ms (foundation 1.3). Only
/// meaningful for the release build, so it is ignored in a normal run:
/// `cargo build --release -p rain-hook` and then `cargo test --release -p rain-backend --test
/// hook_process hook_budget -- --ignored`.
#[test]
#[ignore = "release build only"]
fn hook_budget() {
    let pipe = unique_pipe_name();
    let mut times: Vec<Duration> = (0..30)
        .map(|_| run_hook(&["event"], EVENT.as_bytes(), &[("RAIN_PIPE", &pipe)]).elapsed)
        .collect();
    times.sort();
    assert!(
        times[15] < Duration::from_millis(50),
        "median {:?}",
        times[15]
    );
}
