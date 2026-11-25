//! `Resolved`: the settings as the other modules read them. Section 06 6.1.7.
//!
//! Other modules never parse `settings.toml` and never call `GET /settings`. They call
//! `settings::Service::resolved()`. Every path in it has gone through the one normaliser of the
//! backend (`core::paths::normalize`), after `%NAME%` was expanded, so the glyph that Settings
//! shows for a path and the folder that projects and games use always agree. An entry that is not
//! a usable path is left out of `Resolved` but stays in the API value, so the user sees it.

use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

use rain_types::CountdownSeconds;

use super::detect::Detected;
use super::rules;
use super::store::SettingsFile;
use crate::core::paths::normalize;

/// The note format when neither the user nor the vault has one.
pub const DEFAULT_NOTE_FORMAT: &str = "YYYY-MM-DD";
/// The countdown when `settings.toml` has none.
pub const DEFAULT_COUNTDOWN: CountdownSeconds = CountdownSeconds::Seconds10;

/// Plain Rust, not an API type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The chosen vault, normalised, whether or not the folder exists: the override when
    /// `vault_path` is set (so that a wrong path can be told from no path), else the only vault
    /// that Obsidian knows, else `None`.
    pub vault: Option<PathBuf>,
    /// Relative to the vault, `/` as the separator, may be empty.
    pub daily_note_folder: String,
    /// Never empty.
    pub daily_note_format: String,
    /// Normalised, absolute, without repeats.
    pub project_roots: Vec<PathBuf>,
    pub game_folders: Vec<PathBuf>,
    pub steam_enabled: bool,
    pub steam_path: Option<PathBuf>,
    pub epic_enabled: bool,
    pub epic_path: Option<PathBuf>,
    /// The folder of the other Epic launcher when the PC has both and the user typed no path.
    /// A typed `epic_path` means "read exactly this", so it leaves this empty.
    pub epic_extra_path: Option<PathBuf>,
    pub riot_enabled: bool,
    pub riot_path: Option<PathBuf>,
    pub countdown_seconds: u32,
    pub monitor_claude: bool,
}

impl Resolved {
    /// Settings that are not in the file, with the detected values behind the overrides.
    pub fn build(file: &SettingsFile, detected: &Detected) -> Self {
        let vault = match override_path(&file.vault_path) {
            OverridePath::Set(path) => Some(path),
            OverridePath::Unusable => None,
            OverridePath::Absent => detected.single_vault().map(|vault| vault.path.clone()),
        };
        Self {
            vault,
            daily_note_folder: note_folder(file, detected),
            daily_note_format: note_format(file, detected),
            project_roots: folders(&file.project_roots),
            game_folders: folders(&file.game_folders),
            steam_enabled: file.steam_enabled.unwrap_or(true),
            steam_path: path_with_fallback(&file.steam_path, detected.steam.as_ref()),
            epic_enabled: file.epic_enabled.unwrap_or(true),
            epic_path: path_with_fallback(&file.epic_path, detected.epic.as_ref()),
            epic_extra_path: detected
                .epic_extra
                .clone()
                .filter(|_| file.epic_path.trim().is_empty()),
            riot_enabled: file.riot_enabled.unwrap_or(true),
            riot_path: path_with_fallback(&file.riot_path, detected.riot.as_ref()),
            countdown_seconds: file
                .game_mode_countdown_seconds
                .unwrap_or(DEFAULT_COUNTDOWN) as u32,
            monitor_claude: file.monitor_claude.unwrap_or(false),
        }
    }
}

/// The vault that the user typed, normalised, for the detector that reads its notes settings.
/// `None` when the user typed none or typed something unusable.
pub fn vault_override(file: &SettingsFile) -> Option<PathBuf> {
    match override_path(&file.vault_path) {
        OverridePath::Set(path) => Some(path),
        OverridePath::Unusable | OverridePath::Absent => None,
    }
}

/// What the user typed in one of the path overrides.
enum OverridePath {
    Absent,
    /// Typed, and a usable absolute path.
    Set(PathBuf),
    /// Typed, but not a usable path (relative, `..`, a device path).
    Unusable,
}

fn override_path(text: &str) -> OverridePath {
    if text.trim().is_empty() {
        return OverridePath::Absent;
    }
    match normalize(&expand_env(text), true) {
        Ok(path) => OverridePath::Set(path),
        Err(_) => OverridePath::Unusable,
    }
}

/// An override beats detection, even when its folder does not exist (the games module then
/// reports the source as failed). An unusable override is not replaced by the detected value:
/// the user typed something, and silently reading another folder would hide the mistake.
fn path_with_fallback(text: &str, detected: Option<&PathBuf>) -> Option<PathBuf> {
    match override_path(text) {
        OverridePath::Set(path) => Some(path),
        OverridePath::Unusable => None,
        OverridePath::Absent => detected.cloned(),
    }
}

fn note_folder(file: &SettingsFile, detected: &Detected) -> String {
    rules::note_folder(&file.daily_note_folder)
        .ok()
        .filter(|folder| !folder.is_empty())
        .or_else(|| detected.notes.folder.clone())
        .unwrap_or_default()
}

fn note_format(file: &SettingsFile, detected: &Detected) -> String {
    let typed = file.daily_note_format.trim();
    if !typed.is_empty() {
        return typed.to_owned();
    }
    detected
        .notes
        .format
        .clone()
        .unwrap_or_else(|| DEFAULT_NOTE_FORMAT.to_owned())
}

/// Normalised folders without repeats (the same folder in another spelling or case counts
/// once). An entry that is not a usable path is dropped.
fn folders(entries: &[String]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    entries
        .iter()
        .filter_map(|entry| normalize(&expand_env(entry), true).ok())
        .filter(|path| seen.insert(path.to_string_lossy().to_lowercase()))
        .collect()
}

/// Replaces every `%NAME%` whose environment variable exists; a name that does not exist stays
/// as typed. This runs before `core::paths::normalize`, which expands nothing.
pub fn expand_env(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find('%') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('%') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        match env::var(name) {
            Ok(value) if !name.is_empty() => {
                out.push_str(&value);
                rest = &after[close + 1..];
            }
            // Not a variable: keep the first `%` and look again after it, so that `%a %SystemDrive%`
            // still finds the second pair.
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::settings::detect::{NoteSettings, VaultEntry};

    fn vault(path: &str) -> VaultEntry {
        VaultEntry {
            id: "id".into(),
            path: PathBuf::from(path),
        }
    }

    fn detected_with_one_vault() -> Detected {
        Detected {
            vaults: vec![vault(r"C:\Detected")],
            steam: Some(PathBuf::from(r"C:\Steam")),
            epic: Some(PathBuf::from(r"C:\Epic")),
            epic_extra: None,
            riot: Some(PathBuf::from(r"C:\Riot")),
            notes: NoteSettings {
                format: Some("DD-MM-YYYY".into()),
                folder: Some("Daily".into()),
            },
        }
    }

    #[test]
    fn the_other_epic_launcher_is_read_too_unless_the_user_typed_a_path() {
        let mut detected = detected_with_one_vault();
        detected.epic_extra = Some(PathBuf::from(r"C:\OldEpic"));
        let auto = Resolved::build(&SettingsFile::default(), &detected);
        assert_eq!(auto.epic_path, Some(PathBuf::from(r"C:\Epic")));
        assert_eq!(auto.epic_extra_path, Some(PathBuf::from(r"C:\OldEpic")));

        let file = SettingsFile {
            epic_path: r"E:\Epic".into(),
            ..SettingsFile::default()
        };
        let typed = Resolved::build(&file, &detected);
        assert_eq!(typed.epic_path, Some(PathBuf::from(r"E:\Epic")));
        assert_eq!(
            typed.epic_extra_path, None,
            "a typed path means exactly this folder"
        );
    }

    #[test]
    fn a_pc_with_nothing_set_and_nothing_found_has_the_defaults() {
        let resolved = Resolved::build(&SettingsFile::default(), &Detected::default());
        assert_eq!(resolved.vault, None);
        assert_eq!(resolved.daily_note_folder, "");
        assert_eq!(resolved.daily_note_format, "YYYY-MM-DD");
        assert!(resolved.project_roots.is_empty() && resolved.game_folders.is_empty());
        assert!(resolved.steam_enabled && resolved.epic_enabled);
        assert_eq!((resolved.steam_path, resolved.epic_path), (None, None));
        assert_eq!(resolved.countdown_seconds, 10);
        assert!(!resolved.monitor_claude);
    }

    #[test]
    fn detection_fills_what_the_user_left_empty() {
        let resolved = Resolved::build(&SettingsFile::default(), &detected_with_one_vault());
        assert_eq!(resolved.vault, Some(PathBuf::from(r"C:\Detected")));
        assert_eq!(resolved.daily_note_folder, "Daily");
        assert_eq!(resolved.daily_note_format, "DD-MM-YYYY");
        assert_eq!(resolved.steam_path, Some(PathBuf::from(r"C:\Steam")));
        assert_eq!(resolved.epic_path, Some(PathBuf::from(r"C:\Epic")));
        assert!(resolved.riot_enabled, "on unless the user turned it off");
        assert_eq!(resolved.riot_path, Some(PathBuf::from(r"C:\Riot")));
        let typed = SettingsFile {
            riot_path: r"E:\Riot".into(),
            riot_enabled: Some(false),
            ..SettingsFile::default()
        };
        let resolved = Resolved::build(&typed, &detected_with_one_vault());
        assert!(!resolved.riot_enabled);
        assert_eq!(resolved.riot_path, Some(PathBuf::from(r"E:\Riot")));
    }

    #[test]
    fn an_override_beats_detection_even_when_the_folder_does_not_exist() {
        let file = SettingsFile {
            vault_path: r"D:\My Vault".into(),
            daily_note_format: "YYYY.MM.DD".into(),
            daily_note_folder: "Journal".into(),
            steam_path: r"E:\Steam".into(),
            epic_path: r"E:\Epic".into(),
            ..SettingsFile::default()
        };
        let resolved = Resolved::build(&file, &detected_with_one_vault());
        assert_eq!(resolved.vault, Some(PathBuf::from(r"D:\My Vault")));
        assert_eq!(resolved.daily_note_format, "YYYY.MM.DD");
        assert_eq!(resolved.daily_note_folder, "Journal");
        assert_eq!(resolved.steam_path, Some(PathBuf::from(r"E:\Steam")));
        assert_eq!(resolved.epic_path, Some(PathBuf::from(r"E:\Epic")));
    }

    #[test]
    fn with_several_vaults_and_no_override_there_is_no_vault() {
        let mut detected = detected_with_one_vault();
        detected.vaults.push(vault(r"C:\Second"));
        assert_eq!(
            Resolved::build(&SettingsFile::default(), &detected).vault,
            None
        );
    }

    #[test]
    fn an_unusable_override_is_not_replaced_by_what_was_detected() {
        let file = SettingsFile {
            vault_path: "relative\\folder".into(),
            steam_path: "C:\\a\\..\\b".into(),
            ..SettingsFile::default()
        };
        let resolved = Resolved::build(&file, &detected_with_one_vault());
        assert_eq!(resolved.vault, None);
        assert_eq!(resolved.steam_path, None);
        assert_eq!(vault_override(&file), None);
    }

    #[test]
    fn the_folder_lists_are_normalised_deduplicated_and_drop_what_is_not_a_path() {
        let file = SettingsFile {
            project_roots: vec![
                "D:/Code/".into(),
                r"d:\code".into(),
                r#""C:\Fake\Library""#.into(),
                r"relative\folder".into(),
                r"C:\a\..\b".into(),
                r"\\?\C:\x".into(),
                r"\\server\share\Work".into(),
            ],
            game_folders: vec!["E:\\".into()],
            ..SettingsFile::default()
        };
        let resolved = Resolved::build(&file, &Detected::default());
        assert_eq!(
            resolved.project_roots,
            [
                PathBuf::from(r"D:\Code"),
                PathBuf::from(r"C:\Fake\Library"),
                PathBuf::from(r"\\server\share\Work"),
            ]
        );
        assert_eq!(resolved.game_folders, [PathBuf::from("E:\\")]);
    }

    #[test]
    fn a_variable_that_exists_is_expanded_and_one_that_does_not_stays() {
        let drive = env::var("SystemDrive").expect("Windows sets SystemDrive");
        assert_eq!(
            expand_env(r"%SystemDrive%\Games"),
            format!(r"{drive}\Games")
        );
        assert_eq!(
            expand_env(r"%NO_SUCH_VARIABLE_RAIN%\x"),
            r"%NO_SUCH_VARIABLE_RAIN%\x"
        );
        assert_eq!(expand_env(r"100%\x"), r"100%\x");
        assert_eq!(expand_env(""), "");
    }

    #[test]
    fn a_stray_percent_sign_does_not_hide_a_later_variable() {
        let drive = env::var("SystemDrive").expect("Windows sets SystemDrive");
        assert_eq!(expand_env(r"%a %SystemDrive%\x"), format!(r"%a {drive}\x"));
    }

    #[test]
    fn a_variable_in_a_list_entry_is_expanded_before_the_path_is_checked() {
        let file = SettingsFile {
            game_folders: vec![r"%SystemDrive%\Games".into()],
            ..SettingsFile::default()
        };
        let drive = env::var("SystemDrive").unwrap().to_uppercase();
        let resolved = Resolved::build(&file, &Detected::default());
        assert_eq!(
            resolved.game_folders,
            [PathBuf::from(format!(r"{drive}\Games"))]
        );
    }

    #[test]
    fn the_other_settings_follow_the_file() {
        let file = SettingsFile {
            steam_enabled: Some(false),
            epic_enabled: Some(false),
            game_mode_countdown_seconds: Some(CountdownSeconds::Seconds30),
            monitor_claude: Some(true),
            ..SettingsFile::default()
        };
        let resolved = Resolved::build(&file, &Detected::default());
        assert!(!resolved.steam_enabled && !resolved.epic_enabled);
        assert_eq!(resolved.countdown_seconds, 30);
        assert!(resolved.monitor_claude);
    }
}
