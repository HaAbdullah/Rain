//! What a tool call looks like to the user, and what Rain answers to a permission request.
//! Section 03 6.5.2 (summary and detail), 6.5.5 (the answer) and 6.5.7 ("Always allow").
//!
//! Everything here is a pure function of the JSON that Claude Code sent, so it is tested with
//! plain values. Nothing here logs: a command or a file text can hold anything.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rain_types::PermissionAnswer;
use serde_json::{Map, Value, json};

/// A summary is one line and at most this many characters.
pub const SUMMARY_MAX: usize = 120;
/// A detail is at most this many characters.
pub const DETAIL_MAX: usize = 4096;
/// Tools whose answer needs more than "allow": a question to the user, a plan to approve. Rain
/// cannot answer them (it would have to fill in `updatedInput`), so it shows them as "answer in
/// the terminal".
pub const TYPED_ANSWER_TOOLS: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];

/// Whether "Always allow" for the edit tools may use a `setMode` suggestion of Claude Code. The
/// shape of that suggestion was never seen to work (gate item G1.12 of section 03 checks it), so
/// until someone has seen it work the answer is "allowed once". Switching this to `true` is the
/// only change that gate result needs.
const EDIT_ALWAYS_PROVEN: bool = false;

const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Cuts `text` to `max` characters; a cut text ends with `...`.
pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(3)).collect();
    format!("{kept}...")
}

/// The first line of `text`, trimmed.
pub fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

// ---- summary and detail ----

/// `path` relative to `cwd` when it lies under it (case does not matter, `/` equals `\`), else
/// as it is.
pub fn relative_path(path: &str, cwd: &str) -> String {
    let normal = |text: &str| text.replace('/', "\\");
    let (path_n, cwd_n) = (normal(path), normal(cwd));
    let cwd_n = cwd_n.trim_end_matches('\\');
    if cwd_n.is_empty() || path_n.len() <= cwd_n.len() {
        return path.to_owned();
    }
    let (head, tail) = path_n.split_at_checked(cwd_n.len()).unwrap_or(("", ""));
    if head.eq_ignore_ascii_case(cwd_n) && tail.starts_with('\\') {
        tail.trim_start_matches('\\').to_owned()
    } else {
        path.to_owned()
    }
}

fn string_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

/// One line for a row: "npm install", "src\main.rs", "Grep pattern".
pub fn summary(tool: &str, input: &Value, cwd: &str) -> String {
    let line = match tool {
        "Bash" => first_line(string_field(input, "command").unwrap_or("")).to_owned(),
        "Edit" | "Write" => relative_path(string_field(input, "file_path").unwrap_or(""), cwd),
        "MultiEdit" => {
            let count = input
                .get("edits")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let path = relative_path(string_field(input, "file_path").unwrap_or(""), cwd);
            format!("{path} ({count} edits)")
        }
        "NotebookEdit" => relative_path(string_field(input, "notebook_path").unwrap_or(""), cwd),
        "Read" | "Glob" | "Grep" | "WebFetch" | "WebSearch" => {
            ["file_path", "path", "pattern", "url", "query"]
                .iter()
                .find_map(|key| string_field(input, key))
                .map(|text| first_line(text).to_owned())
                .unwrap_or_default()
        }
        name if name.starts_with("mcp__") => name.to_owned(),
        _ => input
            .as_object()
            .and_then(|map| map.values().find_map(Value::as_str))
            .map(|text| first_line(text).to_owned())
            .unwrap_or_default(),
    };
    let line = if line.is_empty() {
        tool.to_owned()
    } else {
        line
    };
    cut(&line, SUMMARY_MAX)
}

/// The text of the pane: exactly what will run, or a diff-like text for an edit. The UI tints
/// lines that start with `---`, `+++` or `@@`, so no UI change is needed for it.
pub fn detail(tool: &str, input: &Value, cwd: &str) -> String {
    let text = match tool {
        // Only the command: a description that Claude wrote is not what runs.
        "Bash" => string_field(input, "command").unwrap_or("").to_owned(),
        "Edit" => {
            let path = relative_path(string_field(input, "file_path").unwrap_or(""), cwd);
            let mut out = diff_head(&path, "edit");
            push_edit(
                &mut out,
                string_field(input, "old_string").unwrap_or(""),
                string_field(input, "new_string").unwrap_or(""),
            );
            out
        }
        "MultiEdit" => {
            let path = relative_path(string_field(input, "file_path").unwrap_or(""), cwd);
            let mut out = format!("--- {path}\n+++ {path}\n");
            let edits = input.get("edits").and_then(Value::as_array);
            for (index, edit) in edits.into_iter().flatten().enumerate() {
                out.push_str(&format!("@@ edit {} @@\n", index + 1));
                push_edit(
                    &mut out,
                    string_field(edit, "old_string").unwrap_or(""),
                    string_field(edit, "new_string").unwrap_or(""),
                );
            }
            out
        }
        "Write" => {
            let path = relative_path(string_field(input, "file_path").unwrap_or(""), cwd);
            let mut out = diff_head(&path, "new content");
            for line in string_field(input, "content").unwrap_or("").lines() {
                out.push_str(&format!("+{line}\n"));
            }
            out
        }
        "NotebookEdit" => string_field(input, "new_source").unwrap_or("").to_owned(),
        _ => pretty(input),
    };
    cut_detail(text.trim_end_matches('\n'))
}

fn diff_head(path: &str, hunk: &str) -> String {
    format!("--- {path}\n+++ {path}\n@@ {hunk} @@\n")
}

fn push_edit(out: &mut String, old: &str, new: &str) {
    for line in old.lines() {
        out.push_str(&format!("-{line}\n"));
    }
    for line in new.lines() {
        out.push_str(&format!("+{line}\n"));
    }
}

/// Cuts a detail to `DETAIL_MAX` characters and says how much was left out.
fn cut_detail(text: &str) -> String {
    let total = text.chars().count();
    if total <= DETAIL_MAX {
        return text.to_owned();
    }
    let kept: String = text.chars().take(DETAIL_MAX).collect();
    format!("{kept}\n... {} more characters", total - DETAIL_MAX)
}

// ---- JSON helpers ----

/// The same value with the keys of every object in sorted order.
pub fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let sorted: Map<String, Value> = keys
                .into_iter()
                .map(|key| (key.clone(), canonical(&map[key])))
                .collect();
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Pretty JSON with sorted keys, so the same input always gives the same text. A plain string is
/// shown as it is.
pub fn pretty(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(&canonical(other)).unwrap_or_default(),
    }
}

/// A number that is equal for equal inputs, whatever the order of their keys. A permission
/// request has no tool call id, so a later `PostToolUse` is matched to it by tool name and this
/// hash (section 03 6.5.6).
pub fn input_hash(input: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    canonical(input).to_string().hash(&mut hasher);
    hasher.finish()
}

// ---- the answer ----

/// What Rain says to Claude Code, as the exact JSON text that the hook program prints.
pub fn decision_json(answer: PermissionAnswer, session_rule: Option<&Value>) -> String {
    let decision = match (answer, session_rule) {
        (PermissionAnswer::Deny, _) => {
            json!({ "behavior": "deny", "message": "Denied from Rain", "interrupt": false })
        }
        (PermissionAnswer::Always, Some(rule)) => {
            json!({ "behavior": "allow", "updatedPermissions": [rule] })
        }
        // `Once`, and `Always` with no rule to add: a plain allow. `onFailure` is never set: it
        // would turn a failure into a denial.
        _ => json!({ "behavior": "allow" }),
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
        .to_string()
}

/// What "Always allow" turns into.
#[derive(Debug, Clone, PartialEq)]
pub enum AlwaysChoice {
    /// A rule for this session only. `update` is the entry of `updatedPermissions`; its
    /// destination is always `session`, so nothing is written to any Claude settings file.
    Rule { update: Value, rule_text: String },
    /// No rule could be made: the answer is "allow once".
    Degrade,
}

/// Chooses the rule, in this order (section 03 6.5.7): a suggestion of Claude Code; the exact
/// command for `Bash`; the edit tools only with a proven `setMode`; the tool for an MCP tool;
/// else nothing.
pub fn choose_always(tool: &str, input: &Value, suggestions: Option<&Value>) -> AlwaysChoice {
    let entries = suggestions.and_then(Value::as_array);

    // 1. What Claude Code itself suggested, with the destination forced to this session.
    let suggested = entries.into_iter().flatten().find(|entry| {
        entry.get("type").and_then(Value::as_str) == Some("addRules")
            && entry.get("behavior").and_then(Value::as_str) == Some("allow")
    });
    if let Some(entry) = suggested {
        let mut update = entry.clone();
        if let Some(object) = update.as_object_mut() {
            object.insert("destination".to_owned(), json!("session"));
        }
        return AlwaysChoice::Rule {
            rule_text: rule_text_of(&update, tool),
            update,
        };
    }

    // 2. A Bash command: the exact command. A `*` is a wildcard in a rule, so a command with one
    // would allow more than the user saw.
    if tool == "Bash" {
        let command = string_field(input, "command").unwrap_or("").trim();
        if command.is_empty() || command.contains('*') {
            return AlwaysChoice::Degrade;
        }
        return AlwaysChoice::Rule {
            update: add_rules(tool, Some(command)),
            rule_text: format!("Bash({command})"),
        };
    }

    // 3. The edit tools, only with a mode change that Claude Code suggested and that is proven.
    if EDIT_TOOLS.contains(&tool) {
        if EDIT_ALWAYS_PROVEN {
            let mode = entries
                .into_iter()
                .flatten()
                .find(|entry| entry.get("type").and_then(Value::as_str) == Some("setMode"));
            if let Some(entry) = mode {
                let mut update = entry.clone();
                if let Some(object) = update.as_object_mut() {
                    object.insert("destination".to_owned(), json!("session"));
                }
                return AlwaysChoice::Rule {
                    update,
                    rule_text: "Accept edits for this session".to_owned(),
                };
            }
        }
        return AlwaysChoice::Degrade;
    }

    // 4. An MCP tool: the tool itself. A content on an MCP rule matches nothing.
    if tool.starts_with("mcp__") {
        return AlwaysChoice::Rule {
            update: add_rules(tool, None),
            rule_text: tool.to_owned(),
        };
    }

    // 5. Anything else.
    AlwaysChoice::Degrade
}

fn add_rules(tool: &str, content: Option<&str>) -> Value {
    let rule = match content {
        Some(content) => json!({ "toolName": tool, "ruleContent": content }),
        None => json!({ "toolName": tool }),
    };
    json!({
        "type": "addRules",
        "rules": [rule],
        "behavior": "allow",
        "destination": "session",
    })
}

/// `Tool(content)` or `Tool` for the first rule of an `addRules` entry.
fn rule_text_of(update: &Value, fallback_tool: &str) -> String {
    let first = update
        .get("rules")
        .and_then(Value::as_array)
        .and_then(|rules| rules.first());
    let name = first
        .and_then(|rule| string_field(rule, "toolName"))
        .unwrap_or(fallback_tool);
    match first.and_then(|rule| string_field(rule, "ruleContent")) {
        Some(content) if !content.is_empty() => format!("{name}({content})"),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CWD: &str = r"C:\work\demo";

    #[test]
    fn a_bash_command_is_shown_in_full_and_its_first_line_is_the_summary() {
        let input = json!({ "command": "npm install\nnpm test", "description": "Install it" });
        assert_eq!(summary("Bash", &input, CWD), "npm install");
        assert_eq!(detail("Bash", &input, CWD), "npm install\nnpm test");
    }

    #[test]
    fn an_edit_is_a_diff_with_a_path_relative_to_the_folder() {
        let input = json!({
            "file_path": r"C:\work\demo\src\main.rs",
            "old_string": "let a = 1;\nlet b = 2;",
            "new_string": "let a = 3;"
        });
        assert_eq!(summary("Edit", &input, CWD), r"src\main.rs");
        assert_eq!(
            detail("Edit", &input, CWD),
            "--- src\\main.rs\n+++ src\\main.rs\n@@ edit @@\n-let a = 1;\n-let b = 2;\n+let a = 3;"
        );
        // A path outside the folder stays as it is; case and slashes do not matter inside it.
        let outside = json!({ "file_path": r"D:\other\x.rs" });
        assert_eq!(summary("Edit", &outside, CWD), r"D:\other\x.rs");
        let slashes = json!({ "file_path": "c:/WORK/demo/a.rs" });
        assert_eq!(summary("Write", &slashes, CWD), "a.rs");
    }

    #[test]
    fn a_multi_edit_has_one_block_for_each_edit() {
        let input = json!({
            "file_path": r"C:\work\demo\a.rs",
            "edits": [
                { "old_string": "x", "new_string": "y" },
                { "old_string": "p", "new_string": "q" }
            ]
        });
        assert_eq!(summary("MultiEdit", &input, CWD), "a.rs (2 edits)");
        assert_eq!(
            detail("MultiEdit", &input, CWD),
            "--- a.rs\n+++ a.rs\n@@ edit 1 @@\n-x\n+y\n@@ edit 2 @@\n-p\n+q"
        );
    }

    #[test]
    fn a_write_and_a_notebook_edit() {
        let write = json!({ "file_path": r"C:\work\demo\n.txt", "content": "one\ntwo" });
        assert_eq!(
            detail("Write", &write, CWD),
            "--- n.txt\n+++ n.txt\n@@ new content @@\n+one\n+two"
        );
        let notebook =
            json!({ "notebook_path": r"C:\work\demo\n.ipynb", "new_source": "print(1)" });
        assert_eq!(summary("NotebookEdit", &notebook, CWD), "n.ipynb");
        assert_eq!(detail("NotebookEdit", &notebook, CWD), "print(1)");
    }

    #[test]
    fn read_like_tools_use_the_first_known_string() {
        assert_eq!(
            summary("Grep", &json!({ "pattern": "todo", "path": "src" }), CWD),
            "src"
        );
        assert_eq!(
            summary("WebFetch", &json!({ "url": "https://example.test/a" }), CWD),
            "https://example.test/a"
        );
        assert_eq!(summary("Read", &json!({}), CWD), "Read");
    }

    #[test]
    fn mcp_and_unknown_tools() {
        let input = json!({ "b": "second", "a": "first" });
        assert_eq!(summary("mcp__files__list", &input, CWD), "mcp__files__list");
        // Sorted keys: the same input always gives the same text.
        assert_eq!(
            detail("mcp__files__list", &input, CWD),
            "{\n  \"a\": \"first\",\n  \"b\": \"second\"\n}"
        );
        assert_eq!(
            summary("Mystery", &json!({ "n": 5, "s": "text" }), CWD),
            "text"
        );
        assert_eq!(summary("Mystery", &json!({ "n": 5 }), CWD), "Mystery");
    }

    #[test]
    fn long_texts_are_cut_and_say_how_much_is_missing() {
        let long = "x".repeat(300);
        let line = summary("Bash", &json!({ "command": long }), CWD);
        assert_eq!(line.chars().count(), SUMMARY_MAX);
        assert!(line.ends_with("..."));

        let big = "y".repeat(DETAIL_MAX + 25);
        let text = detail("Bash", &json!({ "command": big }), CWD);
        assert!(text.ends_with("\n... 25 more characters"));
        assert_eq!(
            text.chars().count(),
            DETAIL_MAX + "\n... 25 more characters".len()
        );
    }

    #[test]
    fn the_hash_ignores_the_order_of_keys_but_not_the_values() {
        let a = json!({ "x": 1, "y": { "p": [1, 2], "q": "t" } });
        let b = json!({ "y": { "q": "t", "p": [1, 2] }, "x": 1 });
        let c = json!({ "x": 2, "y": { "p": [1, 2], "q": "t" } });
        assert_eq!(input_hash(&a), input_hash(&b));
        assert_ne!(input_hash(&a), input_hash(&c));
    }

    #[test]
    fn the_three_answers_are_the_documented_json() {
        let allow: Value =
            serde_json::from_str(&decision_json(PermissionAnswer::Once, None)).unwrap();
        assert_eq!(
            allow,
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } } })
        );
        let deny: Value =
            serde_json::from_str(&decision_json(PermissionAnswer::Deny, None)).unwrap();
        assert_eq!(
            deny,
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": {
                "behavior": "deny", "message": "Denied from Rain", "interrupt": false } } })
        );
        let rule = add_rules("Bash", Some("npm install"));
        let always: Value =
            serde_json::from_str(&decision_json(PermissionAnswer::Always, Some(&rule))).unwrap();
        assert_eq!(
            always,
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": {
                "behavior": "allow",
                "updatedPermissions": [{
                    "type": "addRules",
                    "rules": [{ "toolName": "Bash", "ruleContent": "npm install" }],
                    "behavior": "allow",
                    "destination": "session"
                }] } } })
        );
    }

    #[test]
    fn a_suggestion_of_claude_code_comes_first_with_the_destination_forced_to_session() {
        let suggestions = json!([
            { "type": "setMode", "mode": "acceptEdits", "destination": "localSettings" },
            { "type": "addRules", "behavior": "allow", "destination": "projectSettings",
              "rules": [{ "toolName": "Bash", "ruleContent": "npm install:*" }] }
        ]);
        let choice = choose_always(
            "Bash",
            &json!({ "command": "npm install left-pad" }),
            Some(&suggestions),
        );
        let AlwaysChoice::Rule { update, rule_text } = choice else {
            panic!("a rule was expected");
        };
        assert_eq!(update["destination"], "session");
        assert_eq!(rule_text, "Bash(npm install:*)");
    }

    #[test]
    fn bash_gets_the_exact_command_unless_it_holds_a_star() {
        let choice = choose_always("Bash", &json!({ "command": "npm install" }), None);
        assert_eq!(
            choice,
            AlwaysChoice::Rule {
                update: add_rules("Bash", Some("npm install")),
                rule_text: "Bash(npm install)".to_owned()
            }
        );
        assert_eq!(
            choose_always("Bash", &json!({ "command": "rm *.tmp" }), None),
            AlwaysChoice::Degrade
        );
        assert_eq!(
            choose_always("Bash", &json!({}), None),
            AlwaysChoice::Degrade
        );
    }

    #[test]
    fn edit_tools_stay_at_once_even_with_a_set_mode_suggestion_until_it_is_proven() {
        let suggestions =
            json!([{ "type": "setMode", "mode": "acceptEdits", "destination": "session" }]);
        for tool in EDIT_TOOLS {
            assert_eq!(
                choose_always(tool, &json!({}), Some(&suggestions)),
                AlwaysChoice::Degrade
            );
        }
    }

    #[test]
    fn an_mcp_tool_gets_its_name_and_other_tools_get_nothing() {
        let choice = choose_always("mcp__files__list", &json!({ "path": "." }), None);
        assert_eq!(
            choice,
            AlwaysChoice::Rule {
                update: add_rules("mcp__files__list", None),
                rule_text: "mcp__files__list".to_owned()
            }
        );
        assert_eq!(
            choose_always("WebFetch", &json!({ "url": "x" }), None),
            AlwaysChoice::Degrade
        );
    }
}
