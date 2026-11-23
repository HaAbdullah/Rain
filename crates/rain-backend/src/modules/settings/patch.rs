//! Turns the body of `PATCH /settings` into a plan, and refuses what is not allowed.
//! Section 06 6.2, rules R2 and R3 (table V).
//!
//! Pure: it checks and converts, and touches neither disk nor state. The first failure rejects
//! the whole patch. A plan has the edits of `settings.toml`, or one effect on the PC (start with
//! Windows, the data folder, the Claude switch) that the service carries out.

use std::collections::HashSet;

use rain_types::SettingsPatch;
use toml_edit::Value;

use super::SOURCE;
use super::resolved::expand_env;
use super::rules::{self, Reject};
use super::store::KeyEdit;
use crate::core::error::{ApiError, ApiResult};
use crate::core::paths::normalize;
use crate::store::toml_file::{string_list_value, string_value};

/// A change that does more than write `settings.toml`. It is the only field of its patch (R2),
/// because "all or none" then holds without rolling back a registry value or an installed hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Write or remove the autostart entry of Windows.
    StartWithWindows(bool),
    /// Change the data folder: `""` means the default folder.
    DataDir(String),
    /// Install or remove the Claude hooks, then write the key.
    MonitorClaude(bool),
}

/// What a valid patch asks for.
#[derive(Debug, Default)]
pub struct Plan {
    /// The keys to set or remove in `settings.toml`.
    pub edits: Vec<KeyEdit>,
    pub effect: Option<Effect>,
}

impl Plan {
    /// True for `{}`: nothing to do.
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.effect.is_none()
    }
}

/// Checks `patch` and says what it asks for.
pub fn plan(patch: SettingsPatch) -> ApiResult<Plan> {
    let effect = effect_of(&patch);
    if let Some(effect) = effect {
        if has_other_fields(&patch) {
            return Err(refusal("onePerRequest", "Change this setting on its own."));
        }
        if let Effect::DataDir(text) = &effect {
            rules::path_text(text.trim()).map_err(rejected)?;
        }
        return Ok(Plan {
            edits: Vec::new(),
            effect: Some(effect),
        });
    }
    Ok(Plan {
        edits: file_edits(patch)?,
        effect: None,
    })
}

/// The effect that the patch names, when it names one. Two of them together is not a valid patch
/// either; `has_other_fields` finds that, since the second one counts as another field.
fn effect_of(patch: &SettingsPatch) -> Option<Effect> {
    if let Some(on) = patch.start_with_windows {
        return Some(Effect::StartWithWindows(on));
    }
    if let Some(folder) = &patch.data_dir {
        return Some(Effect::DataDir(folder.clone()));
    }
    patch.monitor_claude.map(Effect::MonitorClaude)
}

/// Whether the patch holds a field besides the one effect.
fn has_other_fields(patch: &SettingsPatch) -> bool {
    let effect_fields = SettingsPatch {
        start_with_windows: patch.start_with_windows,
        data_dir: patch.data_dir.clone(),
        monitor_claude: patch.monitor_claude,
        ..SettingsPatch::default()
    };
    let named_effects = [
        patch.start_with_windows.is_some(),
        patch.data_dir.is_some(),
        patch.monitor_claude.is_some(),
    ]
    .iter()
    .filter(|named| **named)
    .count();
    named_effects > 1 || *patch != effect_fields
}

fn file_edits(patch: SettingsPatch) -> ApiResult<Vec<KeyEdit>> {
    let mut edits = Vec::new();
    let mut set = |key: &'static str, value: Option<Value>| edits.push(KeyEdit { key, value });
    if let Some(on) = patch.hide_on_blur {
        set("hide_on_blur", Some(Value::from(on)));
    }
    if let Some(seconds) = patch.reset_after_seconds {
        set("reset_after_seconds", Some(Value::from(seconds as i64)));
    }
    if let Some(text) = patch.vault_path {
        set("vault_path", path_override(&text)?);
    }
    if let Some(text) = patch.daily_note_format {
        set("daily_note_format", note_format_override(&text)?);
    }
    if let Some(text) = patch.daily_note_folder {
        set("daily_note_folder", note_folder_override(&text)?);
    }
    if let Some(roots) = patch.project_roots {
        set("project_roots", Some(folder_list(roots)?));
    }
    if let Some(folders) = patch.game_folders {
        set("game_folders", Some(folder_list(folders)?));
    }
    if let Some(on) = patch.steam_enabled {
        set("steam_enabled", Some(Value::from(on)));
    }
    if let Some(text) = patch.steam_path {
        set("steam_path", path_override(&text)?);
    }
    if let Some(on) = patch.epic_enabled {
        set("epic_enabled", Some(Value::from(on)));
    }
    if let Some(text) = patch.epic_path {
        set("epic_path", path_override(&text)?);
    }
    if let Some(seconds) = patch.game_mode_countdown_seconds {
        set(
            "game_mode_countdown_seconds",
            Some(Value::from(seconds as i64)),
        );
    }
    Ok(edits)
}

fn refusal(reason: &'static str, message: &str) -> ApiError {
    ApiError::invalid(message)
        .with_source(SOURCE)
        .with_reason(reason)
}

fn rejected(reject: Reject) -> ApiError {
    refusal(reject.reason(), reject.message())
}

/// A path override. Empty means "clear the override": the key leaves the file.
fn path_override(text: &str) -> ApiResult<Option<Value>> {
    let text = text.trim();
    rules::path_text(text).map_err(rejected)?;
    Ok((!text.is_empty()).then(|| string_value(text)))
}

fn note_format_override(text: &str) -> ApiResult<Option<Value>> {
    let text = text.trim();
    rules::note_format(text).map_err(rejected)?;
    Ok((!text.is_empty()).then(|| string_value(text)))
}

/// The folder is stored in its clean form: `Daily`, never `\Daily\`.
fn note_folder_override(text: &str) -> ApiResult<Option<Value>> {
    let stored = rules::note_folder(text).map_err(rejected)?;
    Ok((!stored.is_empty()).then(|| string_value(&stored)))
}

/// A list of folders: entries are trimmed, empty ones dropped, and a folder that is already in
/// the list (the same path in another spelling or case) is dropped; the first spelling stays.
fn folder_list(entries: Vec<String>) -> ApiResult<Value> {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    for entry in entries {
        let entry = entry.trim();
        rules::path_text(entry).map_err(rejected)?;
        if !entry.is_empty() && seen.insert(identity(entry)) {
            kept.push(entry.to_owned());
        }
    }
    rules::folder_list(&kept).map_err(rejected)?;
    Ok(string_list_value(&kept))
}

/// What makes two entries the same folder: the normalised path in lower case. An entry that is
/// not a usable path is compared by its own text.
fn identity(entry: &str) -> String {
    match normalize(&expand_env(entry), true) {
        Ok(path) => path.to_string_lossy().to_lowercase(),
        Err(_) => entry.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::{CountdownSeconds, ResetAfterSeconds};

    /// The planned edits as (key, written TOML value or `None` for a removal).
    fn edits(patch: SettingsPatch) -> Vec<(&'static str, Option<String>)> {
        let plan = plan(patch).unwrap();
        assert_eq!(plan.effect, None);
        plan.edits
            .into_iter()
            .map(|edit| (edit.key, edit.value.map(|v| v.to_string())))
            .collect()
    }

    fn refused(patch: SettingsPatch) -> ApiError {
        plan(patch).expect_err("the patch is refused")
    }

    fn reason(patch: SettingsPatch) -> String {
        refused(patch).body.reason.expect("a reason")
    }

    #[test]
    fn an_empty_patch_asks_for_nothing() {
        assert!(plan(SettingsPatch::default()).unwrap().is_empty());
    }

    #[test]
    fn each_field_becomes_the_toml_key_of_section_06() {
        let planned = edits(SettingsPatch {
            hide_on_blur: Some(false),
            reset_after_seconds: Some(ResetAfterSeconds::Seconds300),
            vault_path: Some(r"D:\Vault".into()),
            daily_note_format: Some("YYYY-MM-DD".into()),
            daily_note_folder: Some("/Journal/".into()),
            project_roots: Some(vec![r"D:\Code".into()]),
            game_folders: Some(vec![r"D:\Games".into()]),
            steam_enabled: Some(false),
            steam_path: Some(r"D:\Steam".into()),
            epic_enabled: Some(true),
            epic_path: Some(r"D:\Epic".into()),
            game_mode_countdown_seconds: Some(CountdownSeconds::Seconds20),
            ..SettingsPatch::default()
        });
        let text = |key: &str| {
            planned
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(text("hide_on_blur"), "false");
        assert_eq!(text("reset_after_seconds"), "300");
        assert_eq!(text("vault_path"), r"'D:\Vault'");
        assert_eq!(text("daily_note_format"), "'YYYY-MM-DD'");
        assert_eq!(text("daily_note_folder"), "'Journal'");
        assert_eq!(text("project_roots"), r"['D:\Code']");
        assert_eq!(text("game_folders"), r"['D:\Games']");
        assert_eq!(text("steam_enabled"), "false");
        assert_eq!(text("steam_path"), r"'D:\Steam'");
        assert_eq!(text("epic_enabled"), "true");
        assert_eq!(text("epic_path"), r"'D:\Epic'");
        assert_eq!(text("game_mode_countdown_seconds"), "20");
        assert_eq!(planned.len(), 12);
    }

    #[test]
    fn an_empty_text_clears_the_override() {
        let planned = edits(SettingsPatch {
            vault_path: Some("  ".into()),
            daily_note_format: Some(String::new()),
            daily_note_folder: Some("/".into()),
            steam_path: Some(String::new()),
            epic_path: Some(String::new()),
            ..SettingsPatch::default()
        });
        assert_eq!(planned.len(), 5);
        assert!(planned.iter().all(|(_, value)| value.is_none()));
    }

    #[test]
    fn a_text_is_trimmed_before_it_is_stored() {
        let planned = edits(SettingsPatch {
            vault_path: Some("  D:\\Vault \n".into()),
            ..SettingsPatch::default()
        });
        assert_eq!(planned[0].1.as_deref(), Some(r"'D:\Vault'"));
    }

    #[test]
    fn the_folder_lists_drop_empties_and_duplicates_and_keep_the_first_spelling() {
        let planned = edits(SettingsPatch {
            project_roots: Some(vec![
                "D:/Code/".into(),
                "  ".into(),
                r"d:\code".into(),
                r"D:\Work".into(),
            ]),
            game_folders: Some(vec![r"D:\Games".into(), "d:/games/".into()]),
            ..SettingsPatch::default()
        });
        assert_eq!(planned[0].1.as_deref(), Some(r"['D:/Code/', 'D:\Work']"));
        assert_eq!(planned[1].1.as_deref(), Some(r"['D:\Games']"));
    }

    #[test]
    fn two_entries_that_are_not_paths_are_the_same_only_when_their_text_is() {
        let planned = edits(SettingsPatch {
            project_roots: Some(vec!["Games".into(), "games".into(), "Other".into()]),
            ..SettingsPatch::default()
        });
        assert_eq!(planned[0].1.as_deref(), Some("['Games', 'Other']"));
    }

    #[test]
    fn an_empty_folder_list_is_written_as_an_empty_list_not_removed() {
        let planned = edits(SettingsPatch {
            game_folders: Some(vec![]),
            ..SettingsPatch::default()
        });
        assert_eq!(planned[0].1.as_deref(), Some("[]"));
    }

    #[test]
    fn the_reasons_of_section_06_table_8_1() {
        let long = "x".repeat(1025);
        let cases: Vec<(SettingsPatch, &str)> = vec![
            (
                SettingsPatch {
                    vault_path: Some(long.clone()),
                    ..SettingsPatch::default()
                },
                "tooLong",
            ),
            (
                SettingsPatch {
                    steam_path: Some("a\nb".into()),
                    ..SettingsPatch::default()
                },
                "badText",
            ),
            (
                SettingsPatch {
                    project_roots: Some(vec![long.clone()]),
                    ..SettingsPatch::default()
                },
                "tooLong",
            ),
            (
                SettingsPatch {
                    game_folders: Some((0..33).map(|n| format!(r"D:\Games{n}")).collect()),
                    ..SettingsPatch::default()
                },
                "tooMany",
            ),
            (
                SettingsPatch {
                    daily_note_format: Some("a:b".into()),
                    ..SettingsPatch::default()
                },
                "badFormat",
            ),
            (
                SettingsPatch {
                    daily_note_folder: Some(r"C:\Notes".into()),
                    ..SettingsPatch::default()
                },
                "badFolder",
            ),
            (
                SettingsPatch {
                    daily_note_folder: Some("../outside".into()),
                    ..SettingsPatch::default()
                },
                "badFolder",
            ),
            (
                SettingsPatch {
                    data_dir: Some(long),
                    ..SettingsPatch::default()
                },
                "tooLong",
            ),
        ];
        for (patch, expected) in cases {
            assert_eq!(reason(patch.clone()), expected, "{patch:?}");
        }
    }

    #[test]
    fn duplicates_do_not_count_towards_the_limit_of_32_folders() {
        let roots: Vec<String> = (0..40).map(|n| format!(r"D:\Code{}", n % 32)).collect();
        assert!(
            plan(SettingsPatch {
                project_roots: Some(roots),
                ..SettingsPatch::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn an_external_effect_field_alone_becomes_an_effect_and_no_edits() {
        let cases = [
            (
                SettingsPatch {
                    start_with_windows: Some(true),
                    ..SettingsPatch::default()
                },
                Effect::StartWithWindows(true),
            ),
            (
                SettingsPatch {
                    data_dir: Some(r"D:\RainData".into()),
                    ..SettingsPatch::default()
                },
                Effect::DataDir(r"D:\RainData".into()),
            ),
            (
                SettingsPatch {
                    monitor_claude: Some(false),
                    ..SettingsPatch::default()
                },
                Effect::MonitorClaude(false),
            ),
        ];
        for (patch, expected) in cases {
            let planned = plan(patch).unwrap();
            assert_eq!(planned.effect, Some(expected));
            assert!(planned.edits.is_empty());
        }
    }

    #[test]
    fn an_external_effect_field_must_be_alone() {
        let with_another_setting = SettingsPatch {
            start_with_windows: Some(true),
            hide_on_blur: Some(false),
            ..SettingsPatch::default()
        };
        let two_effects = SettingsPatch {
            data_dir: Some(String::new()),
            monitor_claude: Some(true),
            ..SettingsPatch::default()
        };
        for patch in [with_another_setting, two_effects] {
            let error = refused(patch);
            assert_eq!(error.status, axum::http::StatusCode::BAD_REQUEST);
            assert_eq!(error.body.reason.as_deref(), Some("onePerRequest"));
        }
    }
}
