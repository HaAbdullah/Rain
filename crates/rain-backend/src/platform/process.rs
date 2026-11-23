//! Processes: starting a program that must outlive Rain, listing what runs, ending a process by
//! its id, and waiting for a process to end. Foundation section 11.6.
//!
//! What lives here and why it is not `std::process::Command`:
//! - `hidden_command` is the only way the backend starts a helper program (`git`, `claude`): no
//!   console window, no input. `clippy.toml` forbids `Command::new` everywhere else.
//! - `spawn_detached` starts a user's program (a game, an editor) so that it is never a child
//!   that dies with Rain, and never inside a kill-on-close job object (decision C10, D15).
//! - `ExitWatcher` waits for a process to end without parking a thread.

use std::ffi::{OsStr, c_void};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::oneshot;
use tracing::warn;
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetProcessTimes, INFINITE, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, RegisterWaitForSingleObject, TerminateProcess, UnregisterWaitEx,
    WT_EXECUTEONLYONCE, WaitForSingleObject,
};
use windows::core::{BOOL, PWSTR};

/// `DETACHED_PROCESS`: no console. It is not combined with `CREATE_NO_WINDOW`; the two exclude
/// each other.
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
/// What Windows answers when the job of the parent forbids breakaway: "access denied".
const ERROR_ACCESS_DENIED: i32 = 5;
/// How far a process's creation time may differ from the expected one and still be the same
/// process: 10 ms in FILETIME ticks (100 ns). Section 03 6.3.3.
const START_TOLERANCE_TICKS: u64 = 100_000;

/// A helper program that runs invisibly: no console window and no input. The caller sets the
/// arguments, the output and the environment. `kill_on_drop` is not set: whoever starts a
/// process decides what happens to it when the handle goes.
pub fn hidden_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0);
    command
}

// ---- listing and ending ----

/// One process, as the Toolhelp snapshot knows it: the image NAME only, without a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
}

/// Every running process (2.4 ms for 262 processes, spike `process`).
pub fn list_processes() -> Vec<ProcessEntry> {
    let mut processes = Vec::with_capacity(300);
    // SAFETY: the snapshot handle is closed below; `entry` is a valid `PROCESSENTRY32W` whose
    // `dwSize` is set, as the two calls require.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return processes;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                processes.push(ProcessEntry {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    name: String::from_utf16_lossy(&entry.szExeFile[..length]),
                });
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    processes
}

/// The full path of the program file of every process that Rain may ask. A process that
/// Windows does not let a normal user open (protected and system processes: about 114 of 262
/// in the spike) is left out. Compare paths, never only names (foundation 11.6). 4.7 ms.
pub fn list_images() -> Vec<String> {
    let mut buffer = vec![0u16; 1024];
    let mut images = Vec::with_capacity(200);
    for process in list_processes().into_iter().filter(|p| p.pid != 0) {
        // SAFETY: the handle is closed right after the query; `buffer` is valid for the length
        // that is passed, and the call updates the length to what it wrote.
        let image = unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process.pid)
            else {
                continue;
            };
            let mut length = buffer.len() as u32;
            let queried = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            );
            let _ = CloseHandle(handle);
            queried
                .is_ok()
                .then(|| String::from_utf16_lossy(&buffer[..length as usize]))
        };
        images.extend(image);
    }
    images
}

/// Which programs run, for the modules that must tell "is it open" (section 01 for apps,
/// section 02 for projects and games). A trait, so that a test says which programs run without
/// looking at the real PC.
pub trait ProcessLister: Send + Sync {
    /// The full path of the program file of every process that Rain may ask. Blocking.
    fn images(&self) -> Vec<String>;
}

/// The real list of this PC.
pub struct SystemProcessLister;

impl ProcessLister for SystemProcessLister {
    fn images(&self) -> Vec<String> {
        list_images()
    }
}

/// Whether a process with this id is running. A pid that is not ours to open counts as gone.
pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: the handle is closed before the function returns.
    unsafe {
        let Ok(handle) = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        ) else {
            return false;
        };
        let state = WaitForSingleObject(handle, 0);
        let _ = CloseHandle(handle);
        state == WAIT_TIMEOUT
    }
}

/// The creation time of a process as a Windows FILETIME (100 ns ticks since 1601). With the pid
/// it names a process even after the pid is reused. `None` when the process cannot be opened.
pub fn creation_time(pid: u32) -> Option<u64> {
    // SAFETY: the handle is closed before the function returns.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let time = creation_time_of(handle);
        let _ = CloseHandle(handle);
        time
    }
}

fn creation_time_of(process: HANDLE) -> Option<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: the four out-parameters are valid for writes.
    unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

/// Ends the process with this id at once (`TerminateProcess`). Only for a process whose id Rain
/// got from the process itself: never found by its image name (`--stop --force` reads the pid
/// from `GET /ping`).
pub fn terminate_pid(pid: u32) -> io::Result<()> {
    // SAFETY: the handle is closed before the function returns.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid).map_err(io::Error::other)?;
        let ended = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
        ended.map_err(io::Error::other)
    }
}

// ---- starting a program that outlives Rain ----

/// Whether a started program is really independent of the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detached {
    /// The program is outside any job of the backend and lives on when the backend ends.
    Free,
    /// The backend itself runs inside a job that forbids breakaway, so the program is inside it
    /// too and may end with the backend.
    InJob,
}

/// A program that `spawn_detached` started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spawned {
    pub pid: u32,
    /// FILETIME of the creation of the process; 0 when Windows would not tell.
    pub creation_time: u64,
    pub detached: Detached,
}

/// Starts `exe` with the argument text `args_raw` exactly as given (no quoting is added) in the
/// folder `cwd`, detached: no console, its own process group, breakaway from any job of the
/// parent, no input or output. It is never a child that Rain waits for, and it is never put into
/// a kill-on-close job (foundation 2.3). If the job of the parent forbids breakaway (Windows
/// answers "access denied"), the start is retried without it and the result says `InJob`.
pub fn spawn_detached(exe: &Path, args_raw: &str, cwd: &Path) -> io::Result<Spawned> {
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    match start(exe, args_raw, cwd, flags | CREATE_BREAKAWAY_FROM_JOB) {
        Ok(spawned) => Ok(spawned),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
            let mut spawned = start(exe, args_raw, cwd, flags)?;
            // "Access denied" for breakaway means a job forbids it, but another cause is
            // possible, so ask whether the backend is in a job at all.
            if this_process_is_in_a_job() {
                spawned.detached = Detached::InJob;
                warn!("a program was started inside a job and may end with the backend");
            }
            Ok(spawned)
        }
        Err(e) => Err(e),
    }
}

fn start(exe: &Path, args_raw: &str, cwd: &Path, flags: u32) -> io::Result<Spawned> {
    // `std::process::Command` is forbidden by clippy.toml for ordinary code. This is the one
    // place that must use it: `raw_arg` keeps the argument text exact, which tokio does not offer.
    #[allow(clippy::disallowed_methods)]
    let mut command = std::process::Command::new(exe);
    let child = command
        .raw_arg(args_raw)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(flags)
        .spawn()?;
    // The program is already running, so a failure here must not be reported as a failed start
    // (the caller would show an error or start it twice). 0 means "unknown": the later checks of
    // pid plus creation time then do not match, and the caller falls back to its read-time check.
    let creation_time = creation_time_of(HANDLE(child.as_raw_handle())).unwrap_or_else(|| {
        warn!(
            pid = child.id(),
            "the creation time of a started program is unknown"
        );
        0
    });
    // `Child` has no kill-on-drop: the program goes on.
    Ok(Spawned {
        pid: child.id(),
        creation_time,
        detached: Detached::Free,
    })
}

/// Whether the current process is inside a job object (any job).
pub fn this_process_is_in_a_job() -> bool {
    let mut inside = BOOL(0);
    // SAFETY: `-1` is the pseudo handle of the current process; `inside` is valid for writes.
    let known = unsafe { IsProcessInJob(HANDLE(-1isize as *mut c_void), None, &mut inside) };
    known.is_ok() && inside.as_bool()
}

// ---- waiting for a process to end ----

/// Calls back when a process ends, through `RegisterWaitForSingleObject`: the operating system
/// signals the exit, so no thread is parked and no timer runs (foundation 11.5). Dropping the
/// watcher cancels the wait.
pub struct ExitWatcher {
    wait: HANDLE,
    process: HANDLE,
    shared: Arc<Shared>,
    /// For `open_for_wait`: completes when the process has ended.
    exit: Option<oneshot::Receiver<()>>,
}

struct Shared {
    /// Taken by the one call of the callback.
    callback: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    fired: AtomicBool,
}

// SAFETY: the two handles are plain kernel handles, used only to unregister and close them.
unsafe impl Send for ExitWatcher {}

impl ExitWatcher {
    /// Watches the process `pid` whose creation time is `expected_start` (a FILETIME, with a
    /// tolerance of 10 ms), and runs `callback` on a thread of the system's pool when it ends.
    /// The callback must be quick and only pass a message on. `None` when the process cannot
    /// be opened (it has ended, it is protected) or is a different process with the same pid:
    /// the caller then relies on its read-time check.
    pub fn register(
        pid: u32,
        expected_start: u64,
        callback: impl FnOnce() + Send + 'static,
    ) -> Option<Self> {
        // SAFETY: the process handle is owned by the returned watcher, or closed here.
        let process = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            )
            .ok()?
        };
        let same = creation_time_of(process)
            .is_some_and(|actual| actual.abs_diff(expected_start) <= START_TOLERANCE_TICKS);
        if !same {
            // SAFETY: the handle was just opened and is not used again.
            unsafe {
                let _ = CloseHandle(process);
            }
            return None;
        }
        let shared = Arc::new(Shared {
            callback: Mutex::new(Some(Box::new(callback))),
            fired: AtomicBool::new(false),
        });
        let context = Arc::into_raw(Arc::clone(&shared));
        let mut wait = HANDLE::default();
        // SAFETY: `process` is a valid handle; `on_exit` has the signature that the call needs;
        // `context` is a share of `shared` that the callback, or `Drop`, gives back.
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                process,
                Some(on_exit),
                Some(context.cast()),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if registered.is_err() {
            // SAFETY: the registration failed, so the callback will never run: take the share
            // back, and close the handle.
            unsafe {
                drop(Arc::from_raw(context));
                let _ = CloseHandle(process);
            }
            return None;
        }
        Some(Self {
            wait,
            process,
            shared,
            exit: None,
        })
    }

    /// Like `register`, with a future in place of a callback: see [`ExitWatcher::exited`].
    pub fn open_for_wait(pid: u32, creation_time: u64) -> Option<Self> {
        let (sender, receiver) = oneshot::channel();
        let mut watcher = Self::register(pid, creation_time, move || {
            let _ = sender.send(());
        })?;
        watcher.exit = Some(receiver);
        Some(watcher)
    }

    /// Completes when the process has ended (at once when it had ended before the watch began).
    /// Only for a watcher made by `open_for_wait`; for another one it never completes.
    pub async fn exited(&mut self) {
        match self.exit.take() {
            // A closed channel means the watcher was dropped, which cannot happen while `self`
            // is borrowed, so only a real exit completes this.
            Some(receiver) => {
                let _ = receiver.await;
            }
            None => std::future::pending().await,
        }
    }
}

/// The callback of the thread pool: runs the closure once and gives back its share of `Shared`.
unsafe extern "system" fn on_exit(context: *mut c_void, _timed_out: bool) {
    // SAFETY: `context` is the pointer from `Arc::into_raw` in `register`, and
    // `WT_EXECUTEONLYONCE` calls this at most once, so the share is taken back exactly once.
    let shared = unsafe { Arc::from_raw(context.cast::<Shared>()) };
    shared.fired.store(true, Ordering::SeqCst);
    let callback = shared
        .callback
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(callback) = callback {
        callback();
    }
}

impl Drop for ExitWatcher {
    fn drop(&mut self) {
        // SAFETY: `INVALID_HANDLE_VALUE` makes the call wait until a running callback has
        // finished, so afterwards the callback is either done or will never run. Both handles
        // are owned by this watcher and not used again.
        unsafe {
            let _ = UnregisterWaitEx(self.wait, Some(INVALID_HANDLE_VALUE));
            if !self.shared.fired.load(Ordering::SeqCst) {
                // The callback never ran, so its share of `Shared` was never taken back.
                let leaked = Arc::as_ptr(&self.shared);
                Arc::decrement_strong_count(leaked);
            }
            let _ = CloseHandle(self.process);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A harmless program that runs for about `seconds` seconds: `ping` on the loopback address.
    async fn slow_child(seconds: u32) -> tokio::process::Child {
        let mut command = hidden_command("cmd.exe");
        command.args(["/c", &format!("ping -n {} 127.0.0.1 >nul", seconds + 1)]);
        command.spawn().unwrap()
    }

    #[test]
    fn the_snapshot_lists_this_test_process_with_its_parent() {
        let me = std::process::id();
        let list = list_processes();
        let mine = list
            .iter()
            .find(|p| p.pid == me)
            .expect("this process is listed");
        assert!(mine.name.to_lowercase().ends_with(".exe"));
        assert_ne!(mine.parent_pid, 0);
    }

    #[test]
    fn the_images_include_this_test_program_by_its_full_path() {
        let me = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .to_lowercase();
        let images = list_images();
        assert!(images.iter().any(|image| image.to_lowercase() == me));
        assert!(
            images.iter().all(|image| image.contains('\\')),
            "full paths, not names"
        );
    }

    #[test]
    fn the_system_lister_is_the_list_of_images() {
        let lister: &dyn ProcessLister = &SystemProcessLister;
        assert!(!lister.images().is_empty());
    }

    #[test]
    fn this_process_is_alive_and_has_a_creation_time_and_a_made_up_pid_is_not_alive() {
        assert!(pid_alive(std::process::id()));
        assert!(creation_time(std::process::id()).is_some_and(|t| t > 0));
        assert!(!pid_alive(0x7FFF_FFF0));
        assert_eq!(creation_time(0x7FFF_FFF0), None);
    }

    #[tokio::test]
    async fn a_helper_starts_without_a_window_and_can_be_ended_by_its_pid() {
        let mut child = slow_child(30).await;
        let pid = child.id().unwrap();
        assert!(pid_alive(pid));

        terminate_pid(pid).unwrap();
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("the helper ended")
            .unwrap();
        assert!(!pid_alive(pid));
    }

    #[tokio::test]
    async fn a_detached_program_starts_with_exact_arguments_and_reports_its_time() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("made by the child.txt");
        // The argument text holds a quoted path with a space: it must arrive exactly as written.
        let args = format!(r#"/c echo done > "{}""#, marker.display());
        let cmd = Path::new(&std::env::var("ComSpec").unwrap()).to_owned();

        let spawned = spawn_detached(&cmd, &args, temp.path()).unwrap();
        assert!(spawned.creation_time > 0);
        // Either answer is right; which one depends on whether the test runner is in a job.
        assert!(matches!(spawned.detached, Detached::Free | Detached::InJob));

        for _ in 0..100 {
            if marker.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the detached program did not write its marker file");
    }

    #[tokio::test]
    async fn a_watcher_completes_when_the_process_ends() {
        let mut child = slow_child(1).await;
        let pid = child.id().unwrap();
        let started = creation_time(pid).unwrap();
        let mut watcher =
            ExitWatcher::open_for_wait(pid, started).expect("the process can be watched");

        tokio::time::timeout(Duration::from_secs(10), watcher.exited())
            .await
            .expect("the watcher noticed the exit");
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn a_watcher_for_a_process_that_has_already_ended_completes_at_once() {
        let mut child = slow_child(0).await;
        let pid = child.id().unwrap();
        let started = creation_time(pid).unwrap();
        child.wait().await.unwrap();
        // The handle can still be opened while the child object holds it, or not at all.
        if let Some(mut watcher) = ExitWatcher::open_for_wait(pid, started) {
            tokio::time::timeout(Duration::from_secs(5), watcher.exited())
                .await
                .expect("an ended process is already signalled");
        }
    }

    #[tokio::test]
    async fn a_watcher_for_another_process_with_the_same_pid_is_refused() {
        let mut child = slow_child(5).await;
        let pid = child.id().unwrap();
        let wrong_start = creation_time(pid).unwrap() + 10_000_000; // one second later
        assert!(ExitWatcher::open_for_wait(pid, wrong_start).is_none());
        child.kill().await.unwrap();
    }

    #[tokio::test]
    async fn dropping_a_watcher_before_the_exit_cancels_it_without_calling_back() {
        let mut child = slow_child(30).await;
        let pid = child.id().unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&called);
        let watcher = ExitWatcher::register(pid, creation_time(pid).unwrap(), move || {
            flag.store(true, Ordering::SeqCst);
        })
        .unwrap();

        drop(watcher);
        child.kill().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !called.load(Ordering::SeqCst),
            "a cancelled wait never calls back"
        );
    }

    #[test]
    fn the_test_process_reports_whether_it_is_inside_a_job_without_failing() {
        // Either answer is valid; the call must work.
        let _ = this_process_is_in_a_job();
    }
}
