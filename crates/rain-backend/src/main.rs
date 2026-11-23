//! The program `rain-backend.exe`. Foundation section 1.3 lists its flags.
//!
//! It is built for the Windows subsystem, so it never has a console window. The shell starts it
//! detached and it logs to a file. A person who runs it in a terminal with a flag that prints
//! (`--console`, `--stop`, `--version`) gets the terminal's console through `attach_parent_console`.

#![windows_subsystem = "windows"]

use std::env;
use std::process::ExitCode;

use rain_backend::config::Config;
use rain_backend::core::logging;
use rain_backend::platform::console::attach_parent_console;
use rain_backend::{StartError, StopOutcome, run, stop};
use rain_types::limits::{BLOCKING_KEEPALIVE, BLOCKING_MAX, RUNTIME_WORKERS};
use tracing::{error, info};

const USAGE: &str = "Usage: rain-backend [--console | --stop [--force] | --version]";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    /// Run the server. With `console`, log to the terminal as well.
    Serve {
        console: bool,
    },
    /// Ask a running backend to stop. With `force`, end its process when it does not leave.
    Stop {
        force: bool,
    },
    Version,
}

fn parse_flags(args: &[String]) -> Result<Command, String> {
    let (mut version, mut stop, mut console, mut force) = (false, false, false, false);
    for arg in args {
        match arg.as_str() {
            "--version" => version = true,
            "--stop" => stop = true,
            "--console" => console = true,
            "--force" => force = true,
            unknown => return Err(format!("Unknown flag {unknown}. {USAGE}")),
        }
    }
    if force && !stop {
        return Err(format!("--force goes with --stop. {USAGE}"));
    }
    Ok(if version {
        Command::Version
    } else if stop {
        Command::Stop { force }
    } else {
        Command::Serve { console }
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match parse_flags(&args) {
        Ok(Command::Version) => {
            attach_parent_console();
            println!("{}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Stop { force }) => {
            attach_parent_console();
            stop_running(force)
        }
        Ok(Command::Serve { console }) => {
            if console {
                attach_parent_console();
            }
            serve(console)
        }
        Err(message) => {
            attach_parent_console();
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

/// The runtime of foundation section 11.5: two worker threads are enough, and idle blocking
/// threads end after a few seconds.
fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(RUNTIME_WORKERS)
        .max_blocking_threads(BLOCKING_MAX)
        .thread_keep_alive(BLOCKING_KEEPALIVE)
        .enable_all()
        .build()
}

fn stop_running(force: bool) -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let result = build_runtime()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| runtime.block_on(stop(&config.pipe_name, force)));
    match result {
        Ok(StopOutcome::NotRunning) => {
            println!("Rain backend was not running.");
            ExitCode::SUCCESS
        }
        Ok(StopOutcome::Stopped) => {
            println!("Rain backend stopped.");
            ExitCode::SUCCESS
        }
        Ok(StopOutcome::Killed) => {
            println!("Rain backend did not stop in time and was ended.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn serve(console: bool) -> ExitCode {
    let mut config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            // Nothing is logged yet, and without `--console` nobody sees this message. A start
            // that fails here is a setup mistake, found by running the program in a terminal.
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    config.log_to_console = console;
    // Kept until the end of `serve`: dropping the guard writes out the queued log lines.
    let _log_guard = match logging::init(&config) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("could not start the log: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    let runtime = match build_runtime() {
        Ok(runtime) => runtime,
        Err(e) => {
            error!(error = %e, "could not start the async runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        // A second backend on the same pipe: not a failure, and not worth more than a log line.
        Err(StartError::AlreadyRunning) => {
            info!("another Rain backend is already running on this pipe; this one ends");
            ExitCode::SUCCESS
        }
        Err(StartError::Failed(e)) => {
            error!(error = %format!("{e:#}"), "the backend failed");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(flags: &[&str]) -> Result<Command, String> {
        let args: Vec<String> = flags.iter().map(|flag| flag.to_string()).collect();
        parse_flags(&args)
    }

    #[test]
    fn no_flag_runs_the_server() {
        assert_eq!(parse(&[]), Ok(Command::Serve { console: false }));
    }

    #[test]
    fn console_runs_the_server_with_a_console() {
        assert_eq!(parse(&["--console"]), Ok(Command::Serve { console: true }));
    }

    #[test]
    fn stop_and_version_are_commands_of_their_own() {
        assert_eq!(parse(&["--stop"]), Ok(Command::Stop { force: false }));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(parse(&["--stop", "--version"]), Ok(Command::Version));
    }

    #[test]
    fn force_goes_with_stop_in_either_order() {
        assert_eq!(
            parse(&["--stop", "--force"]),
            Ok(Command::Stop { force: true })
        );
        assert_eq!(
            parse(&["--force", "--stop"]),
            Ok(Command::Stop { force: true })
        );
    }

    #[test]
    fn force_alone_is_refused_with_the_usage() {
        let message = parse(&["--force"]).unwrap_err();
        assert!(message.contains("--force") && message.contains("Usage"));
    }

    #[test]
    fn an_unknown_flag_is_refused_with_the_usage() {
        let message = parse(&["--nap"]).unwrap_err();
        assert!(message.contains("--nap") && message.contains("Usage"));
    }
}
