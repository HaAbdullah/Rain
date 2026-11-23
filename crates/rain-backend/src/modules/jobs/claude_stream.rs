//! Turns the stream of a headless Claude run into log lines. Section 03 6.11.8.
//!
//! `claude -p --output-format stream-json` prints one JSON object per line. Rain keeps what helps
//! a person to see why a run did what it did, and drops file contents and the rest. A line that
//! is not JSON is shown as it is. Pure: no state, no I/O.

use rain_types::LogLevel;
use serde_json::Value;

/// A text block is cut to this many characters in the log.
const TEXT_MAX: usize = 1000;
/// A tool summary and an error line are cut to this many characters.
const SUMMARY_MAX: usize = 200;

/// The closing object of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeResult {
    /// `subtype` is "success" and `is_error` is not true.
    pub success: bool,
    pub subtype: String,
    pub turns: Option<u64>,
    pub cost_usd: Option<f64>,
    pub errors: Vec<String>,
}

/// What one stream line gives.
#[derive(Debug, Default, PartialEq)]
pub struct Mapped {
    pub lines: Vec<(LogLevel, String)>,
    /// The Claude session id of the run, from the first object.
    pub session_id: Option<String>,
    pub result: Option<ClaudeResult>,
}

pub fn map_stdout_line(line: &str) -> Mapped {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        let text = line.trim_end();
        return if text.is_empty() {
            Mapped::default()
        } else {
            Mapped {
                lines: vec![(LogLevel::Info, text.to_owned())],
                ..Mapped::default()
            }
        };
    };
    let kind = value["type"].as_str().unwrap_or_default();
    let subtype = value["subtype"].as_str().unwrap_or_default();
    match (kind, subtype) {
        ("system", "init") => {
            let model = value["model"].as_str().unwrap_or("unknown");
            Mapped {
                lines: vec![(
                    LogLevel::Info,
                    format!("Claude session started (model {model})"),
                )],
                session_id: value["session_id"].as_str().map(str::to_owned),
                result: None,
            }
        }
        ("system", "api_retry") => {
            let attempt = value["attempt"].as_u64().unwrap_or(0);
            let max = value["max_retries"].as_u64().unwrap_or(0);
            only(
                LogLevel::Warn,
                format!("Retrying the request (attempt {attempt} of {max})"),
            )
        }
        ("system", "permission_denied") => {
            let tool = value["tool_name"]
                .as_str()
                .or_else(|| value["tool"].as_str())
                .unwrap_or("a tool");
            only(LogLevel::Warn, format!("Denied {tool}"))
        }
        ("assistant", _) => assistant_lines(&value),
        ("user", _) => tool_error_lines(&value),
        ("result", _) => result_object(&value),
        // `hook_started`, `hook_response` and anything unknown carry nothing for the reader.
        _ => Mapped::default(),
    }
}

fn only(level: LogLevel, text: String) -> Mapped {
    Mapped {
        lines: vec![(level, text)],
        ..Mapped::default()
    }
}

fn assistant_lines(value: &Value) -> Mapped {
    let mut lines = Vec::new();
    for block in value["message"]["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(text) = block["text"].as_str().filter(|t| !t.trim().is_empty()) {
                    lines.push((LogLevel::Info, cut(text.trim(), TEXT_MAX)));
                }
            }
            Some("tool_use") => {
                let name = block["name"].as_str().unwrap_or("tool");
                let summary = tool_summary(&block["input"]);
                let text = if summary.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}: {summary}")
                };
                lines.push((LogLevel::Info, text));
            }
            _ => {}
        }
    }
    Mapped {
        lines,
        ..Mapped::default()
    }
}

/// One line about what a tool was asked to do: its command, path, pattern or similar.
fn tool_summary(input: &Value) -> String {
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "description",
    ] {
        if let Some(text) = input[key].as_str() {
            let first = text.lines().next().unwrap_or_default().trim();
            return cut(first, SUMMARY_MAX);
        }
    }
    String::new()
}

/// A tool result that is an error gets a warning line; other results (file contents) are dropped.
fn tool_error_lines(value: &Value) -> Mapped {
    let mut lines = Vec::new();
    for block in value["message"]["content"].as_array().into_iter().flatten() {
        if block["type"] == "tool_result" && block["is_error"] == true {
            let text = match &block["content"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            let first = text.lines().next().unwrap_or_default().trim();
            lines.push((LogLevel::Warn, format!("  {}", cut(first, SUMMARY_MAX))));
        }
    }
    Mapped {
        lines,
        ..Mapped::default()
    }
}

fn result_object(value: &Value) -> Mapped {
    let subtype = value["subtype"].as_str().unwrap_or("unknown").to_owned();
    let success = subtype == "success" && value["is_error"] != true;
    let errors = value["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.as_str().map(|text| cut(text, SUMMARY_MAX)))
        .collect();
    Mapped {
        lines: Vec::new(),
        session_id: None,
        result: Some(ClaudeResult {
            success,
            subtype,
            turns: value["num_turns"].as_u64(),
            cost_usd: value["total_cost_usd"].as_f64(),
            errors,
        }),
    }
}

/// `text` cut to `max` characters, with a mark that it was cut.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Mapped {
        map_stdout_line(&value.to_string())
    }

    #[test]
    fn claude_stream_map() {
        let init = map(
            json!({ "type": "system", "subtype": "init", "session_id": "sess-1", "model": "claude-test" }),
        );
        assert_eq!(
            init.lines[0].1,
            "Claude session started (model claude-test)"
        );
        assert_eq!(init.session_id.as_deref(), Some("sess-1"));

        let assistant = map(json!({ "type": "assistant", "message": { "content": [
            { "type": "text", "text": "I will check the notes." },
            { "type": "tool_use", "name": "Bash", "input": { "command": "git status\nsecond line" } },
            { "type": "tool_use", "name": "Skill", "input": {} }
        ] } }));
        let texts: Vec<_> = assistant.lines.iter().map(|l| l.1.as_str()).collect();
        assert_eq!(
            texts,
            ["I will check the notes.", "Bash: git status", "Skill"]
        );

        let failed_tool = map(json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "is_error": true, "content": "Permission denied\nmore" }
        ] } }));
        assert_eq!(
            failed_tool.lines,
            [(LogLevel::Warn, "  Permission denied".to_owned())]
        );

        let fine_tool = map(json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "content": "the whole file contents" }
        ] } }));
        assert!(fine_tool.lines.is_empty(), "file contents are not logged");

        let retry = map(
            json!({ "type": "system", "subtype": "api_retry", "attempt": 2, "max_retries": 5 }),
        );
        assert_eq!(
            retry.lines[0],
            (
                LogLevel::Warn,
                "Retrying the request (attempt 2 of 5)".to_owned()
            )
        );

        let denied =
            map(json!({ "type": "system", "subtype": "permission_denied", "tool_name": "Write" }));
        assert_eq!(denied.lines[0], (LogLevel::Warn, "Denied Write".to_owned()));

        for skipped in ["hook_started", "hook_response", "something_new"] {
            assert_eq!(
                map(json!({ "type": "system", "subtype": skipped })),
                Mapped::default()
            );
        }
    }

    #[test]
    fn a_line_that_is_not_json_is_shown_as_it_is_and_an_empty_one_is_dropped() {
        assert_eq!(
            map_stdout_line("plain text from the program").lines,
            [(LogLevel::Info, "plain text from the program".to_owned())]
        );
        assert_eq!(map_stdout_line("   "), Mapped::default());
    }

    #[test]
    fn the_result_object_says_whether_the_run_worked() {
        let ok = map(
            json!({ "type": "result", "subtype": "success", "is_error": false, "num_turns": 4, "total_cost_usd": 0.5 }),
        );
        let result = ok.result.unwrap();
        assert!(result.success);
        assert_eq!((result.turns, result.cost_usd), (Some(4), Some(0.5)));

        let flagged = map(json!({ "type": "result", "subtype": "success", "is_error": true }));
        assert!(!flagged.result.unwrap().success);

        let failed = map(
            json!({ "type": "result", "subtype": "error_max_turns", "errors": ["Ran out of turns"] }),
        );
        let result = failed.result.unwrap();
        assert!(!result.success);
        assert_eq!(result.subtype, "error_max_turns");
        assert_eq!(result.errors, ["Ran out of turns"]);
    }

    #[test]
    fn long_text_is_cut() {
        let long = "x".repeat(3000);
        let mapped = map(
            json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": long }] } }),
        );
        assert_eq!(mapped.lines[0].1.chars().count(), 1003);
        assert!(mapped.lines[0].1.ends_with("..."));
    }
}
