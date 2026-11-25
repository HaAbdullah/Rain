//! Files and tables of the `jobs` module: `jobs.toml` and the tables of section 03 5.1.

use std::collections::HashMap;

use rain_types::{JobRun, Status};
use rusqlite::{Connection, OptionalExtension, params};

use super::model::{FILE_NAME, JobsFile};
use crate::core::error::ApiResult;
use crate::store::db::{Db, DbError};
use crate::store::hub::ConfigHub;
use crate::store::toml_file::TomlFile;

/// Runs kept per job; older runs and their log lines are deleted at the end of a run.
pub const JOB_RUNS_KEPT: i64 = 20;

/// The file Rain writes at the first start (section 03 4.1.3). It has no job.
pub const DEFAULT_FILE: &str = r#"schema_version = 1

# Scheduled jobs. One [[job]] table per job. Rain shows them in the Jobs tab,
# runs them by schedule, and writes back only `enabled` (and `id` when it is missing).
# Edit this file by hand; Rain reloads it at once. Times are local time.
#
# A command job:
#
# [[job]]
# id = "backup-notes"                  # optional, stable name of the job
# name = "Back up notes"
# kind = "cron"
# schedule = "30 2 * * *"              # minute hour day month weekday (five fields)
# enabled = false
# command = "robocopy"
# args = ["D:\\notes", "E:\\backup\\notes", "/MIR"]
# working_dir = "D:\\"                 # optional
# timeout_minutes = 30                 # optional
# catch_up = true                      # run once at the next start when a run was missed
#
# A Claude job (headless `claude -p`, nobody can answer a permission prompt):
#
# [[job]]
# name = "Morning skill run"
# kind = "claude"
# schedule = "0 6 * * *"
# enabled = false
# skill = "my-skill"                   # a skill that exists in your Claude setup
# prompt = "Optional extra instructions."
# permission_mode = "dontAsk"          # deny everything that is not listed below
# allowed_tools = ["Skill", "Read", "Write", "Edit", "Glob", "Grep"]
# timeout_minutes = 60
"#;

/// Opens `jobs.toml`. Per-entry checks happen in `model::load`, so the whole file fails only when
/// it is not valid TOML.
pub async fn open(hub: &ConfigHub) -> ApiResult<TomlFile<JobsFile>> {
    TomlFile::open(hub, FILE_NAME, |_| Ok(())).await
}

// ---- runs ----

/// A new run, as it is written when it starts.
pub struct NewRun {
    pub id: String,
    pub job_id: String,
    /// `schedule`, `manual` or `catchup`.
    pub trigger: &'static str,
    pub started_at: String,
}

pub async fn insert_run(db: &Db, run: NewRun) -> Result<(), DbError> {
    db.call(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO jobs_run (id, job_id, trigger_kind, status, started_at)
             VALUES (?1, ?2, ?3, 'running', ?4)",
            params![run.id, run.job_id, run.trigger, run.started_at],
        )
        .map(|_| ())
    })
    .await
}

/// How a run ended, as it is written.
pub struct EndedRun {
    pub id: String,
    pub job_id: String,
    pub status: Status,
    pub ended_at: String,
    pub duration_ms: u64,
    pub exit_code: Option<i64>,
    pub session_id: Option<String>,
    pub log_lines: u64,
    pub log_omitted: u64,
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Running => "running",
        Status::Success => "success",
        Status::Failed => "failed",
        // `idle` is "stopped on purpose"; `waiting` does not exist for a run.
        Status::Idle | Status::Waiting => "idle",
    }
}

fn status_of(name: &str) -> Status {
    match name {
        "running" => Status::Running,
        "success" => Status::Success,
        "failed" => Status::Failed,
        _ => Status::Idle,
    }
}

/// Stores the Claude session id that a hook reported for a run. A run that is not in the table
/// (an id nobody made) changes nothing; the answer says whether a row was changed.
pub async fn attach_session(db: &Db, run_id: String, session_id: String) -> Result<bool, DbError> {
    db.call(move |conn| {
        let changed = conn.execute(
            "UPDATE jobs_run SET claude_session_id = ?2 WHERE id = ?1",
            params![run_id, session_id],
        )?;
        Ok(changed > 0)
    })
    .await
}

/// Writes the end of a run and, in the same transaction, deletes the runs of this job beyond the
/// newest `JOB_RUNS_KEPT` together with their log lines (retention, section 03 5.1).
pub async fn finish_run(db: &Db, run: EndedRun) -> Result<(), DbError> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE jobs_run SET status = ?2, ended_at = ?3, duration_ms = ?4, exit_code = ?5,
                    claude_session_id = COALESCE(?6, claude_session_id), log_lines = ?7,
                    log_omitted = ?8
             WHERE id = ?1",
            params![
                run.id,
                status_name(run.status),
                run.ended_at,
                run.duration_ms as i64,
                run.exit_code,
                run.session_id,
                run.log_lines as i64,
                run.log_omitted as i64
            ],
        )?;
        let old =
            "SELECT id FROM jobs_run WHERE job_id = ?1 ORDER BY started_at DESC LIMIT -1 OFFSET ?2";
        tx.execute(
            &format!("DELETE FROM jobs_run_log WHERE run_id IN ({old})"),
            params![run.job_id, JOB_RUNS_KEPT],
        )?;
        tx.execute(
            &format!("DELETE FROM jobs_run WHERE id IN ({old})"),
            params![run.job_id, JOB_RUNS_KEPT],
        )?;
        tx.commit()
    })
    .await
}

/// A run that was still `running` when the backend started was cut off (a crash, a kill). It
/// is closed as failed, with one line in its log that says so. Returns the ids that were closed.
pub async fn close_running(db: &Db, ended_at: String) -> Result<Vec<String>, DbError> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let ids: Vec<String> = {
            let mut statement = tx.prepare("SELECT id FROM jobs_run WHERE status = 'running'")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for id in &ids {
            let next_seq: i64 = tx.query_row(
                "SELECT COALESCE(MAX(seq) + 1, 0) FROM jobs_run_log WHERE run_id = ?1",
                [id],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO jobs_run_log (run_id, seq, ts, level, text)
                 VALUES (?1, ?2, ?3, 'error', 'Rain stopped while this run was in progress')",
                params![id, next_seq, ended_at],
            )?;
            tx.execute(
                "UPDATE jobs_run SET status = 'failed', ended_at = ?2,
                        log_lines = log_lines + 1
                 WHERE id = ?1",
                params![id, ended_at],
            )?;
        }
        tx.commit()?;
        Ok(ids)
    })
    .await
}

/// The newest `JOB_RUNS_KEPT` runs of every job, newest first inside each job.
pub async fn load_recent(db: &Db) -> Result<HashMap<String, Vec<JobRun>>, DbError> {
    db.call(|conn| {
        let mut statement = conn.prepare(
            "SELECT id, job_id, status, started_at, duration_ms, exit_code
             FROM jobs_run ORDER BY started_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(JobRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                status: status_of(&row.get::<_, String>(2)?),
                started_at: row.get(3)?,
                duration_ms: row.get::<_, Option<i64>>(4)?.map(|ms| ms.max(0) as u64),
                exit_code: row.get(5)?,
                outcome: None,
            })
        })?;
        let mut by_job: HashMap<String, Vec<JobRun>> = HashMap::new();
        for run in rows {
            let run = run?;
            let list = by_job.entry(run.job_id.clone()).or_default();
            if (list.len() as i64) < JOB_RUNS_KEPT {
                list.push(run);
            }
        }
        Ok(by_job)
    })
    .await
}

/// Whether a run exists in the history.
pub async fn run_exists(db: &Db, run_id: String) -> Result<bool, DbError> {
    db.call(move |conn| {
        conn.query_row("SELECT 1 FROM jobs_run WHERE id = ?1", [run_id], |_| Ok(()))
            .optional()
            .map(|found| found.is_some())
    })
    .await
}

/// Deletes the runs of jobs that are not in `jobs.toml` any more when they started before
/// `cutoff` (thirty days, section 03 5.1), with their log lines and their scheduler memory.
pub async fn delete_removed(
    db: &Db,
    present_ids: Vec<String>,
    cutoff: String,
) -> Result<(), DbError> {
    db.call(move |conn| {
        let known: Vec<String> = {
            let mut statement = conn.prepare("SELECT DISTINCT job_id FROM jobs_run")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let tx = conn.transaction()?;
        for job_id in known.iter().filter(|id| !present_ids.contains(id)) {
            tx.execute(
                "DELETE FROM jobs_run_log WHERE run_id IN
                   (SELECT id FROM jobs_run WHERE job_id = ?1 AND started_at < ?2)",
                params![job_id, cutoff],
            )?;
            tx.execute(
                "DELETE FROM jobs_run WHERE job_id = ?1 AND started_at < ?2",
                params![job_id, cutoff],
            )?;
        }
        tx.execute(
            "DELETE FROM jobs_state WHERE job_id NOT IN (SELECT job_id FROM jobs_run)
               AND updated_at < ?1",
            [cutoff],
        )?;
        tx.commit()
    })
    .await
}

// ---- the scheduler's memory ----

/// `handled_until` of every job, as the API text.
pub async fn load_handled(db: &Db) -> Result<HashMap<String, String>, DbError> {
    db.call(|conn| {
        let mut statement = conn.prepare("SELECT job_id, handled_until FROM jobs_state")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<HashMap<String, String>>>()
    })
    .await
}

pub async fn save_handled(
    db: &Db,
    job_id: String,
    handled_until: String,
    now: String,
) -> Result<(), DbError> {
    db.call(move |conn: &mut Connection| {
        conn.execute(
            "INSERT INTO jobs_state (job_id, handled_until, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (job_id) DO UPDATE
               SET handled_until = excluded.handled_until, updated_at = excluded.updated_at",
            params![job_id, handled_until, now],
        )
        .map(|_| ())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::jobs::MIGRATIONS;
    use crate::store::migrations;

    async fn db() -> (Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("rain.db"));
        db.open(0).await.unwrap();
        migrations::run_all(
            &db,
            vec![("jobs", MIGRATIONS)],
            "2026-01-01T00:00:00.000Z".into(),
        )
        .await
        .unwrap();
        (db, dir)
    }

    fn started(n: u32) -> NewRun {
        NewRun {
            id: format!("run-{n:04}"),
            job_id: "job-a".into(),
            trigger: "schedule",
            started_at: format!("2026-11-21T09:{:02}:00.000Z", n % 60),
        }
    }

    fn ended(n: u32, status: Status) -> EndedRun {
        EndedRun {
            id: format!("run-{n:04}"),
            job_id: "job-a".into(),
            status,
            ended_at: "2026-11-21T10:00:00.000Z".into(),
            duration_ms: 1500,
            exit_code: Some(0),
            session_id: None,
            log_lines: 1,
            log_omitted: 0,
        }
    }

    #[tokio::test]
    async fn a_run_is_written_finished_and_read_back_newest_first() {
        let (db, _dir) = db().await;
        for n in 1..=3 {
            insert_run(&db, started(n)).await.unwrap();
        }
        finish_run(&db, ended(1, Status::Success)).await.unwrap();
        finish_run(&db, ended(2, Status::Failed)).await.unwrap();

        let recent = load_recent(&db).await.unwrap();
        let runs = &recent["job-a"];
        let ids: Vec<_> = runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["run-0003", "run-0002", "run-0001"]);
        assert_eq!(runs[0].status, Status::Running);
        assert_eq!(runs[1].status, Status::Failed);
        assert_eq!(runs[2].duration_ms, Some(1500));
        assert!(run_exists(&db, "run-0002".into()).await.unwrap());
        assert!(!run_exists(&db, "run-9999".into()).await.unwrap());
    }

    #[tokio::test]
    async fn an_attached_session_survives_the_end_of_the_run() {
        let (db, _dir) = db().await;
        insert_run(&db, started(1)).await.unwrap();
        async fn read(db: &Db) -> Option<String> {
            db.call(|c| {
                c.query_row(
                    "SELECT claude_session_id FROM jobs_run WHERE id = 'run-0001'",
                    [],
                    |r| r.get::<_, Option<String>>(0),
                )
            })
            .await
            .unwrap()
        }
        assert!(
            attach_session(&db, "run-0001".into(), "sess-1".into())
                .await
                .unwrap()
        );
        // A run that is not in the table changes nothing.
        assert!(
            !attach_session(&db, "run-9999".into(), "sess-2".into())
                .await
                .unwrap()
        );
        // The end of a run that never saw a session id in its own output keeps the attached one.
        finish_run(&db, ended(1, Status::Success)).await.unwrap();
        assert_eq!(read(&db).await.as_deref(), Some("sess-1"));
        // An id from the run's own output replaces it.
        let mut again = ended(1, Status::Success);
        again.session_id = Some("sess-3".into());
        finish_run(&db, again).await.unwrap();
        assert_eq!(read(&db).await.as_deref(), Some("sess-3"));
    }

    #[tokio::test]
    async fn only_the_newest_twenty_runs_of_a_job_are_kept() {
        let (db, _dir) = db().await;
        for n in 1..=25 {
            insert_run(&db, started(n)).await.unwrap();
            db.call(move |c| {
                c.execute(
                    "INSERT INTO jobs_run_log (run_id, seq, ts, level, text)
                     VALUES (?1, 0, 't', 'info', 'x')",
                    [format!("run-{n:04}")],
                )
            })
            .await
            .unwrap();
            finish_run(&db, ended(n, Status::Success)).await.unwrap();
        }
        let recent = load_recent(&db).await.unwrap();
        assert_eq!(recent["job-a"].len(), 20);
        assert_eq!(recent["job-a"][0].id, "run-0025");
        assert_eq!(recent["job-a"][19].id, "run-0006");
        let log_rows: i64 = db
            .call(|c| c.query_row("SELECT COUNT(*) FROM jobs_run_log", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(log_rows, 20, "the lines of the deleted runs went with them");
    }

    #[tokio::test]
    async fn jobs_start_closes_running_rows() {
        let (db, _dir) = db().await;
        insert_run(&db, started(1)).await.unwrap();
        insert_run(&db, started(2)).await.unwrap();
        finish_run(&db, ended(2, Status::Success)).await.unwrap();

        let closed = close_running(&db, "2026-11-22T00:00:00.000Z".into())
            .await
            .unwrap();
        assert_eq!(closed, ["run-0001"]);
        let recent = load_recent(&db).await.unwrap();
        let run = recent["job-a"].iter().find(|r| r.id == "run-0001").unwrap();
        assert_eq!(run.status, Status::Failed);
        let text: String = db
            .call(|c| {
                c.query_row(
                    "SELECT text FROM jobs_run_log WHERE run_id = 'run-0001'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(text, "Rain stopped while this run was in progress");
        assert!(close_running(&db, "x".into()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn handled_until_is_saved_and_replaced() {
        let (db, _dir) = db().await;
        save_handled(
            &db,
            "job-a".into(),
            "2026-11-21T09:00:00.000Z".into(),
            "n".into(),
        )
        .await
        .unwrap();
        save_handled(
            &db,
            "job-a".into(),
            "2026-11-22T09:00:00.000Z".into(),
            "n".into(),
        )
        .await
        .unwrap();
        let handled = load_handled(&db).await.unwrap();
        assert_eq!(handled.len(), 1);
        assert_eq!(handled["job-a"], "2026-11-22T09:00:00.000Z");
    }

    #[tokio::test]
    async fn old_runs_of_removed_jobs_are_deleted_and_the_rest_stay() {
        let (db, _dir) = db().await;
        let mut old = started(1);
        old.job_id = "job-gone".into();
        old.id = "run-old".into();
        old.started_at = "2026-01-01T00:00:00.000Z".into();
        insert_run(&db, old).await.unwrap();
        let mut recent = started(2);
        recent.job_id = "job-gone".into();
        recent.id = "run-recent".into();
        recent.started_at = "2026-11-20T00:00:00.000Z".into();
        insert_run(&db, recent).await.unwrap();
        insert_run(&db, started(3)).await.unwrap();

        delete_removed(&db, vec!["job-a".into()], "2026-10-01T00:00:00.000Z".into())
            .await
            .unwrap();
        let left = load_recent(&db).await.unwrap();
        assert_eq!(left["job-gone"].len(), 1);
        assert_eq!(left["job-gone"][0].id, "run-recent");
        assert_eq!(left["job-a"].len(), 1);
    }

    #[test]
    fn the_default_file_has_no_job_and_the_documented_header() {
        let doc: toml::Table = toml::from_str(DEFAULT_FILE).unwrap();
        assert_eq!(doc.len(), 1);
        assert_eq!(doc["schema_version"].as_integer(), Some(1));
        assert!(DEFAULT_FILE.contains("# [[job]]"));
    }
}
