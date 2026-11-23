//! The health registry: which data files could not be read, and which data sources work.
//! Foundation section 6.9. It answers `GET /health` and feeds the topic `health`.

use std::sync::{Mutex, PoisonError};

use rain_types::{ConfigIssue, HealthTopic, ShellStatus, SourceState, SourceStatus};

use super::bus::EventBus;

pub struct Health {
    start_id: String,
    version: &'static str,
    bus: EventBus,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    shell: ShellStatus,
    config_issues: Vec<ConfigIssue>,
    sources: Vec<SourceStatus>,
}

impl Health {
    pub fn new(start_id: String, version: &'static str, bus: EventBus) -> Self {
        Self {
            start_id,
            version,
            bus,
            inner: Mutex::default(),
        }
    }

    /// A module reports the state of one of its sources ("steam: off, not installed").
    /// The new state replaces the old one of the same name and is published on the topic `health`.
    pub fn set_source(&self, name: &str, state: SourceState, message: Option<String>) {
        let status = SourceStatus {
            name: name.to_owned(),
            state,
            message,
        };
        {
            let mut inner = self.lock();
            match inner.sources.iter_mut().find(|s| s.name == name) {
                Some(existing) if *existing == status => return, // nothing changed
                Some(existing) => *existing = status,
                None => inner.sources.push(status),
            }
        }
        self.publish();
    }

    /// A source that works again, or never mattered again (the hotkey is registered after all):
    /// it leaves the report. Publishes only when the source was there.
    pub fn clear_source(&self, name: &str) {
        let removed = {
            let mut inner = self.lock();
            let before = inner.sources.len();
            inner.sources.retain(|source| source.name != name);
            inner.sources.len() != before
        };
        if removed {
            self.publish();
        }
    }

    /// Replaces the issues of one data file (`settings.toml`): an empty list says the file is fine
    /// again. Publishes only when the list of that file really changed.
    pub fn set_file_issues(&self, file: &str, issues: Vec<ConfigIssue>) {
        {
            let mut inner = self.lock();
            let current = inner
                .config_issues
                .iter()
                .filter(|issue| issue.file == file);
            if current.eq(issues.iter()) {
                return;
            }
            inner.config_issues.retain(|issue| issue.file != file);
            inner.config_issues.extend(issues);
        }
        self.publish();
    }

    /// The shell reported its facts (`PUT /internal/shell`). Publishes when they changed.
    pub fn set_shell(&self, hotkey_registered: bool) {
        let status = ShellStatus {
            reported: true,
            hotkey_registered: Some(hotkey_registered),
        };
        {
            let mut inner = self.lock();
            if inner.shell == status {
                return;
            }
            inner.shell = status;
        }
        self.publish();
    }

    /// What `GET /health` answers.
    pub fn snapshot(&self) -> rain_types::Health {
        let inner = self.lock();
        rain_types::Health {
            start_id: self.start_id.clone(),
            version: self.version.to_owned(),
            shell: inner.shell.clone(),
            config_issues: inner.config_issues.clone(),
            sources: inner.sources.clone(),
        }
    }

    fn publish(&self) {
        self.bus.publish::<HealthTopic>(&self.snapshot());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while the lock was held cannot leave `Inner` half written: every change is one
        // assignment or one push. So a poisoned lock is safe to use.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health() -> (Health, EventBus) {
        let bus = EventBus::new();
        (Health::new("start-1".into(), "0.1.0", bus.clone()), bus)
    }

    #[test]
    fn a_new_registry_is_empty() {
        let (health, _) = health();
        let snapshot = health.snapshot();
        assert_eq!(snapshot.start_id, "start-1");
        assert!(snapshot.config_issues.is_empty());
        assert!(snapshot.sources.is_empty());
        assert!(!snapshot.shell.reported);
    }

    #[test]
    fn a_source_is_replaced_by_the_next_report_of_the_same_name() {
        let (health, _) = health();
        health.set_source("steam", SourceState::Off, Some("not installed".into()));
        health.set_source("vault", SourceState::Ok, None);
        health.set_source("steam", SourceState::Ok, None);

        let sources = health.snapshot().sources;
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name, "steam");
        assert_eq!(sources[0].state, SourceState::Ok);
    }

    #[test]
    fn a_change_is_published_on_the_topic_health() {
        let (health, bus) = health();
        let mut receiver = bus.subscribe();
        health.set_source("steam", SourceState::Failed, Some("cannot read".into()));

        let frame = receiver.try_recv().unwrap();
        assert_eq!(frame.topic, "health");
        let payload: rain_types::Frame = serde_json::from_str(&frame.json).unwrap();
        assert_eq!(payload.payload["sources"][0]["name"], "steam");
    }

    fn issue(file: &str, message: &str) -> ConfigIssue {
        ConfigIssue {
            file: file.into(),
            line: Some(3),
            message: message.into(),
        }
    }

    #[test]
    fn the_issues_of_one_file_are_replaced_and_the_others_stay() {
        let (health, _) = health();
        health.set_file_issues(
            "settings.toml",
            vec![issue("settings.toml", "expected a value")],
        );
        health.set_file_issues("pins.toml", vec![issue("pins.toml", "bad")]);
        health.set_file_issues("settings.toml", vec![issue("settings.toml", "now another")]);

        let issues = health.snapshot().config_issues;
        assert_eq!(issues.len(), 2);
        let settings = issues.iter().find(|i| i.file == "settings.toml").unwrap();
        assert_eq!(settings.message, "now another");

        health.set_file_issues("settings.toml", vec![]);
        assert_eq!(
            health.snapshot().config_issues.len(),
            1,
            "the file is fine again"
        );
    }

    #[test]
    fn the_same_report_of_a_source_again_publishes_nothing() {
        let (health, bus) = health();
        health.set_source("steam", SourceState::Off, Some("not installed".into()));
        let mut receiver = bus.subscribe();
        health.set_source("steam", SourceState::Off, Some("not installed".into()));
        assert!(receiver.try_recv().is_err());
        health.set_source("steam", SourceState::Ok, None);
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn a_cleared_source_leaves_the_report_and_is_published_once() {
        let (health, bus) = health();
        health.set_source("hotkey", SourceState::Failed, Some("taken".into()));
        let mut receiver = bus.subscribe();

        health.clear_source("hotkey");
        health.clear_source("hotkey");

        assert!(health.snapshot().sources.is_empty());
        assert!(receiver.try_recv().is_ok());
        assert!(
            receiver.try_recv().is_err(),
            "the second call changed nothing"
        );
    }

    #[test]
    fn an_unchanged_list_of_issues_publishes_nothing() {
        let (health, bus) = health();
        health.set_file_issues("settings.toml", vec![]);
        health.set_file_issues("settings.toml", vec![issue("settings.toml", "bad")]);
        let mut receiver = bus.subscribe();
        health.set_file_issues("settings.toml", vec![issue("settings.toml", "bad")]);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn the_shell_report_is_kept_and_published_once_per_change() {
        let (health, bus) = health();
        let mut receiver = bus.subscribe();
        health.set_shell(true);
        health.set_shell(true);
        health.set_shell(false);

        let shell = health.snapshot().shell;
        assert!(shell.reported);
        assert_eq!(shell.hotkey_registered, Some(false));
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_ok());
        assert!(
            receiver.try_recv().is_err(),
            "the repeated report was silent"
        );
    }
}
