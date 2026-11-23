//! Secrets (tokens, links with a key in them). Foundation section 10.8.
//!
//! No feature stores a secret in this round, so only the trait and the in-memory implementation
//! exist. The first feature that needs one adds the Windows Credential Manager implementation
//! (`keyring-core` with `windows-native-keyring-store`, proven in the spike `crates`) in this
//! file and the two dependency lines; nothing else changes.
//!
//! A TOML file may hold a reference such as `secret = "<name>"`, never the value.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use super::error::ApiResult;

pub trait Secrets: Send + Sync {
    /// `None` means "not set". It is never an error and never a crash.
    fn get(&self, name: &str) -> ApiResult<Option<String>>;
    fn set(&self, name: &str, value: &str) -> ApiResult<()>;
    /// Removing a secret that is not there is a success.
    fn delete(&self, name: &str) -> ApiResult<()>;
}

/// Secrets kept in memory. The tests use it, so they never touch the real Credential Manager.
#[derive(Default)]
pub struct MemorySecrets {
    values: Mutex<HashMap<String, String>>,
}

impl MemorySecrets {
    fn values(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Secrets for MemorySecrets {
    fn get(&self, name: &str) -> ApiResult<Option<String>> {
        Ok(self.values().get(name).cloned())
    }

    fn set(&self, name: &str, value: &str) -> ApiResult<()> {
        self.values().insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> ApiResult<()> {
        self.values().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_stored_read_and_deleted() {
        let secrets = MemorySecrets::default();
        assert_eq!(secrets.get("calendar-link").unwrap(), None);
        secrets.set("calendar-link", "value-1").unwrap();
        assert_eq!(
            secrets.get("calendar-link").unwrap().as_deref(),
            Some("value-1")
        );
        secrets.set("calendar-link", "value-2").unwrap();
        assert_eq!(
            secrets.get("calendar-link").unwrap().as_deref(),
            Some("value-2")
        );
        secrets.delete("calendar-link").unwrap();
        assert_eq!(secrets.get("calendar-link").unwrap(), None);
        secrets.delete("calendar-link").unwrap();
    }
}
