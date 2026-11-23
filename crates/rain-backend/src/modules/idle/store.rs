//! Files and tables of the `idle` module: `idle-jobs.toml` and the tables of section 03 5.2.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::model::{FILE_NAME, IdleFile};
use crate::core::error::ApiResult;
use crate::store::db::{Db, DbError};
use crate::store::hub::ConfigHub;
use crate::store::toml_file::TomlFile;

/// Runs kept per job; the lines of older runs are deleted through the jobs module.
pub const IDLE_RUNS_KEPT: i64 = 5;

/// The file Rain writes at the first start (section 03 4.2.1). It has no job: idle jobs start
/// empty (decisions section 5).
pub const DEFAULT_FILE: &str = r#"schema_version = 1

# Idle jobs: programs that run while you are away from the PC.
# A job starts after `idle_after_minutes` without keyboard or mouse input, is frozen when you come back,
# and continues after the same time of quiet. Only one job per resource (cpu or gpu) runs at a time.
# Rain reloads this file at once. Rain writes back only `enabled` (and `id` when it is missing).
#
# [[idle_job]]
# id = "idle-thumbnail-cache"            # optional, stable name of the job
# name = "Rebuild thumbnail cache"
# enabled = false
# resource = "cpu"                       # "cpu" or "gpu"
# idle_after_minutes = 10
# command = "D:\\tools\\thumbs.exe"
# args = ["--all"]
# rerun = "daily"                        # "daily", "weekly", "every_idle" or "manual"
# expected_minutes = 30                  # optional: gives an estimate when the program reports no progress
#
# The program may print lines like these to report progress:
#   rain-progress: 42%
#   rain-eta: 12
"#;

pub async fn open(hub: &ConfigHub) -> ApiResult<TomlFile<IdleFile>> {
    TomlFile::open(hub, FILE_NAME, |_| Ok(())).await
}

/// What is remembered of a job between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remembered {
    pub last_started_at: Option<String>,
    pub last_ended_at: Option<String>,
    /// `success` or `failed`.
    pub last_result: Option<String>,
    /// The id of the newest run, for the log route.
    pub last_run_id: Option<String>,
}

/// The state of every job from the database, and the ids of the runs that were cut off.
pub struct Loaded {
    pub remembered: HashMap<String, Remembered>,
    pub interrupted: Vec<(String, String)>,
}

/// Closes the runs that were still `running` when the backend started (their process died with
/// Rain) and reads what is remembered of every job.
pub async fn load(db: &Db, now: String) -> Result<Loaded, DbError> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let interrupted: Vec<(String, String)> = {
            let mut statement =
                tx.prepare("SELECT id, job_id FROM idle_run WHERE status = 'running'")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        tx.execute(
            "UPDATE idle_run SET status = 'stopped', ended_at = ?1 WHERE status = 'running'",
            [&now],
        )?;
        let mut remembered: HashMap<String, Remembered> = HashMap::new();
        {
            let mut statement = tx.prepare(
                "SELECT job_id, last_started_at, last_ended_at, last_result FROM idle_state",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    Remembered {
                        last_started_at: row.get(1)?,
                        last_ended_at: row.get(2)?,
                        last_result: row.get(3)?,
                        last_run_id: None,
                    },
                ))
            })?;
            for row in rows {
                let (id, state) = row?;
                remembered.insert(id, state);
            }
        }
        {
            let mut statement =
                tx.prepare("SELECT job_id, id FROM idle_run ORDER BY started_at ASC, id ASC")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (job_id, run_id) = row?;
                remembered.entry(job_id).or_default().last_run_id = Some(run_id);
            }
        }
        tx.commit()?;
        Ok(Loaded {
            remembered,
            interrupted,
        })
    })
    .await
}

/// A run starts.
pub async fn insert_run(
    db: &Db,
    id: String,
    job_id: String,
    started_at: String,
    manual: bool,
) -> Result<(), DbError> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO idle_run (id, job_id, started_at, status, manual)
             VALUES (?1, ?2, ?3, 'running', ?4)",
            params![id, job_id, started_at, manual],
        )?;
        tx.execute(
            "INSERT INTO idle_state (job_id, last_started_at) VALUES (?1, ?2)
             ON CONFLICT (job_id) DO UPDATE SET last_started_at = excluded.last_started_at",
            params![job_id, started_at],
        )?;
        tx.commit()
    })
    .await
}

/// A run ends. `result` is `success` or `failed` for a run that finished by itself, `stopped` for
/// one that was ended on purpose. Returns the ids of the runs that fall out of the newest
/// `IDLE_RUNS_KEPT` (the caller deletes their log lines through the jobs module).
pub async fn finish_run(
    db: &Db,
    id: String,
    job_id: String,
    ended_at: String,
    result: &'static str,
    exit_code: Option<i64>,
) -> Result<Vec<String>, DbError> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE idle_run SET status = ?2, ended_at = ?3, exit_code = ?4 WHERE id = ?1",
            params![id, result, ended_at, exit_code],
        )?;
        if result != "stopped" {
            tx.execute(
                "INSERT INTO idle_state (job_id, last_ended_at, last_result) VALUES (?1, ?2, ?3)
                 ON CONFLICT (job_id) DO UPDATE
                   SET last_ended_at = excluded.last_ended_at, last_result = excluded.last_result",
                params![job_id, ended_at, result],
            )?;
        }
        let old: Vec<String> = {
            let mut statement = tx.prepare(
                "SELECT id FROM idle_run WHERE job_id = ?1
                 ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET ?2",
            )?;
            let rows = statement.query_map(params![job_id, IDLE_RUNS_KEPT], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for old_id in &old {
            tx.execute("DELETE FROM idle_run WHERE id = ?1", [old_id])?;
        }
        tx.commit()?;
        Ok(old)
    })
    .await
}

/// The id of the newest run of a job.
pub async fn newest_run(db: &Db, job_id: String) -> Result<Option<String>, DbError> {
    db.call(move |conn| {
        conn.query_row(
            "SELECT id FROM idle_run WHERE job_id = ?1 ORDER BY started_at DESC, id DESC LIMIT 1",
            [job_id],
            |row| row.get(0),
        )
        .optional()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::idle::MIGRATIONS;
    use crate::store::migrations;

    async fn db() -> (Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("rain.db"));
        db.open(0).await.unwrap();
        migrations::run_all(
            &db,
            vec![("idle", MIGRATIONS)],
            "2026-01-01T00:00:00.000Z".into(),
        )
        .await
        .unwrap();
        (db, dir)
    }

    async fn run(db: &Db, n: u32, result: &'static str) -> Vec<String> {
        let id = format!("irun-{n:04}");
        let started = format!("2026-11-21T09:{:02}:00.000Z", n % 60);
        insert_run(db, id.clone(), "idle-a".into(), started, false)
            .await
            .unwrap();
        finish_run(
            db,
            id,
            "idle-a".into(),
            "2026-11-21T10:00:00.000Z".into(),
            result,
            Some(0),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn only_the_newest_five_runs_are_kept_and_the_old_ids_are_returned() {
        let (db, _dir) = db().await;
        for n in 1..=5 {
            assert!(run(&db, n, "success").await.is_empty());
        }
        assert_eq!(run(&db, 6, "success").await, ["irun-0001"]);
        assert_eq!(run(&db, 7, "failed").await, ["irun-0002"]);
        assert_eq!(
            newest_run(&db, "idle-a".into()).await.unwrap().as_deref(),
            Some("irun-0007")
        );
    }

    #[tokio::test]
    async fn the_result_is_remembered_but_a_stopped_run_does_not_change_it() {
        let (db, _dir) = db().await;
        run(&db, 1, "failed").await;
        run(&db, 2, "stopped").await;
        let loaded = load(&db, "now".into()).await.unwrap();
        let state = &loaded.remembered["idle-a"];
        assert_eq!(state.last_result.as_deref(), Some("failed"));
        assert_eq!(
            state.last_ended_at.as_deref(),
            Some("2026-11-21T10:00:00.000Z")
        );
        assert_eq!(
            state.last_started_at.as_deref(),
            Some("2026-11-21T09:02:00.000Z")
        );
        assert_eq!(state.last_run_id.as_deref(), Some("irun-0002"));
    }

    #[tokio::test]
    async fn runs_that_were_cut_off_are_closed_as_stopped_and_named() {
        let (db, _dir) = db().await;
        insert_run(
            &db,
            "irun-9".into(),
            "idle-a".into(),
            "2026-11-21T09:00:00.000Z".into(),
            true,
        )
        .await
        .unwrap();
        let loaded = load(&db, "2026-11-22T00:00:00.000Z".into()).await.unwrap();
        assert_eq!(
            loaded.interrupted,
            [("irun-9".to_owned(), "idle-a".to_owned())]
        );
        let again = load(&db, "later".into()).await.unwrap();
        assert!(again.interrupted.is_empty());
    }

    #[test]
    fn the_default_file_has_no_job() {
        let doc: toml::Table = toml::from_str(DEFAULT_FILE).unwrap();
        assert_eq!(doc.len(), 1);
        assert!(DEFAULT_FILE.contains("# [[idle_job]]"));
    }
}
