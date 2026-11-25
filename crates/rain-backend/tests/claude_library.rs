//! The Claude library routes, on a throwaway Claude folder. Every name and text here is made up.
//! The point of the secret test: a value that looks like a secret never leaves the backend.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::TestBackend;
use common::fakes::FakeOpener;
use hyper::StatusCode;
use rain_types::{ClaudeLibrary, LibraryKind, LibraryText};
use serde_json::{Value, json};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("a fixture step")).expect("a fixture step");
    std::fs::write(path, text).expect("a fixture step");
}

const SECRET: &str = "sk-live-0123456789abcdefghijklmnopqrstuv";

fn fill(root: &Path) {
    write(
        &root.join("skills/alpha/SKILL.md"),
        "---\nname: alpha\ndescription: Does the alpha thing\n---\n# Alpha\nSteps.",
    );
    write(&root.join("CLAUDE.md"), "# My rules\nBe brief.");
    write(
        &root.join("agents/helper.md"),
        "---\nname: helper\n---\nhelps",
    );
    write(&root.join("commands/team/ship.md"), "ship it");
    write(
        &root.join("projects/C--work-shop/memory/note.md"),
        "---\nname: note\ndescription: a note\nmetadata:\n  type: user\n---\nbody",
    );
    // Files that hold secrets, which the library must never open or show.
    write(
        &root.join(".credentials.json"),
        &format!("{{\"token\": \"{SECRET}\"}}"),
    );
    write(
        &root.join("settings.json"),
        &json!({
            "hooks": { "PreToolUse": [ { "matcher": "Bash", "hooks": [
                { "type": "command", "command": format!("check.sh --token {SECRET}") } ] } ] },
            "mcpServers": { "files": { "command": "npx", "env": { "API_KEY": SECRET } } },
            "enabledPlugins": { "demo@market": true }
        })
        .to_string(),
    );
    write(
        &root.join(".claude.json"),
        &json!({
            "oauthAccount": { "token": SECRET },
            "mcpServers": { "remote": { "type": "http", "url": "https://x.test",
                "headers": { "Authorization": format!("Bearer {SECRET}") } } }
        })
        .to_string(),
    );
}

async fn start() -> TestBackend {
    let dir = tempfile::tempdir().expect("a fixture step");
    let root = dir.path().join("claude-home");
    fill(&root);
    let backend = TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .start()
        .await;
    // The folder lives as long as the test program.
    std::mem::forget(dir);
    backend
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_library_lists_every_kind_and_the_setup_names() {
    let backend = start().await;
    let library: ClaudeLibrary = backend.get("/claude/library").await;
    assert!(library.found);
    let kinds: Vec<_> = library.items.iter().map(|item| item.kind).collect();
    for kind in [
        LibraryKind::Skill,
        LibraryKind::Instruction,
        LibraryKind::Agent,
        LibraryKind::Command,
        LibraryKind::Memory,
    ] {
        assert!(kinds.contains(&kind), "{kind:?} is missing");
    }
    let note = library
        .items
        .iter()
        .find(|item| item.name == "note")
        .expect("the memory file");
    assert_eq!(note.memory_type.as_deref(), Some("user"));
    assert_eq!(note.scope, "Project work-shop");

    let mcp: Vec<_> = library
        .setup
        .mcp_servers
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert!(mcp.contains(&"files") && mcp.contains(&"remote"), "{mcp:?}");
    assert_eq!(library.setup.hooks.len(), 1);
    assert_eq!(library.setup.hooks[0].event, "PreToolUse");
    assert!(library.setup.plugins.iter().any(|p| p.name == "demo"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_never_leaves_the_backend() {
    let backend = start().await;
    let (status, body) = backend.send("GET", "/claude/library", None).await;
    assert_eq!(status, StatusCode::OK);
    let wire = body.to_string();
    assert!(!wire.contains(SECRET), "a secret is in the list: {wire}");
    assert!(
        !wire.contains("credentials"),
        "the credentials file is listed"
    );
    // The hook shows its command, with the token replaced.
    assert!(wire.contains("check.sh --token ***"), "{wire}");

    // The text route gives a listed file only: the credentials file has no id.
    let library: ClaudeLibrary = backend.get("/claude/library").await;
    for item in &library.items {
        let (status, text) = backend
            .send("GET", &format!("/claude/library/item?id={}", item.id), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!text.to_string().contains(SECRET));
    }
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_text_route_reads_listed_files_only() {
    let backend = start().await;
    let library: ClaudeLibrary = backend.get("/claude/library").await;
    let skill = library
        .items
        .iter()
        .find(|item| item.kind == LibraryKind::Skill)
        .expect("the skill");
    let text: LibraryText = backend
        .get(&format!("/claude/library/item?id={}", skill.id))
        .await;
    assert!(text.body.contains("# Alpha"));
    assert!(
        text.facts
            .iter()
            .any(|fact| fact.key == "name" && fact.value == "alpha")
    );

    for bad in [
        "cl-0000000000000000",
        "..%5C..%5Cwindows",
        "%2e%2e%2fsettings",
    ] {
        let (status, body): (StatusCode, Value) = backend
            .send("GET", &format!("/claude/library/item?id={bad}"), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{bad}: {body}");
    }
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_an_unlisted_id_starts_nothing() {
    let opener = Arc::new(FakeOpener::default());
    let dir = tempfile::tempdir().expect("a fixture step");
    let root = dir.path().join("claude-home");
    fill(&root);
    let backend = TestBackend::builder()
        .config(move |config| config.claude_dir = Some(root.clone()))
        .opener(opener.clone())
        .start()
        .await;
    let (status, _) = backend
        .send(
            "POST",
            "/claude/library/open",
            Some(json!({ "id": "cl-1234" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(opener.requests().is_empty());
    backend.stop().await;
}
