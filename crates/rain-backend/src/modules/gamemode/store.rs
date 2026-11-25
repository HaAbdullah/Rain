//! The record of what game mode stopped, in SQLite. Section 06 section 5.
//!
//! History, not cache: losing `rain.db` loses only the recap shown after the next start.
//! Claude session titles are never stored (a title can hold prompt text). Only how many
//! sessions were left running is.
//!
//! Every function takes the `Db` and runs one short call on the database thread.

use rain_types::{RecapOutcome, WindDownKind};
use rusqlite::{OptionalExtension, Row, params};

use crate::core::error::ApiResult;
use crate::store::db::Db;

/// The newest records that are kept (`GAMEMODE_RECORDS_KEPT`).
pub const RECORDS_KEPT: i64 = 20;

/// What is known when the wind-down begins.
#[derive(Debug, Clone)]
pub struct NewRecord {
    pub started_at: String,
    pub countdown_seconds: u32,
    pub then_launch_game_id: Option<String>,
    pub then_launch_title: Option<String>,
}

/// What is known when the wind-down ends.
#[derive(Debug, Clone)]
pub struct Finish {
    pub finished_at: String,
    pub waited: bool,
    pub forced: bool,
    /// `Some(true)`: the game started. `Some(false)`: it failed. `None`: none was asked for.
    pub launched: Option<bool>,
    pub launch_error: Option<String>,
    /// The game started, but inside a job of Rain (it may close with Rain).
    pub launched_in_job: bool,
}

/// One step of the wind-down that stopped (or left) something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub kind: WindDownKind,
    pub name: String,
    pub outcome: RecapOutcome,
    pub count: u32,
}

/// A finished record, as the recap needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRow {
    pub started_at: String,
    pub finished_at: Option<String>,
    /// `completed` or `incomplete`; a `running` record is never read back.
    pub complete: bool,
    pub then_launch_title: Option<String>,
    pub launch_error: Option<String>,
    pub launched_in_job: bool,
    pub seen: bool,
}

fn kind_text(kind: WindDownKind) -> &'static str {
    match kind {
        WindDownKind::Session => "session",
        WindDownKind::OtherApp => "otherApp",
        WindDownKind::Job => "job",
        WindDownKind::IdleJob => "idleJob",
        WindDownKind::Service => "service",
        WindDownKind::Window => "window",
    }
}

fn kind_of(text: &str) -> WindDownKind {
    match text {
        "session" => WindDownKind::Session,
        "otherApp" => WindDownKind::OtherApp,
        "job" => WindDownKind::Job,
        "idleJob" => WindDownKind::IdleJob,
        "service" => WindDownKind::Service,
        _ => WindDownKind::Window,
    }
}

fn outcome_text(outcome: RecapOutcome) -> &'static str {
    match outcome {
        RecapOutcome::Stopped => "stopped",
        RecapOutcome::Forced => "forced",
        RecapOutcome::Failed => "failed",
        RecapOutcome::LeftRunning => "leftRunning",
    }
}

fn outcome_of(text: &str) -> RecapOutcome {
    match text {
        "forced" => RecapOutcome::Forced,
        "failed" => RecapOutcome::Failed,
        "leftRunning" => RecapOutcome::LeftRunning,
        _ => RecapOutcome::Stopped,
    }
}

/// Starts a record with outcome `running` and returns its id. The records beyond the newest
/// `RECORDS_KEPT` are deleted in the same transaction (their items go with them: `ON DELETE
/// CASCADE`).
pub async fn begin_record(db: &Db, record: NewRecord) -> ApiResult<i64> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO gamemode_record
               (started_at, outcome, countdown_seconds, then_launch_game_id, then_launch_title)
             VALUES (?1, 'running', ?2, ?3, ?4)",
            params![
                record.started_at,
                record.countdown_seconds,
                record.then_launch_game_id,
                record.then_launch_title
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "DELETE FROM gamemode_record WHERE id NOT IN
               (SELECT id FROM gamemode_record ORDER BY id DESC LIMIT ?1)",
            params![RECORDS_KEPT],
        )?;
        tx.commit()?;
        Ok(id)
    })
    .await
    .map_err(|e| e.into_api("Could not record game mode"))
}

/// Adds one finished step. `seq` orders the steps of one record.
pub async fn add_item(db: &Db, record_id: i64, seq: u32, item: ItemRow) -> ApiResult<()> {
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO gamemode_record_item (record_id, seq, kind, name, outcome, count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record_id,
                seq,
                kind_text(item.kind),
                item.name,
                outcome_text(item.outcome),
                item.count
            ],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| e.into_api("Could not record game mode"))
}

/// Closes the record as `completed`.
pub async fn finish_record(db: &Db, record_id: i64, finish: Finish) -> ApiResult<()> {
    db.call(move |conn| {
        let launch_result = finish
            .launched
            .map(|ok| if ok { "started" } else { "failed" });
        conn.execute(
            "UPDATE gamemode_record
                SET outcome = 'completed', finished_at = ?2, waited = ?3, forced = ?4,
                    launch_result = ?5, launch_error = ?6, launched_in_job = ?7
              WHERE id = ?1",
            params![
                record_id,
                finish.finished_at,
                finish.waited,
                finish.forced,
                launch_result,
                finish.launch_error,
                finish.launched_in_job
            ],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| e.into_api("Could not record game mode"))
}

/// At backend start: a record that is still `running` belongs to a backend that died during the
/// wind-down.
pub async fn mark_incomplete(db: &Db) -> ApiResult<()> {
    db.call(|conn| {
        conn.execute(
            "UPDATE gamemode_record SET outcome = 'incomplete' WHERE outcome = 'running'",
            [],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| e.into_api("Could not read the game mode record"))
}

/// The newest record that is not `running`, with its items in the order they were done.
pub async fn latest(db: &Db) -> ApiResult<Option<(RecordRow, Vec<ItemRow>)>> {
    db.call(|conn| {
        let record = conn
            .query_row(
                "SELECT id, started_at, finished_at, outcome, then_launch_title, launch_error,
                        seen_at IS NOT NULL, launched_in_job
                   FROM gamemode_record WHERE outcome <> 'running' ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, record_of(row)?)),
            )
            .optional()?;
        let Some((id, record)) = record else {
            return Ok(None);
        };
        let mut statement = conn.prepare(
            "SELECT kind, name, outcome, count FROM gamemode_record_item
              WHERE record_id = ?1 ORDER BY seq",
        )?;
        let items = statement
            .query_map(params![id], |row| {
                Ok(ItemRow {
                    kind: kind_of(&row.get::<_, String>(0)?),
                    name: row.get(1)?,
                    outcome: outcome_of(&row.get::<_, String>(2)?),
                    count: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some((record, items)))
    })
    .await
    .map_err(|e| e.into_api("Could not read the game mode record"))
}

fn record_of(row: &Row<'_>) -> rusqlite::Result<RecordRow> {
    Ok(RecordRow {
        started_at: row.get(1)?,
        finished_at: row.get(2)?,
        complete: row.get::<_, String>(3)? == "completed",
        then_launch_title: row.get(4)?,
        launch_error: row.get(5)?,
        seen: row.get(6)?,
        launched_in_job: row.get(7)?,
    })
}

/// The notice was shown: every finished record counts as seen.
pub async fn mark_seen(db: &Db, at: String) -> ApiResult<()> {
    db.call(move |conn| {
        conn.execute(
            "UPDATE gamemode_record SET seen_at = ?1 WHERE seen_at IS NULL AND outcome <> 'running'",
            params![at],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| e.into_api("Could not save that the notice was shown"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::store::migrations::Migration;

    /// A database with this module's schema in a temporary folder.
    async fn db() -> (Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(PathBuf::from(dir.path()).join("test.db"));
        db.open(0).await.expect("the database opened");
        let migrations = [
            Migration {
                id: 1,
                name: "create_record",
                sql: include_str!("migrations/0001_create_record.sql"),
            },
            Migration {
                id: 2,
                name: "launch_in_job",
                sql: include_str!("migrations/0002_launch_in_job.sql"),
            },
            Migration {
                id: 3,
                name: "other_apps",
                sql: include_str!("migrations/0003_other_apps.sql"),
            },
        ];
        db.call(move |conn| {
            for migration in &migrations {
                conn.execute_batch(migration.sql)?;
            }
            Ok(())
        })
        .await
        .expect("the schema was created");
        (db, dir)
    }

    fn new_record(at: &str) -> NewRecord {
        NewRecord {
            started_at: at.to_owned(),
            countdown_seconds: 10,
            then_launch_game_id: None,
            then_launch_title: Some("Game A".to_owned()),
        }
    }

    fn done() -> Finish {
        Finish {
            finished_at: "2026-11-22T10:00:09.000Z".into(),
            waited: true,
            forced: true,
            launched: Some(false),
            launch_error: Some("It did not start.".into()),
            launched_in_job: true,
        }
    }

    fn job(name: &str, outcome: RecapOutcome) -> ItemRow {
        ItemRow {
            kind: WindDownKind::Job,
            name: name.into(),
            outcome,
            count: 1,
        }
    }

    #[tokio::test]
    async fn a_record_runs_through_begin_items_finish_and_seen() {
        let (db, _dir) = db().await;
        assert!(latest(&db).await.unwrap().is_none());

        let id = begin_record(&db, new_record("2026-11-22T10:00:00.000Z"))
            .await
            .unwrap();
        assert!(
            latest(&db).await.unwrap().is_none(),
            "a running record is not a recap"
        );
        add_item(&db, id, 0, job("Demo wait", RecapOutcome::Stopped))
            .await
            .unwrap();
        add_item(&db, id, 1, job("Other", RecapOutcome::Failed))
            .await
            .unwrap();
        finish_record(&db, id, done()).await.unwrap();

        let (record, items) = latest(&db).await.unwrap().expect("a finished record");
        assert!(record.complete);
        assert!(!record.seen);
        assert_eq!(record.then_launch_title.as_deref(), Some("Game A"));
        assert_eq!(record.launch_error.as_deref(), Some("It did not start."));
        assert!(record.launched_in_job);
        assert_eq!(
            items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["Demo wait", "Other"]
        );
        assert_eq!(items[1].outcome, RecapOutcome::Failed);

        mark_seen(&db, "2026-11-22T11:00:00.000Z".into())
            .await
            .unwrap();
        assert!(latest(&db).await.unwrap().unwrap().0.seen);
    }

    #[tokio::test]
    async fn a_running_record_becomes_incomplete_at_start() {
        let (db, _dir) = db().await;
        begin_record(&db, new_record("2026-11-22T10:00:00.000Z"))
            .await
            .unwrap();
        mark_incomplete(&db).await.unwrap();
        let (record, _) = latest(&db).await.unwrap().expect("now readable");
        assert!(!record.complete);
        assert_eq!(record.finished_at, None);
    }

    #[tokio::test]
    async fn the_twenty_first_record_deletes_the_oldest_and_its_items() {
        let (db, _dir) = db().await;
        let first = begin_record(&db, new_record("t0")).await.unwrap();
        add_item(&db, first, 0, job("Old", RecapOutcome::Stopped))
            .await
            .unwrap();
        for n in 1..=RECORDS_KEPT {
            begin_record(&db, new_record(&format!("t{n}")))
                .await
                .unwrap();
        }
        let (records, items) = db
            .call(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM gamemode_record", [], |r| {
                        r.get::<_, i64>(0)
                    })?,
                    conn.query_row("SELECT COUNT(*) FROM gamemode_record_item", [], |r| {
                        r.get::<_, i64>(0)
                    })?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(records, RECORDS_KEPT);
        assert_eq!(
            items, 0,
            "ON DELETE CASCADE removed the items of the deleted record"
        );
    }
}
