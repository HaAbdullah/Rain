//! What only the shell knows, as the shell last reported it. Foundation section 6.5.
//!
//! The shell sends its facts with `PUT /internal/shell` after every `hello`. They are kept in
//! memory only: a new backend start has to learn them again.

use std::sync::{Arc, PoisonError, RwLock};

use rain_types::ShellFacts;

/// Cloning is cheap: all clones share the same facts.
#[derive(Clone, Default)]
pub struct ShellFactsCell(Arc<RwLock<Option<ShellFacts>>>);

impl ShellFactsCell {
    /// The facts of the last report, or `None` while no shell has ever reported.
    pub fn get(&self) -> Option<ShellFacts> {
        self.read().clone()
    }

    /// Stores a report. Returns true when it differs from the previous one.
    pub fn set(&self, facts: ShellFacts) -> bool {
        let mut stored = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let changed = stored.as_ref() != Some(&facts);
        *stored = Some(facts);
        changed
    }

    /// Whether the summon hotkey works. True while no shell has reported, so that a plain browser
    /// (no shell at all) shows no false warning.
    pub fn hotkey_available(&self) -> bool {
        self.read()
            .as_ref()
            .is_none_or(|facts| facts.hotkey_registered)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Option<ShellFacts>> {
        // `set` replaces the whole value in one assignment, so a poisoned lock holds a whole value.
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(hotkey_registered: bool) -> ShellFacts {
        ShellFacts {
            hotkey_registered,
            shell_exe: "rain.exe".into(),
            shell_version: "0.1.0".into(),
            ..ShellFacts::default()
        }
    }

    #[test]
    fn the_hotkey_counts_as_available_until_a_shell_says_otherwise() {
        let cell = ShellFactsCell::default();
        assert!(cell.get().is_none());
        assert!(cell.hotkey_available());

        cell.set(facts(false));
        assert!(!cell.hotkey_available());
        cell.set(facts(true));
        assert!(cell.hotkey_available());
    }

    #[test]
    fn set_tells_whether_the_report_changed() {
        let cell = ShellFactsCell::default();
        assert!(cell.set(facts(true)), "the first report is a change");
        assert!(!cell.set(facts(true)), "the same report again is not");
        assert!(cell.set(facts(false)));
    }

    #[test]
    fn clones_share_the_facts() {
        let cell = ShellFactsCell::default();
        let clone = cell.clone();
        cell.set(facts(false));
        assert!(!clone.hotkey_available());
    }
}
