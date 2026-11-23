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
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use rain_backend::core::clock::Clock;
use rain_backend::core::launch_contract::{OpenError, OpenRequest, Opener, Started};
use rain_backend::core::winddown::{Counts, StopOutcome, Stopped, WindDownPort};
use rain_backend::modules::settings::{Detect, NoteSettings, VaultEntry};
use rain_backend::platform::process::ProcessLister;
use rain_backend::platform::registry::{RunKey, RunState};
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
}

impl FakeOpener {
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
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            scripted.unwrap_or(Ok(Started {
                pid: Some(pid),
                creation_time: Some(u64::from(pid)),
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
}

impl Default for FakeWindDown {
    fn default() -> Self {
        Self {
            counts: Mutex::default(),
            running: watch::channel(0).0,
            calls: Mutex::default(),
            stops: Mutex::default(),
            stop_delay: Mutex::default(),
        }
    }
}

impl FakeWindDown {
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
    }

    fn stop_all(&self, _budget: Duration) -> BoxFuture<'_, Vec<Stopped>> {
        lock(&self.calls).push("stop_all".to_owned());
        let delay = *lock(&self.stop_delay);
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            std::mem::take(&mut *lock(&self.stops))
        })
    }
}
