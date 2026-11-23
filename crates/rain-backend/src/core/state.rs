//! The shared state of the backend: one value, cloned into every handler and task.
//! Foundation section 11.3.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::UNIX_EPOCH;

use super::bus::EventBus;
use super::clock::{Clock, SystemClock};
use super::health::Health;
use super::launch_contract::{NotBuiltOpener, Opener};
use super::media::MediaRegistry;
use super::secrets::{MemorySecrets, Secrets};
use super::shell_facts::ShellFactsCell;
use super::shutdown::{Activity, ShutdownHandle};
use crate::config::Config;
use crate::modules::{
    disk, gamemode, games, idle, jobs, launcher, life, lifecycle, machine, projects, sessions,
    settings,
};
use crate::store::db::Db;
use crate::store::hub::ConfigHub;

/// Cloning an `AppState` is cheap: all clones are the same state. Its parts are reached through
/// `Deref`, so code reads `state.bus.publish(..)` and `state.config.sandbox`.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub config: Config,
    /// Unique for every start of the backend process (foundation section 5.2).
    pub start_id: String,
    /// Full path of the running program, for `GET /ping`.
    pub exe_path: String,
    pub bus: EventBus,
    pub health: Arc<Health>,
    /// The user's data folder and the TOML files in it.
    pub hub: ConfigHub,
    /// What the shell last reported (`PUT /internal/shell`).
    pub shell_facts: ShellFactsCell,
    /// SQLite for history and caches (foundation 10.5). Opened at start, closed at the end of a
    /// stop.
    pub db: Db,
    /// Who provides the images of `GET /media/{kind}/{id}`.
    pub media: MediaRegistry,
    pub secrets: Arc<dyn Secrets>,
    pub clock: Arc<dyn Clock>,
    pub shutdown: Arc<ShutdownHandle>,
    pub activity: Activity,
    pub slots: Slots,
    /// Parts that a test replaced (`AppStateBuilder::with`).
    seams: Seams,
    opener: Seam<dyn Opener>,
    /// True from the start of game mode: schedulers and launchers refuse new work.
    game_mode_active: AtomicBool,
    /// Modules that could not start, with the reason their routes answer (`moduleFailed`,
    /// `dbTooNew`).
    failed_modules: Mutex<HashMap<&'static str, &'static str>>,
}

impl Deref for AppState {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// A reference to the state that does not keep it alive. A service holds one when it must call
/// other modules later: the state owns the services, so a strong reference would be a cycle.
#[derive(Clone)]
pub struct WeakAppState(Weak<Inner>);

impl WeakAppState {
    /// The state, or `None` after the backend has been dropped (only at the very end).
    pub fn upgrade(&self) -> Option<AppState> {
        self.0.upgrade().map(AppState)
    }
}

impl AppState {
    pub fn downgrade(&self) -> WeakAppState {
        WeakAppState(Arc::downgrade(&self.0))
    }

    pub fn builder(config: Config) -> AppStateBuilder {
        AppStateBuilder {
            config,
            clock: Arc::new(SystemClock),
            secrets: None,
            opener: None,
            seams: Seams::default(),
        }
    }

    // ---- the services of the modules ----
    // `state.sessions().list()` reads better than `state.slots.sessions.get().list()`. Each one
    // panics when its module has not been initialised: a programming error (see `Slot::get`).

    pub fn settings(&self) -> Arc<settings::Service> {
        self.slots.settings.get()
    }
    pub fn lifecycle(&self) -> Arc<lifecycle::Service> {
        self.slots.lifecycle.get()
    }
    pub fn machine(&self) -> Arc<machine::Service> {
        self.slots.machine.get()
    }
    pub fn disk(&self) -> Arc<disk::Service> {
        self.slots.disk.get()
    }
    pub fn launcher(&self) -> Arc<launcher::Service> {
        self.slots.launcher.get()
    }
    pub fn projects(&self) -> Arc<projects::Service> {
        self.slots.projects.get()
    }
    pub fn games(&self) -> Arc<games::Service> {
        self.slots.games.get()
    }
    pub fn sessions(&self) -> Arc<sessions::Service> {
        self.slots.sessions.get()
    }
    pub fn jobs(&self) -> Arc<jobs::Service> {
        self.slots.jobs.get()
    }
    pub fn idle(&self) -> Arc<idle::Service> {
        self.slots.idle.get()
    }
    pub fn life(&self) -> Arc<life::Service> {
        self.slots.life.get()
    }
    pub fn gamemode(&self) -> Arc<gamemode::Service> {
        self.slots.gamemode.get()
    }

    // ---- the one launch seam ----

    /// The one way a program is started (foundation 11.6). A test's fake, else the real opener
    /// that the launcher installed, else an opener that fails every start.
    pub fn opener(&self) -> Arc<dyn Opener> {
        self.opener
            .get()
            .unwrap_or_else(|| Arc::new(NotBuiltOpener))
    }

    /// The launcher's `init` installs the real opener here. It does nothing when a test has
    /// replaced the opener.
    pub fn install_default_opener(&self, opener: Arc<dyn Opener>) {
        self.opener.set_default(opener);
    }

    /// Another part that a test may have replaced, named by its trait:
    /// `state.seam::<dyn AppSource>()`. A module's `init` asks for it and falls back to its
    /// production part when there is none.
    pub fn seam<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.seams.get::<T>()
    }

    // ---- game mode ----

    /// True while game mode is not `off`: modules start no new scheduled job, idle job, scan,
    /// refresh or note read, and answer `conflict` (reason `gameMode`) when asked to.
    pub fn game_mode_active(&self) -> bool {
        self.game_mode_active.load(Ordering::SeqCst)
    }

    pub fn set_game_mode_active(&self, active: bool) {
        self.game_mode_active.store(active, Ordering::SeqCst);
    }

    // ---- modules that did not start ----

    /// Records that a module is out of service. Its routes then answer `unavailable` with
    /// `reason`.
    pub fn fail_module(&self, module: &'static str, reason: &'static str) {
        self.failed_modules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(module, reason);
    }

    pub fn module_failure(&self, module: &str) -> Option<&'static str> {
        self.failed_modules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module)
            .copied()
    }
}

/// Parts of the state that a test may replace, found by the type of their trait.
#[derive(Default)]
struct Seams(HashMap<TypeId, Box<dyn Any + Send + Sync>>);

impl Seams {
    fn insert<T: ?Sized + Send + Sync + 'static>(&mut self, part: Arc<T>) {
        self.0.insert(TypeId::of::<T>(), Box::new(part));
    }

    fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.0
            .get(&TypeId::of::<T>())
            .and_then(|part| part.downcast_ref::<Arc<T>>())
            .cloned()
    }
}

/// A part with a production default that a module installs in its `init`, and that a test can
/// replace before the backend starts. The replacement wins over the default.
struct Seam<T: ?Sized> {
    replaced: OnceLock<Arc<T>>,
    default: OnceLock<Arc<T>>,
}

impl<T: ?Sized> Seam<T> {
    fn new(replaced: Option<Arc<T>>) -> Self {
        let seam = Self {
            replaced: OnceLock::new(),
            default: OnceLock::new(),
        };
        if let Some(part) = replaced {
            // The cell was just created, so it is empty.
            let _ = seam.replaced.set(part);
        }
        seam
    }

    fn set_default(&self, part: Arc<T>) {
        // A second default is ignored: the first module that installs one decides.
        let _ = self.default.set(part);
    }

    fn get(&self) -> Option<Arc<T>> {
        self.replaced.get().or_else(|| self.default.get()).cloned()
    }
}

/// Builds the state. A test replaces the parts it wants to control.
pub struct AppStateBuilder {
    config: Config,
    clock: Arc<dyn Clock>,
    secrets: Option<Arc<dyn Secrets>>,
    opener: Option<Arc<dyn Opener>>,
    seams: Seams,
}

impl AppStateBuilder {
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The secrets store. The default is one in memory: no feature stores a secret yet, and no
    /// test may touch the real Credential Manager.
    pub fn secrets(mut self, secrets: Arc<dyn Secrets>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    /// Replaces the opener, so that no program is started: `FakeOpener` in the tests.
    pub fn opener(mut self, opener: Arc<dyn Opener>) -> Self {
        self.opener = Some(opener);
        self
    }

    /// Replaces a part that a module reaches through a trait, for example
    /// `.with::<dyn AppSource>(Arc::new(FakeApps::new(..)))`. The module's `init` asks for it
    /// with `state.seam::<dyn AppSource>()`.
    pub fn with<T: ?Sized + Send + Sync + 'static>(mut self, part: Arc<T>) -> Self {
        self.seams.insert(part);
        self
    }

    pub fn build(self) -> AppState {
        let bus = EventBus::new();
        let start_id = new_start_id(self.clock.as_ref());
        let health = Arc::new(Health::new(
            start_id.clone(),
            env!("CARGO_PKG_VERSION"),
            bus.clone(),
        ));
        let hub = ConfigHub::new(self.config.data_dir.clone(), Arc::clone(&health));
        let db = Db::new(self.config.local_dir.join("rain.db"));
        let exe_path = std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        AppState(Arc::new(Inner {
            config: self.config,
            start_id,
            exe_path,
            bus,
            health,
            hub,
            shell_facts: ShellFactsCell::default(),
            db,
            media: MediaRegistry::default(),
            secrets: self
                .secrets
                .unwrap_or_else(|| Arc::new(MemorySecrets::default())),
            clock: self.clock,
            shutdown: Arc::new(ShutdownHandle::new()),
            activity: Activity::default(),
            slots: Slots::default(),
            seams: self.seams,
            opener: Seam::new(self.opener),
            game_mode_active: AtomicBool::new(false),
            failed_modules: Mutex::default(),
        }))
    }
}

/// The time of the start in milliseconds and the process id, both in hexadecimal: `19a3f1c2-1f4c`.
fn new_start_id(clock: &dyn Clock) -> String {
    let millis = clock
        .now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or_default();
    format!("{millis:x}-{:x}", std::process::id())
}

/// The place of one module's `Service`. Empty at first; the module's `init` fills it.
pub struct Slot<T> {
    module: &'static str,
    cell: OnceLock<Arc<T>>,
}

impl<T> Slot<T> {
    fn new(module: &'static str) -> Self {
        Self {
            module,
            cell: OnceLock::new(),
        }
    }

    /// Called once, by the module's `init`. A second call is a bug and panics.
    pub fn set(&self, service: T) {
        assert!(
            self.cell.set(Arc::new(service)).is_ok(),
            "the {} module was initialised twice",
            self.module
        );
    }

    /// The service. Panics when the module was not initialised: that is a programming error, and
    /// the start-up test catches it.
    pub fn get(&self) -> Arc<T> {
        match self.cell.get() {
            Some(service) => Arc::clone(service),
            None => panic!(
                "the {} module was used before it was initialised",
                self.module
            ),
        }
    }
}

/// One slot for each module, in the order of `modules::MODULES`.
pub struct Slots {
    pub settings: Slot<settings::Service>,
    pub lifecycle: Slot<lifecycle::Service>,
    pub machine: Slot<machine::Service>,
    pub disk: Slot<disk::Service>,
    pub launcher: Slot<launcher::Service>,
    pub projects: Slot<projects::Service>,
    pub games: Slot<games::Service>,
    pub sessions: Slot<sessions::Service>,
    pub jobs: Slot<jobs::Service>,
    pub idle: Slot<idle::Service>,
    pub life: Slot<life::Service>,
    pub gamemode: Slot<gamemode::Service>,
}

impl Default for Slots {
    fn default() -> Self {
        Self {
            settings: Slot::new("settings"),
            lifecycle: Slot::new("lifecycle"),
            machine: Slot::new("machine"),
            disk: Slot::new("disk"),
            launcher: Slot::new("launcher"),
            projects: Slot::new("projects"),
            games: Slot::new("games"),
            sessions: Slot::new("sessions"),
            jobs: Slot::new("jobs"),
            idle: Slot::new("idle"),
            life: Slot::new("life"),
            gamemode: Slot::new("gamemode"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    struct FixedClock(SystemTime);

    impl Clock for FixedClock {
        fn now(&self) -> SystemTime {
            self.0
        }
    }

    fn state() -> AppState {
        let config = Config::for_test("rain-unit", "data".into(), "local".into());
        AppState::builder(config).build()
    }

    #[test]
    fn the_start_id_is_the_clock_time_and_the_process_id_in_hex() {
        let clock = FixedClock(UNIX_EPOCH + Duration::from_millis(0x19a3_f1c2));
        let id = new_start_id(&clock);
        assert_eq!(id, format!("19a3f1c2-{:x}", std::process::id()));
    }

    #[test]
    fn a_test_can_replace_the_clock() {
        let config = Config::for_test("rain-unit", "data".into(), "local".into());
        let clock = Arc::new(FixedClock(UNIX_EPOCH + Duration::from_millis(255)));
        let state = AppState::builder(config).clock(clock).build();
        assert!(state.start_id.starts_with("ff-"));
        assert_eq!(state.clock.now(), UNIX_EPOCH + Duration::from_millis(255));
    }

    #[test]
    #[should_panic(expected = "the settings module was used before it was initialised")]
    fn an_empty_slot_names_its_module_when_it_is_used() {
        let slots = Slots::default();
        slots.settings.get();
    }

    #[test]
    #[should_panic(expected = "initialised twice")]
    fn a_slot_cannot_be_filled_twice() {
        let slots = Slots::default();
        slots.gamemode.set(gamemode::Service);
        slots.gamemode.set(gamemode::Service);
    }

    trait Part: Send + Sync {
        fn name(&self) -> &'static str;
    }

    struct Fake;

    impl Part for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
    }

    #[test]
    fn a_part_that_a_test_replaced_is_found_by_its_trait() {
        let config = Config::for_test("rain-unit", "data".into(), "local".into());
        let plain = AppState::builder(config.clone()).build();
        assert!(plain.seam::<dyn Part>().is_none());

        let replaced = AppState::builder(config)
            .with::<dyn Part>(Arc::new(Fake))
            .build();
        assert_eq!(replaced.seam::<dyn Part>().unwrap().name(), "fake");
    }

    struct TaggedOpener(&'static str);

    impl Opener for TaggedOpener {
        fn open(
            &self,
            _request: super::super::launch_contract::OpenRequest,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<
                super::super::launch_contract::Started,
                super::super::launch_contract::OpenError,
            >,
        > {
            let tag = self.0;
            Box::pin(async move {
                Err(super::super::launch_contract::OpenError {
                    ui_text: tag,
                    code: None,
                    log_reason: String::new(),
                })
            })
        }
    }

    async fn tag_of(state: &AppState) -> &'static str {
        let request = super::super::launch_contract::OpenRequest::ShellExec {
            file: "x".into(),
            params: None,
            dir: None,
        };
        state.opener().open(request).await.unwrap_err().ui_text
    }

    #[tokio::test]
    async fn the_opener_is_the_replacement_then_the_installed_default_then_a_failing_one() {
        assert_eq!(tag_of(&state()).await, "Failed to start");

        let state = state();
        state.install_default_opener(Arc::new(TaggedOpener("default")));
        assert_eq!(tag_of(&state).await, "default");

        let config = Config::for_test("rain-unit", "data".into(), "local".into());
        let replaced = AppState::builder(config)
            .opener(Arc::new(TaggedOpener("fake")))
            .build();
        replaced.install_default_opener(Arc::new(TaggedOpener("default")));
        assert_eq!(tag_of(&replaced).await, "fake", "a test's fake wins");
    }

    #[test]
    fn a_failed_module_is_remembered_with_its_reason() {
        let state = state();
        assert_eq!(state.module_failure("jobs"), None);
        state.fail_module("jobs", "dbTooNew");
        assert_eq!(state.module_failure("jobs"), Some("dbTooNew"));
    }

    #[test]
    fn game_mode_is_off_until_it_is_set() {
        let state = state();
        assert!(!state.game_mode_active());
        state.set_game_mode_active(true);
        assert!(state.game_mode_active());
    }
}
