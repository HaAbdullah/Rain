//! What a source of games hands to the library: plain data, no I/O. Section 02, 6.8.

use std::path::PathBuf;

use rain_types::{GameSource, SourceState};

/// How a game is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// `steam://rungameid/<appid>`.
    Steam { appid: String },
    /// `com.epicgames.launcher://apps/<AppName>?action=launch&silent=true`.
    Epic { app_name: String },
    /// A direct start of a program file (folder games and hand-added games).
    Local,
}

/// One game as a source found it, before the user's corrections and the live facts (running,
/// pinned, play time of Rain's own record) are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The stable text that `games.toml` and SQLite use: `steam:<appid>`, `epic:<AppName>`,
    /// `dir:<folder>`, `custom:<id>`.
    pub key: String,
    pub id: String,
    pub title: String,
    pub source: GameSource,
    pub launch: Launch,
    /// The folder of the game, when there is one.
    pub install: Option<PathBuf>,
    /// A size the source knows by itself (Steam's `SizeOnDisk`, Epic's `InstallSize`).
    pub size_bytes: Option<u64>,
    /// A local image file that can serve as the cover.
    pub cover: Option<PathBuf>,
    /// Steam's own play time and last played, in whole minutes and Unix seconds.
    pub playtime_minutes: u32,
    pub last_played_unix: Option<i64>,
    /// Hidden by the user (a correction in `games.toml`).
    pub hidden: bool,
    /// Changes when the cover file changes; part of the cover URL.
    pub cover_version: Option<String>,
    /// Only for local games.
    pub local: Option<LocalInfo>,
}

/// What only a local game (a folder game or a hand-added one) has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    /// The program file in use, `None` when there is none yet.
    pub exe: Option<PathBuf>,
    /// True while `exe` is Rain's guess and not the user's choice.
    pub exe_guessed: bool,
    /// Extra command-line text (a hand edit of `games.toml`).
    pub args: Option<String>,
    /// True while the program file exists (or there is none to miss).
    pub installed: bool,
    /// True for a game added by hand.
    pub removable: bool,
}

/// What one source says: its state, a sentence for the user, and its games.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceResult {
    pub state: SourceState,
    pub message: Option<String>,
    pub items: Vec<Found>,
}

impl SourceResult {
    pub fn off(message: &str) -> Self {
        Self {
            state: SourceState::Off,
            message: Some(message.to_owned()),
            items: Vec::new(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            state: SourceState::Failed,
            message: Some(message.into()),
            items: Vec::new(),
        }
    }

    pub fn ok(items: Vec<Found>, message: Option<String>) -> Self {
        Self {
            state: SourceState::Ok,
            message,
            items,
        }
    }
}

/// A local image of a game and what the media route needs to know about it.
pub fn is_image_extension(path: &std::path::Path) -> bool {
    image_content_type(path).is_some()
}

/// The `Content-Type` for a cover file, from its extension.
pub fn image_content_type(path: &std::path::Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}
