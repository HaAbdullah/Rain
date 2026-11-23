//! The SQLite tables of this module (section 02, 5.2). Every function is one call to the
//! database thread. Timestamps are `YYYY-MM-DDTHH:MM:SS.mmmZ` text.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use crate::core::error::ApiError;
use crate::store::db::Db;
use crate::store::migrations::Migration;

pub const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "create_games",
    sql: include_str!("migrations/0001_games.sql"),
}];

fn failed(error: crate::store::db::DbError, doing: &str) -> ApiError {
    error.into_api(doing).with_source("games")
}

// ---- play time ----

/// What Rain recorded about launching one game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Play {
    pub last_played_at: Option<String>,
    pub play_seconds: u64,
    pub launches: u32,
}

pub async fn load_plays(db: &Db) -> Result<HashMap<String, Play>, ApiError> {
    db.call(|conn| {
        let mut statement = conn
            .prepare("SELECT game_key, last_played_at, play_seconds, launches FROM games_play")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Play {
                    last_played_at: row.get(1)?,
                    play_seconds: row.get::<_, i64>(2)?.max(0) as u64,
                    launches: row.get::<_, i64>(3)?.max(0) as u32,
                },
            ))
        })?;
        rows.collect()
    })
    .await
    .map_err(|e| failed(e, "Could not read the play history"))
}

/// A launch: `last_played_at = now`, one more launch.
pub async fn record_launch(db: &Db, key: String, now: String) -> Result<(), ApiError> {
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO games_play (game_key, last_played_at, play_seconds, launches)
             VALUES (?1, ?2, 0, 1)
             ON CONFLICT(game_key) DO UPDATE SET last_played_at = ?2, launches = launches + 1",
            params![key, now],
        )
        .map(|_| ())
    })
    .await
    .map_err(|e| failed(e, "Could not record the launch"))
}

pub async fn add_play_seconds(db: &Db, key: String, seconds: u64) -> Result<(), ApiError> {
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO games_play (game_key, play_seconds) VALUES (?1, ?2)
             ON CONFLICT(game_key) DO UPDATE SET play_seconds = play_seconds + ?2",
            params![key, seconds as i64],
        )
        .map(|_| ())
    })
    .await
    .map_err(|e| failed(e, "Could not record the play time"))
}

// ---- sessions ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: i64,
    pub game_key: String,
    pub pid: u32,
    pub process_start: u64,
    pub folder: Option<String>,
    pub started_at: String,
    pub last_seen_at: String,
}

pub async fn insert_session(
    db: &Db,
    key: String,
    pid: u32,
    process_start: u64,
    folder: Option<String>,
    now: String,
) -> Result<i64, ApiError> {
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO games_session (game_key, pid, process_start, folder, started_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![key, i64::from(pid), process_start as i64, folder, now],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .await
    .map_err(|e| failed(e, "Could not record the running game"))
}

pub async fn delete_session(db: &Db, id: i64) -> Result<(), ApiError> {
    db.call(move |conn| {
        conn.execute("DELETE FROM games_session WHERE id = ?1", params![id])
            .map(|_| ())
    })
    .await
    .map_err(|e| failed(e, "Could not record the running game"))
}

pub async fn open_sessions(db: &Db) -> Result<Vec<SessionRow>, ApiError> {
    db.call(|conn| {
        let mut statement = conn.prepare(
            "SELECT id, game_key, pid, process_start, folder, started_at, last_seen_at
             FROM games_session ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                game_key: row.get(1)?,
                pid: row.get::<_, i64>(2)? as u32,
                process_start: row.get::<_, i64>(3)?.max(0) as u64,
                folder: row.get(4)?,
                started_at: row.get(5)?,
                last_seen_at: row.get(6)?,
            })
        })?;
        rows.collect()
    })
    .await
    .map_err(|e| failed(e, "Could not read the running games"))
}

/// At the stop of the backend: every open session was alive now.
pub async fn touch_sessions(db: &Db, now: String) -> Result<(), ApiError> {
    db.call(move |conn| {
        conn.execute("UPDATE games_session SET last_seen_at = ?1", params![now])
            .map(|_| ())
    })
    .await
    .map_err(|e| failed(e, "Could not record the running games"))
}

// ---- caches ----

#[derive(Debug, Clone, PartialEq)]
pub struct GuessRow {
    pub signature: String,
    pub exe_rel: Option<String>,
    pub guessed_at: String,
}

pub async fn guess_get(db: &Db, folder_key: String) -> Result<Option<GuessRow>, ApiError> {
    db.call(move |conn| {
        conn.query_row(
            "SELECT signature, exe_rel, guessed_at FROM games_guess_cache WHERE folder_key = ?1",
            params![folder_key],
            |row| {
                Ok(GuessRow {
                    signature: row.get(0)?,
                    exe_rel: row.get(1)?,
                    guessed_at: row.get(2)?,
                })
            },
        )
        .optional()
    })
    .await
    .map_err(|e| failed(e, "Could not read the program guesses"))
}

pub async fn guess_put(
    db: &Db,
    folder_key: String,
    signature: String,
    exe_rel: Option<String>,
    score: Option<f64>,
    now: String,
) -> Result<(), ApiError> {
    db.call(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO games_guess_cache (folder_key, signature, exe_rel, score, guessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![folder_key, signature, exe_rel, score, now],
        )
        .map(|_| ())
    })
    .await
    .map_err(|e| failed(e, "Could not save the program guess"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeRow {
    pub size_bytes: u64,
    pub measured_at: String,
    pub signature: String,
}

pub async fn sizes_all(db: &Db) -> Result<HashMap<String, SizeRow>, ApiError> {
    db.call(|conn| {
        let mut statement = conn.prepare(
            "SELECT folder_key, size_bytes, measured_at, signature FROM games_size_cache",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                SizeRow {
                    size_bytes: row.get::<_, i64>(1)?.max(0) as u64,
                    measured_at: row.get(2)?,
                    signature: row.get(3)?,
                },
            ))
        })?;
        rows.collect()
    })
    .await
    .map_err(|e| failed(e, "Could not read the game sizes"))
}

pub async fn size_put(
    db: &Db,
    folder_key: String,
    size_bytes: u64,
    signature: String,
    now: String,
) -> Result<(), ApiError> {
    db.call(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO games_size_cache (folder_key, size_bytes, measured_at, signature)
             VALUES (?1, ?2, ?3, ?4)",
            params![folder_key, size_bytes as i64, now, signature],
        )
        .map(|_| ())
    })
    .await
    .map_err(|e| failed(e, "Could not save the game size"))
}

/// `POST /games/refresh` drops both caches.
pub async fn clear_caches(db: &Db) -> Result<(), ApiError> {
    db.call(|conn| {
        conn.execute_batch("DELETE FROM games_guess_cache; DELETE FROM games_size_cache;")
    })
    .await
    .map_err(|e| failed(e, "Could not clear the game caches"))
}

/// Rows of the two caches that were not refreshed since `cutoff` (RFC 3339 text compares in
/// time order) are deleted.
pub async fn prune_caches(db: &Db, cutoff: String) -> Result<(), ApiError> {
    db.call(move |conn| {
        conn.execute(
            "DELETE FROM games_guess_cache WHERE guessed_at < ?1",
            params![cutoff],
        )?;
        conn.execute(
            "DELETE FROM games_size_cache WHERE measured_at < ?1",
            params![cutoff],
        )
        .map(|_| ())
    })
    .await
    .map_err(|e| failed(e, "Could not clean the game caches"))
}
