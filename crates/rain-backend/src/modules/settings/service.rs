//! The logic of the settings: read them, change them, tell the page, and tell the other modules
//! what they resolve to. Section 06 6.1 and 6.2.
//!
//! Other modules call this `Service` and never parse `settings.toml` themselves:
//! `resolved()` and `watch()` give them the values, `expand_env` and `vault_id` the two helpers
//! that must exist once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rain_types::limits::RESTART_DELAY;
use rain_types::{SUMMON_HOTKEY, Settings, SettingsPatch, SettingsTopic, StopReason};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::SOURCE;
use super::datadir::{self, Folders};
use super::detect::{Detect, Detected, Detector, same_folder};
use super::patch::{Effect, Plan, plan};
use super::resolved::{Resolved, expand_env as expand, vault_override};
use super::store::{KeyEdit, SettingsFile, apply_edits};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::shell_facts::ShellFactsCell;
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::core::throttle::Throttle;
use crate::store::toml_file::TomlFile;
use toml_edit::Value;

/// How often the `settings` topic may be published (section 06 2.3).
const TOPIC_RATE_PER_SECOND: u32 = 1;

pub struct Service {
    file: TomlFile<SettingsFile>,
    detector: Arc<Detector>,
    throttle: Throttle<SettingsTopic>,
    shell_facts: ShellFactsCell,
    /// The other modules and the parts of the state that a change needs. Weak: the state owns
    /// this service.
    state: WeakAppState,
    /// What the other modules read. It changes when the file or the detection changes.
    resolved: watch::Sender<Arc<Resolved>>,
    /// The data folder that a change has chosen and the restart will use. Until the restart the
    /// settings show it, so that the screen already says "Data folder changed".
    pending_data_dir: Mutex<Option<String>>,
    /// What `hotkeyAvailable` was when the page was last told.
    hotkey_available_sent: AtomicBool,
}

impl Service {
    /// Builds the service, looks at the PC once, and fills `Resolved`.
    pub async fn start(
        file: TomlFile<SettingsFile>,
        state: &AppState,
        detect: Arc<dyn Detect>,
    ) -> Self {
        let detector = Arc::new(Detector::new(detect, Arc::clone(&state.clock)));
        let first = {
            let (file_value, detector) = (file.get(), Arc::clone(&detector));
            blocking::run(move || {
                let detected = detector.detected(vault_override(&file_value).as_deref());
                Resolved::build(&file_value, &detected)
            })
            .await
            .unwrap_or_else(|_| Resolved::build(&SettingsFile::default(), &Detected::default()))
        };
        Self {
            file,
            detector,
            throttle: Throttle::new(state.bus.clone(), TOPIC_RATE_PER_SECOND),
            shell_facts: state.shell_facts.clone(),
            state: state.downgrade(),
            resolved: watch::channel(Arc::new(first)).0,
            pending_data_dir: Mutex::new(None),
            hotkey_available_sent: AtomicBool::new(state.shell_facts.hotkey_available()),
        }
    }

    // ---- what other modules call ----

    /// The settings as the other modules read them. Cheap: the value is kept. When the detection
    /// is older than 30 seconds it is looked up again first (a few small reads that take well
    /// under a millisecond; the age is checked here, never by a timer).
    pub fn resolved(&self) -> Arc<Resolved> {
        if self.detector.is_stale() {
            self.recompute(&self.file.get());
        }
        Arc::clone(&self.resolved.borrow())
    }

    /// Fires when `settings.toml` changes what `resolved()` returns.
    pub fn watch(&self) -> watch::Receiver<Arc<Resolved>> {
        self.resolved.subscribe()
    }

    /// Expands `%NAME%` from the environment; a name that does not exist stays as typed. The
    /// one place where this happens: `POST /paths/check` calls it too, so that the glyph in
    /// Settings and the value in `Resolved` agree.
    pub fn expand_env(&self, raw: &str) -> String {
        expand(raw)
    }

    /// The key of the chosen vault in Obsidian's own list, for `obsidian://` links. `None` when
    /// there is no vault or Obsidian does not know it. The vault is matched by path: case does
    /// not matter, `/` equals `\`, a closing slash is ignored.
    pub fn vault_id(&self) -> Option<String> {
        let resolved = self.resolved();
        let vault = resolved.vault.as_deref()?;
        let detected = self
            .detector
            .detected(vault_override(&self.file.get()).as_deref());
        detected
            .vaults
            .iter()
            .find(|entry| same_folder(&entry.path, vault))
            .map(|entry| entry.id.clone())
    }

    // ---- the routes ----

    /// `GET /settings`. Looks at the file first, so that an edit by hand shows at once.
    pub async fn read(&self) -> Settings {
        self.refresh().await;
        self.compose().await
    }

    /// `PATCH /settings`: checks the patch, does what it asks, and returns the new state.
    pub async fn update(&self, patch: SettingsPatch) -> ApiResult<Settings> {
        self.refresh().await;
        // R4: a file that does not parse refuses every change, before anything is applied.
        self.file.writable().map_err(|e| e.with_source(SOURCE))?;
        let format = patch.daily_note_format.clone();
        let plan = plan(patch)?;
        if plan.is_empty() {
            return Ok(self.compose().await); // `{}` is a valid request that changes nothing
        }
        self.check_note_format(format.as_deref().unwrap_or_default())?;
        self.apply(plan).await?;
        self.refresh().await;
        Ok(self.compose().await)
    }

    /// Called by the handler of `PUT /internal/shell` when the shell's facts changed. Publishes
    /// the settings only when `hotkeyAvailable` changed (core cannot build a `Settings`).
    pub async fn on_shell_facts_changed(&self) {
        let available = self.shell_facts.hotkey_available();
        if self.hotkey_available_sent.swap(available, Ordering::SeqCst) != available {
            self.publish().await;
        }
    }

    /// Sends the current settings to every connected page (at most once a second; the newest
    /// wins).
    pub async fn publish(&self) {
        let settings = self.compose().await;
        self.throttle.publish(&settings);
    }

    /// Publishes the settings whenever the file's value changes: after a write of Rain and after
    /// a hand edit that the watcher took in. Also keeps `resolved()` current. Runs until `stop`
    /// is cancelled. It waits for an event and never wakes by itself.
    pub async fn publish_changes(self: Arc<Self>, stop: CancellationToken) {
        let mut changes = self.file.subscribe();
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                changed = changes.changed() => {
                    // An error means the file object is gone: the backend is ending.
                    if changed.is_err() {
                        return;
                    }
                    let value = Arc::clone(&changes.borrow_and_update());
                    self.recompute_blocking(value).await;
                    self.publish().await;
                }
            }
        }
    }

    // ---- doing what a patch asks ----

    async fn apply(&self, plan: Plan) -> ApiResult<()> {
        let Plan { edits, effect } = plan;
        let clears_detection = edits.iter().any(|edit| {
            matches!(
                edit.key,
                "vault_path" | "steam_path" | "epic_path" | "riot_path"
            )
        });
        match effect {
            None => self.write(edits).await?,
            Some(Effect::StartWithWindows(on)) => {
                self.lifecycle()?.set_start_with_windows(on).await?;
                self.publish().await; // the file did not change, so nothing else tells the page
            }
            Some(Effect::DataDir(text)) => self.change_data_dir(&text).await?,
            Some(Effect::MonitorClaude(on)) => self.set_monitor_claude(on).await?,
        }
        if clears_detection {
            self.detector.invalidate();
        }
        Ok(())
    }

    async fn write(&self, edits: Vec<KeyEdit>) -> ApiResult<()> {
        self.file
            .edit(move |document| apply_edits(document, edits))
            .await
            .map(|_| ())
            .map_err(|error| error.with_source(SOURCE))
    }

    /// `monitorClaude`: the Claude settings are changed first and the key is written only after
    /// that worked, so that a failure leaves everything as it was.
    async fn set_monitor_claude(&self, on: bool) -> ApiResult<()> {
        let state = self.app_state()?;
        let sessions = state.sessions();
        if on {
            sessions.install_hooks().await?;
        } else {
            sessions.remove_hooks().await?;
        }
        self.write(vec![KeyEdit {
            key: "monitor_claude",
            value: Some(Value::from(on)),
        }])
        .await
    }

    /// The data folder: checks, copy-when-empty, `machine.toml`, then the restart that makes
    /// every module read the new folder.
    async fn change_data_dir(&self, text: &str) -> ApiResult<()> {
        let state = self.app_state()?;
        let config = &state.config;
        if config.data_dir_from_env {
            return Err(ApiError::conflict(
                "environment",
                "The data folder is set by RAIN_DATA_DIR.",
            )
            .with_source(SOURCE));
        }
        if state.game_mode_active() {
            return Err(ApiError::conflict("gameMode", "Game mode is running.").with_source(SOURCE));
        }
        if state.activity.busy() {
            return Err(ApiError::conflict(
                "busy",
                "Wait until the running jobs end, then try again.",
            )
            .with_source(SOURCE));
        }

        let request = text.to_owned();
        let (current, local, default, machine) = (
            config.data_dir.clone(),
            config.local_dir.clone(),
            config.default_data_dir.clone(),
            config.machine_file.clone(),
        );
        let chosen = blocking::run(move || {
            let folders = Folders {
                current: &current,
                local: &local,
                default: &default,
                machine_file: &machine,
            };
            datadir::change(&request, &folders)
        })
        .await??;

        *self.pending() = Some(chosen.to_string_lossy().into_owned());
        self.publish().await;
        // The page gets `RESTART_DELAY` to finish its own refetch. A timer of foundation 11.5
        // (T4): one sleep for one operation, dropped when it fires.
        let shutdown = Arc::clone(&state.shutdown);
        spawn_named("settings-restart", async move {
            tokio::time::sleep(RESTART_DELAY).await;
            shutdown.request(StopReason::Restart);
        });
        Ok(())
    }

    fn check_note_format(&self, format: &str) -> ApiResult<()> {
        let format = format.trim();
        if format.is_empty() {
            return Ok(());
        }
        let state = self.app_state()?;
        state.life().check_note_format(format).map_err(|_| {
            ApiError::invalid("Not a valid date format")
                .with_source(SOURCE)
                .with_reason("badFormat")
        })
    }

    // ---- composing `Settings` ----

    /// Fills every field: the file's value, else the detected value, else the default.
    async fn compose(&self) -> Settings {
        let file = self.file.get();
        let resolved = Arc::clone(&self.resolved.borrow());
        let detected = self.detector.detected(vault_override(&file).as_deref());
        let (start_with_windows, monitor_claude_state, data_dir) = self.live_values().await;
        Settings {
            hotkey: SUMMON_HOTKEY.to_owned(),
            hotkey_available: self.shell_facts.hotkey_available(),
            hide_on_blur: file.hide_on_blur.unwrap_or(true),
            reset_after_seconds: file
                .reset_after_seconds
                .unwrap_or(rain_types::ResetAfterSeconds::Seconds60),
            start_with_windows,
            data_dir,
            vault_path: shown(
                &file.vault_path,
                detected.single_vault().map(|v| v.path.as_path()),
            ),
            daily_note_format: resolved.daily_note_format.clone(),
            daily_note_folder: resolved.daily_note_folder.clone(),
            project_roots: file.project_roots.clone(),
            game_folders: file.game_folders.clone(),
            steam_enabled: resolved.steam_enabled,
            steam_path: shown(&file.steam_path, detected.steam.as_deref()),
            epic_enabled: resolved.epic_enabled,
            epic_path: shown(&file.epic_path, detected.epic.as_deref()),
            riot_enabled: resolved.riot_enabled,
            riot_path: shown(&file.riot_path, detected.riot.as_deref()),
            // There is no calendar in this round.
            calendar_connected: false,
            monitor_claude: resolved.monitor_claude,
            monitor_claude_state,
            game_mode_countdown_seconds: file
                .game_mode_countdown_seconds
                .unwrap_or(super::resolved::DEFAULT_COUNTDOWN),
        }
    }

    /// The values that come from other modules and the PC: the autostart entry, the state of the
    /// Claude hooks, and the data folder (the pending one after a change).
    async fn live_values(&self) -> (bool, rain_types::MonitorClaudeState, String) {
        let Some(state) = self.state.upgrade() else {
            return (false, rain_types::MonitorClaudeState::Off, String::new());
        };
        let data_dir = self
            .pending()
            .clone()
            .unwrap_or_else(|| state.config.data_dir.to_string_lossy().into_owned());
        (
            state.lifecycle().start_with_windows().await,
            state.sessions().hooks_state(),
            data_dir,
        )
    }

    // ---- keeping `Resolved` current ----

    /// Looks at the file (a hand edit shows through `refresh`) and makes sure the detection is
    /// not older than its time to live.
    async fn refresh(&self) {
        self.file.refresh().await;
        let value = self.file.get();
        if self.detector.is_stale() {
            self.recompute_blocking(value).await;
        } else {
            self.recompute(&value);
        }
    }

    async fn recompute_blocking(&self, file: Arc<SettingsFile>) {
        let detector = Arc::clone(&self.detector);
        let built = blocking::run(move || {
            let detected = detector.detected(vault_override(&file).as_deref());
            Resolved::build(&file, &detected)
        })
        .await;
        if let Ok(resolved) = built {
            self.store_resolved(resolved);
        }
    }

    /// Inline form for `resolved()`, which cannot wait.
    fn recompute(&self, file: &SettingsFile) {
        let detected = self.detector.detected(vault_override(file).as_deref());
        self.store_resolved(Resolved::build(file, &detected));
    }

    /// Replaces `resolved` and wakes the watchers only when it really changed.
    fn store_resolved(&self, resolved: Resolved) {
        self.resolved.send_if_modified(|current| {
            if **current == resolved {
                return false;
            }
            *current = Arc::new(resolved);
            true
        });
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.pending_data_dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn app_state(&self) -> ApiResult<AppState> {
        self.state.upgrade().ok_or_else(|| {
            ApiError::unavailable(SOURCE, "Rain is closing.").with_reason("shuttingDown")
        })
    }

    fn lifecycle(&self) -> ApiResult<Arc<crate::modules::lifecycle::Service>> {
        Ok(self.app_state()?.lifecycle())
    }
}

/// What the Settings screen shows for a path setting: what the user typed, else what Rain
/// detected, else nothing.
fn shown(typed: &str, detected: Option<&Path>) -> String {
    let typed = typed.trim();
    if !typed.is_empty() {
        return typed.to_owned();
    }
    detected
        .map(PathBuf::from)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_path_is_shown_as_typed_else_the_detected_one_else_nothing() {
        assert_eq!(
            shown("  D:/My Vault ", Some(Path::new(r"C:\Detected"))),
            "D:/My Vault"
        );
        assert_eq!(shown("", Some(Path::new(r"C:\Detected"))), r"C:\Detected");
        assert_eq!(shown("  ", None), "");
    }
}
