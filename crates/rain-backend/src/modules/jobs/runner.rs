//! The shared child runner: starts a program inside an owned job object and follows it until it
//! ends, times out or is cancelled. Section 03 6.11.9. Scheduled jobs and idle jobs both use it.
//!
//! The program and everything it starts live in one job object with kill-on-close, so ending the
//! run ends the whole tree, also when the backend itself crashes (foundation D15).

use std::io;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::command::SpawnSpec;
use crate::platform::job::{JobWaiter, OutputLine, OwnedJob};
use crate::platform::process::hidden_command;

/// How long the output may take to end after the tree was killed.
const DRAIN_WAIT: Duration = Duration::from_secs(2);

/// A program that runs. Keep `job` to end or freeze it; give the rest to `drive`.
pub struct Started {
    pub job: Arc<OwnedJob>,
    waiter: JobWaiter,
    lines: mpsc::Receiver<OutputLine>,
}

/// Starts the program of `spec`. An error is the plain text Windows gave ("program not found").
pub fn start(spec: &SpawnSpec) -> io::Result<Started> {
    let mut command = hidden_command(&spec.program);
    command.args(&spec.args);
    if let Some(raw) = &spec.raw_args {
        command.raw_arg(raw);
    }
    command.current_dir(&spec.cwd);
    // Names are compared case-insensitively on Windows.
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_uppercase();
        if spec
            .env_remove_prefixes
            .iter()
            .any(|prefix| upper.starts_with(prefix))
        {
            command.env_remove(&name);
        }
    }
    for (name, value) in &spec.env_set {
        command.env(name, value);
    }

    let job = Arc::new(OwnedJob::new()?);
    let child = job.spawn_with_input(command, spec.stdin.clone())?;
    let (waiter, lines) = child.split();
    Ok(Started { job, waiter, lines })
}

/// Waits for the next change of the freeze flag; never ends when there is no flag.
async fn frozen_changed(
    frozen: &mut Option<watch::Receiver<bool>>,
) -> Result<(), watch::error::RecvError> {
    match frozen {
        Some(frozen) => frozen.changed().await,
        None => std::future::pending().await,
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The program ended by itself. `None`: Windows gave no exit code.
    Exited(Option<u32>),
    TimedOut,
    /// The cancel token fired (game mode, or Rain stopping).
    Cancelled,
}

/// Follows the run. `on_line` gets every line of output, in order. `frozen` (idle jobs): while it
/// holds `true` the time limit does not run. Always ends the whole tree before it returns, and
/// reads the output to its end, so no line is lost.
pub async fn drive(
    started: Started,
    timeout: Duration,
    cancel: &CancellationToken,
    mut frozen: Option<watch::Receiver<bool>>,
    mut on_line: impl FnMut(OutputLine),
) -> End {
    let Started {
        job,
        mut waiter,
        mut lines,
    } = started;
    let mut lines_open = true;
    let mut remaining = timeout;
    // `None` while frozen: the time limit counts only the time the program may run.
    let mut armed_at = Some(Instant::now());

    let end = loop {
        let deadline = async {
            match armed_at {
                Some(at) => tokio::time::sleep_until(at + remaining).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            line = lines.recv(), if lines_open => match line {
                Some(line) => on_line(line),
                None => lines_open = false,
            },
            status = waiter.wait() => {
                let code = status.ok().and_then(|s| s.code()).map(|c| c as u32);
                break End::Exited(code);
            }
            () = deadline => break End::TimedOut,
            () = cancel.cancelled() => break End::Cancelled,
            changed = frozen_changed(&mut frozen), if frozen.is_some() => {
                if changed.is_err() {
                    frozen = None;
                    continue;
                }
                let is_frozen = frozen.as_ref().is_some_and(|f| *f.borrow());
                if is_frozen {
                    if let Some(at) = armed_at.take() {
                        remaining = remaining.saturating_sub(at.elapsed());
                    }
                } else if armed_at.is_none() {
                    armed_at = Some(Instant::now());
                }
            }
        }
    };

    // Whatever the program left behind goes with it, then the last lines are read.
    let _ = job.terminate();
    let drain = async {
        while let Some(line) = lines.recv().await {
            on_line(line);
        }
    };
    let _ = tokio::time::timeout(DRAIN_WAIT, drain).await;
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::job::Stream;

    fn spec(program: &str, args: &[&str]) -> SpawnSpec {
        SpawnSpec {
            program: program.into(),
            args: args.iter().map(Into::into).collect(),
            raw_args: None,
            cwd: std::env::temp_dir(),
            env_set: vec![("RAIN_TEST_VALUE".to_owned(), "seen".to_owned())],
            env_remove_prefixes: Vec::new(),
            stdin: None,
        }
    }

    async fn run(spec: &SpawnSpec, timeout: Duration) -> (End, Vec<OutputLine>) {
        let started = start(spec).expect("the program started");
        let mut lines = Vec::new();
        let end = drive(started, timeout, &CancellationToken::new(), None, |line| {
            lines.push(line)
        })
        .await;
        (end, lines)
    }

    #[tokio::test]
    async fn a_program_that_ends_gives_its_output_and_exit_code() {
        let (end, lines) = run(
            &spec("cmd.exe", &["/c", "echo from %RAIN_TEST_VALUE% & exit 3"]),
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(end, End::Exited(Some(3)));
        assert_eq!(lines[0].text.trim_end(), "from seen");
        assert_eq!(lines[0].stream, Stream::Stdout);
    }

    #[tokio::test]
    async fn the_time_limit_ends_the_whole_tree() {
        let started = start(&spec("ping.exe", &["-n", "60", "127.0.0.1"])).unwrap();
        let job = Arc::clone(&started.job);
        let end = drive(
            started,
            Duration::from_millis(400),
            &CancellationToken::new(),
            None,
            |_| {},
        )
        .await;
        assert_eq!(end, End::TimedOut);
        assert!(
            job.is_empty() || {
                // The member list is updated a moment after a process ends.
                tokio::time::sleep(Duration::from_millis(300)).await;
                job.is_empty()
            }
        );
    }

    #[tokio::test]
    async fn a_cancel_ends_the_run_at_once() {
        let started = start(&spec("ping.exe", &["-n", "60", "127.0.0.1"])).unwrap();
        let cancel = CancellationToken::new();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stopper.cancel();
        });
        let began = std::time::Instant::now();
        let end = drive(started, Duration::from_secs(60), &cancel, None, |_| {}).await;
        assert_eq!(end, End::Cancelled);
        assert!(began.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn a_program_that_cannot_start_is_an_error() {
        let mut missing = spec("rain-no-such-program-4821.exe", &[]);
        missing.cwd = std::env::temp_dir();
        assert!(start(&missing).is_err());
    }

    #[tokio::test]
    async fn the_time_limit_stands_still_while_the_run_is_frozen() {
        let started = start(&spec("ping.exe", &["-n", "60", "127.0.0.1"])).unwrap();
        let (freeze, frozen) = watch::channel(true);
        let cancel = CancellationToken::new();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            // Frozen for much longer than the limit, then released.
            tokio::time::sleep(Duration::from_millis(900)).await;
            freeze.send_replace(false);
            tokio::time::sleep(Duration::from_millis(1500)).await;
            stopper.cancel();
        });
        let end = drive(
            started,
            Duration::from_millis(500),
            &cancel,
            Some(frozen),
            |_| {},
        )
        .await;
        // 900 ms frozen did not count; after the release 500 ms of running time pass first.
        assert_eq!(end, End::TimedOut);
    }
}
