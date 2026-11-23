//! The parts of this module that touch the PC, each behind a trait, so that a test says what the
//! PC looks like (foundation 12.2). Section 03 10.2 lists them.
//!
//! The real implementations are thin wrappers over `platform/`. A test replaces a part with
//! `TestBackend::builder().with::<dyn ProcessProbe>(Arc::new(fake))`; `Service::new` asks
//! `state.seam::<dyn ..>()` and falls back to the real one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::future::BoxFuture;

use crate::platform::process::{self, ExitWatcher};
use crate::platform::windows_focus::{self, FocusWindow};

/// A process in a chain of parents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    /// The program name, lower case, without a folder: `windowsterminal.exe`.
    pub name: String,
}

/// What the module asks about processes.
pub trait ProcessProbe: Send + Sync {
    /// The creation time (FILETIME) of a process, `None` when it does not exist or cannot be
    /// opened. Cheap: about 20 microseconds.
    fn creation_filetime(&self, pid: u32) -> Option<u64>;
    /// Whether the process still runs. A process that has ended but is still held open by
    /// someone (the exit watch holds a handle) can still be opened, so this asks the process
    /// itself and does not rely on `creation_filetime`.
    fn is_running(&self, pid: u32) -> bool;
    /// The program name of a process, lower case. Blocking (one process snapshot).
    fn image_name(&self, pid: u32) -> Option<String>;
    /// The process and its parents, nearest first, at most `levels`, stopping before the shell
    /// of Windows and Rain's own programs. Blocking.
    fn chain(&self, pid: u32, levels: usize) -> Vec<ProcInfo>;
    /// The children, grandchildren and so on of a process. A child counts only when it was
    /// created after its parent, which guards against a reused pid. Blocking.
    fn descendants(&self, pid: u32) -> Vec<u32>;
    /// Ends a process at once.
    fn terminate(&self, pid: u32) -> std::io::Result<()>;
}

/// Programs that are never part of the chain of a session: they are the places where a chain
/// would leave the session's own terminal or app and start to cover everything.
const CHAIN_STOPS: [&str; 5] = [
    "explorer.exe",
    "services.exe",
    "svchost.exe",
    "rain.exe",
    "rain-backend.exe",
];

pub struct SystemProbe;

impl ProcessProbe for SystemProbe {
    fn creation_filetime(&self, pid: u32) -> Option<u64> {
        process::creation_time(pid)
    }

    fn is_running(&self, pid: u32) -> bool {
        process::pid_alive(pid)
    }

    fn image_name(&self, pid: u32) -> Option<String> {
        process::list_processes()
            .into_iter()
            .find(|entry| entry.pid == pid)
            .map(|entry| entry.name.to_lowercase())
    }

    fn chain(&self, pid: u32, levels: usize) -> Vec<ProcInfo> {
        let table = process::list_processes();
        let mut chain = Vec::new();
        let mut next = Some(pid);
        while let Some(current) = next {
            if chain.len() >= levels {
                break;
            }
            let Some(entry) = table.iter().find(|entry| entry.pid == current) else {
                break;
            };
            let name = entry.name.to_lowercase();
            if CHAIN_STOPS.contains(&name.as_str()) {
                break;
            }
            chain.push(ProcInfo { pid: current, name });
            next = Some(entry.parent_pid).filter(|parent| *parent != 0 && *parent != current);
        }
        chain
    }

    fn descendants(&self, pid: u32) -> Vec<u32> {
        let table = process::list_processes();
        let mut found = vec![pid];
        let mut index = 0;
        while index < found.len() {
            let parent = found[index];
            let parent_start = process::creation_time(parent);
            for entry in table.iter().filter(|entry| entry.parent_pid == parent) {
                let child_start = process::creation_time(entry.pid);
                let after_parent = match (parent_start, child_start) {
                    (Some(parent), Some(child)) => child >= parent,
                    _ => false,
                };
                if after_parent && !found.contains(&entry.pid) {
                    found.push(entry.pid);
                }
            }
            index += 1;
        }
        found.remove(0);
        found
    }

    fn terminate(&self, pid: u32) -> std::io::Result<()> {
        process::terminate_pid(pid)
    }
}

/// Tells when a process ends.
pub trait ExitSource: Send + Sync {
    /// Calls `on_exit` when the process with this pid and creation time ends. The returned value
    /// keeps the watch alive; dropping it cancels the watch. `None` when the process cannot be
    /// watched (it ended, it is protected): the read-time check then finds the end.
    fn watch(
        &self,
        pid: u32,
        start: u64,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Option<Box<dyn Send>>;
}

pub struct SystemExits;

impl ExitSource for SystemExits {
    fn watch(
        &self,
        pid: u32,
        start: u64,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Option<Box<dyn Send>> {
        ExitWatcher::register(pid, start, on_exit).map(|watcher| Box::new(watcher) as Box<dyn Send>)
    }
}

/// The windows of the desktop.
pub trait WindowApi: Send + Sync {
    /// Visible top-level windows in z-order, the one in front first. Blocking.
    fn windows(&self) -> Vec<FocusWindow>;
    /// Tries to bring a window to the front and says whether it is in front afterwards. Blocking.
    fn bring_to_front(&self, hwnd: isize) -> bool;
    /// Flashes the taskbar button of a window.
    fn flash(&self, hwnd: isize);
}

pub struct SystemWindows;

impl WindowApi for SystemWindows {
    fn windows(&self) -> Vec<FocusWindow> {
        windows_focus::top_level_windows()
    }

    fn bring_to_front(&self, hwnd: isize) -> bool {
        windows_focus::bring_to_front(hwnd)
    }

    fn flash(&self, hwnd: isize) {
        windows_focus::flash(hwnd);
    }
}

/// The hook program that the installer copies and checks.
pub trait ForwarderProgram: Send + Sync {
    /// The file to copy to the local folder: `rain-hook.exe` next to the backend. `None` when it
    /// is not there.
    fn source(&self) -> Option<PathBuf>;
    /// Starts the copied program with `selftest` and says whether it exited with code 0 in two
    /// seconds.
    fn selftest<'a>(&'a self, program: &'a Path) -> BoxFuture<'a, bool>;
}

/// How long the self test of the copied program may take.
const SELFTEST_LIMIT: Duration = Duration::from_secs(2);

pub struct SystemForwarder;

impl ForwarderProgram for SystemForwarder {
    fn source(&self) -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let source = exe.with_file_name("rain-hook.exe");
        source.is_file().then_some(source)
    }

    fn selftest<'a>(&'a self, program: &'a Path) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let run = process::hidden_command(program).arg("selftest").status();
            matches!(
                tokio::time::timeout(SELFTEST_LIMIT, run).await,
                Ok(Ok(status)) if status.success()
            )
        })
    }
}
