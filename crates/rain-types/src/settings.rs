//! `Settings` (what `GET /settings` returns) and `SettingsPatch` (the body of `PATCH /settings`).

use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use ts_rs::TS;

/// The chord the shell registers. It is a fact, not a choice: `Settings.hotkey` always holds it.
pub const SUMMON_HOTKEY: &str = "Ctrl+Backquote";

// Number unions. `serde_repr` writes the enum as its number and REJECTS every other number when
// reading, so the backend validates a request body without any extra code. The `ts(type)` line
// gives TypeScript the exact union instead of `number` (foundation section 9.2).

/// Seconds without input before Rain returns to Home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, TS)]
#[repr(u32)]
#[ts(type = "30 | 60 | 300")]
pub enum ResetAfterSeconds {
    Seconds30 = 30,
    Seconds60 = 60,
    Seconds300 = 300,
}

/// Seconds of the game mode countdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, TS)]
#[repr(u32)]
#[ts(type = "5 | 10 | 20 | 30")]
pub enum CountdownSeconds {
    Seconds5 = 5,
    Seconds10 = 10,
    Seconds20 = 20,
    Seconds30 = 30,
}

/// State of the switch "Monitor Claude sessions". Computed by the sessions module (section 03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MonitorClaudeState {
    /// The switch is off and nothing of Rain is in the Claude settings.
    Off,
    /// On, the entries and the forwarder are present, and an event arrived in this backend run.
    On,
    /// On, the entries and the forwarder are present, but no event has arrived yet.
    OnInactive,
    /// On, but an entry or the forwarder is missing or changed, or the file cannot be read.
    NeedsRepair,
}

/// Every setting the Settings screen shows. Section 06 3.1 says where each value comes from.
///
/// `hotkey`, `hotkey_available`, `calendar_connected` and `monitor_claude_state` are facts, not
/// choices: they cannot be changed through `SettingsPatch`. For `data_dir`, `vault_path`,
/// `daily_note_format`, `daily_note_folder`, `steam_path` and `epic_path` the value is the
/// effective one (the override, else what Rain detected, else the default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,
    pub hotkey_available: bool,
    pub hide_on_blur: bool,
    pub reset_after_seconds: ResetAfterSeconds,
    pub start_with_windows: bool,
    pub data_dir: String,
    pub vault_path: String,
    pub daily_note_format: String,
    pub daily_note_folder: String,
    pub project_roots: Vec<String>,
    pub game_folders: Vec<String>,
    pub steam_enabled: bool,
    pub steam_path: String,
    pub epic_enabled: bool,
    pub epic_path: String,
    pub calendar_connected: bool,
    pub monitor_claude: bool,
    #[ts(inline)]
    pub monitor_claude_state: MonitorClaudeState,
    pub game_mode_countdown_seconds: CountdownSeconds,
}

/// Body of `PATCH /settings`: the settings the user may change. An absent field means
/// "unchanged". A field that is not listed here (including the read-only facts of `Settings`)
/// is rejected, because unknown fields are denied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct SettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_on_blur: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<ResetAfterSeconds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_with_windows: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_note_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_note_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_roots: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_folders: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_claude: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_mode_countdown_seconds: Option<CountdownSeconds>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::{Value, json};

    fn sample_settings_json() -> Value {
        json!({
            "hotkey": "Ctrl+`",
            "hotkeyAvailable": true,
            "hideOnBlur": true,
            "resetAfterSeconds": 60,
            "startWithWindows": false,
            "dataDir": "C:\\RainData",
            "vaultPath": "",
            "dailyNoteFormat": "YYYY-MM-DD",
            "dailyNoteFolder": "",
            "projectRoots": ["C:\\Projects"],
            "gameFolders": [],
            "steamEnabled": true,
            "steamPath": "",
            "epicEnabled": false,
            "epicPath": "",
            "calendarConnected": false,
            "monitorClaude": false,
            "monitorClaudeState": "off",
            "gameModeCountdownSeconds": 10
        })
    }

    #[test]
    fn settings_use_camel_case_keys_and_numbers_for_the_unions() {
        let settings: Settings = serde_json::from_value(sample_settings_json()).unwrap();
        assert_eq!(settings.reset_after_seconds, ResetAfterSeconds::Seconds60);
        assert_eq!(
            settings.game_mode_countdown_seconds,
            CountdownSeconds::Seconds10
        );
        assert_eq!(round_trip(&settings), sample_settings_json());
    }

    #[test]
    fn reset_after_seconds_accepts_only_30_60_300() {
        for allowed in [30, 60, 300] {
            let value: ResetAfterSeconds = serde_json::from_value(json!(allowed)).unwrap();
            assert_eq!(serde_json::to_value(value).unwrap(), json!(allowed));
        }
        for refused in [0, 45, 59, 61, 301, -60] {
            assert!(
                serde_json::from_value::<ResetAfterSeconds>(json!(refused)).is_err(),
                "{refused} must be rejected"
            );
        }
        assert!(serde_json::from_value::<ResetAfterSeconds>(json!("60")).is_err());
    }

    #[test]
    fn countdown_seconds_accepts_only_5_10_20_30() {
        for allowed in [5, 10, 20, 30] {
            let value: CountdownSeconds = serde_json::from_value(json!(allowed)).unwrap();
            assert_eq!(serde_json::to_value(value).unwrap(), json!(allowed));
        }
        for refused in [0, 1, 15, 25, 60] {
            assert!(
                serde_json::from_value::<CountdownSeconds>(json!(refused)).is_err(),
                "{refused} must be rejected"
            );
        }
    }

    #[test]
    fn an_empty_patch_serialises_to_an_empty_object() {
        assert_eq!(round_trip(&SettingsPatch::default()), json!({}));
    }

    #[test]
    fn a_patch_keeps_only_the_fields_that_were_set() {
        let patch = SettingsPatch {
            hide_on_blur: Some(false),
            reset_after_seconds: Some(ResetAfterSeconds::Seconds300),
            project_roots: Some(vec![]),
            ..SettingsPatch::default()
        };
        assert_eq!(
            round_trip(&patch),
            json!({ "hideOnBlur": false, "resetAfterSeconds": 300, "projectRoots": [] })
        );
    }

    #[test]
    fn a_patch_with_a_number_outside_the_union_is_rejected() {
        let body = json!({ "resetAfterSeconds": 45 });
        assert!(serde_json::from_value::<SettingsPatch>(body).is_err());
        let body = json!({ "gameModeCountdownSeconds": 15 });
        assert!(serde_json::from_value::<SettingsPatch>(body).is_err());
    }

    #[test]
    fn a_patch_rejects_read_only_and_unknown_fields() {
        for body in [
            json!({ "hotkey": "Ctrl+K" }),
            json!({ "hotkeyAvailable": false }),
            json!({ "calendarConnected": true }),
            json!({ "monitorClaudeState": "on" }),
            json!({ "noSuchSetting": 1 }),
        ] {
            assert!(
                serde_json::from_value::<SettingsPatch>(body.clone()).is_err(),
                "{body} must be rejected"
            );
        }
    }

    #[test]
    fn monitor_claude_state_uses_camel_case_names() {
        let names: Vec<_> = [
            MonitorClaudeState::Off,
            MonitorClaudeState::On,
            MonitorClaudeState::OnInactive,
            MonitorClaudeState::NeedsRepair,
        ]
        .iter()
        .map(|state| serde_json::to_value(state).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                json!("off"),
                json!("on"),
                json!("onInactive"),
                json!("needsRepair")
            ]
        );
    }

    #[test]
    fn a_patch_names_the_new_writable_fields() {
        let patch = SettingsPatch {
            data_dir: Some(String::new()),
            daily_note_folder: Some("Daily".into()),
            game_folders: Some(vec!["C:\\Games".into()]),
            monitor_claude: Some(true),
            ..SettingsPatch::default()
        };
        assert_eq!(
            round_trip(&patch),
            json!({
                "dataDir": "", "dailyNoteFolder": "Daily",
                "gameFolders": ["C:\\Games"], "monitorClaude": true
            })
        );
    }

    #[test]
    fn a_null_in_a_patch_means_unchanged() {
        // A client should leave the key out; serde reads `null` into an `Option` as `None`.
        let patch: SettingsPatch = serde_json::from_value(json!({ "hideOnBlur": null })).unwrap();
        assert_eq!(patch, SettingsPatch::default());
    }
}
