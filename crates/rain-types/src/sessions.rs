//! The Claude session types: `ClaudeSession`, its permission request and the bodies and results of
//! the session routes. Feature section 03 (3.1).
//!
//! What the module that fills these types does (where each field comes from, when a session is
//! "waiting") is written in `docs/backend/03-sessions-jobs-idle.md`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::common::Status;

/// What the user answers to a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PermissionAnswer {
    Once,
    Always,
    Deny,
}

/// One open permission request of a session. It lives in memory only, never in a database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    // "req-" plus 8 hex characters, made by Rain; opaque to the UI.
    pub id: String,
    // The tool name as Claude Code sends it: "Bash", "Edit", "mcp__server__tool".
    pub tool: String,
    // One line, at most 120 characters.
    pub summary: String,
    // The exact command or a diff-like text, at most 4096 characters.
    pub detail: String,
    // The time Rain received the event.
    pub requested_at: String,
    // True while the hook program still waits for an answer from Rain. False once it gave up
    // (after about 20 seconds) or when the tool needs a typed answer (a question, a plan): the
    // request is still open in Claude Code, but only the terminal or the app can answer it.
    pub answerable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SessionEventKind {
    Prompt,
    Tool,
    Edit,
    Message,
    Permission,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub at: String,
    #[ts(inline)]
    pub kind: SessionEventKind,
    // At most 200 characters.
    pub text: String,
}

/// Where the session runs. It decides what focus and stop can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SessionHost {
    Terminal,
    Desktop,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ClaudeSession {
    // The Claude session id (a UUID string).
    pub id: String,
    pub title: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    // A short label such as "sonnet 4.5"; "unknown" when nothing says.
    pub model: String,
    #[ts(inline)]
    pub status: Status,
    pub started_at: String,
    pub last_activity_at: String,
    // The size of the context of the last request, not a running total.
    pub tokens: u64,
    // At most 600 characters.
    pub last_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<PermissionRequest>,
    // Newest first, at most 8.
    pub recent: Vec<SessionEvent>,
    #[ts(inline)]
    pub host: SessionHost,
    // The process still runs. False for ended sessions (status success or failed).
    pub alive: bool,
}

/// The body of `POST /sessions/{id}/permission`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionAnswerBody {
    pub request_id: String,
    #[ts(inline)]
    pub answer: PermissionAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PermissionAnswerResult {
    // What really happened: "always" becomes "once" when no session rule could be made.
    #[ts(inline)]
    pub applied: PermissionAnswer,
    // For applied = always: the rule that now holds for this session, for example
    // "Bash(npm install)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_text: Option<String>,
}

/// What `POST /sessions/{id}/focus` managed to do. `none` is not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FocusKind {
    // A specific window of the session's host was brought to the front.
    Window,
    // The host application was brought to the front, but not the session's tab.
    App,
    // Nothing could be done.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusResult {
    #[ts(inline)]
    pub focused: FocusKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn session() -> ClaudeSession {
        ClaudeSession {
            id: "11111111-2222-3333-4444-555555555555".into(),
            title: "Fix the parser".into(),
            cwd: "C:\\work\\demo".into(),
            project_id: None,
            branch: None,
            model: "sonnet 4.5".into(),
            status: Status::Waiting,
            started_at: "2026-11-21T09:30:00.000Z".into(),
            last_activity_at: "2026-11-21T09:31:00.000Z".into(),
            tokens: 36_000,
            last_message: "Done.".into(),
            request: Some(PermissionRequest {
                id: "req-0a1b2c3d".into(),
                tool: "Bash".into(),
                summary: "npm install".into(),
                detail: "npm install".into(),
                requested_at: "2026-11-21T09:31:00.000Z".into(),
                answerable: true,
            }),
            recent: Vec::new(),
            host: SessionHost::Terminal,
            alive: true,
        }
    }

    #[test]
    fn a_session_uses_camel_case_and_leaves_absent_fields_out() {
        let json = round_trip(&session());
        assert_eq!(json["lastActivityAt"], "2026-11-21T09:31:00.000Z");
        assert_eq!(json["host"], "terminal");
        assert_eq!(json["request"]["requestedAt"], "2026-11-21T09:31:00.000Z");
        assert!(json.get("projectId").is_none());
        assert!(json.get("branch").is_none());
    }

    #[test]
    fn the_answer_body_rejects_unknown_fields_and_unknown_answers() {
        let ok: PermissionAnswerBody =
            serde_json::from_value(json!({ "requestId": "req-1", "answer": "always" })).unwrap();
        assert_eq!(ok.answer, PermissionAnswer::Always);
        assert!(
            serde_json::from_value::<PermissionAnswerBody>(
                json!({ "requestId": "req-1", "answer": "once", "extra": 1 })
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<PermissionAnswerBody>(
                json!({ "requestId": "req-1", "answer": "maybe" })
            )
            .is_err()
        );
    }

    #[test]
    fn the_result_leaves_out_a_missing_rule() {
        let once = PermissionAnswerResult {
            applied: PermissionAnswer::Once,
            rule_text: None,
        };
        assert_eq!(round_trip(&once), json!({ "applied": "once" }));
        let always = PermissionAnswerResult {
            applied: PermissionAnswer::Always,
            rule_text: Some("Bash(npm install)".into()),
        };
        assert_eq!(
            round_trip(&always),
            json!({ "applied": "always", "ruleText": "Bash(npm install)" })
        );
    }

    #[test]
    fn focus_results_are_written_as_words() {
        for (kind, word) in [
            (FocusKind::Window, "window"),
            (FocusKind::App, "app"),
            (FocusKind::None, "none"),
        ] {
            assert_eq!(
                round_trip(&FocusResult { focused: kind }),
                json!({ "focused": word })
            );
        }
    }
}
