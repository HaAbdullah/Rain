//! Epic Games: the games the Epic launcher has installed. Section 02, 6.10.
//!
//! Nothing here was verified on a PC with Epic installed: the facts come from the documented
//! field names of the launcher's manifest files (`*.item`, JSON). The reader is defensive: every
//! field is optional, a file that does not fit is skipped, and when every file fails to parse the
//! source says `failed`, which is how a changed file format shows.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use rain_types::GameSource;
use serde_json::Value;
use tracing::warn;

use super::model::{Found, Launch, SourceResult};
use super::names::slug;

const ITEM_MAX: u64 = 1024 * 1024;

pub fn scan(enabled: bool, path: Option<&Path>) -> SourceResult {
    if !enabled {
        return SourceResult::off("Turned off in Settings");
    }
    let Some(path) = path else {
        return SourceResult::off("Epic Games is not installed");
    };
    // The manifests are in the folder itself or in one of two usual places below it. The first
    // place that holds manifest files wins; else the first place that exists (an empty library).
    let places: Vec<PathBuf> = [
        path.to_path_buf(),
        path.join("Manifests"),
        path.join("Data").join("Manifests"),
    ]
    .into_iter()
    .filter(|candidate| candidate.is_dir())
    .collect();
    let Some(folder) = places
        .iter()
        .find(|place| !item_files(place).is_empty())
        .or(places.first())
    else {
        return SourceResult::failed(format!("Epic folder not found: {}", path.display()));
    };

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
        return SourceResult::failed("Could not read the Epic Games library");
    }
    SourceResult::ok(items, None)
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
    let mut bytes = Vec::new();
    fs::File::open(file)
        .map_err(|_| ())?
        .take(ITEM_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > ITEM_MAX {
        return Err(());
    }
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
    let plain = app_name.len() <= 128
        && app_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    if !plain || item.get("bIsIncompleteInstall").and_then(Value::as_bool) == Some(true) {
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
                app_name: "FixtureArena_1".into()
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

        let result = scan(true, Some(temp.path()));
        assert_eq!(result.state, rain_types::SourceState::Ok);
        assert_eq!(result.items.len(), 1);
    }

    #[test]
    fn the_states_off_failed_and_garbage() {
        assert_eq!(
            scan(false, None).message.as_deref(),
            Some("Turned off in Settings")
        );
        let absent = scan(true, None);
        assert_eq!(absent.state, rain_types::SourceState::Off);
        assert_eq!(
            absent.message.as_deref(),
            Some("Epic Games is not installed")
        );

        let temp = tempfile::tempdir().unwrap();
        let gone = scan(true, Some(&temp.path().join("nothing")));
        assert_eq!(gone.state, rain_types::SourceState::Failed);

        for name in ["1.item", "2.item", "3.item"] {
            write(&temp.path().join("Manifests").join(name), b"{ broken");
        }
        let garbage = scan(true, Some(temp.path()));
        assert_eq!(garbage.state, rain_types::SourceState::Failed);
        assert_eq!(
            garbage.message.as_deref(),
            Some("Could not read the Epic Games library")
        );
    }

    #[test]
    fn an_empty_folder_is_fine() {
        let temp = tempfile::tempdir().unwrap();
        let result = scan(true, Some(temp.path()));
        assert_eq!(result.state, rain_types::SourceState::Ok);
        assert!(result.items.is_empty());
    }
}
