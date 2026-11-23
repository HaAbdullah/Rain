//! Epic Games: the games the Epic launcher has installed. Section 02, 6.10.
//!
//! Two launchers exist and a PC can have either or both:
//! - the OLD launcher keeps one `*.item` manifest (JSON) per game in `...\Data\Manifests`; this
//!   file reads them. Nothing here was verified on a PC with the old launcher: the facts come
//!   from the documented field names, so the reader is defensive: every field is optional, a file
//!   that does not fit is skipped, and when every file fails to parse the source says `failed`,
//!   which is how a changed file format shows;
//! - the NEW launcher keeps a journal and caches in `%LOCALAPPDATA%\com.epicgames.eglv2`;
//!   `epic_data.rs` reads them (6.10.1).
//!
//! A folder is told apart by what is inside: `installations.jsonl` means the new format.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use rain_types::GameSource;
use serde_json::Value;
use tracing::warn;

use super::epic_data;
use super::model::{EpicCatalog, Found, Launch, SourceResult};
use super::names::slug;

const ITEM_MAX: u64 = 1024 * 1024;

/// Why `read_capped` gave nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadProblem {
    /// The file is missing or cannot be read.
    Unreadable,
    /// The file is longer than the limit.
    TooLarge,
}

/// The bytes of a file of at most `max` bytes. Never reads more than `max + 1` bytes, so a huge
/// file costs nothing. Blocking.
pub fn read_capped(file: &Path, max: u64) -> Result<Vec<u8>, ReadProblem> {
    let mut bytes = Vec::new();
    fs::File::open(file)
        .map_err(|_| ReadProblem::Unreadable)?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadProblem::Unreadable)?;
    if bytes.len() as u64 > max {
        return Err(ReadProblem::TooLarge);
    }
    Ok(bytes)
}

/// The address that makes the launcher start a game. The old launcher's games are named by the
/// app name alone; the new launcher wants `<namespace>%3A<itemId>%3A<AppName>` (the colons are
/// escaped). Every part is plain characters: the readers checked them.
pub fn launch_url(app_name: &str, catalog: Option<&EpicCatalog>) -> String {
    let target = match catalog {
        Some(catalog) => format!("{}%3A{}%3A{app_name}", catalog.namespace, catalog.item_id),
        None => app_name.to_owned(),
    };
    format!("com.epicgames.launcher://apps/{target}?action=launch&silent=true")
}

/// `path` is the folder in use (the setting, else what was detected); `extra` is the folder of the
/// other launcher when a PC has both and the user typed no path. Either may hold either format.
pub fn scan(enabled: bool, path: Option<&Path>, extra: Option<&Path>) -> SourceResult {
    if !enabled {
        return SourceResult::off("Turned off in Settings");
    }
    let mut folders: Vec<&Path> = path.into_iter().collect();
    if let Some(extra) = extra
        && !folders.contains(&extra)
    {
        folders.push(extra);
    }
    if folders.is_empty() {
        return SourceResult::off("Epic Games is not installed");
    }

    // Each folder gives a list of games or a sentence about why not. The new format is read
    // first and wins when a game is in both: the new launcher is the one in use.
    let mut results: Vec<Result<Vec<Found>, String>> = Vec::new();
    let mut manifest_results: Vec<Result<Vec<Found>, String>> = Vec::new();
    for folder in &folders {
        let has_journal = folder.join(epic_data::JOURNAL_FILE).is_file();
        if has_journal {
            results.push(epic_data::read(folder));
        }
        // The manifests are in the folder itself or in one of two usual places below it. The
        // first place that holds manifest files wins; else the first place that exists (an
        // empty library). A folder of the new format needs no manifests.
        let places: Vec<PathBuf> = [
            folder.to_path_buf(),
            folder.join("Manifests"),
            folder.join("Data").join("Manifests"),
        ]
        .into_iter()
        .filter(|candidate| candidate.is_dir())
        .collect();
        let with_items = places.iter().find(|place| !item_files(place).is_empty());
        match (with_items, places.first(), has_journal) {
            (Some(place), _, _) => manifest_results.push(read_manifests(place)),
            (None, _, true) => {}
            (None, Some(place), false) => manifest_results.push(read_manifests(place)),
            (None, None, false) => {
                manifest_results.push(Err(format!("Epic folder not found: {}", folder.display())))
            }
        }
    }
    results.extend(manifest_results);

    let mut games: HashMap<String, Found> = HashMap::new();
    let mut problems: Vec<String> = Vec::new();
    let mut any_ok = false;
    for result in results {
        match result {
            Ok(found) => {
                any_ok = true;
                for game in found {
                    games.entry(game.key.clone()).or_insert(game);
                }
            }
            Err(message) => problems.push(message),
        }
    }
    if !any_ok {
        // Nothing could be read: the first sentence says what.
        return SourceResult::failed(
            problems
                .into_iter()
                .next()
                .unwrap_or_else(|| "Could not read the Epic Games library".to_owned()),
        );
    }
    // One folder worked and another did not: the games show, and the sentence says what is missing.
    SourceResult::ok(games.into_values().collect(), problems.into_iter().next())
}

/// The games of one folder of `*.item` manifests. `Err` is a sentence for the user.
fn read_manifests(folder: &Path) -> Result<Vec<Found>, String> {
    let files = item_files(folder);
    let (mut items, mut unparsable) = (Vec::new(), 0usize);
    for file in &files {
        match read_item(file) {
            Ok(Some(game)) => items.push(game),
            Ok(None) => {}
            Err(()) => {
                unparsable += 1;
                warn!(file = ?file.file_name(), "an Epic manifest could not be read");
            }
        }
    }
    if !files.is_empty() && unparsable == files.len() {
        return Err("Could not read the Epic Games library".to_owned());
    }
    Ok(items)
}

/// The `*.item` files of a folder (not recursive), in name order.
fn item_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|file| {
                    file.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("item"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// `Err` when the file is not JSON, `Ok(None)` when it is JSON that is not a listed game.
fn read_item(file: &Path) -> Result<Option<Found>, ()> {
    let bytes = read_capped(file, ITEM_MAX).map_err(|_| ())?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    Ok(parse_item(&value, &|folder| Path::new(folder).is_dir()))
}

/// The game of one manifest, or `None` for an add-on, an unfinished install, an entry of the
/// launcher itself or a manifest without what is needed. Pure: `folder_exists` is the one look at
/// the disk.
pub fn parse_item(item: &Value, folder_exists: &dyn Fn(&str) -> bool) -> Option<Found> {
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
    };
    let display_name = text("DisplayName")?;
    let app_name = text("AppName")?;
    // The name goes into a URL, so only plain characters are allowed.
    if !epic_data::is_plain_id(app_name)
        || item.get("bIsIncompleteInstall").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    // Add-ons and DLC name the game they belong to.
    if text("MainGameAppName").is_some_and(|main| main != app_name) {
        return None;
    }
    if let Some(categories) = item.get("AppCategories").and_then(Value::as_array) {
        let is_game = categories
            .iter()
            .filter_map(Value::as_str)
            .any(|category| category.eq_ignore_ascii_case("games"));
        if !is_game {
            return None;
        }
    }
    let location = text("InstallLocation").filter(|folder| folder_exists(folder))?;
    // `InstallSize` is a number in the files seen, but a string is accepted too.
    let size_bytes = match item.get("InstallSize") {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    };
    Some(Found {
        key: format!("epic:{app_name}"),
        id: format!("game-epic-{}", slug(app_name)),
        title: display_name.to_owned(),
        source: GameSource::Epic,
        launch: Launch::Epic {
            app_name: app_name.to_owned(),
            catalog: None,
        },
        install: Some(PathBuf::from(location)),
        size_bytes,
        cover: None,
        playtime_minutes: 0,
        last_played_unix: None,
        hidden: false,
        cover_version: None,
        local: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exists(_: &str) -> bool {
        true
    }

    fn good() -> Value {
        json!({
            "DisplayName": "Fixture Arena",
            "AppName": "FixtureArena_1",
            "InstallLocation": "D:\\Epic\\Fixture Arena",
            "InstallSize": 4096,
            "bIsIncompleteInstall": false,
            "AppCategories": ["public", "games", "applications"],
        })
    }

    #[test]
    fn a_normal_game_is_listed() {
        let game = parse_item(&good(), &exists).unwrap();
        assert_eq!(game.key, "epic:FixtureArena_1");
        assert_eq!(game.id, "game-epic-fixturearena-1");
        assert_eq!(game.title, "Fixture Arena");
        assert_eq!(game.size_bytes, Some(4096));
        assert_eq!(
            game.launch,
            Launch::Epic {
                app_name: "FixtureArena_1".into(),
                catalog: None,
            }
        );
    }

    #[test]
    fn what_is_not_a_game_is_dropped() {
        let mut dlc = good();
        dlc["MainGameAppName"] = json!("Other");
        assert!(parse_item(&dlc, &exists).is_none(), "DLC");
        let mut main = good();
        main["MainGameAppName"] = json!("FixtureArena_1");
        assert!(parse_item(&main, &exists).is_some(), "the game itself");
        let mut incomplete = good();
        incomplete["bIsIncompleteInstall"] = json!(true);
        assert!(parse_item(&incomplete, &exists).is_none());
        let mut tool = good();
        tool["AppCategories"] = json!(["applications"]);
        assert!(parse_item(&tool, &exists).is_none(), "no `games` category");
        let mut without = good();
        without.as_object_mut().unwrap().remove("AppCategories");
        assert!(
            parse_item(&without, &exists).is_some(),
            "no categories at all"
        );
        let mut spaced = good();
        spaced["AppName"] = json!("Fixture Arena");
        assert!(
            parse_item(&spaced, &exists).is_none(),
            "a name with a space"
        );
        let mut nowhere = good();
        nowhere.as_object_mut().unwrap().remove("InstallLocation");
        assert!(parse_item(&nowhere, &exists).is_none());
        assert!(parse_item(&good(), &|_| false).is_none(), "folder is gone");
    }

    #[test]
    fn the_size_may_be_text() {
        let mut item = good();
        item["InstallSize"] = json!("8192");
        assert_eq!(parse_item(&item, &exists).unwrap().size_bytes, Some(8192));
        item["InstallSize"] = json!(-5);
        assert_eq!(parse_item(&item, &exists).unwrap().size_bytes, None);
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_folder_of_manifests_is_read_and_a_bom_is_removed() {
        let temp = tempfile::tempdir().unwrap();
        let install = temp.path().join("installed");
        fs::create_dir_all(&install).unwrap();
        let mut item = good();
        item["InstallLocation"] = json!(install.to_string_lossy());
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(item.to_string().as_bytes());
        let manifests = temp.path().join("Data").join("Manifests");
        write(&manifests.join("a.item"), &bytes);
        write(&manifests.join("b.item"), b"not json");
        write(&manifests.join("notes.txt"), b"ignored");

        let result = scan(true, Some(temp.path()), None);
        assert_eq!(result.state, rain_types::SourceState::Ok);
        assert_eq!(result.items.len(), 1);
    }

    #[test]
    fn the_states_off_failed_and_garbage() {
        assert_eq!(
            scan(false, None, None).message.as_deref(),
            Some("Turned off in Settings")
        );
        let absent = scan(true, None, None);
        assert_eq!(absent.state, rain_types::SourceState::Off);
        assert_eq!(
            absent.message.as_deref(),
            Some("Epic Games is not installed")
        );

        let temp = tempfile::tempdir().unwrap();
        let gone = scan(true, Some(&temp.path().join("nothing")), None);
        assert_eq!(gone.state, rain_types::SourceState::Failed);

        for name in ["1.item", "2.item", "3.item"] {
            write(&temp.path().join("Manifests").join(name), b"{ broken");
        }
        let garbage = scan(true, Some(temp.path()), None);
        assert_eq!(garbage.state, rain_types::SourceState::Failed);
        assert_eq!(
            garbage.message.as_deref(),
            Some("Could not read the Epic Games library")
        );
    }

    #[test]
    fn an_empty_folder_is_fine() {
        let temp = tempfile::tempdir().unwrap();
        let result = scan(true, Some(temp.path()), None);
        assert_eq!(result.state, rain_types::SourceState::Ok);
        assert!(result.items.is_empty());
    }

    // ---- the new launcher ----

    /// A data folder of the new launcher with one invented game, installed in `<root>\Fixture Quest`.
    fn make_v2(data: &Path, root: &Path) -> PathBuf {
        let install = root.join("Fixture Quest");
        fs::create_dir_all(&install).unwrap();
        let record = json!({
            "installationId": "i1", "complete": true, "platform": "Windows",
            "dir": install.to_string_lossy(), "namespace": "fixturens",
            "itemId": "00112233445566778899aabbccddeeff", "artifactId": "FixtureQuest_1",
        });
        write(
            &data.join("installations.jsonl"),
            format!(
                "{{\"version\":1,\"entries\":1}}
{record}
"
            )
            .as_bytes(),
        );
        install
    }

    #[test]
    fn the_launch_address_has_the_three_parts_for_the_new_launcher_only() {
        let catalog = EpicCatalog {
            namespace: "fixturens".into(),
            item_id: "00112233445566778899aabbccddeeff".into(),
        };
        assert_eq!(
            launch_url("FixtureQuest_1", Some(&catalog)),
            "com.epicgames.launcher://apps/fixturens%3A00112233445566778899aabbccddeeff%3AFixtureQuest_1?action=launch&silent=true"
        );
        assert_eq!(
            launch_url("FixtureArena_1", None),
            "com.epicgames.launcher://apps/FixtureArena_1?action=launch&silent=true"
        );
    }

    #[test]
    fn a_data_folder_of_the_new_launcher_lists_its_games() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("eglv2");
        make_v2(&data, temp.path());
        let result = scan(true, Some(&data), None);
        assert_eq!(result.state, rain_types::SourceState::Ok);
        assert_eq!(result.items.len(), 1);
        let game = &result.items[0];
        assert_eq!(game.key, "epic:FixtureQuest_1");
        assert_eq!(
            game.title, "FixtureQuest_1",
            "no library cache: the app name"
        );
        assert!(matches!(
            &game.launch,
            Launch::Epic {
                catalog: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn a_game_in_both_formats_is_listed_once_and_the_new_format_wins() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("eglv2");
        make_v2(&data, temp.path());
        // The same app name in an old manifest, next to the journal and in another folder.
        let old_install = temp.path().join("old");
        fs::create_dir_all(&old_install).unwrap();
        let mut item = good();
        item["AppName"] = json!("FixtureQuest_1");
        item["DisplayName"] = json!("Old Name");
        item["InstallLocation"] = json!(old_install.to_string_lossy());
        write(
            &temp
                .path()
                .join("old-epic")
                .join("Manifests")
                .join("a.item"),
            item.to_string().as_bytes(),
        );
        let mut other = good();
        other["InstallLocation"] = json!(old_install.to_string_lossy());
        write(
            &temp
                .path()
                .join("old-epic")
                .join("Manifests")
                .join("b.item"),
            other.to_string().as_bytes(),
        );

        let both = scan(true, Some(&data), Some(&temp.path().join("old-epic")));
        assert_eq!(both.state, rain_types::SourceState::Ok);
        let mut keys: Vec<&str> = both.items.iter().map(|g| g.key.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["epic:FixtureArena_1", "epic:FixtureQuest_1"]);
        let quest = both
            .items
            .iter()
            .find(|g| g.key == "epic:FixtureQuest_1")
            .unwrap();
        assert!(
            matches!(
                &quest.launch,
                Launch::Epic {
                    catalog: Some(_),
                    ..
                }
            ),
            "the new format"
        );

        // The same folder holding both kinds gives the same answer.
        fs::copy(
            data.join("installations.jsonl"),
            temp.path().join("old-epic").join("installations.jsonl"),
        )
        .unwrap();
        let one_folder = scan(true, Some(&temp.path().join("old-epic")), None);
        assert_eq!(one_folder.items.len(), 2);
    }

    #[test]
    fn a_journal_that_cannot_be_used_fails_the_source_and_names_the_file() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join("installations.jsonl"),
            b"garbage
more garbage
",
        );
        let result = scan(true, Some(temp.path()), None);
        assert_eq!(result.state, rain_types::SourceState::Failed);
        assert_eq!(
            result.message.as_deref(),
            Some("Could not read installations.jsonl")
        );
        assert!(result.items.is_empty());
    }

    #[test]
    fn one_good_folder_keeps_the_source_ok_when_the_other_is_broken() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("eglv2");
        make_v2(&data, temp.path());
        let broken = temp.path().join("broken");
        write(
            &broken.join("installations.jsonl"),
            b"garbage
",
        );
        let result = scan(true, Some(&data), Some(&broken));
        assert_eq!(result.state, rain_types::SourceState::Ok);
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.message.as_deref(),
            Some("Could not read installations.jsonl")
        );
    }
}
