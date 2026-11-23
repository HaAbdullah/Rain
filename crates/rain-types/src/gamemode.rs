//! Game mode: the state of the Game mode screen, and what it did last time.
//! Feature section 06 3.2.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::common::IconRef;

/// Where game mode is. `off` is also the state when it never ran.
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GameModePhase {
    Off,
    Review,
    Countdown,
    WindingDown,
    Done,
}

/// What the user can answer on the Game mode screen.
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GameModeChoice {
    Wait,
    StopAnyway,
    CloseNow,
    Cancel,
}

/// What a row of the checklist stands for. `idleJob` on the wire.
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindDownKind {
    Session,
    Job,
    IdleJob,
    Service,
    Window,
}

/// What stopping the row does: nothing bad (`safe`), loses unfinished work (`interrupts`), or
/// nothing at all because the thing is never stopped (`leftRunning`, the user's Claude sessions).
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindDownEffect {
    Safe,
    Interrupts,
    LeftRunning,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindDownStatus {
    Pending,
    Stopping,
    Stopped,
}

/// One row of the checklist.
#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindDownItem {
    pub id: String,
    pub label: String,
    pub icon: IconRef,
    #[ts(inline)]
    pub kind: WindDownKind,
    pub count: u32,
    #[ts(inline)]
    pub effect: WindDownEffect,
    #[ts(inline)]
    pub status: WindDownStatus,
}

/// What happened to one kind of thing in the last game mode.
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecapOutcome {
    Stopped,
    Forced,
    Failed,
    LeftRunning,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecapItem {
    #[ts(inline)]
    pub kind: WindDownKind,
    pub label: String,
    pub count: u32,
    #[ts(inline)]
    pub outcome: RecapOutcome,
    // At most 5 names of jobs; empty for sessions (a title can hold prompt text).
    pub names: Vec<String>,
}

/// What the last game mode did. Shown once, on the first load afterwards.
#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GameModeRecap {
    // ISODateTime: when the wind-down ended (or began, if it never ended).
    pub at: String,
    // False: the backend died during the wind-down.
    pub complete: bool,
    pub items: Vec<RecapItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_error: Option<String>,
    // Only ever `true`: the game started, but inside a job of Rain, so it may close together
    // with Rain ("The game may close with Rain").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched_in_job: Option<bool>,
    pub seen: bool,
}

/// Everything the Game mode screen draws. Every frame of the topic `gameMode` is a whole one.
#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GameModeState {
    #[ts(inline)]
    pub phase: GameModePhase,
    pub waiting: bool,
    pub seconds_left: u32,
    pub countdown_seconds: u32,
    pub items: Vec<WindDownItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then_launch_game_id: Option<String>,
    // Only ever `true`: "Wait" ended after 3 minutes with work still running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_timed_out: Option<bool>,
    // The game that should start after the wind-down could not be started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_error: Option<String>,
    // Only in phase `off`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<GameModeRecap>,
    // Grows with every change within one backend process, so a client can drop a late state.
    pub rev: u64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct StartGameModeBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then_launch_game_id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameModeChoiceBody {
    #[ts(inline)]
    pub choice: GameModeChoice,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::IconKind;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn item() -> WindDownItem {
        WindDownItem {
            id: "wind-jobs".into(),
            label: "Scheduled jobs".into(),
            icon: IconRef {
                kind: IconKind::Lucide,
                value: "calendar-clock".into(),
                tint: None,
                fallback: None,
            },
            kind: WindDownKind::IdleJob,
            count: 2,
            effect: WindDownEffect::LeftRunning,
            status: WindDownStatus::Pending,
        }
    }

    fn minimal() -> GameModeState {
        GameModeState {
            phase: GameModePhase::Off,
            waiting: false,
            seconds_left: 0,
            countdown_seconds: 10,
            items: vec![item()],
            then_launch_game_id: None,
            wait_timed_out: None,
            launch_error: None,
            last: None,
            rev: 1,
        }
    }

    #[test]
    fn a_minimal_state_has_no_optional_keys_and_uses_the_wire_names() {
        let json = round_trip(&minimal());
        assert_eq!(json["phase"], "off");
        assert_eq!(json["items"][0]["kind"], "idleJob");
        assert_eq!(json["items"][0]["effect"], "leftRunning");
        assert!(json.get("last").is_none());
        assert!(json.get("waitTimedOut").is_none());
    }

    #[test]
    fn a_full_state_with_a_recap_round_trips() {
        let mut state = minimal();
        state.phase = GameModePhase::WindingDown;
        state.then_launch_game_id = Some("game-a".into());
        state.wait_timed_out = Some(true);
        state.launch_error = Some("It did not start.".into());
        state.last = Some(GameModeRecap {
            at: "2026-11-22T10:00:00.000Z".into(),
            complete: true,
            items: vec![RecapItem {
                kind: WindDownKind::Job,
                label: "Scheduled jobs".into(),
                count: 1,
                outcome: RecapOutcome::Forced,
                names: vec!["Demo wait".into()],
            }],
            launched_title: Some("Game A".into()),
            launch_error: None,
            launched_in_job: Some(true),
            seen: false,
        });
        let json = round_trip(&state);
        assert_eq!(json["phase"], "windingDown");
        assert_eq!(json["last"]["items"][0]["outcome"], "forced");
        assert_eq!(json["last"]["launchedInJob"], true);
    }

    #[test]
    fn rev_near_two_to_the_53_is_a_plain_number() {
        let mut state = minimal();
        state.rev = (1 << 53) - 1;
        let json = round_trip(&state);
        assert!(json["rev"].is_u64());
    }

    #[test]
    fn the_bodies_reject_unknown_fields_and_unknown_choices() {
        assert!(serde_json::from_value::<StartGameModeBody>(json!({ "nope": 1 })).is_err());
        assert!(
            serde_json::from_value::<StartGameModeBody>(json!({ "thenLaunchGameId": "a" })).is_ok()
        );
        assert!(
            serde_json::from_value::<GameModeChoiceBody>(json!({ "choice": "later" })).is_err()
        );
        let body: GameModeChoiceBody =
            serde_json::from_value(json!({ "choice": "stopAnyway" })).unwrap();
        assert_eq!(body.choice, GameModeChoice::StopAnyway);
    }
}
