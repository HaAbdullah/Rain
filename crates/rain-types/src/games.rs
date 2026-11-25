//! The game library: what `GET /games` returns and what the user sends to change a game.
//! Feature section 02, section 3.2.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::system::SourceState;

/// Where a game comes from. `custom` is every local game: a folder found in one of the game
/// folders of the settings, and a game the user added by hand. Both share one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum GameSource {
    Steam,
    Epic,
    Riot,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Game {
    pub id: String,
    pub title: String,
    #[ts(inline)]
    pub source: GameSource,
    // "media/game-cover/<id>?v=<hex>"; absent = no art, the UI draws the title tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    // Steam, Epic and Riot: always true (only installed games are listed). Local game: true while its
    // program file exists; a folder game without any program file is true too.
    pub installed: bool,
    pub running: bool,
    // Logical size of the install folder. Absent until measured, or when there is no folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub playtime_minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played_at: Option<String>,
    pub pinned: bool,
    pub hidden: bool,
    // Local games only. Absent = no program file found or chosen yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe_path: Option<String>,
    // Local games only: true while `exePath` is Rain's guess and not the user's choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe_guessed: Option<bool>,
    // The folder of the game, when Rain knows it (every source except a hand-added game).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    // true only for a hand-added game (the only kind "Remove" works for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removable: Option<bool>,
}

/// The state of one source of games. `off` is not an error (Epic is not installed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GameSourceStatus {
    #[ts(inline)]
    pub source: GameSource,
    #[ts(inline)]
    pub state: SourceState,
    // One plain sentence. Shown as it is for `failed`; kept for `off` and `ok` as a hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The whole library. `sources` always has four entries: steam, epic, riot, custom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameLibrary {
    pub games: Vec<Game>,
    pub sources: Vec<GameSourceStatus>,
}

/// A game the user adds by hand (`POST /games`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct CustomGameDraft {
    pub title: String,
    pub exe_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_path: Option<String>,
}

/// Every field is optional; an absent field means "unchanged". An empty string clears:
/// `title: ""` = use the folder name again, `exePath: ""` = go back to the guess,
/// `coverPath: ""` = remove the cover.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct GamePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn full() -> Game {
        Game {
            id: "game-steam-1".into(),
            title: "Fixture Quest".into(),
            source: GameSource::Steam,
            cover_url: Some("media/game-cover/game-steam-1?v=ab".into()),
            installed: true,
            running: false,
            size_bytes: Some(1024),
            playtime_minutes: 61,
            last_played_at: Some("2026-11-21T09:30:00.000Z".into()),
            pinned: false,
            hidden: true,
            exe_path: Some("D:\\Games\\Fixture\\run.exe".into()),
            exe_guessed: Some(true),
            install_path: Some("D:\\Games\\Fixture".into()),
            removable: Some(false),
        }
    }

    fn minimal() -> Game {
        Game {
            id: "game-x".into(),
            title: "X".into(),
            source: GameSource::Custom,
            cover_url: None,
            installed: true,
            running: false,
            size_bytes: None,
            playtime_minutes: 0,
            last_played_at: None,
            pinned: false,
            hidden: false,
            exe_path: None,
            exe_guessed: None,
            install_path: None,
            removable: None,
        }
    }

    #[test]
    fn a_game_round_trips_with_every_field_and_with_none_of_the_optional_ones() {
        let json = round_trip(&full());
        assert_eq!(json["coverUrl"], "media/game-cover/game-steam-1?v=ab");
        assert_eq!(json["playtimeMinutes"], 61);
        let json = round_trip(&minimal());
        assert!(json.get("coverUrl").is_none() && json.get("exePath").is_none());
    }

    #[test]
    fn the_library_and_its_sources_round_trip() {
        let library = GameLibrary {
            games: vec![full(), minimal()],
            sources: vec![GameSourceStatus {
                source: GameSource::Epic,
                state: SourceState::Off,
                message: Some("Epic Games is not installed".into()),
            }],
        };
        let json = round_trip(&library);
        assert_eq!(json["sources"][0]["state"], "off");
    }

    #[test]
    fn a_patch_and_a_draft_refuse_unknown_fields() {
        assert!(serde_json::from_value::<GamePatch>(json!({ "hidden": true, "x": 1 })).is_err());
        assert!(
            serde_json::from_value::<CustomGameDraft>(
                json!({ "title": "a", "exePath": "b", "x": 1 })
            )
            .is_err()
        );
        let patch: GamePatch = serde_json::from_value(json!({ "hidden": false })).unwrap();
        assert_eq!(patch.hidden, Some(false));
        assert_eq!(patch.title, None);
    }

    #[test]
    fn a_library_of_400_games_fits_in_a_frame() {
        let games: Vec<Game> = (0..400)
            .map(|n| Game {
                id: format!("game-steam-{n}"),
                title: format!("A game with a longish title number {n}"),
                cover_url: Some(format!(
                    "media/game-cover/game-steam-{n}?v=0123456789abcdef"
                )),
                ..full()
            })
            .collect();
        let bytes = serde_json::to_vec(&GameLibrary {
            games,
            sources: Vec::new(),
        })
        .unwrap();
        assert!(bytes.len() < 256 * 1024, "{} bytes", bytes.len());
    }
}
