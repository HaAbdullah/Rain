//! The fakes that tests put in place of the real PC. Foundation section 12.2.
//!
//! Machine facts reach a module through a trait, so a test replaces the real part with one of
//! these (`TestBackend::builder().with::<dyn RunKey>(Arc::new(FakeRunKey::default()))`). The
//! harness already replaces the two that read the PC at start (`FakeDetect`, `FakeRunKey`), so no
//! test reads the real Obsidian, Steam or autostart entry.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use rain_backend::core::clock::Clock;
use rain_backend::core::launch_contract::{OpenError, OpenRequest, Opener, Started};
use rain_backend::core::winddown::{Counts, StopOutcome, Stopped, WindDownPort};
use rain_backend::modules::launcher::{AppRecord, AppSource, IconRenderer, PathProbe, SourceError};
use rain_backend::modules::settings::{Detect, NoteSettings, VaultEntry};
use rain_backend::platform::process::ProcessLister;
use rain_backend::platform::registry::{RunKey, RunState};
use rain_backend::platform::shell::RawApp;
use rain_backend::platform::windows::{WindowInfo, WindowLister};
use tokio::sync::watch;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- detection ----

/// What the PC "has". By default nothing: no vault, no Steam, no Epic.
#[derive(Default)]
pub struct FakeDetect {
    pub vaults: Mutex<Vec<VaultEntry>>,
    /// The notes settings by vault folder.
    pub notes: Mutex<Vec<(PathBuf, NoteSettings)>>,
    pub steam: Mutex<Option<PathBuf>>,
    pub epic: Mutex<Option<PathBuf>>,
    /// The other Epic launcher's folder, for a PC that has both.
    pub epic_extra: Mutex<Option<PathBuf>>,
    pub riot: Mutex<Option<PathBuf>>,
}

impl FakeDetect {
    pub fn with_vaults(vaults: Vec<VaultEntry>) -> Self {
        let fake = Self::default();
        *lock(&fake.vaults) = vaults;
        fake
    }

    pub fn vault(id: &str, path: &str) -> VaultEntry {
        VaultEntry {
            id: id.into(),
            path: PathBuf::from(path),
        }
    }
}

impl Detect for FakeDetect {
    fn vaults(&self) -> Vec<VaultEntry> {
        lock(&self.vaults).clone()
    }

    fn daily_notes(&self, vault: &Path) -> NoteSettings {
        lock(&self.notes)
            .iter()
            .find(|(path, _)| path == vault)
            .map(|(_, notes)| notes.clone())
            .unwrap_or_default()
    }

    fn steam(&self) -> Option<PathBuf> {
        lock(&self.steam).clone()
    }

    fn epic(&self) -> Option<PathBuf> {
        lock(&self.epic).clone()
    }

    fn epic_extra(&self) -> Option<PathBuf> {
        lock(&self.epic_extra).clone()
    }

    fn riot(&self) -> Option<PathBuf> {
        lock(&self.riot).clone()
    }
}

// ---- the autostart entry ----

/// An autostart entry in memory that records what was written and can be made to fail.
#[derive(Default)]
pub struct FakeRunKey {
    pub state: Mutex<RunState>,
    pub fail: Mutex<bool>,
    /// Every command that `enable` was given.
    pub writes: Mutex<Vec<String>>,
}

impl FakeRunKey {
    fn check(&self) -> io::Result<()> {
        if *lock(&self.fail) {
            Err(io::Error::other("the fake registry says no"))
        } else {
            Ok(())
        }
    }
}

impl RunKey for FakeRunKey {
    fn read(&self) -> io::Result<RunState> {
        self.check()?;
        Ok(lock(&self.state).clone())
    }

    fn enable(&self, command: &str) -> io::Result<()> {
        self.check()?;
        lock(&self.writes).push(command.to_owned());
        *lock(&self.state) = RunState {
            command: Some(command.to_owned()),
            disabled_by_windows: false,
        };
        Ok(())
    }

    fn disable(&self) -> io::Result<()> {
        self.check()?;
        *lock(&self.state) = RunState::default();
        Ok(())
    }
}

// ---- what runs on the PC ----

/// The programs that "run": a list that the test sets.
#[derive(Default)]
pub struct FakeProcesses(pub Mutex<Vec<String>>);

impl ProcessLister for FakeProcesses {
    fn images(&self) -> Vec<String> {
        lock(&self.0).clone()
    }
}

/// The windows that "are open": a list that the test sets.
#[derive(Default)]
pub struct FakeWindows(pub Mutex<Vec<WindowInfo>>);

impl WindowLister for FakeWindows {
    fn windows(&self) -> Vec<WindowInfo> {
        lock(&self.0).clone()
    }
}

// ---- time ----

/// A clock that only moves when the test says so. Pair it with `tokio::time::pause` for timers.
pub struct FakeClock(Mutex<SystemTime>);

impl FakeClock {
    pub fn new(start: SystemTime) -> Self {
        Self(Mutex::new(start))
    }

    pub fn advance(&self, by: Duration) {
        *lock(&self.0) += by;
    }

    pub fn set(&self, time: SystemTime) {
        *lock(&self.0) = time;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        *lock(&self.0)
    }
}

// ---- starting programs ----

/// The one fake `Opener` (foundation 11.6): records every request and starts nothing.
#[derive(Default)]
pub struct FakeOpener {
    requests: Mutex<Vec<OpenRequest>>,
    script: Mutex<VecDeque<Result<Started, OpenError>>>,
    delay: Mutex<Duration>,
    next_pid: AtomicU32,
    in_job: AtomicBool,
}

impl FakeOpener {
    /// Every started program from now on says it stayed inside a job of the backend.
    pub fn set_in_job(&self, in_job: bool) {
        self.in_job.store(in_job, Ordering::SeqCst);
    }

    /// The requests so far, oldest first.
    pub fn requests(&self) -> Vec<OpenRequest> {
        lock(&self.requests).clone()
    }

    /// The next call answers with this instead of a started process.
    pub fn fail_next(&self, error: OpenError) {
        lock(&self.script).push_back(Err(error));
    }

    /// Every call waits this long before it answers.
    pub fn delay_calls(&self, delay: Duration) {
        *lock(&self.delay) = delay;
    }
}

impl Opener for FakeOpener {
    fn open(&self, request: OpenRequest) -> BoxFuture<'_, Result<Started, OpenError>> {
        lock(&self.requests).push(request);
        let scripted = lock(&self.script).pop_front();
        let delay = *lock(&self.delay);
        let pid = 1000 + self.next_pid.fetch_add(1, Ordering::SeqCst);
        let in_job = self.in_job.load(Ordering::SeqCst);
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            scripted.unwrap_or(Ok(Started {
                pid: Some(pid),
                creation_time: Some(u64::from(pid)),
                in_job,
            }))
        })
    }
}

// ---- game mode ----

/// A `WindDownPort` with a running count that the test sets, a record of the calls, and a stop
/// that can be slow.
pub struct FakeWindDown {
    counts: Mutex<Counts>,
    running: watch::Sender<u32>,
    /// `pause(true)`, `pause(false)` and `stop_all` in the order they were called.
    pub calls: Mutex<Vec<String>>,
    /// What `stop_all` reports.
    pub stops: Mutex<Vec<Stopped>>,
    /// How long `stop_all` takes.
    pub stop_delay: Mutex<Duration>,
    /// A list that several fakes write to, so a test sees the order of calls across them. Each
    /// entry starts with this fake's `name`.
    pub journal: Journal,
    pub name: &'static str,
    /// Asked at each `pause`, and written into the journal as `active=<answer>`: lets a test see
    /// what the game mode flag was at that moment.
    pub probe: Mutex<Option<Box<dyn Fn() -> bool + Send + Sync>>>,
}

/// Calls in order, shared between fakes.
pub type Journal = std::sync::Arc<Mutex<Vec<String>>>;

impl Default for FakeWindDown {
    fn default() -> Self {
        Self {
            counts: Mutex::default(),
            running: watch::channel(0).0,
            calls: Mutex::default(),
            stops: Mutex::default(),
            stop_delay: Mutex::default(),
            journal: Journal::default(),
            name: "port",
            probe: Mutex::new(None),
        }
    }
}

impl FakeWindDown {
    /// A port that writes its calls into `journal` under `name`.
    pub fn named(name: &'static str, journal: Journal) -> Self {
        Self {
            journal,
            name,
            ..Self::default()
        }
    }

    pub fn set_running(&self, running: u32) {
        lock(&self.counts).running = running;
        self.running.send_replace(running);
    }

    pub fn stopped(name: &str) -> Stopped {
        Stopped {
            name: name.into(),
            outcome: StopOutcome::Stopped,
        }
    }
}

impl WindDownPort for FakeWindDown {
    fn counts(&self) -> Counts {
        *lock(&self.counts)
    }

    fn running_watch(&self) -> watch::Receiver<u32> {
        self.running.subscribe()
    }

    fn pause(&self, paused: bool) {
        lock(&self.calls).push(format!("pause({paused})"));
        let probe = lock(&self.probe);
        let note = match probe.as_ref() {
            Some(active) => format!("{}:pause({paused}) active={}", self.name, active()),
            None => format!("{}:pause({paused})", self.name),
        };
        lock(&self.journal).push(note);
    }

    fn stop_all(&self, _budget: Duration) -> BoxFuture<'_, Vec<Stopped>> {
        lock(&self.calls).push("stop_all".to_owned());
        lock(&self.journal).push(format!("{}:stop_all", self.name));
        let delay = *lock(&self.stop_delay);
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            std::mem::take(&mut *lock(&self.stops))
        })
    }
}

// ---- the launcher (section 01) ----

/// The apps that "are installed": a list that the test sets. The default is no apps at all.
#[derive(Default)]
pub struct FakeAppSource {
    items: Mutex<Vec<RawApp>>,
    failing: Mutex<Option<SourceError>>,
    delay: Mutex<Duration>,
    calls: std::sync::atomic::AtomicUsize,
}

impl FakeAppSource {
    /// The made-up apps of `tests/fixtures/launcher/apps-raw.json`.
    pub fn from_fixture() -> Self {
        let fake = Self::default();
        fake.set_items(fixture_apps());
        fake
    }

    pub fn set_items(&self, items: Vec<RawApp>) {
        *lock(&self.items) = items;
    }

    pub fn items(&self) -> Vec<RawApp> {
        lock(&self.items).clone()
    }

    /// Every read fails with this error until `None`.
    pub fn fail_with(&self, error: Option<SourceError>) {
        *lock(&self.failing) = error;
    }

    /// Every read takes this long.
    pub fn delay_reads(&self, delay: Duration) {
        *lock(&self.delay) = delay;
    }

    /// How often the list was read.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AppSource for FakeAppSource {
    fn enumerate(&self) -> BoxFuture<'_, Result<Vec<RawApp>, SourceError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let delay = *lock(&self.delay);
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            match *lock(&self.failing) {
                Some(error) => Err(error),
                None => Ok(lock(&self.items).clone()),
            }
        })
    }
}

/// How `FakeNeighbours` answers `launch_game`.
pub enum LaunchBehaviour {
    Starts,
    /// Starts, but says the game is inside a job of Rain.
    StartsInJob,
    Fails(String),
    /// Never answers (the budget ends it).
    Hangs,
}

/// Everything game mode asks of the other modules, with no real process behind it. There is no
/// method that stops a session, because `Neighbours` has none.
pub struct FakeNeighbours {
    pub jobs: std::sync::Arc<FakeWindDown>,
    pub idle: std::sync::Arc<FakeWindDown>,
    pub sessions: AtomicU32,
    pub games: Mutex<Vec<(String, rain_backend::modules::gamemode::GameInfo)>>,
    pub launch: Mutex<LaunchBehaviour>,
    /// Calls of every fake in order: `jobs:pause(true)`, `scan:cancel`, `launch:<id>` ...
    pub journal: Journal,
    pub scans_cancelled: AtomicU32,
    /// How many times game mode asked for the number of open sessions: once for each time it
    /// measured its rows. A driver that spins makes this number run away.
    pub measures: AtomicU32,
}

impl Default for FakeNeighbours {
    fn default() -> Self {
        let journal = Journal::default();
        Self {
            jobs: std::sync::Arc::new(FakeWindDown::named("jobs", journal.clone())),
            idle: std::sync::Arc::new(FakeWindDown::named("idle", journal.clone())),
            sessions: AtomicU32::new(0),
            games: Mutex::default(),
            launch: Mutex::new(LaunchBehaviour::Starts),
            journal,
            scans_cancelled: AtomicU32::new(0),
            measures: AtomicU32::new(0),
        }
    }
}

impl FakeNeighbours {
    pub fn with_game(self, id: &str, title: &str, installed: bool) -> Self {
        lock(&self.games).push((
            id.to_owned(),
            rain_backend::modules::gamemode::GameInfo {
                title: title.to_owned(),
                installed,
            },
        ));
        self
    }
}

impl rain_backend::modules::gamemode::Neighbours for FakeNeighbours {
    fn open_user_sessions(&self) -> u32 {
        self.measures.fetch_add(1, Ordering::SeqCst);
        self.sessions.load(Ordering::SeqCst)
    }

    fn jobs(&self) -> &dyn WindDownPort {
        &*self.jobs
    }

    fn idle(&self) -> &dyn WindDownPort {
        &*self.idle
    }

    fn cancel_scan(&self) {
        self.scans_cancelled.fetch_add(1, Ordering::SeqCst);
        lock(&self.journal).push("scan:cancel".to_owned());
    }

    fn game(&self, id: &str) -> Option<rain_backend::modules::gamemode::GameInfo> {
        lock(&self.games)
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, info)| info.clone())
    }

    fn launch_game(&self, id: &str) -> BoxFuture<'_, Result<bool, String>> {
        lock(&self.journal).push(format!("launch:{id}"));
        let answer = match &*lock(&self.launch) {
            LaunchBehaviour::Starts => Some(Ok(false)),
            LaunchBehaviour::StartsInJob => Some(Ok(true)),
            LaunchBehaviour::Fails(message) => Some(Err(message.clone())),
            LaunchBehaviour::Hangs => None,
        };
        Box::pin(async move {
            match answer {
                Some(result) => result,
                None => std::future::pending().await,
            }
        })
    }
}

/// The raw items of the launcher fixture.
pub fn fixture_apps() -> Vec<RawApp> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("launcher")
        .join("apps-raw.json");
    let text = std::fs::read_to_string(path).expect("the launcher fixture exists");
    let items: Vec<serde_json::Value> = serde_json::from_str(&text).expect("the fixture is JSON");
    items
        .iter()
        .map(|item| {
            raw_app(
                item["name"].as_str().unwrap_or_default(),
                item["aumid"].as_str().unwrap_or_default(),
                item["target"].as_str().unwrap_or_default(),
                item["args"].as_str().unwrap_or_default(),
            )
        })
        .collect()
}

pub fn raw_app(name: &str, aumid: &str, target: &str, args: &str) -> RawApp {
    RawApp {
        name: name.into(),
        aumid: aumid.into(),
        target: target.into(),
        args: args.into(),
    }
}

/// Draws a 3-byte "PNG" for every app, can fail for chosen app names, and counts the drawings.
#[derive(Default)]
pub struct FakeIconRenderer {
    failing: Mutex<Vec<String>>,
    pub calls: std::sync::atomic::AtomicUsize,
}

impl FakeIconRenderer {
    pub fn fail_for(&self, names: &[&str]) {
        *lock(&self.failing) = names.iter().map(|name| (*name).to_owned()).collect();
    }
}

impl IconRenderer for FakeIconRenderer {
    fn render<'a>(&'a self, app: &'a AppRecord) -> BoxFuture<'a, Option<Vec<u8>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let fails = lock(&self.failing).contains(&app.name);
        Box::pin(async move { (!fails).then(|| b"png".to_vec()) })
    }
}

/// Which program files "do not exist". By default every file exists.
#[derive(Default)]
pub struct FakePathProbe {
    missing: Mutex<Vec<String>>,
}

impl FakePathProbe {
    pub fn set_missing(&self, paths: &[&str]) {
        *lock(&self.missing) = paths.iter().map(|path| path.to_lowercase()).collect();
    }
}

impl PathProbe for FakePathProbe {
    fn exists(&self, path: &Path) -> Option<bool> {
        let text = path.to_string_lossy().to_lowercase();
        Some(!lock(&self.missing).contains(&text))
    }
}
