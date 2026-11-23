//! The log of a run: kept in memory while the run lives, written to SQLite in batches, read back
//! after the run. Section 03 6.11.9. Scheduled runs and idle runs share it (idle run ids start
//! with `irun-`); only this file touches the table `jobs_run_log`.
//!
//! Caps, so that a program that prints without end cannot fill the memory or the disk:
//! - one line keeps at most `LINE_MAX_CHARS` characters;
//! - a run keeps its first `HEAD_MAX` lines and a ring of its last `TAIL_MAX`; between them one
//!   warning says how many lines were left out;
//! - the API returns the last `API_MAX_LINES` lines, each cut to `API_LINE_CHARS`.
//!
//! Log lines are never copied into `tracing` output: a run's output may hold anything.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rain_types::time::iso_from_system_time;
use rain_types::{LogLevel, LogLine};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::core::clock::Clock;
use crate::core::error::{ApiError, ApiResult};
use crate::store::db::Db;

/// The most characters of one stored line.
pub const LINE_MAX_CHARS: usize = 2000;
/// The first lines of a run that are always kept.
pub const HEAD_MAX: usize = 4000;
/// The last lines of a run that are kept when the run printed more than `HEAD_MAX`.
pub const TAIL_MAX: usize = 500;
/// How many lines one answer of the API holds at most.
pub const API_MAX_LINES: usize = 2000;
/// An answer of the API cuts each line to this many characters.
pub const API_LINE_CHARS: usize = 1000;
/// Rows of `jobs_run_log` in total, scheduled and idle runs together.
pub const LOG_ROWS_MAX: i64 = 200_000;

#[derive(Debug, Clone)]
struct Row {
    ts: String,
    level: LogLevel,
    text: String,
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

fn level_of(name: &str) -> LogLevel {
    match name {
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// `text` cut to `max` characters (not bytes).
pub fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

/// How many lines a finished log holds, for the run's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogCounts {
    pub stored: u64,
    pub omitted: u64,
}

struct StoreShared {
    db: Db,
    clock: Arc<dyn Clock>,
    flush_gap: Duration,
    live: Mutex<HashMap<String, Arc<LogSink>>>,
}

/// Where the logs are. Cloning is cheap.
#[derive(Clone)]
pub struct LogStore {
    shared: Arc<StoreShared>,
}

impl LogStore {
    pub fn new(db: Db, clock: Arc<dyn Clock>, flush_gap: Duration) -> Self {
        Self {
            shared: Arc::new(StoreShared {
                db,
                clock,
                flush_gap,
                live: Mutex::default(),
            }),
        }
    }

    fn live(&self) -> MutexGuard<'_, HashMap<String, Arc<LogSink>>> {
        self.shared
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the log of a run. Needs a tokio runtime (the writer is a task that exists only
    /// while lines wait, and ends in `finish`).
    pub fn open(&self, run_id: &str) -> Arc<LogSink> {
        let sink = Arc::new(LogSink {
            run_id: run_id.to_owned(),
            store: self.clone(),
            state: Mutex::default(),
            wake: Notify::new(),
            done: CancellationToken::new(),
            writer: Mutex::new(None),
        });
        let writer = tokio::spawn(Arc::clone(&sink).write_loop());
        *sink.writer.lock().unwrap_or_else(PoisonError::into_inner) = Some(writer);
        self.live().insert(run_id.to_owned(), Arc::clone(&sink));
        sink
    }

    /// Whether the run's log is still being written.
    pub fn is_live(&self, run_id: &str) -> bool {
        self.live().contains_key(run_id)
    }

    /// The lines of a run, oldest first, in the form the API returns: at most the last
    /// `API_MAX_LINES`, each cut to `API_LINE_CHARS`. Empty for a run that has no lines.
    pub async fn read(&self, run_id: &str) -> ApiResult<Vec<LogLine>> {
        let live = self.live().get(run_id).cloned();
        let lines = match live {
            Some(sink) => sink.snapshot(),
            None => self.read_stored(run_id).await?,
        };
        Ok(for_api(lines))
    }

    async fn read_stored(&self, run_id: &str) -> ApiResult<Vec<LogLine>> {
        let run_id = run_id.to_owned();
        self.shared
            .db
            .call(move |conn| {
                let mut statement = conn.prepare_cached(
                    "SELECT ts, level, text FROM jobs_run_log WHERE run_id = ?1 ORDER BY seq",
                )?;
                let rows = statement.query_map([run_id], |row| {
                    Ok(LogLine {
                        ts: row.get(0)?,
                        level: level_of(&row.get::<_, String>(1)?),
                        text: row.get(2)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map_err(|e| e.into_api("Could not read the log"))
    }

    /// Removes the stored lines of one run (the only way idle logs are deleted).
    pub async fn delete(&self, run_id: &str) {
        let run_id = run_id.to_owned();
        let result = self
            .shared
            .db
            .call(move |conn| conn.execute("DELETE FROM jobs_run_log WHERE run_id = ?1", [run_id]))
            .await;
        if let Err(e) = result {
            warn!(error = %e, "could not delete the log of a run");
        }
    }

    /// Adds one line to the end of a run whose log was already closed (a run that was cut off by
    /// a stop). The line goes after the last stored one.
    pub async fn append(&self, run_id: &str, level: LogLevel, text: &str) {
        let run_id = run_id.to_owned();
        let row = Row {
            ts: iso_from_system_time(self.shared.clock.now()),
            level,
            text: cut_chars(text, LINE_MAX_CHARS),
        };
        let result = self
            .shared
            .db
            .call(move |conn| {
                let next: i64 = conn.query_row(
                    "SELECT COALESCE(MAX(seq) + 1, 0) FROM jobs_run_log WHERE run_id = ?1",
                    [&run_id],
                    |r| r.get(0),
                )?;
                conn.execute(
                    "INSERT INTO jobs_run_log (run_id, seq, ts, level, text) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![run_id, next, row.ts, level_name(row.level), row.text],
                )
            })
            .await;
        if let Err(e) = result {
            warn!(error = %e, "could not add a line to a log");
        }
    }

    /// Keeps the table under `LOG_ROWS_MAX`: the oldest runs lose their lines first.
    pub async fn trim_total(&self) {
        let result = self
            .shared
            .db
            .call(|conn| {
                loop {
                    let rows: i64 =
                        conn.query_row("SELECT COUNT(*) FROM jobs_run_log", [], |r| r.get(0))?;
                    if rows <= LOG_ROWS_MAX {
                        return Ok(());
                    }
                    // The oldest run loses its lines first, one run at a time so that no more is lost than
                    // needed; its row then says that no lines are left.
                    let victims: Vec<String> = conn
                        .prepare(
                            "SELECT run_id FROM jobs_run_log GROUP BY run_id
                             ORDER BY MIN(ts) LIMIT 1",
                        )?
                        .query_map([], |r| r.get(0))?
                        .collect::<Result<_, _>>()?;
                    if victims.is_empty() {
                        return Ok(());
                    }
                    for id in &victims {
                        conn.execute("DELETE FROM jobs_run_log WHERE run_id = ?1", [id])?;
                        conn.execute("UPDATE jobs_run SET log_lines = 0 WHERE id = ?1", [id])?;
                    }
                }
            })
            .await;
        if let Err(e) = result {
            warn!(error = %e, "could not trim the stored logs");
        }
    }
}

/// The last `API_MAX_LINES` lines with every line cut; a first warning says that earlier lines
/// are not shown.
fn for_api(mut lines: Vec<LogLine>) -> Vec<LogLine> {
    let cut_head = if lines.len() > API_MAX_LINES {
        let hidden = lines.len() - (API_MAX_LINES - 1);
        lines.drain(..hidden);
        Some((
            hidden,
            lines.first().map(|l| l.ts.clone()).unwrap_or_default(),
        ))
    } else {
        None
    };
    for line in &mut lines {
        if line.text.chars().count() > API_LINE_CHARS {
            line.text = cut_chars(&line.text, API_LINE_CHARS);
        }
    }
    if let Some((hidden, ts)) = cut_head {
        lines.insert(
            0,
            LogLine {
                ts,
                level: LogLevel::Warn,
                text: format!("{hidden} earlier lines are not shown"),
            },
        );
    }
    lines
}

#[derive(Default)]
struct SinkState {
    head: Vec<Row>,
    /// How many rows of `head` are already in the database.
    flushed: usize,
    ring: VecDeque<Row>,
    omitted: u64,
}

/// The log of one run.
pub struct LogSink {
    run_id: String,
    store: LogStore,
    state: Mutex<SinkState>,
    /// Wakes the writer when lines are waiting.
    wake: Notify,
    done: CancellationToken,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl LogSink {
    fn state(&self) -> MutexGuard<'_, SinkState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn info(&self, text: impl AsRef<str>) {
        self.line(LogLevel::Info, text);
    }

    pub fn warn(&self, text: impl AsRef<str>) {
        self.line(LogLevel::Warn, text);
    }

    pub fn error(&self, text: impl AsRef<str>) {
        self.line(LogLevel::Error, text);
    }

    /// Adds one line. Cheap and never blocks: the writer task stores it a moment later.
    pub fn line(&self, level: LogLevel, text: impl AsRef<str>) {
        let row = Row {
            ts: iso_from_system_time(self.store.shared.clock.now()),
            level,
            text: cut_chars(text.as_ref(), LINE_MAX_CHARS),
        };
        let mut state = self.state();
        if state.head.len() < HEAD_MAX {
            state.head.push(row);
            drop(state);
            self.wake.notify_one();
        } else {
            state.ring.push_back(row);
            if state.ring.len() > TAIL_MAX {
                state.ring.pop_front();
                state.omitted += 1;
            }
        }
    }

    /// The writer: sleeps until lines wait, waits a moment so that a burst becomes one batch,
    /// then stores the head lines. It exists only while the run lives (allowed timer T5).
    async fn write_loop(self: Arc<Self>) {
        loop {
            tokio::select! {
                () = self.wake.notified() => {}
                () = self.done.cancelled() => return,
            }
            tokio::select! {
                () = tokio::time::sleep(self.store.shared.flush_gap) => {}
                () = self.done.cancelled() => return,
            }
            self.flush().await;
        }
    }

    /// Writes the head lines that are not stored yet.
    async fn flush(&self) {
        let (rows, first) = {
            let mut state = self.state();
            let first = state.flushed;
            let rows: Vec<Row> = state.head[first..].to_vec();
            state.flushed = state.head.len();
            (rows, first)
        };
        self.store_rows(rows, first as i64).await;
    }

    async fn store_rows(&self, rows: Vec<Row>, first_seq: i64) {
        if rows.is_empty() {
            return;
        }
        let run_id = self.run_id.clone();
        let result = self
            .store
            .shared
            .db
            .call(move |conn| {
                let tx = conn.transaction()?;
                {
                    let mut insert = tx.prepare_cached(
                        "INSERT OR REPLACE INTO jobs_run_log (run_id, seq, ts, level, text)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                    )?;
                    for (offset, row) in rows.iter().enumerate() {
                        insert.execute(rusqlite::params![
                            run_id,
                            first_seq + offset as i64,
                            row.ts,
                            level_name(row.level),
                            row.text
                        ])?;
                    }
                }
                tx.commit()
            })
            .await;
        if let Err(e) = result {
            warn!(error = %e, "could not store log lines");
        }
    }

    /// Everything so far, oldest first: the head, the marker, the tail.
    pub fn snapshot(&self) -> Vec<LogLine> {
        let state = self.state();
        let mut lines: Vec<LogLine> = state.head.iter().map(to_line).collect();
        if state.omitted > 0 {
            lines.push(omitted_line(state.omitted, &state));
        }
        lines.extend(state.ring.iter().map(to_line));
        lines
    }

    /// Ends the log: stops the writer, stores what is left (the marker and the tail), and
    /// removes the run from the live list. Call it once, after the last `line`.
    pub async fn finish(&self) -> LogCounts {
        self.done.cancel();
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(writer) = writer {
            let _ = writer.await;
        }
        self.flush().await;

        let (tail, first_seq, omitted, head_len) = {
            let state = self.state();
            let mut tail = Vec::new();
            if state.omitted > 0 {
                let marker = omitted_line(state.omitted, &state);
                tail.push(Row {
                    ts: marker.ts,
                    level: marker.level,
                    text: marker.text,
                });
            }
            tail.extend(state.ring.iter().cloned());
            (
                tail,
                state.head.len() as i64,
                state.omitted,
                state.head.len(),
            )
        };
        let stored = (head_len + tail.len()) as u64;
        self.store_rows(tail, first_seq).await;
        self.store.live().remove(&self.run_id);
        LogCounts { stored, omitted }
    }
}

fn to_line(row: &Row) -> LogLine {
    LogLine {
        ts: row.ts.clone(),
        level: row.level,
        text: row.text.clone(),
    }
}

/// The warning between the head and the tail.
fn omitted_line(omitted: u64, state: &SinkState) -> LogLine {
    let ts = state
        .ring
        .front()
        .or(state.head.last())
        .map(|row| row.ts.clone())
        .unwrap_or_default();
    LogLine {
        ts,
        level: LogLevel::Warn,
        text: format!("{omitted} lines omitted"),
    }
}

/// An error for a run that nobody knows.
pub fn unknown_run() -> ApiError {
    ApiError::not_found("Run not found").with_reason("unknownRun")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::clock::SystemClock;
    use crate::modules::jobs::MIGRATIONS;
    use crate::store::migrations;

    async fn store() -> (LogStore, tempfile::TempDir) {
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
        (
            LogStore::new(db, Arc::new(SystemClock), Duration::from_millis(10)),
            dir,
        )
    }

    #[test]
    fn a_line_is_cut_by_characters_not_bytes() {
        assert_eq!(cut_chars("abcdef", 3), "abc");
        assert_eq!(cut_chars("ab", 3), "ab");
        assert_eq!(cut_chars("\u{e9}\u{e9}\u{e9}\u{e9}", 2), "\u{e9}\u{e9}");
    }

    #[tokio::test]
    async fn trimming_the_log_table_sets_the_line_count_of_the_trimmed_runs_to_zero() {
        let (store, _dir) = store().await;
        // Two runs: an old one with more lines than the cap allows together with a newer one.
        store
            .shared
            .db
            .call(|conn| {
                for (id, ts) in [("run-old", "2026-01-01T00:00:00.000Z"), ("run-new", "2026-02-01T00:00:00.000Z")] {
                    conn.execute(
                        "INSERT INTO jobs_run (id, job_id, trigger_kind, status, started_at, log_lines)
                         VALUES (?1, 'job-a', 'manual', 'success', ?2, 3)",
                        [id, ts],
                    )?;
                    let rows = if id == "run-old" { LOG_ROWS_MAX } else { 3 };
                    conn.execute(
                        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i + 1 < ?3)
                         INSERT INTO jobs_run_log (run_id, seq, ts, level, text)
                         SELECT ?1, i, ?2, 'info', 'x' FROM n",
                        rusqlite::params![id, ts, rows],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        store.trim_total().await;
        let counts: (i64, i64, i64) = store
            .shared
            .db
            .call(|conn| {
                Ok((
                    conn.query_row("SELECT COUNT(*) FROM jobs_run_log", [], |r| r.get(0))?,
                    conn.query_row(
                        "SELECT log_lines FROM jobs_run WHERE id = 'run-old'",
                        [],
                        |r| r.get(0),
                    )?,
                    conn.query_row(
                        "SELECT log_lines FROM jobs_run WHERE id = 'run-new'",
                        [],
                        |r| r.get(0),
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(
            counts,
            (3, 0, 3),
            "the old run lost its lines and says so; the new one keeps them"
        );
    }

    #[tokio::test]
    async fn log_sink_caps() {
        let (store, _dir) = store().await;
        let sink = store.open("run-test-1");
        for n in 0..(HEAD_MAX + TAIL_MAX + 100) {
            sink.info(format!("line {n}"));
        }
        sink.warn("x".repeat(5000));

        // While the run lives, the lines come from memory.
        let live = sink.snapshot();
        assert_eq!(live[0].text, "line 0");
        assert!(live.iter().any(|l| l.text == "101 lines omitted"));

        let counts = sink.finish().await;
        assert_eq!(counts.omitted, 101);
        assert_eq!(counts.stored as usize, HEAD_MAX + 1 + TAIL_MAX);

        let stored = store.read_stored("run-test-1").await.unwrap();
        assert_eq!(stored.len(), HEAD_MAX + 1 + TAIL_MAX);
        assert_eq!(stored[HEAD_MAX].text, "101 lines omitted");
        assert_eq!(stored[HEAD_MAX].level, LogLevel::Warn);
        assert_eq!(stored.last().unwrap().text.chars().count(), LINE_MAX_CHARS);
        assert!(!store.is_live("run-test-1"));
    }

    #[tokio::test]
    async fn the_api_returns_the_last_lines_with_a_marker_and_cut_text() {
        let (store, _dir) = store().await;
        let sink = store.open("run-test-2");
        for n in 0..2500 {
            sink.info(format!("line {n}"));
        }
        sink.info("y".repeat(1500));
        sink.finish().await;

        let lines = store.read("run-test-2").await.unwrap();
        assert_eq!(lines.len(), API_MAX_LINES);
        assert_eq!(lines[0].level, LogLevel::Warn);
        assert!(lines[0].text.ends_with("earlier lines are not shown"));
        assert_eq!(lines.last().unwrap().text.chars().count(), API_LINE_CHARS);
    }

    #[tokio::test]
    async fn a_running_log_is_readable_and_an_unknown_run_is_empty() {
        let (store, _dir) = store().await;
        let sink = store.open("run-test-3");
        sink.info("first");
        sink.error("second");
        let lines = store.read("run-test-3").await.unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].level, LogLevel::Error);
        assert!(store.read("run-nobody").await.unwrap().is_empty());
        sink.finish().await;
    }

    #[tokio::test]
    async fn delete_removes_the_stored_lines_of_one_run() {
        let (store, _dir) = store().await;
        for id in ["irun-a", "irun-b"] {
            let sink = store.open(id);
            sink.info("hello");
            sink.finish().await;
        }
        store.delete("irun-a").await;
        assert!(store.read("irun-a").await.unwrap().is_empty());
        assert_eq!(store.read("irun-b").await.unwrap().len(), 1);
    }
}
