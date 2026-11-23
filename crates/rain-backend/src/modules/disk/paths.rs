//! Looking at paths: "does this folder exist?" for Settings (`POST /paths/check`) and "show this
//! item in Explorer" (`POST /shell/reveal`). Section 05 6.9.
//!
//! Both start from `core::paths::normalize`, the one function that turns typed text into a clean
//! absolute path. Both probe the file system through a small guard: at most 4 probes at once and
//! 2 seconds each, so that a dead network drive or a sleeping disk cannot use up the threads that
//! the rest of the backend needs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::future::{BoxFuture, join_all};
use rain_types::limits::{
    PATH_MAX_CHARS, PATH_PROBE_PARALLEL, PATH_PROBE_TIMEOUT, PATHS_CHECK_MAX,
};
use rain_types::{PathCheck, PathKind};
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::core::error::{ApiError, ApiResult};
use crate::core::paths::{PathProblem, normalize};
use crate::platform::com::{self, ComError};
use crate::platform::shell;

/// What a probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Missing,
    Dir,
    File,
    /// The item is there but Windows refused to tell more. A check calls it missing (Rain cannot
    /// use it either); a reveal calls it present (Explorer shows it in its folder).
    Denied,
    /// The location did not answer in time.
    Slow,
}

/// Looks at one path. Blocking. The seam of the tests: a fake answers slowly or not at all.
pub trait PathProbe: Send + Sync {
    /// `follow_links` true looks at what a link points to (a broken link is then missing);
    /// false looks at the link itself.
    fn stat(&self, path: &Path, follow_links: bool) -> Probe;
}

/// The real file system.
pub struct StdProbe;

impl PathProbe for StdProbe {
    fn stat(&self, path: &Path, follow_links: bool) -> Probe {
        let metadata = if follow_links {
            std::fs::metadata(path)
        } else {
            std::fs::symlink_metadata(path)
        };
        match metadata {
            Ok(found) if found.is_dir() => Probe::Dir,
            Ok(_) => Probe::File,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Probe::Denied,
            Err(_) => Probe::Missing,
        }
    }
}

/// Shows an item in Explorer. The seam of the tests: no test may open a window.
pub trait Revealer: Send + Sync {
    fn reveal(&self, path: PathBuf) -> BoxFuture<'static, Result<(), ComError>>;
}

/// The real thing: `SHOpenFolderAndSelectItems` on the COM thread (10 s limit).
pub struct ShellRevealer;

impl Revealer for ShellRevealer {
    fn reveal(&self, path: PathBuf) -> BoxFuture<'static, Result<(), ComError>> {
        Box::pin(com::run(move || shell::reveal(&path)))
    }
}

/// The two path routes.
pub struct Paths {
    prober: Arc<dyn PathProbe>,
    revealer: Arc<dyn Revealer>,
    /// A permit lives inside the blocking call, so a probe that hangs keeps its permit until the
    /// operating system gives the thread back.
    permits: Arc<Semaphore>,
}

impl Paths {
    pub fn new(prober: Arc<dyn PathProbe>, revealer: Arc<dyn Revealer>) -> Self {
        Self {
            prober,
            revealer,
            permits: Arc::new(Semaphore::new(PATH_PROBE_PARALLEL)),
        }
    }

    /// One probe: waits for a permit, runs on the blocking pool, and gives up after 2 seconds.
    async fn probe(&self, path: PathBuf, follow_links: bool) -> Probe {
        let prober = Arc::clone(&self.prober);
        let permits = Arc::clone(&self.permits);
        let work = async move {
            let permit = permits.acquire_owned().await.ok()?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                prober.stat(&path, follow_links)
            })
            .await
            .ok()
        };
        match tokio::time::timeout(PATH_PROBE_TIMEOUT, work).await {
            Ok(Some(probe)) => probe,
            // The probe thread panicked, or the semaphore was closed: say "not there".
            Ok(None) => Probe::Missing,
            Err(_) => {
                // The path itself stays out of the log: it may be under the user's profile.
                warn!("a path probe did not answer in time");
                Probe::Slow
            }
        }
    }

    /// `POST /paths/check`. `sent` is what the caller typed; `expanded` is the same list after
    /// `%VAR%` expansion (the handler does that, the settings module owns it).
    pub async fn check(
        &self,
        sent: Vec<String>,
        expanded: Vec<String>,
    ) -> ApiResult<Vec<PathCheck>> {
        if sent.len() > PATHS_CHECK_MAX {
            return Err(ApiError::invalid("Too many paths to check at once").with_reason("tooMany"));
        }
        if sent
            .iter()
            .any(|path| path.chars().count() > PATH_MAX_CHARS)
        {
            return Err(ApiError::invalid("That path is too long").with_reason("tooLong"));
        }
        debug!(count = sent.len(), "checking paths");
        let probes = expanded.iter().map(|text| async move {
            // Any problem with the text is "does not exist", not an error of the request.
            match normalize(text, true) {
                Ok(path) => self.probe(path, true).await,
                Err(_) => Probe::Missing,
            }
        });
        let found = join_all(probes).await;
        Ok(sent
            .into_iter()
            .zip(found)
            .map(|(path, probe)| match probe {
                Probe::Dir => PathCheck {
                    path,
                    exists: true,
                    kind: Some(PathKind::Dir),
                },
                Probe::File => PathCheck {
                    path,
                    exists: true,
                    kind: Some(PathKind::File),
                },
                Probe::Missing | Probe::Denied | Probe::Slow => PathCheck {
                    path,
                    exists: false,
                    kind: None,
                },
            })
            .collect())
    }

    /// `POST /shell/reveal`.
    pub async fn reveal(&self, raw: &str) -> ApiResult<()> {
        let path = normalize(raw, false).map_err(reveal_problem)?;
        // `SHOpenFolderAndSelectItems` on a missing item opens a parent or fails in an unclear
        // way, and the user must hear that the item is gone.
        match self.probe(path.clone(), false).await {
            Probe::Missing => {
                return Err(ApiError::not_found("That file or folder no longer exists")
                    .with_reason("pathMissing"));
            }
            Probe::Slow => {
                return Err(
                    ApiError::unavailable("disk", "That location did not answer")
                        .with_reason("pathSlow"),
                );
            }
            Probe::Dir | Probe::File | Probe::Denied => {}
        }
        self.revealer
            .reveal(path)
            .await
            .map_err(|error| match error {
                ComError::Busy | ComError::TimedOut => {
                    ApiError::unavailable("disk", "Windows did not answer. Try again.")
                        .with_reason("comBusy")
                }
                other => {
                    warn!(error = %other, "Explorer could not show an item");
                    ApiError::failed("Explorer could not show that item")
                        .with_reason("revealFailed")
                }
            })
    }
}

/// The 400 answer for a path that a reveal cannot use.
fn reveal_problem(problem: PathProblem) -> ApiError {
    match problem {
        PathProblem::NotLocal => {
            ApiError::invalid("Rain does not open network locations in Explorer")
                .with_reason("networkPath")
        }
        PathProblem::Empty
        | PathProblem::TooLong
        | PathProblem::BadChar
        | PathProblem::NotAbsolute
        | PathProblem::Dotdot => {
            ApiError::invalid("That path cannot be shown in Explorer").with_reason("badPath")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeReveal {
        shown: Mutex<Vec<PathBuf>>,
        fail: Option<ComError>,
    }

    impl Revealer for FakeReveal {
        fn reveal(&self, path: PathBuf) -> BoxFuture<'static, Result<(), ComError>> {
            self.shown.lock().unwrap().push(path);
            let result = self.fail.clone().map_or(Ok(()), Err);
            Box::pin(std::future::ready(result))
        }
    }

    fn paths(revealer: Arc<FakeReveal>) -> Paths {
        Paths::new(Arc::new(StdProbe), revealer)
    }

    #[tokio::test]
    async fn the_real_probe_tells_folders_files_and_missing_paths_apart() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let here = temp.path().to_string_lossy().into_owned();
        let file_text = file.to_string_lossy().into_owned();
        let missing = format!("{here}\\nope");
        let sent = vec![
            here.clone(),
            file_text.clone(),
            missing.clone(),
            "foo".to_owned(),
        ];
        let answer = paths(Arc::default())
            .check(sent.clone(), sent.clone())
            .await
            .unwrap();
        assert_eq!(answer.len(), 4);
        assert_eq!(
            (answer[0].exists, answer[0].kind),
            (true, Some(PathKind::Dir))
        );
        assert_eq!(
            (answer[1].exists, answer[1].kind),
            (true, Some(PathKind::File))
        );
        assert!(!answer[2].exists && answer[2].kind.is_none());
        assert!(
            !answer[3].exists,
            "a relative path is not an error, it is missing"
        );
        assert_eq!(answer[3].path, "foo", "the text that was sent is echoed");
    }

    #[tokio::test]
    async fn the_limits_of_a_check_are_errors_of_the_request() {
        let many = vec!["C:\\".to_owned(); PATHS_CHECK_MAX + 1];
        let error = paths(Arc::default())
            .check(many.clone(), many)
            .await
            .unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("tooMany"));
        let long = vec!["C".repeat(PATH_MAX_CHARS + 1)];
        let error = paths(Arc::default())
            .check(long.clone(), long)
            .await
            .unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("tooLong"));
        assert!(
            paths(Arc::default())
                .check(vec![], vec![])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_reveal_refuses_paths_it_cannot_use_before_it_looks_at_the_disk() {
        let fake = Arc::new(FakeReveal::default());
        let paths = paths(Arc::clone(&fake));
        for (raw, reason) in [
            ("", "badPath"),
            ("relative\\x", "badPath"),
            ("C:\\a\\..\\b", "badPath"),
            ("C:\\a*", "badPath"),
            ("\\\\srv\\share\\x", "networkPath"),
            ("\\\\?\\C:\\x", "networkPath"),
        ] {
            let error = paths.reveal(raw).await.unwrap_err();
            assert_eq!(error.body.reason.as_deref(), Some(reason), "{raw:?}");
        }
        assert!(fake.shown.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_reveal_passes_an_existing_path_to_the_shell_and_reports_a_missing_one() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let fake = Arc::new(FakeReveal::default());
        let paths = paths(Arc::clone(&fake));
        paths.reveal(&file.to_string_lossy()).await.unwrap();
        assert_eq!(fake.shown.lock().unwrap().len(), 1);

        let gone = temp.path().join("gone.txt");
        let error = paths.reveal(&gone.to_string_lossy()).await.unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("pathMissing"));
        assert_eq!(
            fake.shown.lock().unwrap().len(),
            1,
            "the shell was not asked"
        );
    }

    #[tokio::test]
    async fn a_failing_shell_call_is_an_error_with_the_right_reason() {
        let temp = tempfile::tempdir().unwrap();
        let here = temp.path().to_string_lossy().into_owned();
        for (error, reason) in [
            (ComError::Busy, "comBusy"),
            (ComError::TimedOut, "comBusy"),
            (ComError::Os { code: 5 }, "revealFailed"),
        ] {
            let fake = Arc::new(FakeReveal {
                fail: Some(error),
                ..FakeReveal::default()
            });
            let error = paths(fake).reveal(&here).await.unwrap_err();
            assert_eq!(error.body.reason.as_deref(), Some(reason));
        }
    }
}
