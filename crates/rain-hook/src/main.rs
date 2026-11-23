//! `rain-hook`: the program that Claude Code runs as a hook (foundation 1.2, section 03 6.2).
//!
//! Claude Code starts it for each hook event, writes the event as JSON on its standard input, and
//! waits for it. The program forwards the event to the Rain backend over the named pipe and ends.
//!
//! ```text
//! rain-hook.exe event      [--pipe=<name>] --rain-managed=<id>
//! rain-hook.exe permission [--pipe=<name>] --rain-managed=<id>
//! rain-hook.exe selftest
//! ```
//!
//! The one rule that matters: FAIL OPEN. Every way this program can go wrong (Rain is not running,
//! the pipe is busy, the answer is garbage, the input is too large, even a bug that panics) ends
//! with exit code 0 and an empty standard output. For Claude Code that is the same as "no hook":
//! it shows its own prompt and no "hook error" notice. The only time anything is printed is when
//! Rain answered a permission request: then the answer is printed exactly as Rain wrote it.
//!
//! No dependencies and no `unsafe`: the program starts for every event of every Claude session,
//! so it must stay small and cheap (about 20 ms, most of it Windows creating the process).

#![forbid(unsafe_code)]

mod envelope;
mod names;
mod wire;

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// How long `permission` waits for Rain's answer (foundation appendix A: `HOOK_WAIT`). The
/// backend holds a request for 19 seconds, so it answers first and this is only a safety net.
const HOOK_WAIT: Duration = Duration::from_secs(20);
/// How long `event` waits for the `204`.
const EVENT_WAIT: Duration = Duration::from_millis(300);
/// More input than this is not forwarded at all.
const STDIN_MAX: usize = 3 * 1024 * 1024;
/// A busy pipe is tried again this many times, this far apart.
const BUSY_RETRIES: u32 = 10;
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(5);
/// The answer that is read at most.
const ANSWER_MAX: u64 = 256 * 1024;
/// `hook.log` is cut at the start when it grows past this.
const LOG_MAX: u64 = 256 * 1024;

/// Windows error codes of opening a pipe.
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_PIPE_BUSY: i32 = 231;

fn main() {
    // A panic must not become a non-zero exit: Claude Code would show a "hook error".
    let _ = std::panic::catch_unwind(run);
    std::process::exit(0);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Event,
    Permission,
}

fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match args.first().map(String::as_str) {
        Some("event") => Mode::Event,
        Some("permission") => Mode::Permission,
        // `selftest` proves that the copied program starts. Anything unknown does the same.
        _ => return,
    };
    let Some(input) = read_stdin() else { return };
    let Some(pipe) = pipe_name(&args) else { return };

    let (run_id, job_id) = (env_text("RAIN_RUN_ID"), env_text("RAIN_JOB_ID"));
    let entrypoint = env_text("CLAUDE_CODE_ENTRYPOINT");
    let env = envelope::Env {
        run_id: run_id.as_deref(),
        job_id: job_id.as_deref(),
        entrypoint: entrypoint.as_deref(),
        claude_pid: env_text("CLAUDE_PID").and_then(|text| text.parse().ok()),
    };
    let mode_name = if mode == Mode::Permission {
        "permission"
    } else {
        "event"
    };
    let text = envelope::build(mode_name, &env, &input);

    let wait = match mode {
        Mode::Permission => hook_wait(),
        Mode::Event => EVENT_WAIT,
    };
    let Some(answer) = ask_backend(&pipe, &wire::request(&text), wait, mode) else {
        return;
    };
    // Only an answer to a permission request is ever printed, and exactly as Rain wrote it.
    if mode == Mode::Permission && answer.status == 200 && answer.body.first() == Some(&b'{') {
        let mut out = std::io::stdout();
        let _ = out.write_all(&answer.body);
        let _ = out.flush();
    }
}

/// `RAIN_HOOK_WAIT_MS` can only make the wait shorter. A test uses it so that it does not wait 20
/// seconds; the installer never sets it.
fn hook_wait() -> Duration {
    env_text("RAIN_HOOK_WAIT_MS")
        .and_then(|text| text.parse::<u64>().ok())
        .map_or(HOOK_WAIT, |millis| {
            HOOK_WAIT.min(Duration::from_millis(millis))
        })
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The input of Claude Code, trimmed. `None` when it is too large, is not UTF-8, or is not one
/// JSON object (it must start with `{` and end with `}`).
fn read_stdin() -> Option<String> {
    let mut bytes = Vec::new();
    // One byte more than the limit tells "too large" from "exactly at the limit".
    std::io::stdin()
        .take(STDIN_MAX as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    check_input(bytes)
}

/// The rules for the input, apart from reading it so that a test can try them.
fn check_input(bytes: Vec<u8>) -> Option<String> {
    if bytes.len() > STDIN_MAX {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let text = text.trim();
    (text.starts_with('{') && text.ends_with('}')).then(|| text.to_owned())
}

/// The pipe: `--pipe=<name>` (written by the installer for a sandbox backend), else `RAIN_PIPE`,
/// else the name of this user's backend.
fn pipe_name(args: &[String]) -> Option<String> {
    let from_arg = args.iter().find_map(|arg| arg.strip_prefix("--pipe="));
    let from_env = env_text("RAIN_PIPE");
    let user = std::env::var("USERNAME").unwrap_or_default();
    names::pipe_name_for(
        from_arg.or(from_env.as_deref()),
        &user,
        cfg!(debug_assertions),
    )
}

/// Sends the request and waits for the answer. `None` for every failure.
fn ask_backend(pipe: &str, request: &[u8], wait: Duration, mode: Mode) -> Option<wire::Answer> {
    let path = format!(r"\\.\pipe\{pipe}");
    let mut file = open_pipe(&path)?;
    if let Err(e) = file.write_all(request).and_then(|()| file.flush()) {
        log_failure(&format!("could not write to the pipe ({e})"));
        return None;
    }
    // A blocking read on a pipe has no timeout, so a thread does the reading and this thread
    // waits for it with one. When the wait is over the process exits, and the thread with it.
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut raw = Vec::new();
        let _ = file.take(ANSWER_MAX).read_to_end(&mut raw);
        let _ = sender.send(raw);
    });
    let Ok(raw) = receiver.recv_timeout(wait) else {
        // Rain did not answer in time. For `event` that is normal and not worth a line.
        if mode == Mode::Permission {
            log_failure("no answer from Rain in time");
        }
        return None;
    };
    let Some(answer) = wire::parse_answer(&raw) else {
        log_failure("the answer was not understood");
        return None;
    };
    if answer.status != 200 && answer.status != 204 {
        log_failure(&format!("Rain answered with status {}", answer.status));
        return None;
    }
    Some(answer)
}

/// Opens the pipe for reading and writing. `None` when Rain is not running (the normal case,
/// nothing is logged) and for the other failures (logged).
fn open_pipe(path: &str) -> Option<std::fs::File> {
    for attempt in 0..=BUSY_RETRIES {
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => return Some(file),
            Err(e) => match e.raw_os_error() {
                Some(ERROR_FILE_NOT_FOUND) => return None,
                Some(ERROR_PIPE_BUSY) if attempt < BUSY_RETRIES => {
                    // The workspace forbids `thread::sleep` for the backend (it blocks an async
                    // worker). This program has no runtime and nothing else to do.
                    #[allow(clippy::disallowed_methods)]
                    std::thread::sleep(BUSY_RETRY_DELAY);
                }
                Some(ERROR_PIPE_BUSY) => {
                    log_failure("the pipe is busy");
                    return None;
                }
                Some(ERROR_ACCESS_DENIED) => {
                    log_failure("the pipe refused access");
                    return None;
                }
                _ => {
                    log_failure(&format!("could not open the pipe ({e})"));
                    return None;
                }
            },
        }
    }
    None
}

/// Appends one line to `<local>\logs\hook.log`, where `<local>` is the folder above the one that
/// holds this program (`<local>\bin\rain-hook.exe`). Only failures are logged, never the input.
/// A log that cannot be written is ignored: logging must not make the hook fail.
fn log_failure(message: &str) {
    let Some(path) = log_path() else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    let too_big = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_MAX);
    let mut options = OpenOptions::new();
    options.create(true);
    if too_big {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    if let Ok(mut file) = options.open(&path) {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let _ = writeln!(file, "{seconds} {message}");
    }
}

fn log_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("logs").join("hook.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_must_be_one_json_object_within_the_size_limit() {
        assert_eq!(
            check_input(b"  {\"a\":1}\n".to_vec()),
            Some("{\"a\":1}".to_owned())
        );
        for bad in [&b""[..], b"hello", b"[1]", b"{ cut", b"\xff\xfe{}"] {
            assert_eq!(check_input(bad.to_vec()), None, "{bad:?}");
        }
        let mut big = vec![b'{'];
        big.extend(std::iter::repeat_n(b' ', STDIN_MAX));
        big.push(b'}');
        assert_eq!(check_input(big), None);
    }

    #[test]
    fn the_wait_can_only_get_shorter() {
        // `hook_wait` reads the environment, so the rule is checked on its parts.
        assert_eq!(
            HOOK_WAIT.min(Duration::from_millis(50)),
            Duration::from_millis(50)
        );
        assert_eq!(
            HOOK_WAIT.min(Duration::from_secs(3600)),
            HOOK_WAIT,
            "a longer value does not lengthen it"
        );
    }

    #[test]
    fn the_pipe_name_comes_from_the_argument_before_the_default() {
        let args = vec!["event".to_owned(), "--pipe=rain-test-1".to_owned()];
        assert_eq!(pipe_name(&args).as_deref(), Some("rain-test-1"));
        let bad = vec!["event".to_owned(), "--pipe=bad name".to_owned()];
        assert_eq!(pipe_name(&bad), None, "an invalid name means do nothing");
    }
}
