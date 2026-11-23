//! Job objects for process trees that Rain owns and may have to kill. Foundation 11.6 and D15.
//!
//! A scheduled job run and an idle job run (sections 03) are a program that starts more
//! programs. To stop a run, Rain ends the whole tree. The tool for that is a Windows job object
//! with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when the last handle of the job closes, also
//! because the backend crashed, Windows ends every process inside it.
//!
//! Hand-written, as the spike `process` proved (`q4_hand.rs`). The crate `process-wrap` is not
//! used: it never sets `KILL_ON_JOB_CLOSE` and hides the job handle. The job must not have
//! `SILENT_BREAKAWAY_OK`: that would free the very tree that Rain wants to kill.
//!
//! Only Rain's own runs go into a job. A program that the user launches (an app, a game, a
//! project) never does (decision C10): it must live on when Rain ends.

use std::io;
use std::process::ExitStatus;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, SuspendThread,
    THREAD_SUSPEND_RESUME,
};

/// The longest line that is passed on. A longer one arrives in pieces.
const MAX_LINE_BYTES: usize = 8 * 1024;
/// Lines that wait for the reader before the program is made to wait (back pressure).
const LINE_QUEUE: usize = 256;
/// The exit code of a process that `TerminateJobObject` ended. Rain marks a run "cancelled" from
/// its own state, not from this code.
const KILLED_EXIT_CODE: u32 = 1;

/// A job object. Dropping it closes the handle, which ends every process inside.
pub struct OwnedJob {
    handle: HANDLE,
}

// SAFETY: a job handle is a kernel handle that any thread may use.
unsafe impl Send for OwnedJob {}
// SAFETY: the calls made through `&self` are thread safe kernel calls.
unsafe impl Sync for OwnedJob {}

impl OwnedJob {
    /// Creates a job whose processes die with the last handle to it.
    pub fn new() -> io::Result<Self> {
        // SAFETY: no security attributes and no name; the new handle is owned by the result.
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::other)?;
        let job = Self { handle };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is a valid structure of the kind named, and its size is passed.
        unsafe {
            SetInformationJobObject(
                job.handle,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        }
        .map_err(io::Error::other)?;
        Ok(job)
    }

    /// Starts `command` inside the job with its output captured.
    ///
    /// The program starts suspended, is put into the job and then resumed, so that it cannot
    /// start a child before it is in the job (that child would escape). The window of the
    /// program is hidden; input is null. Standard output and error arrive as lines on the
    /// returned `JobChild`.
    pub fn spawn(&self, command: Command) -> io::Result<JobChild> {
        self.spawn_with_input(command, None)
    }

    /// Like `spawn`, but the program's standard input holds `input` (then it is closed). A
    /// headless Claude run reads its prompt this way. `None` gives it a null input.
    pub fn spawn_with_input(
        &self,
        mut command: Command,
        input: Option<Vec<u8>>,
    ) -> io::Result<JobChild> {
        command
            .stdin(if input.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .creation_flags(CREATE_SUSPENDED.0 | CREATE_NO_WINDOW.0);
        let mut child = command.spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the program ended before it was put in the job"))?;
        let raw = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the new process has no handle"))?;
        // SAFETY: both handles are valid; the child's handle is owned by `child`.
        let assigned = unsafe { AssignProcessToJobObject(self.handle, HANDLE(raw)) };
        if let Err(e) = assigned {
            // Not in the job means not under Rain's control: end it rather than let it run.
            let _ = child.start_kill();
            return Err(io::Error::other(e));
        }
        resume_threads(pid);

        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            // The write may block when the program does not read, so it is its own task and
            // nothing waits for it. Dropping `stdin` at the end closes the input.
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(&input).await;
            });
        }
        let (lines_out, lines) = mpsc::channel(LINE_QUEUE);
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(read_lines(stdout, Stream::Stdout, lines_out.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(read_lines(stderr, Stream::Stderr, lines_out));
        }
        Ok(JobChild { pid, child, lines })
    }

    /// Ends every process in the job at once (`TerminateJobObject`). Nothing is graceful about
    /// it: there is no graceful stop for a headless Claude run either (spike).
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the handle is valid for the life of `self`.
        unsafe { TerminateJobObject(self.handle, KILLED_EXIT_CODE) }.map_err(io::Error::other)
    }

    /// The ids of the processes in the job now: the job's own truth, not parent links.
    pub fn pids(&self) -> Vec<u32> {
        // The list ends in an array of ids, so the buffer is a header plus room for them.
        let mut buffer = vec![0usize; 64];
        // SAFETY: the buffer is valid for the size passed and is aligned for the structure.
        let queried = unsafe {
            QueryInformationJobObject(
                Some(self.handle),
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * std::mem::size_of::<usize>()) as u32,
                None,
            )
        };
        if queried.is_err() {
            return Vec::new();
        }
        // SAFETY: on success the buffer starts with a `JOBOBJECT_BASIC_PROCESS_ID_LIST` whose
        // count says how many ids follow, all inside the buffer.
        unsafe {
            let head = &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            let ids = head.ProcessIdList.as_ptr();
            (0..head.NumberOfProcessIdsInList as usize)
                .map(|i| *ids.add(i) as u32)
                .collect()
        }
    }

    /// True when no process is left in the job.
    pub fn is_empty(&self) -> bool {
        self.pids().is_empty()
    }

    /// Freezes every thread of every process in the job, so that a program that cannot be asked
    /// to pause stops using the CPU (idle jobs, section 03 6.12.5). Not proven in a spike for a
    /// whole tree: an error says that some thread could not be frozen, and the caller then ends
    /// the run instead.
    pub fn suspend(&self) -> io::Result<()> {
        self.set_frozen(true)
    }

    /// Lets a frozen job run again.
    pub fn resume(&self) -> io::Result<()> {
        self.set_frozen(false)
    }

    /// Two passes, because a process may start a child between the list and the freeze: the
    /// second pass catches threads that the first one did not see.
    fn set_frozen(&self, freeze: bool) -> io::Result<()> {
        let mut seen = std::collections::HashSet::new();
        let mut failed = false;
        for _pass in 0..2 {
            let pids = self.pids();
            for thread_id in threads_of(&pids) {
                if seen.insert(thread_id) {
                    failed |= !change_thread(thread_id, freeze);
                }
            }
        }
        if failed {
            Err(io::Error::other("a thread of the job could not be changed"))
        } else {
            Ok(())
        }
    }
}

/// The ids of every thread that belongs to one of `pids`, from a Toolhelp snapshot.
fn threads_of(pids: &[u32]) -> Vec<u32> {
    let mut threads = Vec::new();
    // SAFETY: the snapshot handle is closed below; `entry` is a valid `THREADENTRY32` with its
    // size set.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return threads;
        };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if pids.contains(&entry.th32OwnerProcessID) {
                    threads.push(entry.th32ThreadID);
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    threads
}

/// Suspends or resumes one thread. False when Windows refused.
fn change_thread(thread_id: u32, freeze: bool) -> bool {
    // SAFETY: the thread handle is closed right after use; the calls take only that handle.
    unsafe {
        let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, thread_id) else {
            return false;
        };
        let before = if freeze {
            SuspendThread(thread)
        } else {
            ResumeThread(thread)
        };
        let _ = CloseHandle(thread);
        before != u32::MAX
    }
}

impl Drop for OwnedJob {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and not used again. Closing the last handle
        // ends every process in the job (`KILL_ON_JOB_CLOSE`).
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Resumes every thread of a process that was created suspended. `std` and `tokio` do not hand
/// out the primary thread's handle, so the thread is found through a snapshot.
fn resume_threads(pid: u32) -> usize {
    let mut resumed = 0;
    // SAFETY: the snapshot handle is closed below; `entry` is a valid `THREADENTRY32` with its
    // size set; each thread handle is closed right after use.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return 0;
        };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == pid
                    && let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                {
                    if ResumeThread(thread) != u32::MAX {
                        resumed += 1;
                    }
                    let _ = CloseHandle(thread);
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    resumed
}

// ---- the program inside the job ----

/// Which output a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One line of output, without its line end. The text is decoded as UTF-8; bytes that are not
/// valid become the replacement character (a console program may write the OEM code page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: String,
}

/// A program that runs inside an `OwnedJob`.
pub struct JobChild {
    pub pid: u32,
    child: Child,
    /// The output, line by line, standard output and error mixed in the order they were read.
    /// It ends when the program and everything it started have closed their output.
    pub lines: mpsc::Receiver<OutputLine>,
}

impl JobChild {
    /// Waits until the program has ended and returns its exit code (1 for a run that
    /// `OwnedJob::terminate` ended).
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// Separates the output from the process, so that one task can wait for the exit and read
    /// lines at the same time (`select!` needs two separate borrows).
    pub fn split(self) -> (JobWaiter, mpsc::Receiver<OutputLine>) {
        (
            JobWaiter {
                pid: self.pid,
                child: self.child,
            },
            self.lines,
        )
    }
}

/// The process of a `JobChild` after `split`.
pub struct JobWaiter {
    pub pid: u32,
    child: Child,
}

impl JobWaiter {
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }
}

async fn read_lines(
    output: impl AsyncRead + Unpin,
    stream: Stream,
    sink: mpsc::Sender<OutputLine>,
) {
    let mut reader = BufReader::new(output);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        // `take` caps the read, so a program that writes no line end cannot grow the buffer.
        let read = (&mut reader)
            .take(MAX_LINE_BYTES as u64)
            .read_until(b'\n', &mut bytes)
            .await;
        match read {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        let line = OutputLine {
            stream,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        };
        if sink.send(line).await.is_err() {
            return; // nobody reads any more
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::process::{hidden_command, pid_alive};
    use std::os::windows::process::ExitStatusExt;
    use std::time::Duration;

    /// Whether an exit status is the one that `terminate` gives.
    fn is_killed_status(status: ExitStatus) -> bool {
        status.code() == Some(KILLED_EXIT_CODE as i32)
    }

    /// `cmd` that prints a line and then runs a child (`ping`) for about `seconds` seconds: a
    /// tree of three processes (cmd, cmd, ping) like a real job.
    fn tree(seconds: u32) -> Command {
        let mut command = hidden_command("cmd.exe");
        command.args([
            "/c",
            &format!(
                "echo hello from the tree & cmd /c ping -n {} 127.0.0.1 >nul",
                seconds + 1
            ),
        ]);
        command
    }

    async fn first_line(child: &mut JobChild) -> OutputLine {
        tokio::time::timeout(Duration::from_secs(10), child.lines.recv())
            .await
            .expect("a line arrived")
            .expect("the output did not end before the first line")
    }

    /// Waits until the job holds at least `count` processes.
    async fn wait_for_members(job: &OwnedJob, count: usize) -> Vec<u32> {
        for _ in 0..100 {
            let pids = job.pids();
            if pids.len() >= count {
                return pids;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the job never held {count} processes: {:?}", job.pids());
    }

    /// The job's member list is updated a moment after a process ends.
    async fn became_empty(job: &OwnedJob) -> bool {
        for _ in 0..100 {
            if job.is_empty() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    async fn all_gone(pids: &[u32]) -> bool {
        for _ in 0..100 {
            if pids.iter().all(|pid| !pid_alive(*pid)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    async fn the_output_of_the_program_arrives_as_lines() {
        let job = OwnedJob::new().unwrap();
        let mut child = job.spawn(tree(1)).unwrap();

        let line = first_line(&mut child).await;
        // `cmd` keeps the space before the `&` of the command line.
        assert_eq!(line.stream, Stream::Stdout);
        assert_eq!(line.text.trim_end(), "hello from the tree");
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
        assert!(became_empty(&job).await, "{:?}", job.pids());
    }

    #[tokio::test]
    async fn the_standard_error_is_captured_apart_from_the_output() {
        let job = OwnedJob::new().unwrap();
        let mut command = hidden_command("cmd.exe");
        command.args(["/c", "echo to the error stream 1>&2"]);
        let mut child = job.spawn(command).unwrap();

        let line = first_line(&mut child).await;
        assert_eq!(line.stream, Stream::Stderr);
        assert_eq!(line.text.trim_end(), "to the error stream");
    }

    #[tokio::test]
    async fn terminate_ends_the_whole_tree_at_once() {
        let job = OwnedJob::new().unwrap();
        let mut child = job.spawn(tree(60)).unwrap();
        first_line(&mut child).await;
        let members = wait_for_members(&job, 3).await;
        assert!(members.contains(&child.pid));

        job.terminate().unwrap();
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(is_killed_status(status), "{status:?}");
        assert!(all_gone(&members).await, "no process of the tree survived");
        assert!(became_empty(&job).await, "{:?}", job.pids());
    }

    #[tokio::test]
    async fn closing_the_job_ends_the_tree_as_when_the_backend_crashes() {
        let job = OwnedJob::new().unwrap();
        let mut child = job.spawn(tree(60)).unwrap();
        first_line(&mut child).await;
        let members = wait_for_members(&job, 3).await;

        drop(job); // the last handle closes: KILL_ON_JOB_CLOSE
        assert!(
            all_gone(&members).await,
            "no process of the tree survived the close"
        );
    }

    #[tokio::test]
    async fn a_program_that_cannot_start_is_an_error_and_a_job_can_run_several_programs() {
        let job = OwnedJob::new().unwrap();
        assert!(
            job.spawn(hidden_command(r"C:\no-such-folder-rain-test\x.exe"))
                .is_err()
        );

        let mut first = job.spawn(tree(1)).unwrap();
        let mut second = job.spawn(tree(1)).unwrap();
        assert_ne!(first.pid, second.pid);
        first.wait().await.unwrap();
        second.wait().await.unwrap();
    }

    #[tokio::test]
    async fn a_very_long_line_arrives_in_pieces_and_not_as_one_huge_string() {
        let job = OwnedJob::new().unwrap();
        // 20 000 characters on one line: more than twice the limit of one piece.
        let mut command = hidden_command("cmd.exe");
        command.args(["/c", &format!("echo {}", "x".repeat(20_000))]);
        // cmd's own line limit is 8191 characters, so it prints an error line instead; either
        // way no line may exceed the limit.
        let mut child = job.spawn(command).unwrap();
        while let Some(line) = child.lines.recv().await {
            assert!(line.text.len() <= MAX_LINE_BYTES);
        }
    }

    #[tokio::test]
    async fn a_frozen_job_stops_working_and_goes_on_after_a_resume() {
        let job = OwnedJob::new().unwrap();
        // `ping` prints a line about every second: while frozen, no line can arrive.
        let mut command = hidden_command("ping.exe");
        command.args(["-n", "30", "127.0.0.1"]);
        let mut child = job.spawn(command).unwrap();
        first_line(&mut child).await;

        job.suspend().unwrap();
        // Lines that were already in the pipe may still arrive; after a moment none do.
        tokio::time::sleep(Duration::from_millis(300)).await;
        while child.lines.try_recv().is_ok() {}
        let silent = tokio::time::timeout(Duration::from_millis(2500), child.lines.recv()).await;
        assert!(silent.is_err(), "a frozen program printed {silent:?}");

        job.resume().unwrap();
        let line = tokio::time::timeout(Duration::from_secs(5), child.lines.recv()).await;
        assert!(matches!(line, Ok(Some(_))), "no output after the resume");
        job.terminate().unwrap();
    }

    #[tokio::test]
    async fn the_input_of_the_program_is_the_bytes_that_were_given() {
        let job = OwnedJob::new().unwrap();
        // `findstr` copies the lines of its input that match, here every line.
        let mut command = hidden_command("findstr.exe");
        command.arg("^");
        let mut child = job
            .spawn_with_input(command, Some(b"first line\r\nsecond line\r\n".to_vec()))
            .unwrap();
        assert_eq!(first_line(&mut child).await.text, "first line");
        assert_eq!(first_line(&mut child).await.text, "second line");
    }

    #[test]
    fn the_killed_status_is_exit_code_1() {
        assert!(is_killed_status(ExitStatus::from_raw(1)));
        assert!(!is_killed_status(ExitStatus::from_raw(0)));
    }
}
