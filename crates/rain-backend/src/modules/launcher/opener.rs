//! The production `Opener`: the one place that really starts programs. Section 01 6.8.1.
//!
//! - `Activate` starts a packaged app by its AUMID.
//! - `ShellExec` goes through `ShellExecuteExW` (shortcuts, URIs, folders, `shell:AppsFolder\..`).
//! - `Spawn` starts a process directly, detached and outside every job (games, exact quoting).
//!
//! The launched program is never inside a kill-on-close job: the backend is not in one and Rain
//! puts user programs into none, so closing Rain or stopping the backend leaves them running.
//! Tests never reach this file: they install `FakeOpener` (foundation 12.2).

use futures_util::future::BoxFuture;
use tracing::warn;

use crate::core::blocking;
use crate::core::launch_contract::{OpenError, OpenRequest, Opener, Started};
use crate::platform::com::{self, ComError};
use crate::platform::process::{self, Detached};
use crate::platform::shell::{self, ShellExecuteError};

pub struct WindowsOpener;

impl Opener for WindowsOpener {
    fn open(&self, request: OpenRequest) -> BoxFuture<'_, Result<Started, OpenError>> {
        Box::pin(async move {
            match request {
                OpenRequest::Activate { aumid, args } => activate(aumid, args).await,
                OpenRequest::ShellExec { file, params, dir } => {
                    shell_exec(&file, params.as_deref(), dir.as_deref()).await
                }
                OpenRequest::Spawn { exe, args_raw, cwd } => {
                    let spawned =
                        blocking::run(move || process::spawn_detached(&exe, &args_raw, &cwd))
                            .await
                            .map_err(|_| failed(None, "the start task ended abnormally"))?;
                    match spawned {
                        Ok(spawned) => {
                            if spawned.detached == Detached::InJob {
                                warn!("a program was started inside a job and may end with Rain");
                            }
                            Ok(Started {
                                pid: Some(spawned.pid),
                                creation_time: Some(spawned.creation_time)
                                    .filter(|time| *time != 0),
                                in_job: spawned.detached == Detached::InJob,
                            })
                        }
                        Err(e) => {
                            let code = e.raw_os_error().and_then(|code| u32::try_from(code).ok());
                            Err(failed(code, &format!("the process could not start: {e}")))
                        }
                    }
                }
            }
        })
    }
}

async fn activate(aumid: String, args: Option<String>) -> Result<Started, OpenError> {
    let pid = com::run(move || shell::activate_application(&aumid, args.as_deref()))
        .await
        .map_err(|e| match e {
            ComError::Os { code } => failed(Some((code as u32) & 0xFFFF), "activation failed"),
            other => failed(None, &other.to_string()),
        })?;
    Ok(Started {
        pid: Some(pid),
        creation_time: process::creation_time(pid),
        in_job: false,
    })
}

async fn shell_exec(
    file: &str,
    params: Option<&str>,
    dir: Option<&str>,
) -> Result<Started, OpenError> {
    shell::shell_execute("open", file, params, dir, 0)
        .await
        .map(|()| Started::default())
        .map_err(|e| match e {
            ShellExecuteError::Code(code) => failed(Some(code), "the shell refused to start it"),
            ShellExecuteError::Com(com) => failed(None, &com.to_string()),
        })
}

/// The error of a start call. The text for the user follows the code (section 01 6.8.1); the
/// reason is for the log and never holds a command line.
fn failed(code: Option<u32>, log_reason: &str) -> OpenError {
    OpenError {
        ui_text: ui_text_for(code),
        code,
        log_reason: format!(
            "{log_reason}{}",
            code.map(|c| format!(" (code {c})")).unwrap_or_default()
        ),
    }
}

/// The short text of a failed start for each cause that has its own (the table of 6.8.1).
pub fn ui_text_for(code: Option<u32>) -> &'static str {
    match code {
        Some(2 | 3) => "Executable not found",
        Some(5) => "Access denied",
        Some(31) => "No program for this file",
        Some(740) => "Needs administrator rights",
        Some(1223) => "Cancelled",
        _ => "Failed to start",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cause_has_its_text() {
        assert_eq!(ui_text_for(Some(2)), "Executable not found");
        assert_eq!(ui_text_for(Some(3)), "Executable not found");
        assert_eq!(ui_text_for(Some(5)), "Access denied");
        assert_eq!(ui_text_for(Some(31)), "No program for this file");
        assert_eq!(ui_text_for(Some(740)), "Needs administrator rights");
        assert_eq!(ui_text_for(Some(1223)), "Cancelled");
        assert_eq!(ui_text_for(Some(87)), "Failed to start");
        assert_eq!(ui_text_for(None), "Failed to start");
    }

    #[test]
    fn the_log_reason_names_the_code_and_never_a_command_line() {
        let error = failed(Some(5), "the shell refused to start it");
        assert_eq!(error.code, Some(5));
        assert!(error.log_reason.ends_with("(code 5)"));
        assert_eq!(failed(None, "x").log_reason, "x");
    }

    #[tokio::test]
    async fn a_shell_start_of_a_missing_file_fails_with_the_not_found_text_and_no_dialog() {
        // The one real call of this file that a test may make: the file does not exist, so
        // nothing starts and (with SEE_MASK_FLAG_NO_UI) nothing is shown.
        let error = WindowsOpener
            .open(OpenRequest::ShellExec {
                file: r"C:\no-such-folder-rain-test\missing.exe".into(),
                params: None,
                dir: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.ui_text, "Executable not found", "{error:?}");
    }

    #[tokio::test]
    async fn a_direct_start_of_a_missing_program_fails_without_starting_anything() {
        let error = WindowsOpener
            .open(OpenRequest::Spawn {
                exe: r"C:\no-such-folder-rain-test\missing.exe".into(),
                args_raw: String::new(),
                cwd: std::env::temp_dir(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.ui_text, "Executable not found", "{error:?}");
    }
}
