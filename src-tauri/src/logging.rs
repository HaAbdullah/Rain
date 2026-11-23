//! The shell's log file: `<local folder>\logs\shell.<date>.log` (foundation section 10.7).
//!
//! The setup is the backend's: a daily rolling file, the newest 7 kept, a writer thread that never
//! blocks the caller, and a size budget. In a debug build the log also goes to the console of
//! `npm run tauri dev`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rain_types::env::{
    ENV_DATA_DIR, ENV_LOCAL_DIR, ENV_LOG, app_folder_name, sandbox as is_sandbox,
};
use rain_types::limits::{LOG_FILES_KEPT, LOG_QUEUE_LINES, LOG_SIZE_BUDGET};
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

/// The log file names are `shell.<date>.log`.
const FILE_PREFIX: &str = "shell";
const DEFAULT_FILTER: &str = "info";

/// The machine-local folder of Rain (foundation section 10.1), by the same rule as the backend's:
/// `RAIN_LOCAL_DIR`, else `<RAIN_DATA_DIR>.local`, else `%LOCALAPPDATA%\Rain` (`Rain-dev` in a
/// sandbox). `None` when none of them can be worked out.
fn local_dir_for(
    local_dir_var: Option<PathBuf>,
    data_dir_var: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    sandbox: bool,
) -> Option<PathBuf> {
    if let Some(dir) = local_dir_var {
        return Some(dir);
    }
    if let Some(data_dir) = data_dir_var {
        let mut dir = data_dir.into_os_string();
        dir.push(".local");
        return Some(PathBuf::from(dir));
    }
    local_app_data.map(|base| base.join(app_folder_name(sandbox)))
}

fn local_dir() -> Option<PathBuf> {
    let path = |name: &str| std::env::var_os(name).map(PathBuf::from);
    local_dir_for(
        path(ENV_LOCAL_DIR),
        path(ENV_DATA_DIR),
        path("LOCALAPPDATA"),
        is_sandbox(),
    )
}

/// Starts logging for the whole process. Keep the returned guard until the program ends:
/// dropping it writes out the lines that are still queued.
///
/// A log that cannot be opened is no reason to stop the shell: the log then goes only to the
/// console (which a release build does not have).
pub fn init() -> Option<WorkerGuard> {
    let (file_layer, guard) = match open_log_file() {
        Ok((writer, guard)) => (
            Some(fmt::layer().with_writer(writer).with_ansi(false)),
            Some(guard),
        ),
        Err(error) => {
            eprintln!("could not open the shell log file: {error}");
            (None, None)
        }
    };
    let filter = std::env::var(ENV_LOG)
        .ok()
        .and_then(|text| EnvFilter::try_new(text).ok())
        .unwrap_or_else(|| EnvFilter::new(DEFAULT_FILTER));
    // `Option<Layer>` is a layer that does nothing when it is `None`.
    let console_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(io::stderr));
    let started = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init();
    if let Err(error) = started {
        eprintln!("could not start logging: {error}");
    }
    log_panics();
    guard
}

fn open_log_file() -> io::Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    let dir = local_dir()
        .ok_or_else(|| io::Error::other("the local folder of Rain is unknown"))?
        .join("logs");
    fs::create_dir_all(&dir)?;
    // A failure to trim old logs must not stop the shell.
    if let Err(error) = enforce_size_budget(&dir, FILE_PREFIX, LOG_SIZE_BUDGET) {
        eprintln!("could not trim the old log files: {error}");
    }
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(&dir)
        .map_err(io::Error::other)?;
    Ok(NonBlockingBuilder::default()
        .buffered_lines_limit(LOG_QUEUE_LINES)
        .lossy(true)
        .finish(appender))
}

/// A panic goes to the log as well as to the console, because a release build has no console.
fn log_panics() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "panic");
        default_hook(info);
    }));
}

/// The rolling appender limits the number of files but not their size. This deletes the oldest
/// files of this program until the folder fits the budget. It never deletes the newest file.
/// Returns the number of deleted files.
///
/// The backend has the same function (`core::logging`); the shell cannot link the backend.
fn enforce_size_budget(dir: &Path, prefix: &str, max_total_bytes: u64) -> io::Result<usize> {
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

    fn path(text: &str) -> Option<PathBuf> {
        Some(PathBuf::from(text))
    }

    #[test]
    fn the_local_folder_variable_wins() {
        assert_eq!(
            local_dir_for(path(r"C:\L"), path(r"C:\D"), path(r"C:\A"), false),
            path(r"C:\L")
        );
    }

    #[test]
    fn a_data_folder_variable_gives_a_sibling_local_folder() {
        assert_eq!(
            local_dir_for(None, path(r"C:\T\data"), path(r"C:\A"), false),
            path(r"C:\T\data.local")
        );
    }

    #[test]
    fn the_default_is_below_local_app_data_and_depends_on_the_sandbox() {
        assert_eq!(
            local_dir_for(None, None, path(r"C:\A"), false),
            path(r"C:\A\Rain")
        );
        assert_eq!(
            local_dir_for(None, None, path(r"C:\A"), true),
            path(r"C:\A\Rain-dev")
        );
    }

    #[test]
    fn without_any_variable_there_is_no_local_folder() {
        assert_eq!(local_dir_for(None, None, None, true), None);
    }

    #[test]
    fn the_oldest_files_go_first_and_the_newest_stays() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=5 {
            fs::write(
                dir.path().join(format!("shell.2026-11-0{day}.log")),
                vec![b'x'; 1000],
            )
            .unwrap();
        }
        fs::write(dir.path().join("backend.2026-11-01.log"), vec![b'x'; 5000]).unwrap(); // not ours

        assert_eq!(enforce_size_budget(dir.path(), "shell", 2500).unwrap(), 3);

        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "backend.2026-11-01.log",
                "shell.2026-11-04.log",
                "shell.2026-11-05.log"
            ]
        );
    }

    #[test]
    fn a_single_file_over_the_budget_is_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("shell.2026-11-05.log"), vec![b'x'; 1000]).unwrap();
        assert_eq!(enforce_size_budget(dir.path(), "shell", 10).unwrap(), 0);
    }
}
