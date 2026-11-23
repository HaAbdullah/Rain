//! Logging to a file, with a size budget. Foundation sections 10.7 and 11.7.
//!
//! The setup is the one proven in the spike `crates` (`q6_tracing.rs`, `log_budget.rs`): a daily
//! rolling file, the newest 7 kept, a writer thread that never blocks the caller and drops lines
//! when it falls behind, and a size budget on top because the rolling appender has none.
//! Timestamps and file dates are UTC.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rain_types::limits::{LOG_FILES_KEPT, LOG_QUEUE_LINES, LOG_SIZE_BUDGET};
use tracing::error;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

use crate::config::Config;

/// The log file names are `backend.<date>.log`.
const FILE_PREFIX: &str = "backend";

/// Keep this alive until the program ends: dropping it writes out the lines that are still queued.
pub struct LogGuard {
    _worker: WorkerGuard,
}

/// The folder of the log files.
pub fn log_dir(config: &Config) -> PathBuf {
    config.local_dir.join("logs")
}

/// Starts logging for the whole process. Call it once, from `main`.
pub fn init(config: &Config) -> anyhow::Result<LogGuard> {
    let dir = log_dir(config);
    fs::create_dir_all(&dir)?;
    // A failure to trim old logs must not stop the backend.
    if let Err(e) = enforce_size_budget(&dir, FILE_PREFIX, LOG_SIZE_BUDGET) {
        eprintln!("could not trim the old log files: {e}");
    }

    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(&dir)?;
    let (writer, guard) = NonBlockingBuilder::default()
        .buffered_lines_limit(LOG_QUEUE_LINES)
        .lossy(true)
        .finish(appender);

    let filter = EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let file_layer = fmt::layer().with_writer(writer).with_ansi(false);
    // The console layer exists only with `--console`; `Option<Layer>` is a layer that does nothing
    // when it is `None`.
    let console_layer = config
        .log_to_console
        .then(|| fmt::layer().with_writer(io::stderr));
    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()?;

    // The backend has no console, so a panic must go to the log or nobody will ever see it.
    std::panic::set_hook(Box::new(|info| error!(panic = %info, "panic")));
    Ok(LogGuard { _worker: guard })
}

/// The rolling appender limits the number of files but not their size. This deletes the oldest
/// files of this program until the folder fits the budget. It never deletes the newest file.
/// Returns the number of deleted files.
pub fn enforce_size_budget(dir: &Path, prefix: &str, max_total_bytes: u64) -> io::Result<usize> {
    let mut files: Vec<(String, u64, PathBuf)> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            Some((name, entry.metadata().ok()?.len(), entry.path()))
        })
        .collect();
    files.sort(); // the names hold the date, so name order is age order
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    let mut removed = 0;
    while total > max_total_bytes && files.len() > 1 {
        let (_, len, path) = files.remove(0);
        if fs::remove_file(&path).is_ok() {
            total -= len;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_files_go_first_and_the_newest_stays() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=5 {
            fs::write(
                dir.path().join(format!("backend.2026-11-0{day}.log")),
                vec![b'x'; 1000],
            )
            .unwrap();
        }
        fs::write(dir.path().join("other.txt"), vec![b'x'; 5000]).unwrap(); // not ours

        assert_eq!(enforce_size_budget(dir.path(), "backend", 2500).unwrap(), 3);

        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "backend.2026-11-04.log",
                "backend.2026-11-05.log",
                "other.txt"
            ]
        );
    }

    #[test]
    fn a_single_file_over_the_budget_is_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("backend.2026-11-05.log"), vec![b'x'; 1000]).unwrap();
        assert_eq!(enforce_size_budget(dir.path(), "backend", 10).unwrap(), 0);
    }

    #[test]
    fn the_log_folder_is_below_the_local_folder() {
        let config = Config::for_test("p", "data".into(), PathBuf::from("local"));
        assert_eq!(log_dir(&config), PathBuf::from("local").join("logs"));
    }
}
