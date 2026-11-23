//! The body of `POST /internal/hook`: what `rain-hook` sends. Section 03 6.2.2.
//!
//! These types are NOT in `rain-types`: the hook program has no dependencies and cannot use that
//! crate. They are written to accept what an older hook program sends and what a newer backend
//! may add: no `deny_unknown_fields`, and every field of `env` and of the event is optional. A
//! missing field means "unknown" and is never an error.

use serde::Deserialize;
use serde_json::Value;

/// The only version of the envelope that exists.
pub const ENVELOPE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookMode {
    /// An observation. Claude Code does not wait for the answer.
    Event,
    /// A permission request. The answer decides what Claude Code does.
    Permission,
}

/// What the hook program says about itself and its surroundings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEnv {
    /// Set when the Claude process is a run that Rain itself started (a Claude job).
    pub run_id: Option<String>,
    pub job_id: Option<String>,
    pub entrypoint: Option<String>,
    /// The process id of Claude Code (the variable `CLAUDE_PID` of the hook's environment).
    #[serde(rename = "claudePid")]
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookEnvelope {
    pub v: u32,
    pub mode: HookMode,
    #[serde(default)]
    pub env: HookEnv,
    /// The input of Claude Code, untouched. Read with `serde_json::Value` because its fields
    /// change between versions of Claude Code.
    pub event: Value,
}

impl HookEnvelope {
    /// Parses the body. `None` for anything that is not an envelope of version 1 around a JSON
    /// object.
    pub fn parse(body: &[u8]) -> Option<Self> {
        let envelope: Self = serde_json::from_slice(body).ok()?;
        (envelope.v == ENVELOPE_VERSION && envelope.event.is_object()).then_some(envelope)
    }
}

/// A text field of the event, when it is there and not empty.
pub fn text<'a>(event: &'a Value, key: &str) -> Option<&'a str> {
    event
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_envelope_is_read() {
        let body = br#"{"v":1,"mode":"permission","env":{"runId":"run-1","claudePid":42},"event":{"session_id":"s","tool_name":"Bash"}}"#;
        let envelope = HookEnvelope::parse(body).unwrap();
        assert_eq!(envelope.mode, HookMode::Permission);
        assert_eq!(envelope.env.run_id.as_deref(), Some("run-1"));
        assert_eq!(envelope.env.process_id, Some(42));
        assert_eq!(text(&envelope.event, "session_id"), Some("s"));
    }

    #[test]
    fn unknown_fields_and_a_missing_env_are_fine() {
        let body = br#"{"v":1,"mode":"event","future":true,"event":{"hook_event_name":"Stop","extra":[1]}}"#;
        let envelope = HookEnvelope::parse(body).unwrap();
        assert_eq!(envelope.env.run_id, None);
    }

    #[test]
    fn other_versions_and_broken_bodies_are_refused() {
        for bad in [
            &br#"{"v":2,"mode":"event","event":{}}"#[..],
            br#"{"v":1,"mode":"other","event":{}}"#,
            br#"{"v":1,"mode":"event","event":[1]}"#,
            br#"{"v":1,"mode":"event"}"#,
            b"not json",
            b"",
        ] {
            assert!(
                HookEnvelope::parse(bad).is_none(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
