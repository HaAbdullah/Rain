//! `games.toml`: the user's corrections and the games added by hand. Section 02, 4.2.
//!
//! Two kinds of table:
//! - `[[game]]` changes how Rain shows a game it finds by itself (hide it, a nicer title, another
//!   program file, a cover). It is keyed by the game key.
//! - `[[custom]]` is a game that was added by hand.
//!
//! Reading is tolerant: one bad entry is skipped (and logged) and never fails the file, because
//! people edit this file. Writing goes through `TomlFile::edit`, so comments and order survive.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};
use tracing::warn;

use super::names::path_key;
use crate::core::error::{ApiError, ApiResult};
use crate::store::toml_file::{set_value, string_value};

pub const FILE_NAME: &str = "games.toml";

const HEADER: &str = "\
# Your corrections and hand-added games. Rain edits this file when you use the Games screen.
# A [[game]] entry changes how Rain shows a game it finds by itself. Remove the entry to undo it.
";

pub const TITLE_MAX: usize = 120;
const ARGS_MAX: usize = 500;

/// The file as the typed reader sees it: the entries stay loose values so that one bad entry
/// cannot fail the whole file. `parse` turns them into `Correction` and `Custom`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GamesDoc {
    pub schema_version: Option<u32>,
    pub game: Vec<toml::Value>,
    pub custom: Vec<toml::Value>,
}

/// The user's changes to one found game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Correction {
    pub hidden: bool,
    /// Folder games only.
    pub title: Option<String>,
    /// Folder games only: relative to the folder, or absolute.
    pub exe: Option<String>,
    pub args: Option<String>,
    /// Folder games only: `media/<file>` below the data folder, or an absolute image path.
    pub cover: Option<String>,
}

/// A game added by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Custom {
    pub id: String,
    pub title: String,
    pub exe: String,
    pub args: Option<String>,
    pub cover: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Default)]
pub struct Parsed {
    /// By `canonical_key`.
    pub corrections: HashMap<String, Correction>,
    pub customs: Vec<Custom>,
    /// How many entries were skipped, for the log.
    pub skipped: usize,
    /// Each skipped entry: its table (`game` or `custom`) and its place among the entries of
    /// that table. `GET /health` names them (`Service::report_entry_issues`).
    pub skipped_at: Vec<(&'static str, usize)>,
}

/// The key of a game as `games.toml` and SQLite compare it: `dir:` keys by `path_key`, the
/// others as written. `None` for a key that is not one of the three shapes.
pub fn canonical_key(key: &str) -> Option<String> {
    let key = key.trim();
    if let Some(appid) = key.strip_prefix("steam:") {
        return (!appid.is_empty() && appid.bytes().all(|b| b.is_ascii_digit()))
            .then(|| key.to_owned());
    }
    if let Some(name) = key.strip_prefix("epic:") {
        return plain_epic_name(name).then(|| key.to_owned());
    }
    if let Some(folder) = key.strip_prefix("dir:") {
        return is_absolute(folder).then(|| format!("dir:{}", path_key(folder)));
    }
    None
}

/// The game name of an Epic game goes into a URL, so only plain characters are allowed.
pub fn plain_epic_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// `X:\...` or `\\server\...`.
pub fn is_absolute(text: &str) -> bool {
    let text = text.trim();
    let bytes = text.as_bytes();
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/'))
        || text.starts_with("\\\\")
}

pub fn is_exe(text: &str) -> bool {
    Path::new(text)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

fn is_image_path(text: &str) -> bool {
    super::model::is_image_extension(Path::new(text))
}

/// Reads the entries. A bad entry is skipped with a warning (never its content), a repeated key
/// or id keeps the first.
pub fn parse(doc: &GamesDoc) -> Parsed {
    let mut parsed = Parsed::default();
    for (index, entry) in doc.game.iter().enumerate() {
        match correction(entry) {
            Some((key, correction)) if !parsed.corrections.contains_key(&key) => {
                parsed.corrections.insert(key, correction);
            }
            _ => skip(&mut parsed, "game", index),
        }
    }
    for (index, entry) in doc.custom.iter().enumerate() {
        match custom(entry) {
            Some(custom) if !parsed.customs.iter().any(|known| known.id == custom.id) => {
                parsed.customs.push(custom);
            }
            _ => skip(&mut parsed, "custom", index),
        }
    }
    parsed
}

fn skip(parsed: &mut Parsed, table: &'static str, index: usize) {
    parsed.skipped += 1;
    parsed.skipped_at.push((table, index));
    warn!(table, "an entry of games.toml was skipped");
}

fn text<'a>(entry: &'a toml::Value, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(toml::Value::as_str)
}

/// A string field that may be absent. `Err` when it is present with the wrong type or too long.
fn optional_text(entry: &toml::Value, key: &str, max: usize) -> Result<Option<String>, ()> {
    match entry.get(key) {
        None => Ok(None),
        Some(toml::Value::String(text)) if text.chars().count() <= max => {
            Ok(Some(text.clone()).filter(|text| !text.trim().is_empty()))
        }
        Some(_) => Err(()),
    }
}

fn optional_bool(entry: &toml::Value, key: &str) -> Result<bool, ()> {
    match entry.get(key) {
        None => Ok(false),
        Some(toml::Value::Boolean(flag)) => Ok(*flag),
        Some(_) => Err(()),
    }
}

fn valid_title(title: &str) -> bool {
    let length = title.trim().chars().count();
    (1..=TITLE_MAX).contains(&length)
}

fn correction(entry: &toml::Value) -> Option<(String, Correction)> {
    let key = canonical_key(text(entry, "key")?)?;
    let title = optional_text(entry, "title", TITLE_MAX).ok()?;
    let exe = optional_text(entry, "exe", 1024).ok()?;
    let cover = optional_text(entry, "cover", 1024).ok()?;
    if exe.as_deref().is_some_and(|exe| !is_exe(exe))
        || cover.as_deref().is_some_and(|cover| !is_image_path(cover))
    {
        return None;
    }
    Some((
        key,
        Correction {
            hidden: optional_bool(entry, "hidden").ok()?,
            title: title.map(|title| title.trim().to_owned()),
            exe,
            args: optional_text(entry, "args", ARGS_MAX).ok()?,
            cover,
        },
    ))
}

fn custom(entry: &toml::Value) -> Option<Custom> {
    let id = text(entry, "id")?.trim();
    let id_ok = id.starts_with("game-")
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let title = text(entry, "title")?.trim();
    let exe = text(entry, "exe")?.trim();
    let cover = optional_text(entry, "cover", 1024).ok()?;
    if !id_ok
        || !valid_title(title)
        || !is_exe(exe)
        || !is_absolute(exe)
        || cover.as_deref().is_some_and(|cover| !is_image_path(cover))
    {
        return None;
    }
    Some(Custom {
        id: id.to_owned(),
        title: title.to_owned(),
        exe: exe.to_owned(),
        args: optional_text(entry, "args", ARGS_MAX).ok()?,
        cover,
        hidden: optional_bool(entry, "hidden").ok()?,
    })
}

// ---- writing ----

/// One field of a `[[game]]` or `[[custom]]` entry that an edit sets. `None` removes the key
/// (the field goes back to its default); `Hidden(false)` removes the key too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    Hidden(bool),
    Title(Option<String>),
    Exe(Option<String>),
    Cover(Option<String>),
}

fn apply(table: &mut Table, field: &Field) {
    let (key, new) = match field {
        Field::Hidden(true) => ("hidden", Some(toml_edit::Value::from(true))),
        Field::Hidden(false) => ("hidden", None),
        Field::Title(text) => ("title", text.as_deref().map(string_value)),
        Field::Exe(text) => ("exe", text.as_deref().map(string_value)),
        Field::Cover(text) => ("cover", text.as_deref().map(string_value)),
    };
    match new {
        Some(new) => set_value(table, key, new),
        None => {
            table.remove(key);
        }
    }
}

/// A new file gets its first comment and `schema_version` before the first entry.
fn start_file(document: &mut DocumentMut) {
    if document.to_string().trim().is_empty() {
        document.insert("schema_version", value(1));
        if let Some(mut key) = document.key_mut("schema_version") {
            key.leaf_decor_mut().set_prefix(HEADER);
        }
    }
}

fn tables_mut<'a>(document: &'a mut DocumentMut, name: &str) -> ApiResult<&'a mut ArrayOfTables> {
    let item = document
        .entry(name)
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    item.as_array_of_tables_mut().ok_or_else(|| {
        ApiError::conflict("configChanged", "games.toml changed. Try again.").with_source("games")
    })
}

/// Changes the `[[game]]` entry of `key`: the entry is created when needed and removed again when
/// nothing but its key is left.
pub fn edit_correction(document: &mut DocumentMut, key: &str, fields: &[Field]) -> ApiResult<()> {
    let wanted = canonical_key(key).unwrap_or_else(|| key.to_owned());
    let existing = document
        .get("game")
        .and_then(Item::as_array_of_tables)
        .and_then(|tables| {
            tables.iter().position(|table| {
                table
                    .get("key")
                    .and_then(Item::as_str)
                    .and_then(canonical_key)
                    .is_some_and(|found| found == wanted)
            })
        });
    if existing.is_none() && fields.iter().all(is_default) {
        return Ok(()); // nothing to remove and nothing to add
    }
    start_file(document);
    let tables = tables_mut(document, "game")?;
    let index = match existing {
        Some(index) => index,
        None => {
            let mut table = Table::new();
            set_value(&mut table, "key", string_value(key));
            tables.push(table);
            tables.len() - 1
        }
    };
    if let Some(table) = tables.get_mut(index) {
        for field in fields {
            apply(table, field);
        }
        // Only the key left: every field is back to its default, so the entry goes.
        if table.len() == 1 {
            tables.remove(index);
        }
    }
    if tables.is_empty() {
        document.remove("game");
    }
    Ok(())
}

fn is_default(field: &Field) -> bool {
    matches!(
        field,
        Field::Hidden(false) | Field::Title(None) | Field::Exe(None) | Field::Cover(None)
    )
}

/// Adds a hand-added game.
pub fn add_custom(document: &mut DocumentMut, game: &Custom) -> ApiResult<()> {
    start_file(document);
    let tables = tables_mut(document, "custom")?;
    let mut table = Table::new();
    set_value(&mut table, "id", string_value(&game.id));
    set_value(&mut table, "title", string_value(&game.title));
    set_value(&mut table, "exe", string_value(&game.exe));
    if let Some(args) = &game.args {
        set_value(&mut table, "args", string_value(args));
    }
    if let Some(cover) = &game.cover {
        set_value(&mut table, "cover", string_value(cover));
    }
    if game.hidden {
        set_value(&mut table, "hidden", toml_edit::Value::from(true));
    }
    tables.push(table);
    Ok(())
}

/// Changes a hand-added game. The id must exist, else the file changed under the caller.
pub fn edit_custom(document: &mut DocumentMut, id: &str, fields: &[Field]) -> ApiResult<()> {
    let changed = || {
        ApiError::conflict("configChanged", "games.toml changed. Try again.").with_source("games")
    };
    let table = document
        .get_mut("custom")
        .and_then(Item::as_array_of_tables_mut)
        .and_then(|tables| {
            tables
                .iter_mut()
                .find(|table| table.get("id").and_then(Item::as_str) == Some(id))
        })
        .ok_or_else(changed)?;
    for field in fields {
        apply(table, field);
    }
    Ok(())
}

/// Removes a hand-added game; `false` when it was not there.
pub fn remove_custom(document: &mut DocumentMut, id: &str) -> bool {
    let Some(tables) = document
        .get_mut("custom")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return false;
    };
    let Some(index) = tables
        .iter()
        .position(|table| table.get("id").and_then(Item::as_str) == Some(id))
    else {
        return false;
    };
    tables.remove(index);
    if tables.is_empty() {
        document.remove("custom");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> GamesDoc {
        toml::from_str(text).unwrap()
    }

    fn edit(text: &str, change: impl FnOnce(&mut DocumentMut) -> ApiResult<()>) -> String {
        let mut document: DocumentMut = text.parse().unwrap();
        change(&mut document).unwrap();
        document.to_string()
    }

    const FILE: &str = "\
schema_version = 1

# my note
[[game]]
key = \"steam:123\"
hidden = true   # hidden for a reason

[[game]]
key = \"dir:D:\\\\Games\\\\Some Game\"
title = \"Some Game: Remastered\"
exe = \"bin\\\\x64\\\\SomeGame.exe\"
cover = \"media/game-dir-some-game-3fa9c2d1.jpg\"

[[custom]]
id = \"game-celeste\"
title = \"Celeste\"
exe = \"D:\\\\Elsewhere\\\\Celeste\\\\Celeste.exe\"
";

    #[test]
    fn keys_have_three_shapes_and_dir_keys_compare_by_path() {
        assert_eq!(canonical_key("steam:123").as_deref(), Some("steam:123"));
        assert_eq!(canonical_key("steam:12a"), None);
        assert_eq!(
            canonical_key("epic:Fixture_1").as_deref(),
            Some("epic:Fixture_1")
        );
        assert_eq!(canonical_key("epic:with space"), None);
        assert_eq!(
            canonical_key("dir:d:/Games/Some Game/"),
            canonical_key("dir:D:\\games\\some game")
        );
        assert_eq!(canonical_key("dir:relative"), None);
        assert_eq!(canonical_key("other:1"), None);
    }

    #[test]
    fn the_file_is_read_into_corrections_and_customs() {
        let parsed = parse(&doc(FILE));
        assert_eq!(parsed.skipped, 0);
        assert!(parsed.corrections["steam:123"].hidden);
        let folder = &parsed.corrections[&canonical_key("dir:D:\\Games\\Some Game").unwrap()];
        assert_eq!(folder.title.as_deref(), Some("Some Game: Remastered"));
        assert_eq!(folder.exe.as_deref(), Some("bin\\x64\\SomeGame.exe"));
        assert_eq!(parsed.customs.len(), 1);
        assert_eq!(parsed.customs[0].id, "game-celeste");
    }

    #[test]
    fn a_bad_entry_is_skipped_and_a_duplicate_keeps_the_first() {
        let parsed = parse(&doc("[[game]]\nkey = \"steam:1\"\nhidden = true\n\
             [[game]]\nkey = \"steam:1\"\nhidden = false\n\
             [[game]]\nkey = \"bogus\"\n\
             [[game]]\nkey = \"steam:2\"\nexe = \"x.txt\"\n\
             [[game]]\nkey = \"steam:3\"\nhidden = \"yes\"\n\
             [[custom]]\nid = \"game-a\"\ntitle = \"A\"\nexe = \"relative.exe\"\n\
             [[custom]]\nid = \"game-b\"\ntitle = \"B\"\nexe = \"D:\\\\b.exe\"\n\
             [[custom]]\nid = \"game-b\"\ntitle = \"Again\"\nexe = \"D:\\\\c.exe\"\n"));
        assert_eq!(parsed.corrections.len(), 1);
        assert!(parsed.corrections["steam:1"].hidden);
        assert_eq!(parsed.customs.len(), 1);
        assert_eq!(parsed.customs[0].title, "B");
        assert_eq!(parsed.skipped, 6);
    }

    #[test]
    fn hiding_adds_an_entry_and_showing_removes_it_again_down_to_the_header() {
        let hidden = edit("", |d| {
            edit_correction(d, "steam:7", &[Field::Hidden(true)])
        });
        assert!(hidden.starts_with("# Your corrections"), "{hidden}");
        assert!(hidden.contains("schema_version = 1"));
        assert!(
            hidden.contains("[[game]]\nkey = 'steam:7'\nhidden = true"),
            "{hidden}"
        );
        assert!(parse(&doc(&hidden)).corrections["steam:7"].hidden);

        let shown = edit(&hidden, |d| {
            edit_correction(d, "steam:7", &[Field::Hidden(false)])
        });
        assert!(
            !shown.contains(
                "[[game]]
"
            ),
            "{shown}"
        );
        assert!(shown.contains("schema_version = 1"));
        // Unhiding what was never hidden writes nothing.
        let same = edit("schema_version = 1\n", |d| {
            edit_correction(d, "steam:9", &[Field::Hidden(false)])
        });
        assert_eq!(same, "schema_version = 1\n");
    }

    #[test]
    fn comments_order_and_line_ends_survive_an_edit() {
        let crlf = FILE.replace('\n', "\r\n");
        let after = edit(&crlf, |d| {
            edit_correction(
                d,
                "dir:D:\\Games\\Some Game",
                &[Field::Title(Some("New".into()))],
            )
        });
        assert!(after.contains("# my note"));
        assert!(after.contains("hidden = true   # hidden for a reason"));
        assert!(after.contains("title = 'New'"), "{after}");
        assert!(after.find("steam:123").unwrap() < after.find("Some Game").unwrap());
        assert!(after.find("Some Game").unwrap() < after.find("game-celeste").unwrap());
        // Line ends are put back by `TomlFile::edit` (its own tests), not by this function.
    }

    #[test]
    fn clearing_every_field_removes_the_entry_of_a_folder_game() {
        let after = edit(FILE, |d| {
            edit_correction(
                d,
                "dir:d:/games/some game",
                &[Field::Title(None), Field::Exe(None), Field::Cover(None)],
            )
        });
        assert!(!after.contains("Some Game"), "{after}");
        assert!(after.contains("steam:123"));
    }

    #[test]
    fn custom_games_are_added_edited_and_removed() {
        let custom = Custom {
            id: "game-x".into(),
            title: "X: the 'game'".into(),
            exe: "D:\\x\\x.exe".into(),
            args: None,
            cover: Some("media/game-x-12345678.png".into()),
            hidden: false,
        };
        let added = edit("", |d| add_custom(d, &custom));
        let parsed = parse(&doc(&added));
        assert_eq!(parsed.customs, std::slice::from_ref(&custom));

        let edited = edit(&added, |d| {
            edit_custom(
                d,
                "game-x",
                &[
                    Field::Title(Some("Y".into())),
                    Field::Cover(None),
                    Field::Hidden(true),
                ],
            )
        });
        let parsed = parse(&doc(&edited));
        assert_eq!(parsed.customs[0].title, "Y");
        assert_eq!(parsed.customs[0].cover, None);
        assert!(parsed.customs[0].hidden);

        let mut document: DocumentMut = edited.parse().unwrap();
        assert!(remove_custom(&mut document, "game-x"));
        assert!(!remove_custom(&mut document, "game-x"));
        assert!(!document.to_string().contains("[[custom]]"));
        assert!(edit_custom(&mut document, "game-x", &[]).is_err());
    }
}
