//! The SQLite database: history and caches. Foundation section 10.5.
//!
//! One dedicated OS thread owns the one `Connection`. It waits on a channel (so it costs no CPU
//! while idle), runs the closures that `call` sends it, and answers over a second channel. No
//! `Connection` is ever used on an async task, so the two async workers never wait for a disk.
//! The volume is tiny, so reads use the same thread.
//!
//! Losing `rain.db` loses history and never a user definition: those are TOML files.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use rain_types::limits::SQLITE_CACHE_KIB;
use rusqlite::Connection;
use tokio::sync::oneshot;
use tracing::{error, info, warn};

use crate::core::blocking;
use crate::core::error::ApiError;

/// A closure that the database thread runs.
type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// How long a statement waits for a lock before it fails.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// `call` was used before `open`, or after `close`.
    #[error("the database is not open")]
    NotOpen,
    /// The database thread ended while a call was waiting (it panicked).
    #[error("the database thread ended")]
    ThreadEnded,
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}

impl DbError {
    /// The error for the user. The caller says what it was doing (foundation 4.5: no blanket
    /// conversion), the technical cause goes to the log and not into the message.
    pub fn into_api(self, doing: &str) -> ApiError {
        match self {
            Self::NotOpen => ApiError::unavailable(
                "database",
                format!("{doing}: the database is not available"),
            )
            .with_reason("dbUnavailable"),
            Self::ThreadEnded | Self::Sql(_) => {
                error!(error = %self, "a database call failed");
                ApiError::failed(doing.to_owned())
                    .with_source("database")
                    .with_reason("dbFailed")
            }
        }
    }
}

/// How the database was opened. Anything but `Normal` is reported to the user as a config issue
/// of the file `rain.db`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    Normal,
    /// The file was damaged. It was renamed to this path and a new, empty database was created.
    Recovered {
        damaged_file: PathBuf,
    },
    /// No file could be used at all (the folder is read-only, the disk is full). Rain runs on a
    /// database in memory: history is lost when the backend stops.
    InMemory,
}

/// The handle to the database. Cloning is cheap: all clones use the same thread.
#[derive(Clone)]
pub struct Db {
    shared: Arc<Shared>,
}

struct Shared {
    path: PathBuf,
    open: Mutex<Option<Open>>,
}

struct Open {
    sender: Sender<Job>,
    thread: JoinHandle<()>,
}

impl Db {
    /// A handle for the database file at `path`. Nothing is opened yet.
    pub fn new(path: PathBuf) -> Self {
        Self {
            shared: Arc::new(Shared {
                path,
                open: Mutex::new(None),
            }),
        }
    }

    /// Opens the file and starts the database thread. `now_ms` names a damaged file that has to
    /// be moved away (`rain.db.corrupt-<now_ms>`).
    ///
    /// Corruption policy: when the file cannot be opened or `PRAGMA quick_check` is not `ok`, the
    /// file is renamed and a new one is started; when even that fails, the database lives in
    /// memory. The backend always has a database.
    pub async fn open(&self, now_ms: u128) -> Result<OpenOutcome, DbError> {
        let path = self.shared.path.clone();
        let (connection, outcome) = blocking::run(move || open_connection(&path, now_ms))
            .await
            .map_err(|_| DbError::ThreadEnded)?;

        let (sender, receiver) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("rain-db".to_owned())
            .spawn(move || serve(connection, receiver))
            .map_err(|_| DbError::ThreadEnded)?;
        *self.lock() = Some(Open { sender, thread });
        Ok(outcome)
    }

    /// Runs `work` on the database thread and returns its result. The closure gets the one
    /// connection; for several statements that belong together it opens a transaction.
    pub async fn call<T, F>(&self, work: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |connection| {
            // The caller may have given up waiting; then nobody needs the answer.
            let _ = reply.send(work(connection));
        });
        {
            let guard = self.lock();
            let open = guard.as_ref().ok_or(DbError::NotOpen)?;
            open.sender.send(job).map_err(|_| DbError::ThreadEnded)?;
        }
        answer
            .await
            .map_err(|_| DbError::ThreadEnded)?
            .map_err(DbError::Sql)
    }

    /// The last step of a stop: `PRAGMA wal_checkpoint(TRUNCATE)` so that game mode leaves one
    /// clean file, then the thread ends and the file is closed. Further calls answer `NotOpen`.
    pub async fn close(&self) {
        // The checkpoint runs on the database thread like every other statement. A failure is
        // logged: the data is safe in the write-ahead log anyway.
        if let Err(e) = self
            .call(|connection| {
                connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            })
            .await
        {
            warn!(error = %e, "the final database checkpoint failed");
        }
        let Some(open) = self.lock().take() else {
            return;
        };
        drop(open.sender); // the thread leaves its loop and closes the connection
        // Joining waits for the file to close, so it must not block an async worker.
        let _ = blocking::run(move || open.thread.join()).await;
        info!("database closed");
    }

    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Open>> {
        // The guarded value is replaced as a whole, so a poisoned lock is still consistent.
        self.shared
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The body of the database thread: runs jobs until every sender is gone.
fn serve(mut connection: Connection, jobs: mpsc::Receiver<Job>) {
    while let Ok(job) = jobs.recv() {
        job(&mut connection);
    }
}

// ---- opening the file ----

fn open_connection(path: &Path, now_ms: u128) -> (Connection, OpenOutcome) {
    if let Some(folder) = path.parent()
        && let Err(e) = std::fs::create_dir_all(folder)
    {
        error!(error = %e, "could not create the local folder of the database");
    }
    match open_file(path) {
        Ok(connection) => return (connection, OpenOutcome::Normal),
        Err(e) => error!(error = %e, "the database file is damaged or cannot be opened"),
    }
    let damaged_file = match move_away(path, now_ms) {
        Ok(moved_to) => moved_to,
        Err(e) => {
            error!(error = %e, "the damaged database could not be moved; using memory");
            return (in_memory(), OpenOutcome::InMemory);
        }
    };
    match open_file(path) {
        Ok(connection) => (connection, OpenOutcome::Recovered { damaged_file }),
        Err(e) => {
            error!(error = %e, "a new database file could not be created; using memory");
            (in_memory(), OpenOutcome::InMemory)
        }
    }
}

/// Opens the file, sets the pragmas of foundation 10.5 and checks it is sound.
fn open_file(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    // `journal_mode` answers with a row, so it needs the variant that reads it.
    let mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "journal mode is {mode}, not wal"
        )));
    }
    set_pragmas(&connection)?;
    let verdict: String = connection.pragma_query_value(None, "quick_check", |row| row.get(0))?;
    if verdict != "ok" {
        return Err(rusqlite::Error::InvalidParameterName(
            "quick_check found damage".to_owned(),
        ));
    }
    Ok(connection)
}

fn in_memory() -> Connection {
    // Opening a database in memory does not touch the disk and cannot fail for a normal reason.
    let connection = Connection::open_in_memory().expect("SQLite can open a database in memory");
    if let Err(e) = set_pragmas(&connection) {
        warn!(error = %e, "could not set the pragmas of the in-memory database");
    }
    connection
}

fn set_pragmas(connection: &Connection) -> rusqlite::Result<()> {
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "cache_size", -i64::from(SQLITE_CACHE_KIB))?;
    connection.pragma_update(None, "mmap_size", 0)?;
    Ok(())
}

/// Renames the damaged file (and its write-ahead log and shared-memory file, which belong to it)
/// to `rain.db.corrupt-<now_ms>`. Returns the new name of the main file.
fn move_away(path: &Path, now_ms: u128) -> std::io::Result<PathBuf> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let target = path.with_file_name(format!("{name}.corrupt-{now_ms}"));
    if path.exists() {
        std::fs::rename(path, &target)?;
    }
    for suffix in ["-wal", "-shm"] {
        let companion = path.with_file_name(format!("{name}{suffix}"));
        if companion.exists() {
            // A leftover log of the damaged file would be applied to the new, empty one.
            let moved = path.with_file_name(format!("{name}.corrupt-{now_ms}{suffix}"));
            std::fs::rename(&companion, moved)?;
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Db) {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::new(temp.path().join("local").join("rain.db"));
        (temp, db)
    }

    #[tokio::test]
    async fn a_call_before_open_says_the_database_is_not_open() {
        let (_temp, db) = temp_db();
        let error = db.call(|_| Ok(1)).await.unwrap_err();
        assert!(matches!(error, DbError::NotOpen));
        assert_eq!(
            error.into_api("Could not read").body.reason.as_deref(),
            Some("dbUnavailable")
        );
    }

    #[tokio::test]
    async fn the_file_is_created_with_the_pragmas_of_the_foundation() {
        let (_temp, db) = temp_db();
        assert_eq!(db.open(1).await.unwrap(), OpenOutcome::Normal);

        let (mode, sync, keys, cache, mmap) = db
            .call(|c| {
                let pragma = |name: &str| c.pragma_query_value(None, name, |r| r.get::<_, i64>(0));
                let mode: String = c.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
                Ok((
                    mode,
                    pragma("synchronous")?,
                    pragma("foreign_keys")?,
                    pragma("cache_size")?,
                    pragma("mmap_size")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(sync, 1, "NORMAL");
        assert_eq!(keys, 1);
        assert_eq!(cache, -2048);
        assert_eq!(mmap, 0);
        assert!(db.path().exists());
    }

    #[tokio::test]
    async fn a_value_written_by_one_call_is_read_by_the_next() {
        let (_temp, db) = temp_db();
        db.open(1).await.unwrap();
        db.call(|c| {
            c.execute_batch("CREATE TABLE test_item (id INTEGER PRIMARY KEY, name TEXT)")?;
            c.execute("INSERT INTO test_item (name) VALUES (?1)", ["one"])?;
            Ok(())
        })
        .await
        .unwrap();
        let name: String = db
            .call(|c| c.query_row("SELECT name FROM test_item", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(name, "one");
    }

    #[tokio::test]
    async fn a_sql_error_comes_back_as_an_error_and_the_thread_goes_on() {
        let (_temp, db) = temp_db();
        db.open(1).await.unwrap();
        let error = db.call(|c| c.execute_batch("NOT SQL")).await.unwrap_err();
        assert!(matches!(error, DbError::Sql(_)));
        assert_eq!(db.call(|_| Ok(5)).await.unwrap(), 5);
    }

    #[tokio::test]
    async fn the_data_survives_a_close_and_a_new_open_and_the_log_is_checkpointed() {
        let (_temp, db) = temp_db();
        db.open(1).await.unwrap();
        db.call(|c| {
            c.execute_batch(
                "CREATE TABLE test_item (name TEXT); INSERT INTO test_item VALUES ('kept')",
            )
        })
        .await
        .unwrap();
        db.close().await;

        let wal = db.path().with_file_name("rain.db-wal");
        assert!(
            !wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0,
            "the checkpoint left no log behind"
        );
        assert!(matches!(db.call(|_| Ok(())).await, Err(DbError::NotOpen)));

        db.open(2).await.unwrap();
        let name: String = db
            .call(|c| c.query_row("SELECT name FROM test_item", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(name, "kept");
    }

    #[tokio::test]
    async fn a_damaged_file_is_moved_aside_and_a_new_database_starts() {
        let (_temp, db) = temp_db();
        std::fs::create_dir_all(db.path().parent().unwrap()).unwrap();
        std::fs::write(db.path(), vec![0x42u8; 4096]).unwrap();

        let outcome = db.open(1234).await.unwrap();
        let OpenOutcome::Recovered { damaged_file } = outcome else {
            panic!("expected a recovery, got {outcome:?}");
        };
        assert!(damaged_file.ends_with("rain.db.corrupt-1234"));
        assert!(
            damaged_file.exists(),
            "the damaged file is kept for the user"
        );
        assert_eq!(db.call(|_| Ok(7)).await.unwrap(), 7);
    }

    #[tokio::test]
    async fn when_no_file_can_be_used_the_database_lives_in_memory() {
        let temp = tempfile::tempdir().unwrap();
        // The "file" is a folder, and it cannot be renamed away because it is not empty...
        // a path whose parent is a file cannot be created at all:
        let blocker = temp.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let db = Db::new(blocker.join("rain.db"));

        assert_eq!(db.open(1).await.unwrap(), OpenOutcome::InMemory);
        assert_eq!(db.call(|_| Ok(3)).await.unwrap(), 3);
    }
}
