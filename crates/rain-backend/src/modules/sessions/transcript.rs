//! Reads a Claude Code transcript (`<claude folder>\projects\<folder>\<session id>.jsonl`), read
//! only. Section 03 6.4 (facts from the tail) and 6.8 (the readable log).
//!
//! A transcript is an internal format of Claude Code. It is read line by line as loose JSON with
//! every field optional, and only four kinds of line matter (`user`, `assistant`, `custom-title`,
//! `ai-title`; the log also uses `system`). Every other line, and the large `api-request-blob`
//! lines, are thrown away unread. A transcript can be 13 MB, so only the end of the file is read.
//! Nothing of the text is ever logged.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use rain_types::{LogLevel, LogLine, SessionEvent, SessionEventKind};
use serde_json::Value;

use super::summary::{cut, first_line, summary};

/// The end of the file that is read for the facts of a session.
pub const TAIL_BYTES: u64 = 256 * 1024;
/// What the log route reads first, and the most it ever reads.
const LOG_FIRST_BYTES: u64 = 2 * 1024 * 1024;
const LOG_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// The log tries a larger part of the file when it found fewer lines than this.
const LOG_ENOUGH_LINES: usize = 200;
/// The log has at most this many lines, and one line at most this many characters.
pub const LOG_LINES_MAX: usize = 1000;
const LOG_LINE_CHARS: usize = 1000;
/// One session has at most this many recent events.
const RECENT_MAX: usize = 8;
/// The words of the model name that make a short label.
const MODEL_FAMILIES: [&str; 3] = ["opus", "sonnet", "haiku"];

/// The end of a file, and whether anything in front of it was left out.
pub struct Tail {
    pub bytes: Vec<u8>,
    pub cut_front: bool,
}

/// Reads at most the last `max` bytes of a file, read only.
pub fn read_tail(path: &Path, max: u64) -> std::io::Result<Tail> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let start = length.saturating_sub(max);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes)?;
    Ok(Tail {
        bytes,
        cut_front: start > 0,
    })
}

/// The lines of the kinds in `keep`, parsed. The first line is dropped when the front was cut (it
/// is probably half a line), and a line that does not parse is skipped (the last line of a file
/// that Claude is writing may be incomplete).
pub fn parse_lines(tail: &Tail, keep: &[&str]) -> Vec<Value> {
    let text = String::from_utf8_lossy(&tail.bytes);
    let mut lines = text.split('\n');
    if tail.cut_front {
        lines.next();
    }
    lines
        .filter(|line| {
            keep.iter()
                .any(|kind| line.contains(&format!("\"type\":\"{kind}\"")))
        })
        .filter_map(|line| serde_json::from_str::<Value>(line.trim_end_matches('\r')).ok())
        .filter(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| keep.contains(&kind))
        })
        .collect()
}

/// What the end of a transcript tells about the session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TailFacts {
    /// The model id as written, for `model_label`.
    pub model: Option<String>,
    pub tokens: Option<u64>,
    pub branch: Option<String>,
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub last_message: Option<String>,
    /// Newest first, at most 8.
    pub recent: Vec<SessionEvent>,
}

/// Reads the facts of a session from the lines of its tail (`user`, `assistant`, `custom-title`
/// and `ai-title` lines, oldest first).
pub fn facts(lines: &[Value]) -> TailFacts {
    let mut facts = TailFacts::default();
    let mut custom_title = None;
    let mut ai_title = None;
    let mut recent = Vec::new();

    for line in lines {
        let kind = line.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "custom-title" => custom_title = text_of(line, "customTitle").or(custom_title),
            "ai-title" => ai_title = text_of(line, "aiTitle").or(ai_title),
            "user" | "assistant" if is_side_line(line) => continue,
            _ => {}
        }
        if let Some(branch) = text_of(line, "gitBranch") {
            facts.branch = Some(branch);
        }
        let at = text_of(line, "timestamp").unwrap_or_default();
        match kind {
            "user" => {
                if let Some(prompt) = human_prompt(line) {
                    if facts.first_prompt.is_none() {
                        facts.first_prompt = Some(cut(first_line(&prompt), 60));
                    }
                    recent.push(event(&at, SessionEventKind::Prompt, &prompt));
                }
            }
            "assistant" => {
                let message = line.get("message");
                if let Some(model) = message
                    .and_then(|m| m.get("model"))
                    .and_then(Value::as_str)
                    .filter(|model| !model.starts_with('<'))
                {
                    facts.model = Some(model.to_owned());
                }
                if let Some(total) = message
                    .and_then(|m| m.get("usage"))
                    .and_then(context_tokens)
                {
                    facts.tokens = Some(total);
                }
                let blocks = message
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array);
                for block in blocks.into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = text_of(block, "text").unwrap_or_default();
                            if !text.trim().is_empty() {
                                facts.last_message = Some(cut(text.trim(), 600));
                                recent.push(event(&at, SessionEventKind::Message, &text));
                            }
                        }
                        Some("tool_use") => {
                            let name = text_of(block, "name").unwrap_or_default();
                            let input = block.get("input").cloned().unwrap_or(Value::Null);
                            let line = format!("{name}: {}", summary(&name, &input, ""));
                            recent.push(event(&at, SessionEventKind::Tool, &line));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    facts.title = custom_title.or(ai_title);
    recent.reverse();
    recent.truncate(RECENT_MAX);
    facts.recent = recent;
    facts
}

fn event(at: &str, kind: SessionEventKind, text: &str) -> SessionEvent {
    SessionEvent {
        at: at.to_owned(),
        kind,
        text: cut(first_line(text), 200),
    }
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// A line of a sub-agent, a summary or a note of Claude Code to itself: not part of the talk
/// between the user and Claude.
fn is_side_line(line: &Value) -> bool {
    [
        "isSidechain",
        "isMeta",
        "isCompactSummary",
        "isVisibleInTranscriptOnly",
    ]
    .iter()
    .any(|key| line.get(*key).and_then(Value::as_bool) == Some(true))
}

/// `input + cache read + cache creation + output` of one request: the size of its context.
fn context_tokens(usage: &Value) -> Option<u64> {
    let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let total = number("input_tokens")
        + number("cache_read_input_tokens")
        + number("cache_creation_input_tokens")
        + number("output_tokens");
    (total > 0).then_some(total)
}

/// The text of a user line that the user typed: a string, or the `text` blocks of a list. Lines
/// that are results of tools, or that another program made, have none.
fn human_prompt(line: &Value) -> Option<String> {
    let origin = line
        .get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str);
    if origin.is_some_and(|kind| kind != "human") {
        return None;
    }
    let content = line.get("message")?.get("content")?;
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| text_of(block, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

/// "sonnet 4.5" from an id such as `claude-sonnet-4-5-20250929`: the family word plus the version
/// digits that follow it, joined by a dot. Any other text is kept as it is, and nothing is
/// "unknown".
pub fn model_label(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    if lower.is_empty() {
        return "unknown".to_owned();
    }
    let parts: Vec<&str> = lower.split(['-', '.', '_']).collect();
    // A version number is one or two digits. A date such as 20250929 is not one.
    let is_version = |part: &str| {
        !part.is_empty() && part.len() <= 2 && part.bytes().all(|b| b.is_ascii_digit())
    };
    for family in MODEL_FAMILIES {
        let Some(at) = parts.iter().position(|part| *part == family) else {
            continue;
        };
        // The version can come after the family (`sonnet-4-5`) or before it (`3-5-haiku`).
        let mut version: Vec<&str> = parts[at + 1..]
            .iter()
            .take_while(|part| is_version(part))
            .take(2)
            .copied()
            .collect();
        if version.is_empty() {
            version = parts[..at]
                .iter()
                .rev()
                .take_while(|part| is_version(part))
                .take(2)
                .copied()
                .collect();
            version.reverse();
        }
        return if version.is_empty() {
            family.to_owned()
        } else {
            format!("{family} {}", version.join("."))
        };
    }
    raw.trim().to_owned()
}

// ---- the readable log ----

/// The log of a transcript, oldest first, at most `LOG_LINES_MAX` lines. `Ok(None)` is not
/// possible: a missing or unreadable file is the error.
pub fn read_log(path: &Path) -> std::io::Result<Vec<LogLine>> {
    let length = std::fs::metadata(path)?.len();
    let mut size = LOG_FIRST_BYTES;
    loop {
        let tail = read_tail(path, size)?;
        let lines = parse_lines(&tail, &["user", "assistant", "system"]);
        let mut entries = log_entries(&lines);
        let done = entries.len() >= LOG_ENOUGH_LINES || size >= length || size >= LOG_MAX_BYTES;
        if !done {
            size = LOG_MAX_BYTES.min(size + 8 * 1024 * 1024);
            continue;
        }
        let mut cut_count = 0;
        if entries.len() > LOG_LINES_MAX {
            cut_count = entries.len() - LOG_LINES_MAX;
            entries.drain(..cut_count);
        }
        let note = if cut_count > 0 {
            Some(format!("{cut_count} earlier lines are not shown"))
        } else if tail.cut_front {
            Some("Earlier lines are not shown".to_owned())
        } else {
            None
        };
        if let Some(text) = note {
            let ts = entries.first().map(|e| e.ts.clone()).unwrap_or_default();
            entries.insert(
                0,
                LogLine {
                    ts,
                    level: LogLevel::Warn,
                    text,
                },
            );
        }
        return Ok(entries);
    }
}

/// The table of section 03 6.8, line by line in file order.
pub fn log_entries(lines: &[Value]) -> Vec<LogLine> {
    let mut out = Vec::new();
    for line in lines {
        let ts = text_of(line, "timestamp").unwrap_or_default();
        let push = |out: &mut Vec<LogLine>, level: LogLevel, text: String| {
            out.push(LogLine {
                ts: ts.clone(),
                level,
                text: cut(&text, LOG_LINE_CHARS),
            });
        };
        match line.get("type").and_then(Value::as_str) {
            Some("system") => {
                if line.get("subtype").and_then(Value::as_str) == Some("compact_boundary") {
                    push(
                        &mut out,
                        LogLevel::Warn,
                        "Conversation was compacted".to_owned(),
                    );
                }
            }
            Some("user") if !is_side_line(line) => {
                if let Some(prompt) = human_prompt(line) {
                    push(&mut out, LogLevel::Info, format!("You: {prompt}"));
                }
                let blocks = line
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array);
                for block in blocks.into_iter().flatten() {
                    let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") && is_error
                    {
                        push(
                            &mut out,
                            LogLevel::Error,
                            format!("  {}", first_line(&result_text(block))),
                        );
                    }
                }
            }
            Some("assistant") if !is_side_line(line) => {
                let is_error = line.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true)
                    || line.get("apiError").is_some_and(|v| !v.is_null());
                let blocks = line
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array);
                for block in blocks.into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = text_of(block, "text").unwrap_or_default();
                            if text.trim().is_empty() {
                                continue;
                            }
                            if is_error {
                                push(&mut out, LogLevel::Error, first_line(&text).to_owned());
                            } else {
                                push(&mut out, LogLevel::Info, format!("Claude: {text}"));
                            }
                        }
                        Some("tool_use") => {
                            let name = text_of(block, "name").unwrap_or_default();
                            let input = block.get("input").cloned().unwrap_or(Value::Null);
                            push(
                                &mut out,
                                LogLevel::Info,
                                format!("{name}: {}", summary(&name, &input, "")),
                            );
                        }
                        // Thinking and every other block are skipped.
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The text of a `tool_result` block: a string, or the `text` of its parts.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| text_of(part, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str, at: &str) -> Value {
        json!({ "type": "user", "timestamp": at, "gitBranch": "main", "message": { "role": "user", "content": text } })
    }

    fn assistant(blocks: Value, at: &str, usage: Value) -> Value {
        json!({ "type": "assistant", "timestamp": at, "message": {
            "id": "msg_1", "model": "claude-sonnet-4-5-20250929", "content": blocks, "usage": usage } })
    }

    #[test]
    fn model_labels_come_from_made_up_ids() {
        let cases = [
            ("claude-sonnet-4-5-20250929", "sonnet 4.5"),
            ("claude-opus-4-1-20250805", "opus 4.1"),
            ("claude-3-5-haiku-20241022", "haiku 3.5"),
            ("claude-sonnet-4-20250514", "sonnet 4"),
            ("some-other-model", "some-other-model"),
            ("", "unknown"),
            ("   ", "unknown"),
        ];
        for (raw, label) in cases {
            assert_eq!(model_label(raw), label, "{raw:?}");
        }
    }

    #[test]
    fn facts_come_from_the_newest_lines() {
        let lines = vec![
            json!({ "type": "ai-title", "aiTitle": "AI title" }),
            user(
                "Please fix the parser\nand the lexer",
                "2026-11-21T09:00:00.000Z",
            ),
            assistant(
                json!([{ "type": "text", "text": "Looking." }, { "type": "tool_use", "name": "Bash", "input": { "command": "cargo test" } }]),
                "2026-11-21T09:00:05.000Z",
                json!({ "input_tokens": 10, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 5, "output_tokens": 20 }),
            ),
            json!({ "type": "custom-title", "customTitle": "My title" }),
            assistant(
                json!([{ "type": "text", "text": "All done." }]),
                "2026-11-21T09:00:10.000Z",
                json!({ "input_tokens": 1, "cache_read_input_tokens": 200, "cache_creation_input_tokens": 0, "output_tokens": 30 }),
            ),
        ];
        let facts = facts(&lines);
        assert_eq!(
            facts.title.as_deref(),
            Some("My title"),
            "a custom title beats an AI title"
        );
        assert_eq!(facts.first_prompt.as_deref(), Some("Please fix the parser"));
        assert_eq!(facts.last_message.as_deref(), Some("All done."));
        assert_eq!(facts.tokens, Some(231));
        assert_eq!(facts.branch.as_deref(), Some("main"));
        assert_eq!(facts.model.as_deref(), Some("claude-sonnet-4-5-20250929"));
        let kinds: Vec<_> = facts.recent.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                SessionEventKind::Message,
                SessionEventKind::Tool,
                SessionEventKind::Message,
                SessionEventKind::Prompt
            ],
            "newest first"
        );
        assert_eq!(facts.recent[1].text, "Bash: cargo test");
    }

    #[test]
    fn synthetic_models_and_side_lines_are_ignored() {
        let mut synthetic = assistant(json!([{ "type": "text", "text": "x" }]), "t", json!({}));
        synthetic["message"]["model"] = json!("<synthetic>");
        let mut side = user("a note to itself", "t");
        side["isMeta"] = json!(true);
        let mut from_tool = user("ignored", "t");
        from_tool["origin"] = json!({ "kind": "task-notification" });
        let facts = facts(&[synthetic, side, from_tool]);
        assert_eq!(facts.model, None);
        assert_eq!(facts.first_prompt, None);
    }

    #[test]
    fn only_the_wanted_kinds_are_parsed_and_junk_is_skipped() {
        let text = [
            "tail of a cut line\"}",
            r#"{"type":"api-request-blob","blob":"CANARY-BLOB"}"#,
            r#"{"type":"session-owner","owner":"CANARY-OWNER"}"#,
            r#"{"type":"user","message":{"content":"hello"}}"#,
            "{ not json",
            r#"{"type":"assistant","message":{"content":[]}}"#,
        ]
        .join("\n");
        let tail = Tail {
            bytes: text.into_bytes(),
            cut_front: true,
        };
        let lines = parse_lines(&tail, &["user", "assistant"]);
        assert_eq!(lines.len(), 2);
        // Without a cut front the first line is kept: it is a whole line.
        let whole = Tail {
            bytes: br#"{"type":"user","message":{"content":"first"}}"#.to_vec(),
            cut_front: false,
        };
        assert_eq!(parse_lines(&whole, &["user"]).len(), 1);
    }

    #[test]
    fn the_log_follows_the_table_of_the_section() {
        let lines = vec![
            user("Hello there", "2026-11-21T09:00:00.000Z"),
            assistant(
                json!([
                    { "type": "thinking", "thinking": "CANARY-THINKING" },
                    { "type": "text", "text": "Hi!" },
                    { "type": "tool_use", "name": "Edit", "input": { "file_path": "a.rs" } }
                ]),
                "2026-11-21T09:00:01.000Z",
                json!({}),
            ),
            json!({ "type": "user", "timestamp": "2026-11-21T09:00:02.000Z", "message": { "content": [
                { "type": "tool_result", "is_error": false, "content": "CANARY-FILE-CONTENT" },
                { "type": "tool_result", "is_error": true, "content": "boom\nsecond line" }
            ] } }),
            json!({ "type": "assistant", "isApiErrorMessage": true, "timestamp": "2026-11-21T09:00:03.000Z",
                    "message": { "content": [{ "type": "text", "text": "API Error: overloaded\nmore" }] } }),
            json!({ "type": "system", "subtype": "compact_boundary", "timestamp": "2026-11-21T09:00:04.000Z" }),
            json!({ "type": "system", "subtype": "stop_hook_summary" }),
            {
                let mut l = user("meta", "t");
                l["isMeta"] = json!(true);
                l
            },
        ];
        let log = log_entries(&lines);
        let shown: Vec<(LogLevel, &str)> = log.iter().map(|e| (e.level, e.text.as_str())).collect();
        assert_eq!(
            shown,
            [
                (LogLevel::Info, "You: Hello there"),
                (LogLevel::Info, "Claude: Hi!"),
                (LogLevel::Info, "Edit: a.rs"),
                (LogLevel::Error, "  boom"),
                (LogLevel::Error, "API Error: overloaded"),
                (LogLevel::Warn, "Conversation was compacted"),
            ]
        );
        assert!(!format!("{log:?}").contains("CANARY"));
    }

    #[test]
    fn the_log_is_capped_and_says_so() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("s.jsonl");
        let mut text = String::new();
        for n in 0..1100 {
            text.push_str(&format!(
                "{{\"type\":\"user\",\"timestamp\":\"2026-11-21T09:00:00.000Z\",\"message\":{{\"content\":\"line {n}\"}}}}\n"
            ));
        }
        std::fs::write(&path, text).unwrap();
        let log = read_log(&path).unwrap();
        assert_eq!(log.len(), LOG_LINES_MAX + 1);
        assert_eq!(log[0].level, LogLevel::Warn);
        assert_eq!(log[0].text, "100 earlier lines are not shown");
        assert_eq!(log.last().unwrap().text, "You: line 1099");
        assert!(read_log(&temp.path().join("missing.jsonl")).is_err());
    }

    #[test]
    fn only_the_end_of_a_big_file_is_read() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("big.jsonl");
        std::fs::write(&path, "x".repeat(5000)).unwrap();
        let tail = read_tail(&path, 1000).unwrap();
        assert_eq!(tail.bytes.len(), 1000);
        assert!(tail.cut_front);
        assert!(!read_tail(&path, 10_000).unwrap().cut_front);
    }
}
