//! Riot Games: the games that the Riot Client has installed. Section 02, 6.10.2.
//!
//! The folder in use is the Riot Games data folder (`%ProgramData%\Riot Games`, or the path of the
//! setting). Two things are read from it:
//! - `RiotClientInstalls.json` names the Riot Client program (`rc_default`, else `rc_live`);
//! - `Metadata` has one folder per installed product, named `<product>.<patchline>`
//!   (`league_of_legends.live`). Inside, `<name>.product_settings.yaml` says where the game is
//!   installed and what its shortcut is called, and `<name>.ico` is its picture.
//!
//! Nothing is written there. A product whose install folder is gone is not listed.

use std::fs;
use std::path::{Path, PathBuf};

use rain_types::GameSource;
use serde_json::Value;
use tracing::warn;

use super::epic::{ReadProblem, read_capped};
use super::model::{Found, Launch, SourceResult};
use super::names::slug;
use crate::core::paths::normalize;

pub const CLIENTS_FILE: &str = "RiotClientInstalls.json";
pub const METADATA_DIR: &str = "Metadata";
/// The program of the Riot Client. It runs in the background most of the time, so it never means
/// that a game runs.
pub const CLIENT_EXE: &str = "RiotClientServices.exe";

const FILE_MAX: u64 = 1024 * 1024;

/// What the yaml file of one product says that Rain needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSettings {
    pub install_path: String,
    pub shortcut_name: Option<String>,
}

/// The value of a top-level `key: value` line. A value in double or single quotes loses its
/// quotes; a plain value loses a trailing ` # comment`. Lines that are indented, list items and
/// comments are never top-level lines, so the nested blocks of the file are skipped.
pub fn top_level_value(text: &str, key: &str) -> Option<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for line in text.lines() {
        if line.starts_with([' ', '\t', '#', '-']) {
            continue;
        }
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        return Some(unquote(rest.trim()));
    }
    None
}

fn unquote(value: &str) -> String {
    let mut letters = value.chars();
    match letters.next() {
        Some(quote @ ('"' | '\'')) => {
            let mut out = String::new();
            let mut rest = letters.peekable();
            while let Some(letter) = rest.next() {
                match letter {
                    '\\' if quote == '"' => match rest.next() {
                        Some(escaped) => out.push(escaped),
                        None => break,
                    },
                    '\'' if quote == '\'' && rest.peek() == Some(&'\'') => {
                        rest.next();
                        out.push('\'');
                    }
                    letter if letter == quote => break,
                    letter => out.push(letter),
                }
            }
            out
        }
        _ => value
            .split(" #")
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned(),
    }
}

/// The two facts of one product's yaml text, or `None` when the install path is missing.
pub fn parse_settings(text: &str) -> Option<ProductSettings> {
    let install_path =
        top_level_value(text, "product_install_full_path").filter(|p| !p.is_empty())?;
    Some(ProductSettings {
        install_path,
        shortcut_name: top_level_value(text, "shortcut_name").filter(|name| !name.is_empty()),
    })
}

/// The client program from the text of `RiotClientInstalls.json`: `rc_default`, else `rc_live`.
pub fn client_from_json(text: &str) -> Option<PathBuf> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value = serde_json::from_str(text).ok()?;
    ["rc_default", "rc_live"]
        .iter()
        .filter_map(|key| value.get(*key)?.as_str())
        .filter(|path| !path.is_empty())
        .find_map(|path| normalize(path, false).ok())
}

/// `league_of_legends.live` as `("league_of_legends", "live")`. Only plain characters pass,
/// because both parts go onto a command line.
pub fn split_product(folder_name: &str) -> Option<(&str, &str)> {
    let (product, patchline) = folder_name.split_once('.')?;
    let plain = |text: &str| {
        !text.is_empty()
            && text.len() <= 64
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    };
    (plain(product) && plain(patchline)).then_some((product, patchline))
}

/// The title of a product: the shortcut name without `.lnk`, else the product name read as words
/// (`league_of_legends` is "League Of Legends").
pub fn title_of(product: &str, shortcut_name: Option<&str>) -> String {
    let from_shortcut = shortcut_name
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".lnk") {
                &name[..name.len() - 4]
            } else {
                name
            }
        })
        .map(str::trim)
        .filter(|name| !name.is_empty());
    match from_shortcut {
        Some(title) => title.to_owned(),
        None => product
            .split(['_', '-'])
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut letters = word.chars();
                letters
                    .next()
                    .map(|first| first.to_uppercase().chain(letters).collect::<String>())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// The arguments that make the Riot Client start a product.
pub fn launch_args(product: &str, patchline: &str) -> String {
    format!("--launch-product={product} --launch-patchline={patchline}")
}

/// `folder` is the data folder in use (the setting, else what was detected).
pub fn scan(enabled: bool, folder: Option<&Path>) -> SourceResult {
    if !enabled {
        return SourceResult::off("Turned off in Settings");
    }
    let Some(folder) = folder else {
        return SourceResult::off("Riot Games is not installed");
    };
    if !folder.is_dir() {
        return SourceResult::failed(format!("Riot Games folder not found: {}", folder.display()));
    }

    let metadata = folder.join(METADATA_DIR);
    let mut names: Vec<String> = fs::read_dir(&metadata)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| split_product(name).is_some())
                .collect()
        })
        .unwrap_or_default();
    names.sort();

    let client = read_client(&folder.join(CLIENTS_FILE));
    let client = match client {
        Ok(client) => client,
        // Without a client nothing can be started. That only matters when there are games.
        Err(message) if !names.is_empty() => return SourceResult::failed(message),
        Err(_) => return SourceResult::ok(Vec::new(), None),
    };

    let (mut games, mut broken) = (Vec::new(), Vec::new());
    for name in &names {
        match read_product(&metadata.join(name), name, &client) {
            Ok(Some(game)) => games.push(game),
            Ok(None) => {}
            Err(()) => {
                warn!(product = %name, "a Riot product could not be read");
                broken.push(name.clone());
            }
        }
    }
    if games.is_empty() && !broken.is_empty() {
        return SourceResult::failed("Could not read the Riot Games library");
    }
    // Some products work and another does not: the games show, and the sentence says which.
    SourceResult::ok(
        games,
        broken
            .first()
            .map(|name| format!("Could not read the settings of {name}")),
    )
}

/// The client program, or a sentence for the user.
fn read_client(file: &Path) -> Result<PathBuf, String> {
    let bytes = match read_capped(file, FILE_MAX) {
        Ok(bytes) => bytes,
        Err(ReadProblem::Unreadable) => return Err(format!("Could not read {CLIENTS_FILE}")),
        Err(ReadProblem::TooLarge) => return Err(format!("{CLIENTS_FILE} is too large")),
    };
    let text = String::from_utf8_lossy(&bytes);
    let client = client_from_json(&text)
        .ok_or_else(|| format!("{CLIENTS_FILE} does not name the Riot Client"))?;
    if client.is_file() {
        Ok(client)
    } else {
        Err(format!("Riot Client not found: {}", client.display()))
    }
}

/// One product folder. `Ok(None)` when the game is not installed any more, `Err` when its yaml
/// cannot be used.
fn read_product(folder: &Path, name: &str, client: &Path) -> Result<Option<Found>, ()> {
    let (product, patchline) = split_product(name).ok_or(())?;
    let bytes = read_capped(
        &folder.join(format!("{name}.product_settings.yaml")),
        FILE_MAX,
    )
    .map_err(|_| ())?;
    let settings = parse_settings(&String::from_utf8_lossy(&bytes)).ok_or(())?;
    // The folder comes from a file, so it must be a plain local absolute path.
    let install = normalize(&settings.install_path, false).map_err(|_| ())?;
    if !install.is_dir() {
        return Ok(None);
    }
    Ok(Some(Found {
        key: format!("riot:{name}"),
        id: format!("game-riot-{}", slug(name)),
        title: title_of(product, settings.shortcut_name.as_deref()),
        source: GameSource::Riot,
        launch: Launch::Riot {
            client: client.to_path_buf(),
            product: product.to_owned(),
            patchline: patchline.to_owned(),
        },
        install: Some(install),
        // The size is measured in the background, like the size of a game folder.
        size_bytes: None,
        // The picture is the product's icon; the media route takes its largest image.
        cover: Some(folder.join(format!("{name}.ico"))),
        playtime_minutes: 0,
        last_played_unix: None,
        hidden: false,
        cover_version: None,
        local: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::SourceState;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A Riot data folder in `root\Riot Games` with a client and the products named, each
    /// installed in `root\Games\<name>`. Returns the data folder.
    fn make(root: &Path, products: &[(&str, &str)]) -> PathBuf {
        let data = root.join("Riot Games");
        let client = root.join("Client").join(CLIENT_EXE);
        write(&client, "x");
        write(
            &data.join(CLIENTS_FILE),
            &format!(
                "{{ \"rc_default\": \"{}\", \"rc_live\": \"\" }}",
                client.to_string_lossy().replace('\\', "/")
            ),
        );
        for (name, shortcut) in products {
            let install = root.join("Games").join(name);
            fs::create_dir_all(&install).unwrap();
            write(
                &data
                    .join(METADATA_DIR)
                    .join(name)
                    .join(format!("{name}.product_settings.yaml")),
                &format!(
                    "dependencies:\n    vanguard: true\nproduct_install_full_path: \"{}\"\nshortcut_name: \"{shortcut}\"\nsettings:\n    locale: \"en_US\"\n",
                    install.to_string_lossy().replace('\\', "/")
                ),
            );
        }
        data
    }

    #[test]
    fn the_yaml_reader_takes_top_level_lines_only() {
        let text = "\u{feff}# comment\nlocale_data:\n    shortcut_name: \"Nested\"\nproduct_install_full_path: \"C:/Riot Games/Teamfight Tactics/Live\"\nplain: League of Legends # note\nsingle: 'It''s'\nescaped: \"a\\\"b\"\nshortcut_name: \"Teamfight Tactics.lnk\"\nlist:\n- shortcut_name: x\n";
        assert_eq!(
            top_level_value(text, "product_install_full_path").as_deref(),
            Some("C:/Riot Games/Teamfight Tactics/Live")
        );
        assert_eq!(
            top_level_value(text, "plain").as_deref(),
            Some("League of Legends")
        );
        assert_eq!(top_level_value(text, "single").as_deref(), Some("It's"));
        assert_eq!(top_level_value(text, "escaped").as_deref(), Some("a\"b"));
        assert_eq!(
            top_level_value(text, "shortcut_name").as_deref(),
            Some("Teamfight Tactics.lnk"),
            "the nested line is skipped"
        );
        assert_eq!(top_level_value(text, "missing"), None);
        assert_eq!(top_level_value("not yaml at all", "x"), None);
    }

    #[test]
    fn settings_need_an_install_path() {
        assert_eq!(parse_settings("shortcut_name: \"A.lnk\"\n"), None);
        assert_eq!(parse_settings("product_install_full_path: \"\"\n"), None);
        let found = parse_settings("product_install_full_path: D:/G\n").unwrap();
        assert_eq!(found.install_path, "D:/G");
        assert_eq!(found.shortcut_name, None);
    }

    #[test]
    fn names_titles_and_arguments() {
        assert_eq!(
            split_product("league_of_legends.live"),
            Some(("league_of_legends", "live"))
        );
        assert_eq!(split_product("Riot Client"), None);
        assert_eq!(split_product("RiotGamesApi"), None);
        assert_eq!(split_product("a b.live"), None);
        assert_eq!(split_product("x.live --evil"), None);
        assert_eq!(
            title_of("x", Some("Teamfight Tactics.lnk")),
            "Teamfight Tactics"
        );
        assert_eq!(title_of("league_of_legends", None), "League Of Legends");
        assert_eq!(title_of("valorant", Some(".lnk")), "Valorant");
        assert_eq!(
            launch_args("league_of_legends", "live"),
            "--launch-product=league_of_legends --launch-patchline=live"
        );
    }

    #[test]
    fn the_client_is_the_default_else_the_live_one() {
        assert_eq!(
            client_from_json(
                r#"{"rc_default":"C:/Riot Games/Riot Client/RiotClientServices.exe","rc_live":"D:/x.exe"}"#
            ),
            Some(PathBuf::from(
                r"C:\Riot Games\Riot Client\RiotClientServices.exe"
            ))
        );
        assert_eq!(
            client_from_json(r#"{"rc_live":"D:/x.exe"}"#),
            Some(PathBuf::from(r"D:\x.exe"))
        );
        assert_eq!(client_from_json(r#"{"rc_default":""}"#), None);
        assert_eq!(client_from_json("{ broken"), None);
    }

    #[test]
    fn installed_products_are_listed_with_their_ids_keys_and_launch() {
        let temp = tempfile::tempdir().unwrap();
        let data = make(
            temp.path(),
            &[
                ("league_of_legends.live", "League of Legends.lnk"),
                ("teamfighttactics.live", "Teamfight Tactics.lnk"),
            ],
        );
        // Folders that are not products are ignored.
        write(
            &data.join(METADATA_DIR).join("Riot Client").join("x.yaml"),
            "x: 1\n",
        );
        fs::create_dir_all(data.join(METADATA_DIR).join("RiotGamesApi")).unwrap();

        let result = scan(true, Some(&data));
        assert_eq!(result.state, SourceState::Ok);
        assert_eq!(result.message, None);
        let ids: Vec<&str> = result.items.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "game-riot-league-of-legends-live",
                "game-riot-teamfighttactics-live"
            ]
        );
        let lol = &result.items[0];
        assert_eq!(lol.key, "riot:league_of_legends.live");
        assert_eq!(lol.title, "League of Legends");
        assert_eq!(lol.source, GameSource::Riot);
        assert!(lol.install.as_ref().unwrap().is_dir());
        assert!(
            lol.cover
                .as_ref()
                .unwrap()
                .ends_with("league_of_legends.live.ico")
        );
        match &lol.launch {
            Launch::Riot {
                client,
                product,
                patchline,
            } => {
                assert!(client.ends_with(CLIENT_EXE));
                assert_eq!(
                    (product.as_str(), patchline.as_str()),
                    ("league_of_legends", "live")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_product_without_its_install_folder_is_not_listed() {
        let temp = tempfile::tempdir().unwrap();
        let data = make(
            temp.path(),
            &[
                ("league_of_legends.live", "League of Legends.lnk"),
                ("valorant.live", "VALORANT.lnk"),
            ],
        );
        fs::remove_dir_all(temp.path().join("Games").join("valorant.live")).unwrap();
        let result = scan(true, Some(&data));
        assert_eq!(result.state, SourceState::Ok);
        assert_eq!(result.message, None, "an uninstalled game is not a problem");
        assert_eq!(result.items.len(), 1);
    }

    #[test]
    fn a_broken_yaml_hides_that_product_only() {
        let temp = tempfile::tempdir().unwrap();
        let data = make(
            temp.path(),
            &[
                ("league_of_legends.live", "League of Legends.lnk"),
                ("valorant.live", "VALORANT.lnk"),
            ],
        );
        write(
            &data
                .join(METADATA_DIR)
                .join("valorant.live")
                .join("valorant.live.product_settings.yaml"),
            "\u{0}\u{0} nothing useful\n",
        );
        let result = scan(true, Some(&data));
        assert_eq!(result.state, SourceState::Ok);
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.message.as_deref(),
            Some("Could not read the settings of valorant.live")
        );

        // A product with no yaml file at all counts as broken too, and when it is the only one
        // the source fails.
        let alone = tempfile::tempdir().unwrap();
        let data = make(alone.path(), &[("valorant.live", "VALORANT.lnk")]);
        fs::remove_file(
            data.join(METADATA_DIR)
                .join("valorant.live")
                .join("valorant.live.product_settings.yaml"),
        )
        .unwrap();
        let result = scan(true, Some(&data));
        assert_eq!(result.state, SourceState::Failed);
        assert_eq!(
            result.message.as_deref(),
            Some("Could not read the Riot Games library")
        );
    }

    #[test]
    fn a_missing_or_wrong_client_fails_the_source_when_there_are_games() {
        let temp = tempfile::tempdir().unwrap();
        let data = make(
            temp.path(),
            &[("league_of_legends.live", "League of Legends.lnk")],
        );
        fs::remove_file(data.join(CLIENTS_FILE)).unwrap();
        let result = scan(true, Some(&data));
        assert_eq!(result.state, SourceState::Failed);
        assert!(result.items.is_empty());
        assert_eq!(
            result.message.as_deref(),
            Some("Could not read RiotClientInstalls.json")
        );

        write(&data.join(CLIENTS_FILE), "{ broken");
        assert_eq!(scan(true, Some(&data)).state, SourceState::Failed);
        write(
            &data.join(CLIENTS_FILE),
            r#"{"rc_default":"C:/nowhere/RiotClientServices.exe"}"#,
        );
        let gone = scan(true, Some(&data));
        assert_eq!(gone.state, SourceState::Failed);
        assert!(gone.message.unwrap().starts_with("Riot Client not found"));

        // No games and no client is an empty library, not a failure.
        let empty = tempfile::tempdir().unwrap();
        fs::create_dir_all(empty.path().join("Riot Games")).unwrap();
        let result = scan(true, Some(&empty.path().join("Riot Games")));
        assert_eq!(result.state, SourceState::Ok);
        assert!(result.items.is_empty());
    }

    #[test]
    fn the_states_off_and_failed() {
        assert_eq!(
            scan(false, None).message.as_deref(),
            Some("Turned off in Settings")
        );
        let absent = scan(true, None);
        assert_eq!(absent.state, SourceState::Off);
        assert_eq!(
            absent.message.as_deref(),
            Some("Riot Games is not installed")
        );
        let temp = tempfile::tempdir().unwrap();
        let gone = scan(true, Some(&temp.path().join("nothing")));
        assert_eq!(gone.state, SourceState::Failed);
    }
}
