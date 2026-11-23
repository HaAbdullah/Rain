//! The logic of the lifecycle module: the autostart entry and the reaction to the shell's facts.
//! Section 06 6.4 and 6.5.

use std::sync::Arc;

use rain_types::{ShellFacts, SourceState};
use tracing::{error, info, warn};

use super::autostart::{command_for, same_command};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::health::Health;
use crate::core::shell_facts::ShellFactsCell;
use crate::platform::registry::RunKey;

const SOURCE: &str = "lifecycle";
/// The health source that says the summon hotkey does not work.
const HOTKEY_SOURCE: &str = "hotkey";

pub struct Service {
    run_key: Arc<dyn RunKey>,
    sandbox: bool,
    shell_facts: ShellFactsCell,
    health: Arc<Health>,
}

impl Service {
    pub fn new(
        run_key: Arc<dyn RunKey>,
        sandbox: bool,
        shell_facts: ShellFactsCell,
        health: Arc<Health>,
    ) -> Self {
        Self {
            run_key,
            sandbox,
            shell_facts,
            health,
        }
    }

    /// The effective state of "Start with Windows": an entry exists and Windows has not disabled
    /// it (so a user who switches Rain off in Task Manager sees the switch off). Always `false`
    /// in a sandbox run, which does not even read the real entry.
    pub async fn start_with_windows(&self) -> bool {
        if self.sandbox {
            return false;
        }
        let run_key = Arc::clone(&self.run_key);
        match blocking::run(move || run_key.read()).await {
            Ok(Ok(state)) => state.is_on(),
            Ok(Err(e)) => {
                warn!(error = %e, "the autostart entry could not be read");
                false
            }
            Err(_) => false,
        }
    }

    /// Switches the autostart entry on or off. Switching on needs the path of the shell that
    /// reported itself, so a backend that no shell has ever talked to refuses.
    pub async fn set_start_with_windows(&self, on: bool) -> ApiResult<()> {
        if self.sandbox {
            return Err(ApiError::conflict(
                "sandbox",
                "This setting cannot be changed in a development build.",
            )
            .with_source(SOURCE));
        }
        let run_key = Arc::clone(&self.run_key);
        let changed = if on {
            let command = self.command_from_reported_exe()?;
            blocking::run(move || run_key.enable(&command)).await
        } else {
            blocking::run(move || run_key.disable()).await
        };
        match changed? {
            Ok(()) => Ok(()),
            Err(e) => {
                error!(error = %e, "the autostart entry could not be changed");
                Err(ApiError::failed("Could not change start with Windows.")
                    .with_source(SOURCE)
                    .with_reason("registry"))
            }
        }
    }

    /// The shell reported its facts (`PUT /internal/shell`). Does two things and nothing else:
    /// sets or clears the `hotkey` health source, and repairs the autostart entry when the
    /// shell moved (a reinstall to another folder must not leave a dead entry).
    pub async fn on_shell_facts(&self, facts: &ShellFacts) {
        self.report_hotkey(facts);
        if !self.sandbox {
            self.refresh_autostart_entry(facts).await;
        }
    }

    fn report_hotkey(&self, facts: &ShellFacts) {
        if facts.hotkey_registered {
            self.health.clear_source(HOTKEY_SOURCE);
            return;
        }
        warn!("hotkey not registered");
        let message = facts
            .hotkey_error
            .clone()
            .unwrap_or_else(|| "Another program may be using the summon hotkey".to_owned());
        self.health
            .set_source(HOTKEY_SOURCE, SourceState::Failed, Some(message));
    }

    /// Rewrites the Run value when it exists, is switched on, and differs from the command built
    /// from the reported shell. Nothing is written when there is no value (the user has not
    /// opted in) or when Windows has it disabled (that is the user's choice too).
    async fn refresh_autostart_entry(&self, facts: &ShellFacts) {
        let Ok(wanted) = command_for(&facts.shell_exe) else {
            return;
        };
        let run_key = Arc::clone(&self.run_key);
        let refreshed = blocking::run(move || {
            let state = run_key.read()?;
            match state.command {
                Some(current) if state.is_on() && !same_command(&current, &wanted) => {
                    run_key.enable(&wanted).map(|()| true)
                }
                _ => Ok(false),
            }
        })
        .await;
        match refreshed {
            Ok(Ok(true)) => info!("autostart entry refreshed"),
            Ok(Ok(false)) | Err(_) => {}
            Ok(Err(e)) => warn!(error = %e, "the autostart entry could not be refreshed"),
        }
    }

    fn command_from_reported_exe(&self) -> ApiResult<String> {
        let Some(facts) = self.shell_facts.get() else {
            return Err(
                ApiError::conflict("noShell", "Open the Rain app once, then try again.")
                    .with_source(SOURCE),
            );
        };
        command_for(&facts.shell_exe).map_err(|_| {
            ApiError::invalid("The path of the Rain app cannot be used here.")
                .with_source(SOURCE)
                .with_reason("badPath")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bus::EventBus;
    use crate::platform::registry::RunState;
    use std::io;
    use std::sync::Mutex;

    /// An entry in memory that can be made to fail.
    #[derive(Default)]
    struct FakeRunKey {
        state: Mutex<RunState>,
        fail: Mutex<bool>,
        writes: Mutex<Vec<String>>,
    }

    impl FakeRunKey {
        fn check(&self) -> io::Result<()> {
            if *self.fail.lock().unwrap() {
                Err(io::Error::other("the registry says no"))
            } else {
                Ok(())
            }
        }
    }

    impl RunKey for FakeRunKey {
        fn read(&self) -> io::Result<RunState> {
            self.check()?;
            Ok(self.state.lock().unwrap().clone())
        }

        fn enable(&self, command: &str) -> io::Result<()> {
            self.check()?;
            self.writes.lock().unwrap().push(command.to_owned());
            *self.state.lock().unwrap() = RunState {
                command: Some(command.to_owned()),
                disabled_by_windows: false,
            };
            Ok(())
        }

        fn disable(&self) -> io::Result<()> {
            self.check()?;
            *self.state.lock().unwrap() = RunState::default();
            Ok(())
        }
    }

    struct Setup {
        service: Service,
        key: Arc<FakeRunKey>,
        facts: ShellFactsCell,
        health: Arc<Health>,
    }

    fn setup(sandbox: bool) -> Setup {
        let key = Arc::new(FakeRunKey::default());
        let facts = ShellFactsCell::default();
        let health = Arc::new(Health::new("s".into(), "0.1.0", EventBus::new()));
        let service = Service::new(key.clone(), sandbox, facts.clone(), health.clone());
        Setup {
            service,
            key,
            facts,
            health,
        }
    }

    fn report(setup: &Setup, exe: &str, hotkey_registered: bool) -> ShellFacts {
        let facts = ShellFacts {
            hotkey_registered,
            shell_exe: exe.into(),
            shell_version: "0.1.0".into(),
            ..ShellFacts::default()
        };
        setup.facts.set(facts.clone());
        facts
    }

    #[tokio::test]
    async fn switching_on_writes_the_command_of_the_reported_shell() {
        let setup = setup(false);
        report(&setup, r"C:\Fake\rain.exe", true);

        setup.service.set_start_with_windows(true).await.unwrap();

        assert_eq!(
            setup.key.writes.lock().unwrap().as_slice(),
            [r#""C:\Fake\rain.exe" --hidden"#]
        );
        assert!(setup.service.start_with_windows().await);
        setup.service.set_start_with_windows(false).await.unwrap();
        assert!(!setup.service.start_with_windows().await);
    }

    #[tokio::test]
    async fn an_entry_that_windows_disabled_shows_as_off_and_switching_on_clears_the_mark() {
        let setup = setup(false);
        report(&setup, r"C:\Fake\rain.exe", true);
        *setup.key.state.lock().unwrap() = RunState {
            command: Some("x".into()),
            disabled_by_windows: true,
        };
        assert!(!setup.service.start_with_windows().await);

        setup.service.set_start_with_windows(true).await.unwrap();
        assert!(setup.service.start_with_windows().await);
    }

    #[tokio::test]
    async fn a_sandbox_run_refuses_the_change_and_never_reads_the_entry() {
        let setup = setup(true);
        report(&setup, r"C:\Fake\rain.exe", true);
        *setup.key.state.lock().unwrap() = RunState {
            command: Some("x".into()),
            disabled_by_windows: false,
        };
        let error = setup
            .service
            .set_start_with_windows(true)
            .await
            .unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("sandbox"));
        assert!(
            !setup.service.start_with_windows().await,
            "false without a look"
        );
        assert!(setup.key.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn without_a_reported_shell_switching_on_says_to_open_the_app() {
        let setup = setup(false);
        let error = setup
            .service
            .set_start_with_windows(true)
            .await
            .unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("noShell"));
        // Switching off needs no shell.
        setup.service.set_start_with_windows(false).await.unwrap();
    }

    #[tokio::test]
    async fn a_shell_path_with_a_quote_is_refused() {
        let setup = setup(false);
        report(&setup, "C:\\a\"b\\rain.exe", true);
        let error = setup
            .service
            .set_start_with_windows(true)
            .await
            .unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("badPath"));
    }

    #[tokio::test]
    async fn a_failing_registry_is_a_registry_error_and_reads_as_off() {
        let setup = setup(false);
        report(&setup, r"C:\Fake\rain.exe", true);
        *setup.key.fail.lock().unwrap() = true;

        let error = setup
            .service
            .set_start_with_windows(true)
            .await
            .unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("registry"));
        assert_eq!(error.body.message, "Could not change start with Windows.");
        assert!(!setup.service.start_with_windows().await);
    }

    #[tokio::test]
    async fn the_entry_is_rewritten_when_the_shell_moved() {
        let setup = setup(false);
        let old = ShellFacts {
            shell_exe: r"C:\Old\rain.exe".into(),
            ..ShellFacts::default()
        };
        setup.facts.set(old);
        setup.service.set_start_with_windows(true).await.unwrap();

        let moved = report(&setup, r"D:\New\rain.exe", true);
        setup.service.on_shell_facts(&moved).await;

        assert_eq!(
            setup.key.state.lock().unwrap().command.as_deref(),
            Some(r#""D:\New\rain.exe" --hidden"#)
        );
    }

    #[tokio::test]
    async fn nothing_is_written_when_there_is_no_entry_or_the_user_disabled_it_or_it_is_current() {
        let setup = setup(false);
        let facts = report(&setup, r"D:\New\rain.exe", true);

        setup.service.on_shell_facts(&facts).await; // no entry: the user has not opted in
        assert!(setup.key.writes.lock().unwrap().is_empty());

        *setup.key.state.lock().unwrap() = RunState {
            command: Some(r#""C:\Old\rain.exe" --hidden"#.into()),
            disabled_by_windows: true,
        };
        setup.service.on_shell_facts(&facts).await; // disabled in Task Manager: the user's choice
        assert!(setup.key.writes.lock().unwrap().is_empty());

        *setup.key.state.lock().unwrap() = RunState {
            command: Some(r#""d:\new\RAIN.exe" --hidden"#.into()),
            disabled_by_windows: false,
        };
        setup.service.on_shell_facts(&facts).await; // equal apart from case
        assert!(setup.key.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_sandbox_run_does_not_heal_the_entry() {
        let setup = setup(true);
        *setup.key.state.lock().unwrap() = RunState {
            command: Some(r#""C:\Old\rain.exe" --hidden"#.into()),
            disabled_by_windows: false,
        };
        let facts = report(&setup, r"D:\New\rain.exe", true);
        setup.service.on_shell_facts(&facts).await;
        assert!(setup.key.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_hotkey_source_is_failed_while_the_hotkey_is_not_registered() {
        let setup = setup(true);
        let mut facts = report(&setup, r"C:\Fake\rain.exe", false);
        facts.hotkey_error = Some("The hotkey is already in use".into());
        setup.service.on_shell_facts(&facts).await;

        let sources = setup.health.snapshot().sources;
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "hotkey");
        assert_eq!(sources[0].state, SourceState::Failed);
        assert_eq!(
            sources[0].message.as_deref(),
            Some("The hotkey is already in use")
        );

        facts.hotkey_registered = true;
        setup.service.on_shell_facts(&facts).await;
        assert!(
            setup.health.snapshot().sources.is_empty(),
            "absent when it works"
        );
    }
}
