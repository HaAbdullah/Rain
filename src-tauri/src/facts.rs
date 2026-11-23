//! What only the shell knows, and the report of it to the backend (foundation section 6.5).
//!
//! The backend cannot tell whether the global hotkey could be registered, or where the shell
//! program is (the "Start with Windows" entry must point at it). The shell sends these facts with
//! `PUT /internal/shell` after every `hello`; the backend keeps them in memory.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use hyper::Method;
use hyper::body::Bytes;
use rain_pipe::PipeClient;
use rain_types::limits::API_PREFIX;
use rain_types::{ApiError, ShellFacts};

use crate::backend::timeout_error;

/// A report that takes longer than this is given up; the next `hello` sends it again.
const REPORT_TIMEOUT: Duration = Duration::from_secs(2);

/// The facts of the shell that can change while it runs.
pub struct Facts {
    hotkey: Mutex<Result<(), String>>,
}

impl Facts {
    /// Until the hotkey is registered the facts say that it is not.
    pub fn new() -> Self {
        Self {
            hotkey: Mutex::new(Err("not registered yet".to_owned())),
        }
    }

    /// Records the result of registering the global hotkey. The error text is for the log and
    /// the Settings page, so keep it short.
    pub fn set_hotkey(&self, result: Result<(), String>) {
        *self.hotkey.lock().unwrap_or_else(PoisonError::into_inner) = result;
    }

    /// Tries to register the hotkey again, but only when it is not registered now: the other
    /// program that held the chord may have let go. Returns true when the facts changed, so the
    /// caller knows the backend must be told (foundation section 6.5, section 06 6.5).
    pub fn retry_hotkey(&self, register: impl FnOnce() -> Result<(), String>) -> bool {
        let mut hotkey = self.hotkey.lock().unwrap_or_else(PoisonError::into_inner);
        if hotkey.is_ok() {
            return false;
        }
        let result = register();
        let changed = *hotkey != result;
        *hotkey = result;
        changed
    }

    /// The facts as the backend expects them. `window_visible` is read from the window by the caller.
    pub fn snapshot(&self, window_visible: bool) -> ShellFacts {
        let hotkey = self.hotkey.lock().unwrap_or_else(PoisonError::into_inner);
        ShellFacts {
            hotkey_registered: hotkey.is_ok(),
            hotkey_error: hotkey.clone().err(),
            shell_exe: std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            shell_version: env!("CARGO_PKG_VERSION").to_owned(),
            window_visible,
        }
    }
}

/// `PUT /internal/shell`. The shell may call internal routes; only the page may not.
pub async fn report(client: &PipeClient, facts: &ShellFacts) -> Result<(), ApiError> {
    let body = serde_json::to_vec(facts).expect("the facts are always valid JSON");
    let path = format!("{API_PREFIX}/internal/shell");
    let call = client.request(Method::PUT, &path, Some(Bytes::from(body)), &[]);
    tokio::time::timeout(REPORT_TIMEOUT, call)
        .await
        .unwrap_or_else(|_| Err(timeout_error()))?
        .into_success()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_the_hotkey_is_registered_the_facts_say_it_is_not() {
        let facts = Facts::new().snapshot(false);
        assert!(!facts.hotkey_registered);
        assert!(facts.hotkey_error.is_some());
    }

    #[test]
    fn a_registered_hotkey_has_no_error() {
        let facts = Facts::new();
        facts.set_hotkey(Ok(()));
        let snapshot = facts.snapshot(true);
        assert!(snapshot.hotkey_registered);
        assert_eq!(snapshot.hotkey_error, None);
        assert!(snapshot.window_visible);
    }

    #[test]
    fn a_registered_hotkey_is_not_registered_again() {
        let facts = Facts::new();
        facts.set_hotkey(Ok(()));
        let changed = facts.retry_hotkey(|| panic!("must not try again"));
        assert!(!changed);
    }

    #[test]
    fn a_retry_that_works_changes_the_facts() {
        let facts = Facts::new();
        facts.set_hotkey(Err("already taken".into()));
        assert!(facts.retry_hotkey(|| Ok(())));
        assert!(facts.snapshot(false).hotkey_registered);
    }

    #[test]
    fn a_retry_that_fails_the_same_way_changes_nothing() {
        let facts = Facts::new();
        facts.set_hotkey(Err("already taken".into()));
        assert!(!facts.retry_hotkey(|| Err("already taken".into())));
        assert!(facts.retry_hotkey(|| Err("another reason".into())));
    }
    #[test]
    fn a_failed_registration_carries_its_text() {
        let facts = Facts::new();
        facts.set_hotkey(Err("already taken".into()));
        let snapshot = facts.snapshot(false);
        assert!(!snapshot.hotkey_registered);
        assert_eq!(snapshot.hotkey_error.as_deref(), Some("already taken"));
    }

    #[test]
    fn the_facts_name_this_program_and_version() {
        let snapshot = Facts::new().snapshot(false);
        assert_eq!(snapshot.shell_version, env!("CARGO_PKG_VERSION"));
        assert!(snapshot.shell_exe.to_lowercase().ends_with(".exe"));
    }

    #[test]
    fn the_snapshot_is_the_body_the_backend_reads() {
        let facts = Facts::new();
        facts.set_hotkey(Ok(()));
        let json = serde_json::to_value(facts.snapshot(false)).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in [
            "hotkeyRegistered",
            "shellExe",
            "shellVersion",
            "windowVisible",
        ] {
            assert!(keys.contains(&key), "missing {key}");
        }
        assert!(
            !keys.contains(&"hotkeyError"),
            "an absent error is left out"
        );
    }
}
