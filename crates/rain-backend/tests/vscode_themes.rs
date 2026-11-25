//! The VS Code theme routes on a real backend, with the folders of a made-up VS Code in a
//! temporary folder (nothing of a real theme is used).

#![allow(clippy::unwrap_used)] // a test panics with the failing line, which is its message

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use common::TestBackend;
use hyper::StatusCode;
use rain_backend::modules::vscode_themes::ThemeSource;
use rain_types::{VsCodeCurrentTheme, VsCodeTheme, VsCodeThemeTokens};
use serde_json::Value;
use tempfile::TempDir;

struct Folders {
    extensions: PathBuf,
    settings: PathBuf,
}

impl ThemeSource for Folders {
    fn extensions_dir(&self) -> Option<PathBuf> {
        Some(self.extensions.clone())
    }
    fn settings_file(&self) -> Option<PathBuf> {
        Some(self.settings.clone())
    }
}

/// A made-up extension with one dark theme that includes a base file, and VS Code settings that
/// name it.
fn made_up_vscode() -> (TempDir, Arc<Folders>) {
    let temp = tempfile::tempdir().unwrap();
    let extension = temp.path().join("extensions").join("acme.nightfall-1.0.0");
    fs::create_dir_all(extension.join("themes")).unwrap();
    fs::write(
        extension.join("package.json"),
        r#"{ "publisher": "acme", "name": "nightfall", "contributes": { "themes": [
            { "label": "Nightfall", "uiTheme": "vs-dark", "path": "./themes/nightfall.json" }
        ] } }"#,
    )
    .unwrap();
    fs::write(
        extension.join("themes").join("nightfall.json"),
        "{ // made up\n \"include\": \"./base.json\",\n \"colors\": { \"badge.background\": \"#e0508c\", },\n}",
    )
    .unwrap();
    fs::write(
        extension.join("themes").join("base.json"),
        r##"{ "colors": { "editor.background": "#10141c", "editor.foreground": "#d8dee9" } }"##,
    )
    .unwrap();
    let settings = temp.path().join("User").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, "{ \"workbench.colorTheme\": \"Nightfall\", }").unwrap();
    let folders = Arc::new(Folders {
        extensions: temp.path().join("extensions"),
        settings,
    });
    (temp, folders)
}

async fn start(folders: Arc<Folders>) -> TestBackend {
    TestBackend::builder()
        .with::<dyn ThemeSource>(folders)
        .start()
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lists_the_themes_and_names_the_current_one() {
    let (_temp, folders) = made_up_vscode();
    let backend = start(folders).await;

    let themes: Vec<VsCodeTheme> = backend.get("/themes/vscode").await;
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].id, "acme.nightfall:Nightfall");
    assert!(themes[0].dark);

    let current: VsCodeCurrentTheme = backend.get("/themes/vscode/current").await;
    assert_eq!(current.label.as_deref(), Some("Nightfall"));
    assert_eq!(current.id.as_deref(), Some("acme.nightfall:Nightfall"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn returns_every_token_of_a_theme_with_its_own_colours() {
    let (_temp, folders) = made_up_vscode();
    let backend = start(folders).await;

    let theme: VsCodeThemeTokens = backend
        .get("/themes/vscode/theme?id=acme.nightfall%3ANightfall")
        .await;
    assert_eq!(theme.label, "Nightfall");
    assert_eq!(theme.tokens["background"], "#10141c");
    assert_eq!(theme.tokens["primary"], "#e0508c");
    assert!(theme.tokens.len() > 60);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_theme_is_not_found_and_a_missing_id_is_invalid() {
    let (_temp, folders) = made_up_vscode();
    let backend = start(folders).await;

    let (status, body): (StatusCode, Value) = backend
        .send(
            "GET",
            "/themes/vscode/theme?id=acme.nightfall%3ANothing",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "notFound");

    let (status, _): (StatusCode, Value) = backend.send("GET", "/themes/vscode/theme", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pc_without_vscode_has_no_themes_and_no_current_theme() {
    let temp = tempfile::tempdir().unwrap();
    let backend = start(Arc::new(Folders {
        extensions: temp.path().join("none"),
        settings: temp.path().join("none.json"),
    }))
    .await;

    let themes: Vec<VsCodeTheme> = backend.get("/themes/vscode").await;
    assert!(themes.is_empty());
    let current: Value = backend.get("/themes/vscode/current").await;
    assert_eq!(current, serde_json::json!({}));
    backend.stop().await;
}
