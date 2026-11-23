//! The game library: Steam, Epic, game folders, hand-added games, corrections, covers, launching
//! and play time. Section 02, 10.4. Every game here has an invented name and lives in a temporary
//! folder; nothing starts a program (the opener is the fake of the harness).

#![allow(clippy::unwrap_used)] // a test panics with the failing line, which is its message

mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use common::fakes::{FakeClock, FakeOpener, FakeProcesses};
use common::{PATIENCE, TestBackend};
use futures_util::future::BoxFuture;
use hyper::{Method, StatusCode};
use rain_backend::core::launch_contract::{ItemSource, LaunchPlanner, OpenRequest, StepAction};
use rain_backend::modules::games::{ProcessWaiter, SteamRegistry};
use rain_types::limits::API_PREFIX;
use rain_types::{
    Game, GameLibrary, GameSource, LaunchRun, LaunchTarget, LaunchType, RunStatus, SourceState,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

// ---- fakes ----

struct FakeSteamRegistry {
    active: Option<u32>,
}

impl SteamRegistry for FakeSteamRegistry {
    fn active_user(&self) -> Option<u32> {
        self.active
    }
    fn auto_login_user(&self) -> Option<String> {
        None
    }
    fn running_app_id(&self) -> Option<u32> {
        None
    }
}

/// "Processes" that end when the test says so.
#[derive(Default)]
struct FakeWaiter {
    waits: Mutex<HashMap<u32, oneshot::Sender<()>>>,
    gone: Mutex<HashSet<u32>>,
}

impl FakeWaiter {
    fn end(&self, pid: u32) {
        if let Some(sender) = self.waits.lock().unwrap().remove(&pid) {
            let _ = sender.send(());
        }
    }
}

impl ProcessWaiter for FakeWaiter {
    fn wait_for(&self, pid: u32, _creation_time: u64) -> Option<BoxFuture<'static, ()>> {
        if self.gone.lock().unwrap().contains(&pid) {
            return None;
        }
        let (sender, receiver) = oneshot::channel();
        self.waits.lock().unwrap().insert(pid, sender);
        Some(Box::pin(async move {
            let _ = receiver.await;
        }))
    }
}

// ---- the world ----

const SETTINGS: &str = "schema_version = 1\n\
    steam_path = '{TEMP}\\Steam'\n\
    epic_path = '{TEMP}\\Epic'\n\
    game_folders = ['{TEMP}\\Games']\n";

fn builder() -> common::TestBackendBuilder {
    TestBackend::builder()
        .file("settings.toml", SETTINGS)
        .with::<dyn SteamRegistry>(Arc::new(FakeSteamRegistry { active: Some(1111) }))
}

fn temp_of(backend: &TestBackend) -> PathBuf {
    backend
        .data_dir
        .parent()
        .expect("the data folder is inside the temporary folder")
        .to_path_buf()
}

fn touch(path: &Path, size: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::File::create(path).unwrap().set_len(size).unwrap();
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

fn manifest(appid: &str, name: &str, flags: u32) -> String {
    format!(
        "\"AppState\"\n{{\n\"appid\" \"{appid}\"\n\"name\" \"{name}\"\n\"installdir\" \"{name}\"\n\"StateFlags\" \"{flags}\"\n\"SizeOnDisk\" \"4096\"\n\"LastPlayed\" \"1700000000\"\n}}"
    )
}

/// Two Steam games, a tool, a first download, play time for one game and a cover for the other.
fn make_steam(root: &Path) {
    let steam = root.join("Steam");
    write(
        &steam.join("steamapps").join("libraryfolders.vdf"),
        "\"libraryfolders\" {}",
    );
    for (id, name, flags) in [
        ("1000001", "Steam Quest", 4),
        ("1000002", "Steam Arena", 4),
        ("228980", "Steamworks Common Redistributables", 4),
        ("1000009", "Still Downloading", 1026),
    ] {
        write(
            &steam
                .join("steamapps")
                .join(format!("appmanifest_{id}.acf")),
            &manifest(id, name, flags),
        );
    }
    write(
        &steam
            .join("userdata")
            .join("1111")
            .join("config")
            .join("localconfig.vdf"),
        "\"UserLocalConfigStore\" { \"Software\" { \"Valve\" { \"Steam\" { \"apps\" { \"1000001\" { \"Playtime\" \"125\" } } } } } }",
    );
    fs::create_dir_all(steam.join("appcache").join("librarycache").join("1000001")).unwrap();
    fs::write(
        steam
            .join("appcache")
            .join("librarycache")
            .join("1000001")
            .join("library_600x900.jpg"),
        [0xFF, 0xD8, 0xFF, 0xE0, 1, 2],
    )
    .unwrap();
}

fn make_epic(root: &Path) {
    let installed = root.join("EpicGames").join("Epic Arena");
    fs::create_dir_all(&installed).unwrap();
    let item = json!({
        "DisplayName": "Epic Arena",
        "AppName": "EpicArena_1",
        "InstallLocation": installed.to_string_lossy(),
        "InstallSize": 2048,
        "AppCategories": ["games"],
    });
    write(
        &root
            .join("Epic")
            .join("Data")
            .join("Manifests")
            .join("a.item"),
        &item.to_string(),
    );
}

/// A game folder with the nested layout of a real game and the decoys around it.
fn make_folder_games(root: &Path) {
    let games = root.join("Games");
    touch(
        &games
            .join("Fixture Quest")
            .join("Game")
            .join("FixtureQuest.exe"),
        180 * 1024,
    );
    touch(
        &games
            .join("Fixture Quest")
            .join("tools")
            .join("FixtureQuestEditor.exe"),
        300 * 1024,
    );
    touch(
        &games
            .join("Fixture Quest")
            .join("Game")
            .join("Launcher.exe"),
        90 * 1024,
    );
    touch(
        &games
            .join("Fixture Quest")
            .join("Game")
            .join("CrashReportClient.exe"),
        40 * 1024,
    );
    touch(
        &games
            .join("Fixture Quest")
            .join("_CommonRedist")
            .join("vcredist_x64.exe"),
        25 * 1024,
    );
    touch(
        &games
            .join("Fixture Arena")
            .join("FA_Main")
            .join("bin")
            .join("x64")
            .join("FixtureArena.exe"),
        220 * 1024,
    );
    touch(&games.join("_CommonRedist").join("setup.exe"), 1024);
    touch(&games.join("readme.txt"), 1);
    fs::create_dir_all(games.join("Empty Folder")).unwrap();
}

async fn library(backend: &TestBackend) -> GameLibrary {
    backend.get::<GameLibrary>("/games").await
}

async fn refreshed(backend: &TestBackend) -> GameLibrary {
    let (status, body) = backend.send("POST", "/games/refresh", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

fn game<'a>(library: &'a GameLibrary, id: &str) -> &'a Game {
    library
        .games
        .iter()
        .find(|game| game.id == id)
        .unwrap_or_else(|| {
            panic!(
                "no game {id} in {:?}",
                library.games.iter().map(|g| &g.id).collect::<Vec<_>>()
            )
        })
}

fn state_of(library: &GameLibrary, source: GameSource) -> (SourceState, Option<String>) {
    let status = library
        .sources
        .iter()
        .find(|s| s.source == source)
        .expect("three sources");
    (status.state, status.message.clone())
}

fn games_toml(backend: &TestBackend) -> String {
    fs::read_to_string(backend.data_dir.join("games.toml")).unwrap_or_default()
}

fn target(id: &str) -> LaunchTarget {
    LaunchTarget {
        r#type: LaunchType::Game,
        id: id.to_owned(),
        mode: None,
        args: None,
    }
}

// ---- the library ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_library_has_every_source_with_ids_covers_and_three_states() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_steam(&temp);
    make_epic(&temp);
    make_folder_games(&temp);
    let library = refreshed(&backend).await;

    let ids: Vec<&str> = library.games.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "game-steam-1000002",
            "game-steam-1000001",
            "game-epic-epicarena-1",
            "game-dir-fixture-arena",
            "game-dir-fixture-quest",
        ],
        "sorted by source, then title: Steam Arena, Steam Quest, ..."
    );
    assert_eq!(library.sources.len(), 3);
    for source in [GameSource::Steam, GameSource::Epic, GameSource::Custom] {
        assert_eq!(state_of(&library, source).0, SourceState::Ok, "{source:?}");
    }
    let quest = game(&library, "game-steam-1000001");
    assert_eq!(quest.playtime_minutes, 125);
    assert_eq!(quest.size_bytes, Some(4096));
    assert!(
        quest
            .last_played_at
            .as_deref()
            .unwrap()
            .starts_with("2023-11-14")
    );
    assert!(
        quest
            .cover_url
            .as_deref()
            .unwrap()
            .starts_with("media/game-cover/game-steam-1000001?v=")
    );
    assert!(game(&library, "game-steam-1000002").cover_url.is_none());
    assert!(quest.installed && !quest.running && !quest.hidden && quest.exe_path.is_none());

    // The tool and the first download are not games.
    assert!(
        !ids.iter()
            .any(|id| id.contains("228980") || id.contains("1000009"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_epic_that_is_not_installed_is_off_not_failed() {
    let backend = TestBackend::builder()
        .file(
            "settings.toml",
            "schema_version = 1\nsteam_enabled = false\n",
        )
        .start()
        .await;
    let library = library(&backend).await;
    assert_eq!(
        state_of(&library, GameSource::Epic),
        (
            SourceState::Off,
            Some("Epic Games is not installed".to_owned())
        )
    );
    assert_eq!(
        state_of(&library, GameSource::Steam),
        (SourceState::Off, Some("Turned off in Settings".to_owned()))
    );
    assert!(library.games.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn garbage_in_the_epic_folder_fails_that_source_only() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_folder_games(&temp);
    for name in ["1.item", "2.item", "3.item"] {
        write(
            &temp.join("Epic").join("Manifests").join(name),
            "{ not json",
        );
    }
    let library = refreshed(&backend).await;
    let (state, message) = state_of(&library, GameSource::Epic);
    assert_eq!(state, SourceState::Failed);
    assert_eq!(
        message.as_deref(),
        Some("Could not read the Epic Games library")
    );
    assert_eq!(state_of(&library, GameSource::Custom).0, SourceState::Ok);
    assert_eq!(library.games.len(), 2, "the folder games are still there");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_steam_folder_fails_with_its_path_and_a_missing_game_folder_too() {
    let backend = builder().start().await;
    let library = refreshed(&backend).await;
    let (state, message) = state_of(&library, GameSource::Steam);
    assert_eq!(state, SourceState::Failed);
    assert!(message.unwrap().starts_with("Steam folder not found: "));
    let (state, message) = state_of(&library, GameSource::Custom);
    assert_eq!(state, SourceState::Failed);
    assert!(message.unwrap().starts_with("Game folder not found: "));

    let health: Value = backend.get("/health").await;
    let sources = health["sources"].as_array().unwrap();
    assert!(
        sources
            .iter()
            .any(|s| s["name"] == "gamefolders" && s["state"] == "failed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_out_steam_lists_its_games_without_play_time() {
    let backend = builder()
        .with::<dyn SteamRegistry>(Arc::new(FakeSteamRegistry { active: None }))
        .start()
        .await;
    let temp = temp_of(&backend);
    make_steam(&temp);
    fs::remove_dir_all(temp.join("Steam").join("userdata")).unwrap();
    let library = refreshed(&backend).await;
    let (state, message) = state_of(&library, GameSource::Steam);
    assert_eq!(state, SourceState::Ok);
    assert!(message.unwrap().contains("signed out"));
    assert_eq!(game(&library, "game-steam-1000001").playtime_minutes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folder_games_get_the_program_a_person_would_start_and_the_guess_is_cached() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_folder_games(&temp);
    let library = refreshed(&backend).await;
    let local: Vec<&Game> = library
        .games
        .iter()
        .filter(|g| g.source == GameSource::Custom)
        .collect();
    assert_eq!(
        local.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(),
        ["Fixture Arena", "Fixture Quest"],
        "no _CommonRedist, no loose file, no empty folder"
    );
    let quest = game(&library, "game-dir-fixture-quest");
    assert!(
        quest
            .exe_path
            .as_deref()
            .unwrap()
            .ends_with("Game\\FixtureQuest.exe")
    );
    assert_eq!(quest.exe_guessed, Some(true));
    assert!(quest.installed && quest.removable.is_none());
    assert!(
        game(&library, "game-dir-fixture-arena")
            .exe_path
            .as_deref()
            .unwrap()
            .ends_with("bin\\x64\\FixtureArena.exe")
    );

    // The guesses are in the cache table now.
    let rows: i64 = backend
        .state
        .db
        .call(|c| c.query_row("SELECT count(*) FROM games_guess_cache", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(rows, 2);
    // A new game in the folder, and the old guess is kept while its folder is unchanged.
    let again = library_after_change(&backend).await;
    assert_eq!(
        game(&again, "game-dir-fixture-quest").exe_path,
        quest.exe_path
    );
}

async fn library_after_change(backend: &TestBackend) -> GameLibrary {
    refreshed(backend).await
}

// ---- hiding and corrections ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hiding_keeps_the_game_in_the_list_and_showing_removes_the_entry_again() {
    let backend = builder().start().await;
    make_steam(&temp_of(&backend));
    refreshed(&backend).await;

    let (status, body) = backend
        .send(
            "PATCH",
            "/games/game-steam-1000001",
            Some(json!({ "hidden": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hidden"], true);
    assert!(games_toml(&backend).contains("steam:1000001"));
    assert!(game(&library(&backend).await, "game-steam-1000001").hidden);
    // Idempotent.
    let (status, _) = backend
        .send(
            "PATCH",
            "/games/game-steam-1000001",
            Some(json!({ "hidden": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // Hidden games leave search and pins.
    assert!(
        backend
            .state
            .games()
            .item_for("game-steam-1000001")
            .is_none()
    );

    let (_, body) = backend
        .send(
            "PATCH",
            "/games/game-steam-1000001",
            Some(json!({ "hidden": false })),
        )
        .await;
    assert_eq!(body["hidden"], false);
    assert!(
        !games_toml(&backend).contains("[[game]]\n"),
        "{}",
        games_toml(&backend)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_game_can_be_corrected_and_returned_to_the_guess() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_folder_games(&temp);
    touch(
        &temp
            .join("Games")
            .join("Fixture Quest")
            .join("alt")
            .join("Other.exe"),
        5,
    );
    write(&temp.join("pic.png"), "x");
    fs::write(temp.join("pic.png"), PNG).unwrap();
    refreshed(&backend).await;

    let (status, body) = backend
        .send(
            "PATCH",
            "/games/game-dir-fixture-quest",
            Some(json!({
                "title": "Fixture Quest: Remastered",
                "exePath": "alt\\Other.exe",
                "coverPath": temp.join("pic.png").to_string_lossy(),
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Fixture Quest: Remastered");
    assert_eq!(body["exeGuessed"], false);
    assert!(
        body["exePath"]
            .as_str()
            .unwrap()
            .ends_with("alt\\Other.exe")
    );
    assert!(
        body["coverUrl"]
            .as_str()
            .unwrap()
            .starts_with("media/game-cover/game-dir-fixture-quest?v=")
    );
    let text = games_toml(&backend);
    assert!(text.contains("exe = 'alt\\Other.exe'"), "{text}");
    assert!(
        text.contains("cover = 'media/game-dir-fixture-quest-"),
        "{text}"
    );
    let media: Vec<_> = fs::read_dir(backend.data_dir.join("media"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(media.len(), 1);

    // Back to the guess, the folder name, and no cover.
    let (status, body) = backend
        .send(
            "PATCH",
            "/games/game-dir-fixture-quest",
            Some(json!({ "title": "", "exePath": "", "coverPath": "" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["title"], "Fixture Quest");
    assert_eq!(body["exeGuessed"], true);
    assert!(body.get("coverUrl").is_none());
    assert!(!games_toml(&backend).contains("Fixture Quest: Remastered"));
    assert_eq!(
        fs::read_dir(backend.data_dir.join("media"))
            .unwrap()
            .count(),
        0,
        "the copy is removed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_patch_says_what_is_wrong_with_it() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_steam(&temp);
    make_folder_games(&temp);
    refreshed(&backend).await;

    let reason = |status: StatusCode, body: &Value| {
        (status, body["reason"].as_str().unwrap_or("").to_owned())
    };
    let cases = [
        (
            "game-steam-1000001",
            json!({}),
            StatusCode::BAD_REQUEST,
            "emptyPatch",
        ),
        (
            "game-steam-1000001",
            json!({ "title": "x" }),
            StatusCode::BAD_REQUEST,
            "notEditable",
        ),
        (
            "game-nothing",
            json!({ "hidden": true }),
            StatusCode::NOT_FOUND,
            "unknownId",
        ),
        (
            "game-dir-fixture-quest",
            json!({ "exePath": "missing.exe" }),
            StatusCode::BAD_REQUEST,
            "exeNotFound",
        ),
        (
            "game-dir-fixture-quest",
            json!({ "exePath": "Game\\../../x" }),
            StatusCode::BAD_REQUEST,
            "exeNotFound",
        ),
        (
            "game-dir-fixture-quest",
            json!({ "coverPath": "D:\\nowhere\\c.png" }),
            StatusCode::BAD_REQUEST,
            "coverNotFound",
        ),
    ];
    for (id, patch, status, expected) in cases {
        let (got, body) = backend
            .send("PATCH", &format!("/games/{id}"), Some(patch))
            .await;
        assert_eq!(
            reason(got, &body),
            (status, expected.to_owned()),
            "{id} {body}"
        );
    }
    let (status, body) = backend
        .send(
            "PATCH",
            "/games/game-dir-fixture-quest",
            Some(json!({ "bogus": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

// ---- hand-added games ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_added_by_hand_is_validated_stored_and_removable() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_folder_games(&temp);
    let exe = temp.join("Elsewhere").join("Celeste").join("Celeste.exe");
    touch(&exe, 10);
    fs::write(temp.join("cover.png"), PNG).unwrap();
    write(&temp.join("fake.png"), "text");
    refreshed(&backend).await;

    let post = |draft: Value| {
        let backend = &backend;
        async move { backend.send("POST", "/games", Some(draft)).await }
    };
    let exe_text = exe.to_string_lossy().into_owned();
    let failures = [
        (json!({ "title": " ", "exePath": exe_text }), "noName"),
        (json!({ "title": "A", "exePath": " " }), "noExe"),
        (
            json!({ "title": "A", "exePath": "Celeste.exe" }),
            "exeNotAbsolute",
        ),
        (
            json!({ "title": "A", "exePath": temp.join("nope.exe").to_string_lossy() }),
            "exeNotFound",
        ),
        (
            json!({ "title": "A", "exePath": temp.join("cover.png").to_string_lossy() }),
            "notAnExe",
        ),
        (
            json!({ "title": "A", "exePath": exe_text, "coverPath": "D:\\none.png" }),
            "coverNotFound",
        ),
        (
            json!({ "title": "A", "exePath": exe_text, "coverPath": temp.join("fake.png").to_string_lossy() }),
            "coverNotImage",
        ),
    ];
    for (draft, expected) in failures {
        let (status, body) = post(draft).await;
        assert!(status.is_client_error(), "{expected}: {body}");
        assert_eq!(body["reason"], expected, "{body}");
    }
    // A program that is inside a folder game is that game already.
    let inside = temp
        .join("Games")
        .join("Fixture Quest")
        .join("Game")
        .join("FixtureQuest.exe");
    let (status, body) =
        post(json!({ "title": "Again", "exePath": inside.to_string_lossy() })).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("alreadyListed"))
    );
    assert!(body["message"].as_str().unwrap().contains("Fixture Quest"));

    let (status, body) = post(json!({
        "title": "Celeste", "exePath": exe_text, "coverPath": temp.join("cover.png").to_string_lossy()
    }))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["id"], "game-celeste");
    assert_eq!(body["removable"], true);
    assert_eq!(body["installed"], true);
    assert!(
        body["coverUrl"]
            .as_str()
            .unwrap()
            .contains("media/game-cover/game-celeste?v=")
    );
    assert!(body.get("installPath").is_none());
    let text = games_toml(&backend);
    assert!(
        text.contains("[[custom]]") && text.contains("id = 'game-celeste'"),
        "{text}"
    );
    assert_eq!(
        fs::read_dir(backend.data_dir.join("media"))
            .unwrap()
            .count(),
        1
    );

    // The same program twice, and the same title for another program.
    let (status, body) = post(json!({ "title": "Other", "exePath": exe_text })).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("alreadyListed"))
    );
    let second = temp.join("Elsewhere").join("Celeste2").join("Celeste.exe");
    touch(&second, 10);
    let (_, body) = post(json!({ "title": "Celeste", "exePath": second.to_string_lossy() })).await;
    assert_eq!(body["id"], "game-celeste-2");

    // Removal: its cover goes too; a folder game cannot be removed; an unknown id is fine.
    let (status, _) = backend.send("DELETE", "/games/game-celeste", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        fs::read_dir(backend.data_dir.join("media"))
            .unwrap()
            .count(),
        0
    );
    assert!(
        library(&backend)
            .await
            .games
            .iter()
            .all(|g| g.id != "game-celeste")
    );
    let (status, body) = backend
        .send("DELETE", "/games/game-dir-fixture-quest", None)
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("notRemovable"))
    );
    let (status, _) = backend.send("DELETE", "/games/game-unknown", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// ---- the new Epic launcher ----

const V2_ITEM: &str = "00112233445566778899aabbccddeeff";
const V2_COVER_MD5: &str = "0123456789abcdef0123456789abcdef";

/// The data folder of the new Epic launcher (`<root>\Epic`, which the settings name) with one
/// invented game: a journal, a title, a cover picture, a size, play time and a last-played time.
/// `app` is the app name, so a test can make it equal to an old manifest's.
fn make_epic_v2(root: &Path, app: &str, title: &str) {
    let data = root.join("Epic");
    let install = root.join("EpicGames").join("Fixture Quest");
    fs::create_dir_all(&install).unwrap();
    let record = |complete: bool| {
        json!({
            "installationId": "inst1", "complete": complete, "platform": "Windows",
            "dir": install.to_string_lossy(), "namespace": "fixturens", "itemId": V2_ITEM,
            "artifactId": app, "version": "1.0", "launchExecutable": "Game/Quest.exe",
        })
    };
    write(
        &data.join("installations.jsonl"),
        &format!(
            "{{\"version\":1,\"entries\":2}}
{}
{}
",
            record(false),
            record(true)
        ),
    );
    let account = data
        .join("cache")
        .join("prod")
        .join("account")
        .join("acct1");
    write(
        &account.join("library-cache.json"),
        &json!({"items": [{
            "relationshipId": format!("fixturens:{V2_ITEM}:{app}"),
            "appId": {"namespaceId": "fixturens", "itemId": V2_ITEM, "appName": app},
            "title": title, "owned": true,
        }]})
        .to_string(),
    );
    write(
        &account.join("playtime-cache.json"),
        &json!({"items": [{"accountId": "acct1", "artifactId": app, "totalTime": 7260}]})
            .to_string(),
    );
    write(
        &data.join("cache").join("prod").join("catalog-cache.json"),
        &json!({"items": {V2_ITEM: {"item": {"id": V2_ITEM, "keyImages": [
            {"type": "Landscape", "md5": "ffffffffffffffffffffffffffffffff", "width": 2000, "height": 1000},
            {"type": "Tall", "md5": V2_COVER_MD5, "width": 1200, "height": 1600},
        ]}}}})
        .to_string(),
    );
    fs::create_dir_all(data.join("images")).unwrap();
    fs::write(data.join("images").join(format!("{V2_COVER_MD5}.jpg")), PNG).unwrap();
    write(
        &data.join("cache").join("disk-size-cache.json"),
        &json!({"entries": [{
            "app_id": {"namespaceId": "fixturens", "itemId": V2_ITEM, "appName": app},
            "bytes": 5000000.0, "measured_at_ms": 1,
        }]})
        .to_string(),
    );
    write(
        &data.join("last-played.json"),
        &json!({"acct1": {format!("fixturens:{V2_ITEM}:{app}"): "2026-11-22T05:57:51.736380500+00:00"}})
            .to_string(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_of_the_new_epic_launcher_is_listed_with_its_title_cover_size_and_play_time() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_epic_v2(&temp, "FixtureQuest_1", "Fixture Quest Deluxe");

    // The files were made after the start, so the list is asked for again from the disk, then
    // read as `GET /games` gives it.
    refreshed(&backend).await;
    let library = library(&backend).await;
    let (state, message) = state_of(&library, GameSource::Epic);
    assert_eq!((state, message), (SourceState::Ok, None));
    let epic: Vec<&Game> = library
        .games
        .iter()
        .filter(|g| g.source == GameSource::Epic)
        .collect();
    assert_eq!(
        epic.len(),
        1,
        "the last journal line says the install is complete"
    );
    let quest = epic[0];
    assert_eq!(quest.id, "game-epic-fixturequest-1");
    assert_eq!(quest.title, "Fixture Quest Deluxe");
    assert!(quest.installed);
    assert_eq!(quest.size_bytes, Some(5_000_000));
    assert_eq!(
        quest.playtime_minutes, 121,
        "Epic's own seconds, in minutes"
    );
    assert!(
        quest
            .last_played_at
            .as_deref()
            .unwrap()
            .starts_with("2026-11-22T05:57:51")
    );
    assert!(
        quest
            .install_path
            .as_deref()
            .unwrap()
            .ends_with("Fixture Quest")
    );

    // The portrait picture on disk is the cover, served through the usual media route.
    let cover = quest.cover_url.clone().expect("a local cover");
    assert!(cover.starts_with("media/game-cover/game-epic-fixturequest-1?v="));
    let served = backend
        .client
        .request(Method::GET, &format!("{API_PREFIX}/{cover}"), None, &[])
        .await
        .unwrap();
    assert_eq!(served.status, StatusCode::OK);
    assert_eq!(served.body.as_ref(), PNG);

    // Launched through the launcher with all three parts of the address.
    let plan = backend
        .state
        .games()
        .plan(&target("game-epic-fixturequest-1"))
        .unwrap();
    match &plan.steps[0].action {
        StepAction::ShellExec { file, .. } => assert_eq!(
            file,
            &format!(
                "com.epicgames.launcher://apps/fixturens%3A{V2_ITEM}%3AFixtureQuest_1?action=launch&silent=true"
            )
        ),
        _ => panic!("an Epic game is started through its address"),
    }

    // `hidden` in games.toml works with the same key as for the old launcher.
    let (status, body) = backend
        .send(
            "PATCH",
            "/games/game-epic-fixturequest-1",
            Some(json!({ "hidden": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(games_toml(&backend).contains("epic:FixtureQuest_1"));
    let after = refreshed(&backend).await;
    let hidden = after
        .games
        .iter()
        .find(|g| g.id == "game-epic-fixturequest-1")
        .expect("still in the library");
    assert!(hidden.hidden, "the correction is applied to the new format");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_in_both_epic_formats_is_one_game_and_the_new_format_wins() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    // The old launcher's manifest (`EpicArena_1`) and the new launcher's journal sit in the
    // folder that the settings name; the journal lists the same app name and one more game.
    make_epic(&temp);
    make_epic_v2(&temp, "EpicArena_1", "Epic Arena Renewed");

    let library = refreshed(&backend).await;
    assert_eq!(state_of(&library, GameSource::Epic).0, SourceState::Ok);
    let epic: Vec<&Game> = library
        .games
        .iter()
        .filter(|g| g.source == GameSource::Epic)
        .collect();
    assert_eq!(epic.len(), 1, "one game, not two");
    assert_eq!(epic[0].id, "game-epic-epicarena-1");
    assert_eq!(epic[0].title, "Epic Arena Renewed");
    assert!(
        epic[0]
            .install_path
            .as_deref()
            .unwrap()
            .ends_with("Fixture Quest")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_journal_that_is_garbage_fails_epic_and_names_the_file() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    write(
        &temp.join("Epic").join("installations.jsonl"),
        "garbage
more
",
    );
    let library = refreshed(&backend).await;
    assert_eq!(
        state_of(&library, GameSource::Epic),
        (
            SourceState::Failed,
            Some("Could not read installations.jsonl".to_owned())
        )
    );
    let health: Value = backend.get("/health").await;
    assert!(
        health["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "epic" && s["state"] == "failed")
    );
}

// ---- covers as media ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn covers_are_served_with_an_etag_and_unknown_ids_are_not_found() {
    let backend = builder().start().await;
    make_steam(&temp_of(&backend));
    let library = refreshed(&backend).await;
    let cover = game(&library, "game-steam-1000001")
        .cover_url
        .clone()
        .unwrap();
    let path = format!("{API_PREFIX}/{cover}");
    let first = backend
        .client
        .request(Method::GET, &path, None, &[])
        .await
        .unwrap();
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.headers["content-type"], "image/jpeg");
    assert_eq!(first.body.as_ref(), &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2]);
    let etag = first.headers["etag"].to_str().unwrap().to_owned();
    let again = backend
        .client
        .request(
            Method::GET,
            &path,
            None,
            &[("if-none-match", etag.as_str())],
        )
        .await
        .unwrap();
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);

    for id in ["game-steam-1000002", "game-nothing", "..%2Fsecret"] {
        let missing = backend
            .client
            .request(
                Method::GET,
                &format!("{API_PREFIX}/media/game-cover/{id}"),
                None,
                &[],
            )
            .await
            .unwrap();
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{id}");
    }
}

// ---- size ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_size_of_a_folder_game_arrives_in_the_background() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    touch(
        &temp.join("Games").join("Sized Game").join("Sized.exe"),
        3000,
    );
    touch(
        &temp
            .join("Games")
            .join("Sized Game")
            .join("data")
            .join("a.bin"),
        2000,
    );
    let mut stream = backend.stream().await;
    let first = refreshed(&backend).await;
    let sized = game(&first, "game-dir-sized-game");

    let mut size = sized.size_bytes;
    while size.is_none() {
        let frame = stream.expect_topic("games").await;
        let payload: GameLibrary = serde_json::from_value(frame.payload).unwrap();
        size = game(&payload, "game-dir-sized-game").size_bytes;
    }
    assert_eq!(size, Some(5000));
    // The next list has it at once.
    assert_eq!(
        game(&library(&backend).await, "game-dir-sized-game").size_bytes,
        Some(5000)
    );
}

// ---- reload and the game mode gate ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_of_games_toml_is_taken_up_and_a_broken_file_keeps_the_last_good_copy() {
    let backend = builder().start().await;
    make_steam(&temp_of(&backend));
    make_folder_games(&temp_of(&backend));
    refreshed(&backend).await;
    let mut stream = backend.stream().await;

    write(
        &backend.data_dir.join("games.toml"),
        "schema_version = 1\n[[game]]\nkey = 'steam:1000001'\nhidden = true\n",
    );
    loop {
        let frame = stream.expect_topic("games").await;
        let payload: GameLibrary = serde_json::from_value(frame.payload).unwrap();
        if game(&payload, "game-steam-1000001").hidden {
            break;
        }
    }

    write(&backend.data_dir.join("games.toml"), "[[game]\nbroken");
    tokio::time::sleep(Duration::from_millis(900)).await;
    let library = refreshed(&backend).await;
    assert!(
        game(&library, "game-steam-1000001").hidden,
        "the last good copy stays"
    );
    let (state, message) = state_of(&library, GameSource::Custom);
    assert_eq!(state, SourceState::Failed);
    assert!(message.unwrap().starts_with("games.toml has an error"));
    // Writes are refused until the file is fixed.
    let (status, body) = backend
        .send(
            "PATCH",
            "/games/game-steam-1000002",
            Some(json!({ "hidden": true })),
        )
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("configBroken"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_entry_of_games_toml_is_named_in_health_with_its_line_until_it_is_fixed() {
    let backend = builder()
        .file(
            "games.toml",
            "schema_version = 1

[[game]]
key = 'steam:1000001'
hidden = true

             [[game]]
key = 'nonsense'
hidden = true
",
        )
        .start()
        .await;
    refreshed(&backend).await;
    let health: rain_types::Health = backend.get("/health").await;
    let found: Vec<_> = health
        .config_issues
        .iter()
        .filter(|issue| issue.file == "games.toml")
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].line, Some(7), "the line of the second [[game]]");
    assert!(
        found[0].message.contains("[[game]]"),
        "{}",
        found[0].message
    );
    // The text names no content of the entry.
    assert!(!found[0].message.contains("nonsense"));

    // A fixed file takes the issue away again.
    write(
        &backend.data_dir.join("games.toml"),
        "schema_version = 1
[[game]]
key = 'steam:1000001'
hidden = true
",
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    refreshed(&backend).await;
    let health: rain_types::Health = backend.get("/health").await;
    assert!(
        health
            .config_issues
            .iter()
            .all(|issue| issue.file != "games.toml"),
        "{:?}",
        health.config_issues
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_mode_refuses_a_refresh_and_a_launch_but_not_a_list() {
    let backend = builder().start().await;
    make_steam(&temp_of(&backend));
    refreshed(&backend).await;
    backend.state.set_game_mode_active(true);
    let (status, body) = backend.send("POST", "/games/refresh", None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("gameMode"))
    );
    assert!(!library(&backend).await.games.is_empty());
    let error = backend
        .state
        .games()
        .plan(&target("game-steam-1000001"))
        .err()
        .unwrap();
    assert_eq!(error.body.reason.as_deref(), Some("gameMode"));
    backend.state.set_game_mode_active(false);
}

// ---- launching ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_plan_of_each_source_is_one_step_with_the_right_start() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_steam(&temp);
    make_epic(&temp);
    make_folder_games(&temp);
    refreshed(&backend).await;
    let games = backend.state.games();

    let steam = games.plan(&target("game-steam-1000001")).unwrap();
    assert_eq!(steam.steps.len(), 1);
    assert_eq!(steam.steps[0].label, "Steam Quest");
    match &steam.steps[0].action {
        StepAction::ShellExec { file, .. } => assert_eq!(file, "steam://rungameid/1000001"),
        _ => panic!("a Steam game is started through its address"),
    }
    let epic = games.plan(&target("game-epic-epicarena-1")).unwrap();
    match &epic.steps[0].action {
        StepAction::ShellExec { file, .. } => {
            assert_eq!(
                file,
                "com.epicgames.launcher://apps/EpicArena_1?action=launch&silent=true"
            );
        }
        _ => panic!("an Epic game is started through its address"),
    }
    let local = games.plan(&target("game-dir-fixture-quest")).unwrap();
    let step = &local.steps[0];
    match &step.action {
        StepAction::Spawn { exe, cwd, .. } => {
            assert!(exe.ends_with("Game\\FixtureQuest.exe"));
            assert_eq!(cwd, exe.parent().unwrap(), "in the program's own folder");
        }
        _ => panic!("a local game is started directly"),
    }
    assert!(step.on_spawned.is_some() && step.shell_fallback.is_some());

    // The texts of failures follow the code, never the wording.
    let text = step.error_text.as_ref().unwrap();
    let error = |code| rain_backend::core::launch_contract::OpenError {
        ui_text: "x",
        code,
        log_reason: String::new(),
    };
    assert_eq!(text(&error(Some(5))), "Could not start the program");
    let steam_text = steam.steps[0].error_text.as_ref().unwrap();
    assert_eq!(steam_text(&error(Some(31))), "Steam is not installed");
    assert_eq!(
        steam_text(&error(Some(5))),
        "Could not hand the game to Steam"
    );

    let unknown = games.plan(&target("game-nothing")).err().unwrap();
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body.reason.as_deref(), Some("unknownId"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_without_a_program_or_with_a_vanished_one_fails_before_it_starts() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    touch(&temp.join("Games").join("Only Data").join("data.bin"), 5);
    touch(&temp.join("Games").join("Vanishing").join("v.exe"), 5);
    refreshed(&backend).await;
    fs::remove_file(temp.join("Games").join("Vanishing").join("v.exe")).unwrap();
    let games = backend.state.games();
    let none = games.plan(&target("game-dir-only-data")).unwrap();
    assert_eq!(
        none.steps[0].preset_error.as_deref(),
        Some("No program file. Choose one with Edit")
    );
    let gone = games.plan(&target("game-dir-vanishing")).unwrap();
    assert!(
        gone.steps[0]
            .preset_error
            .as_deref()
            .unwrap()
            .starts_with("Program file not found: ")
    );
}

fn finished_run(id: &str) -> LaunchRun {
    LaunchRun {
        id: "run-1".into(),
        target: target(id),
        status: RunStatus::Success,
        steps: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_launch_sets_last_played_and_a_second_start_while_it_runs_does_nothing() {
    let waiter = Arc::new(FakeWaiter::default());
    let clock = Arc::new(FakeClock::new(SystemTime::now()));
    let backend = builder()
        .with::<dyn ProcessWaiter>(waiter.clone())
        .clock(clock.clone())
        .start()
        .await;
    let temp = temp_of(&backend);
    make_folder_games(&temp);
    refreshed(&backend).await;
    let games = backend.state.games();
    let mut stream = backend.stream().await;

    // The engine starts the program, tells the module, and the module counts the time.
    let plan = games.plan(&target("game-dir-fixture-quest")).unwrap();
    plan.steps[0].on_spawned.as_ref().unwrap()(4242, 4242);
    games
        .finished(&finished_run("game-dir-fixture-quest"))
        .await;
    let started = library(&backend).await;
    let quest = game(&started, "game-dir-fixture-quest");
    assert!(
        quest.last_played_at.is_some(),
        "last played is set before the final frame"
    );
    assert!(quest.running, "a watched session counts as running");

    // A second start while the first runs spawns nothing.
    let again = games.plan(&target("game-dir-fixture-quest")).unwrap();
    assert!(matches!(again.steps[0].action, StepAction::Nothing));
    assert!(again.steps[0].preset_error.is_none());

    clock.advance(Duration::from_secs(10 * 60 + 30));
    waiter.end(4242);
    loop {
        let frame = stream.expect_topic("games").await;
        let payload: GameLibrary = serde_json::from_value(frame.payload).unwrap();
        let quest = game(&payload, "game-dir-fixture-quest");
        if quest.playtime_minutes == 10 && !quest.running {
            break;
        }
    }
    assert_eq!(
        game(&refreshed(&backend).await, "game-dir-fixture-quest").playtime_minutes,
        10
    );
    let rows: i64 = backend
        .state
        .db
        .call(|c| c.query_row("SELECT count(*) FROM games_session", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(rows, 0, "the session row is gone");
    let _ = PATIENCE;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_game_is_found_by_its_folder_and_a_session_survives_a_restart() {
    let waiter = Arc::new(FakeWaiter::default());
    let processes = Arc::new(FakeProcesses::default());
    let clock = Arc::new(FakeClock::new(SystemTime::now()));
    let mut backend = builder()
        .with::<dyn ProcessWaiter>(waiter.clone())
        .with::<dyn rain_backend::platform::process::ProcessLister>(processes.clone())
        .clock(clock.clone())
        .start()
        .await;
    let temp = temp_of(&backend);
    make_folder_games(&temp);
    refreshed(&backend).await;

    // Found by the folder of its process, not by a session.
    let image = temp
        .join("Games")
        .join("Fixture Arena")
        .join("FA_Main")
        .join("bin")
        .join("x64")
        .join("FixtureArena.exe");
    processes
        .0
        .lock()
        .unwrap()
        .push(image.to_string_lossy().into_owned());
    let list = refreshed(&backend).await;
    assert!(game(&list, "game-dir-fixture-arena").running);
    assert!(!game(&list, "game-dir-fixture-quest").running);
    assert!(backend.state.games().any_game_running().await);
    processes.0.lock().unwrap().clear();

    // A game that Rain started when the backend stopped: its session row survives and the next
    // start adds the time up to the last moment the backend saw it alive.
    let games = backend.state.games();
    let plan = games.plan(&target("game-dir-fixture-quest")).unwrap();
    plan.steps[0].on_spawned.as_ref().unwrap()(777, 777);
    games
        .finished(&finished_run("game-dir-fixture-quest"))
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    clock.advance(Duration::from_secs(125));
    waiter.gone.lock().unwrap().insert(777); // it ended while the backend was away
    backend.restart().await;
    let list = library(&backend).await;
    let quest = game(&list, "game-dir-fixture-quest");
    assert_eq!(quest.playtime_minutes, 2, "125 seconds, floored to minutes");
    assert!(!quest.running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_mode_launches_a_game_through_the_one_opener_and_records_it() {
    let opener = Arc::new(FakeOpener::default());
    let backend = builder().opener(opener.clone()).start().await;
    let temp = temp_of(&backend);
    make_steam(&temp);
    make_folder_games(&temp);
    refreshed(&backend).await;
    let games = backend.state.games();

    games
        .launch_after_game_mode("game-steam-1000001")
        .await
        .unwrap();
    let local = games
        .launch_after_game_mode("game-dir-fixture-quest")
        .await
        .unwrap();
    assert!(!local.in_job || cfg!(windows));
    let requests = opener.requests();
    assert!(
        matches!(&requests[0], OpenRequest::ShellExec { file, .. } if file == "steam://rungameid/1000001")
    );
    assert!(
        matches!(&requests[1], OpenRequest::Spawn { exe, cwd, .. } if exe.ends_with("FixtureQuest.exe") && cwd == exe.parent().unwrap())
    );

    let list = library(&backend).await;
    assert!(
        game(&list, "game-dir-fixture-quest")
            .last_played_at
            .is_some()
    );
    let rows: i64 = backend
        .state
        .db
        .call(|c| c.query_row("SELECT count(*) FROM games_session", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(rows, 1, "the session is kept for the next start");
    assert!(games.launch_after_game_mode("game-nothing").await.is_err());
    assert_eq!(
        games.get("game-steam-1000001").unwrap().title,
        "Steam Quest"
    );
    assert!(games.get("game-nothing").is_none());
}

// ---- what other modules see ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_and_recent_offer_only_visible_games_of_working_sources() {
    let backend = builder().start().await;
    let temp = temp_of(&backend);
    make_steam(&temp);
    make_folder_games(&temp);
    refreshed(&backend).await;
    let games = backend.state.games();

    let entries = games.search_entries();
    let titles: Vec<&str> = entries
        .iter()
        .map(|(item, _)| item.title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "Steam Arena",
            "Steam Quest",
            "Fixture Arena",
            "Fixture Quest"
        ]
    );
    assert_eq!(entries[0].0.subtitle.as_deref(), Some("Steam"));
    assert_eq!(entries[2].0.subtitle.as_deref(), Some("Local"));
    assert_eq!(entries[0].1, ["Steam"]);

    backend
        .send(
            "PATCH",
            "/games/game-steam-1000002",
            Some(json!({ "hidden": true })),
        )
        .await;
    assert!(games.item_for("game-steam-1000002").is_none());
    assert!(!games.knows("game-steam-1000002"));
    assert!(games.knows("game-steam-1000001"));
    let recent = games.recent();
    assert_eq!(
        recent.len(),
        1,
        "only Steam Quest has a last played time and is visible"
    );
    assert_eq!(recent[0].1.id, "game-steam-1000001");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_schema_of_the_games_tables_is_in_the_database() {
    let backend = builder().start().await;
    let tables: Vec<String> = backend
        .state
        .db
        .call(|c| {
            let mut s = c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'games_%' ORDER BY name")?;
            s.query_map([], |r| r.get(0))?.collect()
        })
        .await
        .unwrap();
    assert_eq!(
        tables,
        [
            "games_guess_cache",
            "games_play",
            "games_session",
            "games_size_cache"
        ]
    );
}
