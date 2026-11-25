//! `gamemode.toml`: the master switch for closing other programs, and the programs the user
//! keeps. Section 06 6.11.
//!
//! The file is the user's. Rain writes only `close_other_apps` and `keep`, and keeps every
//! comment. A hand edit is read at once (the hub's watcher).

use serde::Deserialize;
use toml_edit::{Item, value};

use super::keep;
use crate::core::error::{ApiError, ApiResult};
use crate::store::hub::ConfigHub;
use crate::store::toml_file::{TomlFile, set_value, string_list_value};

pub const FILE_NAME: &str = "gamemode.toml";

/// The longest `keep` list and the longest name in it.
const KEEP_MAX: usize = 500;
const NAME_MAX: usize = 128;

/// The file Rain writes at the first start.
pub const DEFAULT_FILE: &str = r#"schema_version = 1

# Game mode: what it closes on the whole PC.
# Rain reloads this file at once. You may edit it by hand.

# false: game mode closes only what Rain started, then Rain itself.
close_other_apps = true

# Programs that game mode never closes, by file name. The Game mode screen writes this list when
# you untick a program; you can add names by hand. Windows, drivers, game launchers, anti-cheat,
# Claude and Rain are always kept and need no entry.
keep = []
"#;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OthersFile {
    pub close_other_apps: bool,
    pub keep: Vec<String>,
}

impl Default for OthersFile {
    fn default() -> Self {
        Self {
            close_other_apps: true,
            keep: Vec::new(),
        }
    }
}

fn validate(file: &OthersFile) -> Result<(), String> {
    if file.keep.len() > KEEP_MAX {
        return Err(format!("keep has more than {KEEP_MAX} names"));
    }
    if file
        .keep
        .iter()
        .any(|name| name.trim().is_empty() || name.len() > NAME_MAX)
    {
        return Err(format!("every name in keep is 1 to {NAME_MAX} characters"));
    }
    Ok(())
}

pub async fn open(hub: &ConfigHub) -> ApiResult<TomlFile<OthersFile>> {
    TomlFile::open(hub, FILE_NAME, validate).await
}

/// Whether `keep` holds the program (`id` is its lower-case file name; `.exe` is optional in the
/// file and the case does not matter).
pub fn keeps(file: &OthersFile, id: &str) -> bool {
    let wanted = keep::stem(id);
    file.keep.iter().any(|name| keep::stem(name) == wanted)
}

/// Writes `close_other_apps`.
pub async fn set_enabled(file: &TomlFile<OthersFile>, enabled: bool) -> ApiResult<()> {
    file.edit(move |document| {
        document["close_other_apps"] = value(enabled);
        Ok(())
    })
    .await
    .map(|_| ())
}

/// Adds the program to `keep` (`closing = false`) or takes it out (`closing = true`).
pub async fn set_closing(file: &TomlFile<OthersFile>, id: &str, closing: bool) -> ApiResult<()> {
    let id = id.trim().to_lowercase();
    if id.is_empty() || id.len() > NAME_MAX {
        return Err(ApiError::invalid("That is not a program name").with_reason("badName"));
    }
    file.edit(move |document| {
        let mut names: Vec<String> = match document.get("keep") {
            Some(Item::Value(toml_edit::Value::Array(array))) => array
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        let wanted = keep::stem(&id);
        let before = names.len();
        names.retain(|name| keep::stem(name) != wanted);
        let had = names.len() != before;
        if !closing {
            if names.len() >= KEEP_MAX {
                return Err(ApiError::invalid("The keep list is full").with_reason("tooMany"));
            }
            names.push(id);
        } else if !had {
            return Ok(());
        }
        set_value(document.as_table_mut(), "keep", string_list_value(&names));
        Ok(())
    })
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_is_valid_and_means_close_with_an_empty_keep_list() {
        let file: OthersFile = toml::from_str(DEFAULT_FILE).unwrap();
        assert_eq!(file, OthersFile::default());
        assert!(file.close_other_apps);
        assert!(validate(&file).is_ok());
    }

    #[test]
    fn a_missing_key_means_the_default() {
        let file: OthersFile = toml::from_str("keep = ['discord.exe']").unwrap();
        assert!(file.close_other_apps);
        let file: OthersFile = toml::from_str("close_other_apps = false").unwrap();
        assert!(!file.close_other_apps);
    }

    #[test]
    fn keep_matches_by_name_whatever_the_case_or_the_extension() {
        let file = OthersFile {
            close_other_apps: true,
            keep: vec!["Discord".into(), "slack.EXE".into()],
        };
        assert!(keeps(&file, "discord.exe"));
        assert!(keeps(&file, "SLACK.exe"));
        assert!(!keeps(&file, "chrome.exe"));
    }

    #[test]
    fn empty_and_huge_names_are_refused() {
        let empty = OthersFile {
            keep: vec!["  ".into()],
            ..OthersFile::default()
        };
        assert!(validate(&empty).is_err());
        let long = OthersFile {
            keep: vec!["a".repeat(NAME_MAX + 1)],
            ..OthersFile::default()
        };
        assert!(validate(&long).is_err());
    }
}
