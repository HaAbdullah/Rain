//! The table of sessions that ended (`sessions_ended`), so that "Done" and "Failed" survive a
//! restart of the backend. Section 03 5.3.
//!
//! Retention: rows older than 24 hours are deleted at start and whenever a session ends, and at
//! most 50 rows are kept (the oldest go first). Open permission requests are never stored.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rain_types::time::iso_from_system_time;
use rain_types::{SessionEvent, SessionHost, Status};

use super::book::{ENDED_KEEP, ENDED_KEEP_MAX, Ended, SessionRec, Turn};
use crate::store::db::{Db, DbError};
use crate::store::migrations::Migration;

pub const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "create_sessions_ended",
    sql: include_str!("migrations/0001_sessions.sql"),
}];

/// One row of `sessions_ended`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedRow {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub project_id: Option<String>,
    pub branch: Option<String>,
    pub model: String,
    pub host: String,
    /// `success` or `failed`.
    pub status: String,
    pub started_at: String,
    pub ended_at: String,
    pub last_activity_at: String,
    pub tokens: u64,
    pub last_message: String,
    pub transcript_path: Option<String>,
    pub recent_json: String,
}

/// Stores a session that ended and deletes what is too old or too many.
pub async fn insert(db: &Db, row: EndedRow, now: SystemTime) -> Result<(), DbError> {
    let cutoff = iso_from_system_time(now.checked_sub(ENDED_KEEP).unwrap_or(UNIX_EPOCH));
    db.call(move |conn| {
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT OR REPLACE INTO sessions_ended
               (id, title, cwd, project_id, branch, model, host, status, started_at, ended_at,
                last_activity_at, tokens, last_message, transcript_path, recent_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                row.id,
                row.title,
                row.cwd,
                row.project_id,
                row.branch,
                row.model,
                row.host,
                row.status,
                row.started_at,
                row.ended_at,
                row.last_activity_at,
                i64::try_from(row.tokens).unwrap_or(i64::MAX),
                row.last_message,
                row.transcript_path,
                row.recent_json,
            ],
        )?;
        prune_in(&transaction, &cutoff)?;
        transaction.commit()
    })
    .await
}

/// The rows of the last 24 hours, newest first. Old rows are deleted first.
pub async fn load(db: &Db, now: SystemTime) -> Result<Vec<EndedRow>, DbError> {
    let cutoff = iso_from_system_time(now.checked_sub(ENDED_KEEP).unwrap_or(UNIX_EPOCH));
    db.call(move |conn| {
        prune_in(conn, &cutoff)?;
        let mut statement = conn.prepare(
            "SELECT id, title, cwd, project_id, branch, model, host, status, started_at, ended_at,
                    last_activity_at, tokens, last_message, transcript_path, recent_json
             FROM sessions_ended ORDER BY ended_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(EndedRow {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: row.get(2)?,
                project_id: row.get(3)?,
                branch: row.get(4)?,
                model: row.get(5)?,
                host: row.get(6)?,
                status: row.get(7)?,
                started_at: row.get(8)?,
                ended_at: row.get(9)?,
                last_activity_at: row.get(10)?,
                tokens: u64::try_from(row.get::<_, i64>(11)?).unwrap_or(0),
                last_message: row.get(12)?,
                transcript_path: row.get(13)?,
                recent_json: row.get(14)?,
            })
        })?;
        rows.collect()
    })
    .await
}

fn prune_in(conn: &rusqlite::Connection, cutoff: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sessions_ended WHERE ended_at < ?1", [cutoff])?;
    conn.execute(
        "DELETE FROM sessions_ended WHERE id NOT IN
           (SELECT id FROM sessions_ended ORDER BY ended_at DESC LIMIT ?1)",
        [ENDED_KEEP_MAX as i64],
    )?;
    Ok(())
}

// ---- record <-> row ----

/// The row for a record that has ended. `None` for a record that is still live.
pub fn row_of(rec: &SessionRec, project_id: Option<String>) -> Option<EndedRow> {
    let ended = rec.ended?;
    let (status, _) = rec.status();
    let recent: Vec<&SessionEvent> = rec.recent.iter().collect();
    Some(EndedRow {
        id: rec.id.clone(),
        title: rec.title.resolve(&rec.cwd),
        cwd: rec.cwd.clone(),
        project_id,
        branch: rec.branch.clone(),
        model: super::transcript::model_label(rec.model.as_deref().unwrap_or("")),
        host: match rec.host() {
            SessionHost::Terminal => "terminal",
            SessionHost::Desktop => "desktop",
            SessionHost::Other => "other",
        }
        .to_owned(),
        status: if status == Status::Success {
            "success"
        } else {
            "failed"
        }
        .to_owned(),
        started_at: iso_from_system_time(rec.started_at),
        ended_at: iso_from_system_time(ended.at),
        last_activity_at: iso_from_system_time(rec.last_activity()),
        tokens: rec.tokens,
        last_message: rec.last_message.clone(),
        transcript_path: rec
            .transcript_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        recent_json: serde_json::to_string(&recent).unwrap_or_else(|_| "[]".to_owned()),
    })
}

/// The record for a stored row. It has no process and no registry; it only shows what was
/// stored, and its transcript can still be read.
pub fn rec_of(row: EndedRow) -> Option<SessionRec> {
    let ended_at = parse_iso(&row.ended_at)?;
    let started_at = parse_iso(&row.started_at).unwrap_or(ended_at);
    let mut rec = SessionRec::new(&row.id, started_at);
    rec.cwd = row.cwd;
    rec.branch = row.branch;
    rec.model = Some(row.model);
    rec.title.stored = Some(row.title);
    rec.last_hook_at = parse_iso(&row.last_activity_at).unwrap_or(ended_at);
    rec.turn = Turn::Idle;
    rec.tokens = row.tokens;
    rec.last_message = row.last_message;
    rec.transcript_path = row.transcript_path.map(Into::into);
    rec.recent = serde_json::from_str::<Vec<SessionEvent>>(&row.recent_json)
        .unwrap_or_default()
        .into_iter()
        .collect();
    // The host is kept through the hook's entrypoint field, which `host()` reads.
    rec.env_entrypoint = Some(
        match row.host.as_str() {
            "terminal" => "cli",
            "desktop" => "claude-desktop",
            _ => "other",
        }
        .to_owned(),
    );
    rec.ended = Some(Ended {
        at: ended_at,
        ok: row.status == "success",
    });
    Some(rec)
}

/// `2026-11-21T09:30:00.123Z` as a time. `None` for any other text.
pub fn parse_iso(text: &str) -> Option<SystemTime> {
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || !text.ends_with('Z')
    {
        return None;
    }
    let number = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    let millis = match text.get(19..text.len() - 1) {
        Some("") => 0,
        Some(fraction) if fraction.starts_with('.') => {
            let digits: String = fraction[1..].chars().take(3).collect();
            format!("{digits:0<3}").parse::<i64>().ok()?
        }
        _ => return None,
    };
    // Days since 1970-01-01 (Howard Hinnant's algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    let total_millis = u64::try_from(seconds).ok()? * 1000 + u64::try_from(millis).ok()?;
    Some(UNIX_EPOCH + Duration::from_millis(total_millis))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrations;

    fn row(id: &str, ended_at: &str) -> EndedRow {
        EndedRow {
            id: id.into(),
            title: "A title".into(),
            cwd: r"C:\work\demo".into(),
            project_id: None,
            branch: Some("main".into()),
            model: "sonnet 4.5".into(),
            host: "terminal".into(),
            status: "success".into(),
            started_at: "2026-11-21T09:00:00.000Z".into(),
            ended_at: ended_at.into(),
            last_activity_at: ended_at.into(),
            tokens: 1234,
            last_message: "Done.".into(),
            transcript_path: Some(r"C:\t\a.jsonl".into()),
            recent_json: "[]".into(),
        }
    }

    async fn database() -> (tempfile::TempDir, Db) {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::new(temp.path().join("rain.db"));
        db.open(1).await.unwrap();
        let failed = migrations::run_all(
            &db,
            vec![("sessions", MIGRATIONS)],
            "2026-11-21T00:00:00.000Z".into(),
        )
        .await
        .unwrap();
        assert!(failed.is_empty());
        (temp, db)
    }

    #[test]
    fn iso_times_round_trip() {
        for text in [
            "2026-11-21T09:30:00.123Z",
            "2024-02-29T23:59:59.000Z",
            "1970-01-01T00:00:00.000Z",
        ] {
            let time = parse_iso(text).unwrap();
            assert_eq!(iso_from_system_time(time), text);
        }
        assert_eq!(
            parse_iso("2026-11-21T09:30:00Z"),
            parse_iso("2026-11-21T09:30:00.000Z")
        );
        for bad in ["", "yesterday", "2026-11-21", "2026-11-21 09:30:00.000Z"] {
            assert_eq!(parse_iso(bad), None, "{bad:?}");
        }
    }

    #[tokio::test]
    async fn a_row_is_stored_and_read_back() {
        let (_temp, db) = database().await;
        let now = parse_iso("2026-11-21T12:00:00.000Z").unwrap();
        insert(&db, row("a", "2026-11-21T11:00:00.000Z"), now)
            .await
            .unwrap();
        insert(&db, row("b", "2026-11-21T11:30:00.000Z"), now)
            .await
            .unwrap();
        let rows = load(&db, now).await.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["b", "a"],
            "newest first"
        );
        assert_eq!(rows[1], row("a", "2026-11-21T11:00:00.000Z"));
    }

    #[tokio::test]
    async fn old_rows_and_rows_past_fifty_are_deleted() {
        let (_temp, db) = database().await;
        let now = parse_iso("2026-11-21T12:00:00.000Z").unwrap();
        insert(
            &db,
            row("old", "2026-11-20T11:00:00.000Z"),
            parse_iso("2026-11-20T11:30:00.000Z").unwrap(),
        )
        .await
        .unwrap();
        for n in 0..55 {
            insert(
                &db,
                row(
                    &format!("r{n:02}"),
                    &format!("2026-11-21T11:{n:02}:00.000Z"),
                ),
                now,
            )
            .await
            .unwrap();
        }
        let rows = load(&db, now).await.unwrap();
        assert_eq!(rows.len(), ENDED_KEEP_MAX);
        assert!(rows.iter().all(|r| r.id != "old"), "older than 24 hours");
        assert_eq!(rows[0].id, "r54");
        assert!(
            rows.iter().all(|r| r.id != "r04"),
            "the oldest past 50 went first"
        );
    }

    #[test]
    fn a_row_makes_an_ended_record_that_shows_what_was_stored() {
        let mut stored = row("s1", "2026-11-21T11:00:00.000Z");
        stored.host = "desktop".into();
        stored.status = "failed".into();
        stored.recent_json =
            r#"[{"at":"2026-11-21T10:59:00.000Z","kind":"tool","text":"Bash: ls"}]"#.into();
        let rec = rec_of(stored).unwrap();
        let api = rec.to_api(&|_| None);
        assert_eq!(api.status, Status::Failed);
        assert!(!api.alive);
        assert_eq!(api.title, "A title");
        assert_eq!(api.host, SessionHost::Desktop);
        assert_eq!(api.recent.len(), 1);
        assert_eq!(api.model, "sonnet 4.5");
        // And back to a row: the same data.
        let again = row_of(&rec, None).unwrap();
        assert_eq!(
            (again.status.as_str(), again.host.as_str()),
            ("failed", "desktop")
        );
    }

    #[test]
    fn a_live_record_has_no_row() {
        assert!(row_of(&SessionRec::new("s", UNIX_EPOCH), None).is_none());
    }
}
