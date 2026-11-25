//! The types of the Claude library: what the owner's Claude setup contains. Read only.
//!
//! `GET /claude/library` lists the files (skills, memory, instructions, agents, commands) and the
//! short facts of the setup. `GET /claude/library/item?id=` gives the text of one listed file.
//! No secret ever crosses this API: the setup carries names and short facts only.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a listed file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LibraryKind {
    Skill,
    Memory,
    Instruction,
    Agent,
    Command,
}

/// One markdown file of the Claude setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LibraryItem {
    // Opaque. Only an id of the last list opens a file.
    pub id: String,
    #[ts(inline)]
    pub kind: LibraryKind,
    pub name: String,
    // Empty when the file says nothing about itself.
    pub description: String,
    // "You", "Plugin <name>" or "Project <name>".
    pub scope: String,
    pub path: String,
    // Absent when the file system gave no time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    pub size: u64,
    // Memory files only: the `type` of the front matter (user, feedback, project, reference).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LibraryPlugin {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    // Absent when the setup does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// An MCP server by name. Its command, address, `env` and `headers` are never read out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LibraryMcp {
    pub name: String,
    pub scope: String,
    // stdio, http or sse, when the setup says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LibraryHook {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    // Shortened, and token-like words are replaced by `***`.
    pub command: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOutputStyle {
    pub name: String,
    pub scope: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySetup {
    pub plugins: Vec<LibraryPlugin>,
    pub mcp_servers: Vec<LibraryMcp>,
    pub hooks: Vec<LibraryHook>,
    pub output_styles: Vec<LibraryOutputStyle>,
}

/// The answer of `GET /claude/library`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLibrary {
    // False when the Claude folder does not exist on this PC.
    pub found: bool,
    pub items: Vec<LibraryItem>,
    pub setup: LibrarySetup,
    // The scan stopped at its limit for files, so the list is not complete.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFact {
    pub key: String,
    pub value: String,
}

/// The answer of `GET /claude/library/item`: the front matter as facts, then the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryText {
    pub id: String,
    pub facts: Vec<LibraryFact>,
    pub body: String,
    // The file is longer than the limit and `body` is its beginning.
    pub truncated: bool,
}

/// The body of `POST /claude/library/open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryOpenBody {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;

    #[test]
    fn an_item_without_a_time_or_a_memory_type_leaves_them_out() {
        let json = round_trip(&LibraryItem {
            id: "cl-1".into(),
            kind: LibraryKind::Skill,
            name: "demo".into(),
            description: String::new(),
            scope: "You".into(),
            path: "C:\\x\\SKILL.md".into(),
            modified_at: None,
            size: 12,
            memory_type: None,
        });
        assert_eq!(json["kind"], "skill");
        assert!(json.get("modifiedAt").is_none());
        assert!(json.get("memoryType").is_none());
    }

    #[test]
    fn the_library_round_trips() {
        let json = round_trip(&ClaudeLibrary {
            found: true,
            items: Vec::new(),
            setup: LibrarySetup {
                mcp_servers: vec![LibraryMcp {
                    name: "files".into(),
                    scope: "You".into(),
                    transport: Some("stdio".into()),
                }],
                ..LibrarySetup::default()
            },
            truncated: false,
        });
        assert_eq!(json["setup"]["mcpServers"][0]["name"], "files");
    }
}
