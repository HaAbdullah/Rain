//! Answers about the backend itself: `Ping`, `Health`, and what the shell tells the backend.
//! Foundation sections 6.5 and 6.9.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::StopReason;

/// Answer of `GET /ping`: cheap, no side effects, answered even while shutting down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Ping {
    // Always `rain-backend`. Tells Rain's backend from any other program on the pipe name.
    pub app: String,
    pub version: String,
    pub api_version: u32,
    // Unique for every start of the backend process.
    pub start_id: String,
    pub pid: u32,
    // Full path of the running backend program; the shell compares it for the upgrade rule.
    pub exe_path: String,
    // True when this run must not touch real user data (foundation section 2.7).
    pub sandbox: bool,
    // True while a job, scan or launch is running; an upgrade must wait then.
    pub busy: bool,
}

/// A data file that could not be read (a syntax error in a TOML file, for example).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ConfigIssue {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub message: String,
}

/// Whether a data source works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SourceState {
    Ok,
    /// Not in use, and that is fine (Steam is not installed, for example).
    Off,
    Failed,
}

/// The state of one data source, as a module reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct SourceStatus {
    pub name: String,
    #[ts(inline)]
    pub state: SourceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// What the backend knows about the shell, inside `Health`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ShellStatus {
    // True once a shell has sent its facts (`PUT /internal/shell`).
    pub reported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey_registered: Option<bool>,
}

/// Answer of `GET /health` and payload of the topic `health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub start_id: String,
    pub version: String,
    pub shell: ShellStatus,
    pub config_issues: Vec<ConfigIssue>,
    pub sources: Vec<SourceStatus>,
}

/// Body of `PUT /internal/shell`: what only the shell knows. The backend keeps it in memory.
/// Not part of the frontend contract, so it is not generated to TypeScript.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellFacts {
    pub hotkey_registered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey_error: Option<String>,
    /// Full path of the shell program. The "Start with Windows" entry points at it.
    pub shell_exe: String,
    pub shell_version: String,
    pub window_visible: bool,
}

/// Body of `POST /internal/shutdown`. Not part of the frontend contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShutdownBody {
    pub reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    #[test]
    fn ping_matches_the_documented_answer() {
        let ping = Ping {
            app: "rain-backend".into(),
            version: "0.1.0".into(),
            api_version: 1,
            start_id: "19a3f1c2-1f4c".into(),
            pid: 12345,
            exe_path: "C:\\Rain\\rain-backend.exe".into(),
            sandbox: true,
            busy: false,
        };
        assert_eq!(
            round_trip(&ping),
            json!({
                "app": "rain-backend", "version": "0.1.0", "apiVersion": 1,
                "startId": "19a3f1c2-1f4c", "pid": 12345,
                "exePath": "C:\\Rain\\rain-backend.exe", "sandbox": true, "busy": false
            })
        );
    }

    #[test]
    fn minimal_health_has_empty_lists_and_no_optional_keys() {
        let health = Health {
            start_id: "a".into(),
            version: "0.1.0".into(),
            shell: ShellStatus {
                reported: false,
                hotkey_registered: None,
            },
            config_issues: vec![],
            sources: vec![],
        };
        assert_eq!(
            round_trip(&health),
            json!({
                "startId": "a", "version": "0.1.0", "shell": { "reported": false },
                "configIssues": [], "sources": []
            })
        );
    }

    #[test]
    fn full_health_with_every_optional_field() {
        let health = Health {
            start_id: "a".into(),
            version: "0.1.0".into(),
            shell: ShellStatus {
                reported: true,
                hotkey_registered: Some(false),
            },
            config_issues: vec![ConfigIssue {
                file: "settings.toml".into(),
                line: Some(3),
                message: "expected a value".into(),
            }],
            sources: vec![
                SourceStatus {
                    name: "steam".into(),
                    state: SourceState::Off,
                    message: Some("not installed".into()),
                },
                SourceStatus {
                    name: "vault".into(),
                    state: SourceState::Ok,
                    message: None,
                },
            ],
        };
        assert_eq!(
            round_trip(&health),
            json!({
                "startId": "a", "version": "0.1.0",
                "shell": { "reported": true, "hotkeyRegistered": false },
                "configIssues": [{ "file": "settings.toml", "line": 3, "message": "expected a value" }],
                "sources": [
                    { "name": "steam", "state": "off", "message": "not installed" },
                    { "name": "vault", "state": "ok" }
                ]
            })
        );
    }

    #[test]
    fn source_state_rejects_an_unknown_value() {
        assert!(serde_json::from_value::<SourceState>(json!("broken")).is_err());
    }

    #[test]
    fn shell_facts_read_the_documented_body() {
        let body = json!({
            "hotkeyRegistered": true, "shellExe": "C:\\Rain\\rain.exe",
            "shellVersion": "0.1.0", "windowVisible": false
        });
        let facts: ShellFacts = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(facts.hotkey_error, None);
        assert_eq!(round_trip(&facts), body);
    }

    #[test]
    fn shell_facts_reject_an_unknown_field() {
        let body = json!({
            "hotkeyRegistered": true, "shellExe": "x", "shellVersion": "1",
            "windowVisible": false, "extra": 1
        });
        assert!(serde_json::from_value::<ShellFacts>(body).is_err());
    }

    #[test]
    fn shutdown_body_reads_a_reason_with_and_without_force() {
        let plain: ShutdownBody = serde_json::from_value(json!({ "reason": "stop" })).unwrap();
        assert_eq!(
            plain,
            ShutdownBody {
                reason: StopReason::Stop,
                force: None
            }
        );
        let forced: ShutdownBody =
            serde_json::from_value(json!({ "reason": "upgrade", "force": true })).unwrap();
        assert_eq!(forced.force, Some(true));
    }

    #[test]
    fn shutdown_body_rejects_an_unknown_reason_and_an_unknown_field() {
        assert!(serde_json::from_value::<ShutdownBody>(json!({ "reason": "nap" })).is_err());
        assert!(
            serde_json::from_value::<ShutdownBody>(json!({ "reason": "stop", "now": true }))
                .is_err()
        );
    }
}
