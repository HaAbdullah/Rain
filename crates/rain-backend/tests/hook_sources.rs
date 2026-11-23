//! The hook program has no dependencies, so it carries copies of two pieces of this project: the
//! pipe name rule and the JSON it builds by hand. This test includes the hook's own source files
//! and compares them with the originals, so that the copies cannot drift apart (section 03 6.2.1
//! and 6.2.2).
#![allow(dead_code)]

#[path = "../../rain-hook/src/envelope.rs"]
mod envelope;
#[path = "../../rain-hook/src/names.rs"]
mod names;

use rain_backend::modules::sessions::{HookEnvelope, HookMode};
use rain_types::env::pipe_name_for;
use serde_json::{Value, json};

#[test]
fn pipe_name_matches() {
    let list: Value =
        serde_json::from_str(include_str!("../../rain-types/user-slug-cases.json")).unwrap();
    let mut users: Vec<String> = list["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["name"].as_str().unwrap().to_owned())
        .collect();
    users.extend(["", "Zoë", "名前", "a b.c_d", &"x".repeat(100)].map(str::to_owned));
    for user in &users {
        for sandbox in [false, true] {
            assert_eq!(
                names::pipe_name_for(None, user, sandbox),
                pipe_name_for(None, user, sandbox).ok(),
                "user {user:?}, sandbox {sandbox}"
            );
        }
        assert_eq!(names::user_slug(user), rain_types::env::user_slug(user));
    }
    for name in [
        "rain-test-1.a_B",
        "x",
        "",
        "has space",
        "a\\b",
        "é",
        &"x".repeat(64),
        &"x".repeat(65),
    ] {
        assert_eq!(
            names::pipe_name_for(Some(name), "Ada", false),
            pipe_name_for(Some(name), "Ada", false).ok(),
            "override {name:?}"
        );
    }
}

#[test]
fn envelope_matches_forwarder() {
    let events = [
        json!({ "session_id": "s", "hook_event_name": "Stop" }),
        json!({ "tool_input": { "command": "echo \"quoted\" and \\ backslash\nnew line\ttab" } }),
        json!({ "prompt": "unicode: é ü 名前 😀 and a control \u{1}" }),
        json!({ "nested": { "list": [1, 2.5, null, true, { "k": "v" }] } }),
        json!({}),
    ];
    let envs = [
        envelope::Env {
            run_id: None,
            job_id: None,
            entrypoint: None,
            claude_pid: None,
        },
        envelope::Env {
            run_id: Some("run-20261121-060000-ab12"),
            job_id: Some("job \"x\""),
            entrypoint: Some("cli"),
            claude_pid: Some(4242),
        },
        envelope::Env {
            run_id: Some("back\\slash"),
            job_id: None,
            entrypoint: Some("é名前"),
            claude_pid: None,
        },
    ];
    for event in &events {
        for env in &envs {
            for mode in ["event", "permission"] {
                let text = envelope::build(mode, env, &event.to_string());
                let parsed = HookEnvelope::parse(text.as_bytes()).unwrap_or_else(|| {
                    panic!("the backend cannot read what the hook builds: {text}")
                });
                assert_eq!(parsed.event, *event, "the event is embedded unchanged");
                assert_eq!(
                    parsed.mode,
                    if mode == "event" {
                        HookMode::Event
                    } else {
                        HookMode::Permission
                    }
                );
                assert_eq!(parsed.env.run_id.as_deref(), env.run_id);
                assert_eq!(parsed.env.job_id.as_deref(), env.job_id);
                assert_eq!(parsed.env.entrypoint.as_deref(), env.entrypoint);
                assert_eq!(parsed.env.process_id, env.claude_pid);
            }
        }
    }
}

#[test]
fn the_json_string_escape_survives_a_round_trip() {
    for text in [
        "",
        "plain",
        "a\"b",
        "a\\b",
        "line\nbreak\r\n",
        "\u{0}\u{1f}",
        "名前 😀",
    ] {
        let parsed: String = serde_json::from_str(&envelope::json_string(text)).unwrap();
        assert_eq!(parsed, text);
    }
}
