//! What only the shell knows, as the shell last reported it. Foundation section 6.5.
//!
//! The shell sends its facts with `PUT /internal/shell` after every `hello`, and again when the
//! window is shown or hidden. They are kept in memory only: a new backend start has to learn them
//! again.

use std::sync::{Arc, PoisonError, RwLock};

use rain_types::ShellFacts;
use tokio::sync::watch;

/// Cloning is cheap: all clones share the same facts.
#[derive(Clone)]
pub struct ShellFactsCell(Arc<Inner>);

struct Inner {
    facts: RwLock<Option<ShellFacts>>,
    /// Whether anybody can look at the window now. The modules that read files for the screens
    /// (sessions, notes) wait on this instead of keeping a timer.
    visible: watch::Sender<bool>,
}

impl Default for ShellFactsCell {
    fn default() -> Self {
        // True until a shell says otherwise: a plain browser tab has no shell and is always
        // looked at.
        let (visible, _) = watch::channel(true);
        Self(Arc::new(Inner {
            facts: RwLock::new(None),
            visible,
        }))
    }
}

impl ShellFactsCell {
    /// The facts of the last report, or `None` while no shell has ever reported.
    pub fn get(&self) -> Option<ShellFacts> {
        self.read().clone()
    }

    /// Stores a report. Returns true when it differs from the previous one.
    pub fn set(&self, facts: ShellFacts) -> bool {
        let visible = facts.window_visible;
        let mut stored = self.0.facts.write().unwrap_or_else(PoisonError::into_inner);
        let changed = stored.as_ref() != Some(&facts);
        *stored = Some(facts);
        drop(stored);
        self.0.visible.send_if_modified(|now| {
            let differs = *now != visible;
            *now = visible;
            differs
        });
        changed
    }

    /// Whether the window is shown. True while no shell has reported.
    pub fn window_visible(&self) -> bool {
        *self.0.visible.borrow()
    }

    /// Follows the window: the value changes when the shell shows or hides it.
    pub fn watch_visible(&self) -> watch::Receiver<bool> {
        self.0.visible.subscribe()
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
        self.0.facts.read().unwrap_or_else(PoisonError::into_inner)
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
    fn the_window_counts_as_visible_until_a_shell_hides_it() {
        let cell = ShellFactsCell::default();
        assert!(cell.window_visible());
        let mut watching = cell.watch_visible();

        cell.set(facts(true)); // `window_visible` is false in these facts
        assert!(!cell.window_visible());
        assert!(watching.has_changed().unwrap());
        watching.borrow_and_update();

        // The same state again wakes nobody.
        cell.set(facts(false));
        assert!(!watching.has_changed().unwrap());

        cell.set(ShellFacts {
            window_visible: true,
            ..facts(false)
        });
        assert!(cell.window_visible());
        assert!(watching.has_changed().unwrap());
    }

    #[test]
    fn clones_share_the_facts() {
        let cell = ShellFactsCell::default();
        let clone = cell.clone();
        cell.set(facts(false));
        assert!(!clone.hotkey_available());
    }
}
