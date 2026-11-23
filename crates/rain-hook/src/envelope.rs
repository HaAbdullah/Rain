//! Builds the text that is sent to the backend: a small JSON object around the unchanged input of
//! Claude Code. Done by string concatenation, because the hook has no JSON library. The backend
//! parses it with `serde_json` (`HookEnvelope` in `sessions/hook_wire.rs`); a test of the backend
//! includes this file and parses what it builds. Keep this file free of `crate::` paths.

/// What the hook program says about itself. The input of Claude Code is the event.
pub struct Env<'a> {
    pub run_id: Option<&'a str>,
    pub job_id: Option<&'a str>,
    pub entrypoint: Option<&'a str>,
    pub claude_pid: Option<u32>,
}

/// `"text"` as JSON: quotes, backslashes and control characters are escaped. Everything else,
/// including Unicode, is written as it is.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The envelope for `mode` (`event` or `permission`). `event_json` is the input of Claude Code,
/// already checked to be one JSON object, and is embedded unchanged.
pub fn build(mode: &str, env: &Env<'_>, event_json: &str) -> String {
    let mut fields = Vec::new();
    if let Some(value) = env.run_id {
        fields.push(format!("\"runId\":{}", json_string(value)));
    }
    if let Some(value) = env.job_id {
        fields.push(format!("\"jobId\":{}", json_string(value)));
    }
    if let Some(value) = env.entrypoint {
        fields.push(format!("\"entrypoint\":{}", json_string(value)));
    }
    if let Some(pid) = env.claude_pid {
        fields.push(format!("\"claudePid\":{pid}"));
    }
    format!(
        "{{\"v\":1,\"mode\":{},\"env\":{{{}}},\"event\":{}}}",
        json_string(mode),
        fields.join(","),
        event_json
    )
}
