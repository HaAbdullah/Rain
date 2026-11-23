//! The logic and the in-memory state of idle jobs. Section 03 6.12.
//!
//! An idle job is a program that runs while the user is away. One task (the sampler) looks at how
//! long the PC has been idle; `decide.rs` says what to do; this file does it: starts a run in a
//! job object, freezes it when the user comes back, resumes it after the same time of quiet.
//!
//! The sampler exists only while an idle job is enabled and game mode is off (foundation timer
//! T3): with no enabled idle job this module costs nothing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use rain_types::time::iso_from_system_time;
use rain_types::{
    ConfigIssue, IdleJob, IdleJobs, IdleResource, IdleState, LogLevel, LogLine, SourceState,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::decide::{self, Action, Inputs, JobSim};
use super::model::{self, FILE_NAME, ID_PREFIX, IdleDef, IdleFile, TABLE};
use super::store;
use crate::core::bus::EventBus;
use crate::core::clock::Clock;
use crate::core::error::{ApiError, ApiResult};
use crate::core::health::Health;
use crate::core::shutdown::{Activity, ActivityGuard};
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::core::throttle::Throttle;
use crate::core::winddown::{Counts, StopOutcome, Stopped, WindDownPort};
use crate::modules::jobs::entry::entry_lines;
use crate::modules::jobs::file_edit::{self, saved_reason};
use crate::modules::jobs::log::{LogSink, LogStore};
use crate::modules::jobs::runner::{self, End};
use crate::modules::jobs::{Tuning, command, parse_iso, run_id};
use crate::platform::job::{OutputLine, OwnedJob, Stream};
use crate::store::db::Db;
use crate::store::hub::ConfigHub;
use crate::store::toml_file::TomlFile;

/// The frames of the topic `idleJobs` per second at most.
const TOPIC_RATE: u32 = 4;
/// A progress change below this is not published (one percent point).
const PROGRESS_STEP: f64 = 0.01;
/// How long `stop` waits for the runs to end.
const STOP_WAIT: Duration = Duration::from_secs(4);
/// The PC counts as used when its last input was this recent, whatever else the samples say.
const RECENT_INPUT: Duration = Duration::from_secs(3);
/// A sample may be this much later than expected before the difference counts as input.
const SAMPLE_SLACK: Duration = Duration::from_millis(500);

// ---- what the machine says: two seams that a test replaces ----

/// How long the user has not touched the keyboard or the mouse. `None`: Windows refused, and the
/// user counts as present.
pub trait MachineIdle: Send + Sync {
    fn idle_ms(&self) -> Option<u64>;
}

/// Whether a game runs. A game pad does not move the clock of `GetLastInputInfo`, so a running
/// game counts as "the user is here".
pub trait GamesState: Send + Sync {
    fn any_game_running(&self) -> BoxFuture<'_, bool>;
}

/// The real idle time, from the machine module.
struct SystemIdle(WeakAppState);

impl MachineIdle for SystemIdle {
    fn idle_ms(&self) -> Option<u64> {
        self.0.upgrade()?.machine().idle_ms()
    }
}

/// The games module answers "does a game run?" (section 02 6.15). It holds a weak handle, like
/// `SystemIdle`: the idle module must not keep the application state alive.
struct SystemGames(WeakAppState);

impl GamesState for SystemGames {
    fn any_game_running(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match self.0.upgrade() {
                Some(state) => state.games().any_game_running().await,
                None => false, // the backend is ending
            }
        })
    }
}

/// Why `start_run` did not start a run.
enum StartRefused {
    /// This job runs already.
    AlreadyRunning,
    /// Another job (named here) is using the resource.
    ResourceBusy(String),
}

// ---- state ----

/// Why a run was ended on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopWhy {
    GameMode,
    Shutdown,
    /// The job was switched off (or removed from the file).
    Disabled,
    /// Freezing failed: a run that cannot be paused is ended instead.
    PauseFailed,
}

impl StopWhy {
    fn line(self) -> &'static str {
        match self {
            Self::GameMode => "Stopped by game mode",
            Self::Shutdown => "Stopped because Rain stopped",
            Self::Disabled => "Stopped: the job was switched off",
            Self::PauseFailed => "Could not pause; the run was stopped",
        }
    }
}

/// The run of a job that is in progress (running, or frozen).
struct Live {
    manual: bool,
    cancel: CancellationToken,
    /// The job object, once the program has started.
    job: Option<Arc<OwnedJob>>,
    /// Tells the run's time limit that the run is frozen.
    freeze: watch::Sender<bool>,
    started: SystemTime,
    frozen_since: Option<SystemTime>,
    frozen_total: Duration,
    why: Option<StopWhy>,
    sink: Arc<LogSink>,
}

struct JobRt {
    state: IdleState,
    /// The progress the program reported (never decreases), or the value at the last freeze.
    progress: f64,
    reported: bool,
    reported_eta: Option<u32>,
    /// What the last published frame said, to leave out changes under one percent point.
    published_progress: f64,
    last_started: Option<SystemTime>,
    last_ended: Option<SystemTime>,
    last_run_id: Option<String>,
    live: Option<Live>,
}

impl JobRt {
    fn new(state: IdleState) -> Self {
        Self {
            state,
            progress: if state == IdleState::Success {
                1.0
            } else {
                0.0
            },
            reported: false,
            reported_eta: None,
            published_progress: 0.0,
            last_started: None,
            last_ended: None,
            last_run_id: None,
            live: None,
        }
    }
}

#[derive(Default)]
struct State {
    defs: Arc<Vec<Arc<IdleDef>>>,
    jobs: HashMap<String, JobRt>,
    live_count: u32,
}

struct Inner {
    weak: WeakAppState,
    file: TomlFile<IdleFile>,
    hub: ConfigHub,
    db: Db,
    clock: Arc<dyn Clock>,
    health: Arc<Health>,
    local_dir: PathBuf,
    tuning: Tuning,
    logs: LogStore,
    bus: EventBus,
    publisher: Throttle<IdleJobs>,
    state: Mutex<State>,
    machine: Arc<dyn MachineIdle>,
    games: Arc<dyn GamesState>,
    paused: AtomicBool,
    running: watch::Sender<u32>,
    sampler: Mutex<Option<CancellationToken>>,
    sampler_starts: AtomicU64,
    shutdown: CancellationToken,
    activity: Activity,
    next_run_number: AtomicU64,
}

/// The `idle` module's service. Other modules call it, and only it.
pub struct Service {
    inner: Arc<Inner>,
}

fn unknown_job() -> ApiError {
    ApiError::not_found("Idle job not found").with_reason("unknownIdleJob")
}

fn resource_name(resource: IdleResource) -> &'static str {
    match resource {
        IdleResource::Cpu => "CPU",
        IdleResource::Gpu => "GPU",
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn app(&self) -> Option<AppState> {
        self.weak.upgrade()
    }

    /// True while no new run may start and the sampler must not run: game mode is on.
    fn refusing(&self) -> bool {
        self.paused.load(Ordering::SeqCst) || self.app().is_some_and(|app| app.game_mode_active())
    }

    // ---- the list ----

    /// Progress and estimated minutes left, for the frame and the list.
    fn progress_of(def: &IdleDef, runtime: &JobRt, now: SystemTime) -> (f64, Option<u32>) {
        let Some(live) = &runtime.live else {
            return (runtime.progress.clamp(0.0, 1.0), None);
        };
        let running_time = || {
            let mut elapsed = now
                .duration_since(live.started)
                .unwrap_or_default()
                .saturating_sub(live.frozen_total);
            if let Some(since) = live.frozen_since {
                elapsed = elapsed.saturating_sub(now.duration_since(since).unwrap_or_default());
            }
            elapsed
        };
        let progress = match (runtime.reported, def.expected_minutes) {
            (true, _) => runtime.progress,
            (false, Some(expected)) => {
                let total = Duration::from_secs(u64::from(expected) * 60);
                (running_time().as_secs_f64() / total.as_secs_f64()).min(0.95)
            }
            (false, None) => runtime.progress,
        };
        let eta = if runtime.state != IdleState::Running {
            None
        } else if runtime.reported_eta.is_some() {
            runtime.reported_eta
        } else {
            def.expected_minutes.map(|expected| {
                let left =
                    Duration::from_secs(u64::from(expected) * 60).saturating_sub(running_time());
                left.as_secs().div_ceil(60) as u32
            })
        };
        (progress.clamp(0.0, 1.0), eta)
    }

    fn snapshot(&self, def: &IdleDef, runtime: &JobRt, now: SystemTime) -> IdleJob {
        let (progress, eta_minutes) = Self::progress_of(def, runtime, now);
        IdleJob {
            id: def.id.clone(),
            name: def.name.clone(),
            enabled: def.enabled,
            state: runtime.state,
            progress,
            resource: def.resource,
            idle_after_minutes: def.idle_after_minutes,
            eta_minutes,
            last_run_at: runtime.last_ended.map(iso_from_system_time),
        }
    }

    fn list(&self) -> Vec<IdleJob> {
        let now = self.clock.now();
        let state = self.lock();
        state
            .defs
            .iter()
            .filter_map(|def| {
                state
                    .jobs
                    .get(&def.id)
                    .map(|runtime| self.snapshot(def, runtime, now))
            })
            .collect()
    }

    /// Publishes the whole list, and remembers the progress that the frame showed.
    fn publish(&self) {
        // Nobody listens: no frame, and no timer of the throttle. A client that connects later
        // fetches the list again (the `resync` of the stream).
        if !self.bus.has_receivers() {
            return;
        }
        let list = self.list();
        {
            let mut state = self.lock();
            for job in &list {
                if let Some(runtime) = state.jobs.get_mut(&job.id) {
                    runtime.published_progress = job.progress;
                }
            }
        }
        let payload = serde_json::to_value(&list).unwrap_or_default();
        self.publisher.publish(&payload);
    }

    /// Publishes only when a running job's progress moved by a percent point or more.
    fn publish_if_progress_moved(&self) {
        let now = self.clock.now();
        let moved = {
            let state = self.lock();
            state.defs.iter().any(|def| {
                state.jobs.get(&def.id).is_some_and(|runtime| {
                    runtime.live.is_some()
                        && (Self::progress_of(def, runtime, now).0 - runtime.published_progress)
                            .abs()
                            >= PROGRESS_STEP
                })
            })
        };
        if moved {
            self.publish();
        }
    }

    fn set_running_count(&self, state: &State) {
        self.running.send_replace(state.live_count);
    }

    // ---- the file ----

    /// Takes a (re)loaded `idle-jobs.toml` into the state. A job that was switched off while a
    /// run lives has its run ended; a job that was switched on again after being switched off
    /// while frozen is queued again.
    async fn apply_file(self: &Arc<Self>, file: &IdleFile) {
        let loaded = model::load(file);
        {
            let mut state = self.lock();
            let old = Arc::clone(&state.defs);
            let new: Vec<Arc<IdleDef>> = loaded.defs.into_iter().map(Arc::new).collect();
            for def in &new {
                let was = old.iter().find(|before| before.id == def.id);
                let runtime = state
                    .jobs
                    .entry(def.id.clone())
                    .or_insert_with(|| JobRt::new(IdleState::Queued));
                match was {
                    Some(before) if before.enabled && !def.enabled => {
                        stop_live(runtime, StopWhy::Disabled);
                    }
                    Some(before)
                        if !before.enabled
                            && def.enabled
                            && runtime.state == IdleState::Paused
                            && runtime.live.is_none() =>
                    {
                        runtime.state = IdleState::Queued;
                        runtime.progress = 0.0;
                    }
                    _ => {}
                }
            }
            for before in old.iter() {
                if !new.iter().any(|def| def.id == before.id)
                    && let Some(runtime) = state.jobs.get_mut(&before.id)
                {
                    stop_live(runtime, StopWhy::Disabled);
                }
            }
            state.defs = Arc::new(new);
        }
        self.report_issues(&loaded.skipped).await;
        self.publish();
        self.ensure_sampler();
    }

    async fn report_issues(&self, skipped: &[model::Skipped]) {
        let path = self.hub.file_path(FILE_NAME);
        let lines = if skipped.is_empty() {
            Vec::new()
        } else {
            // A blocking read: also gives `TomlFile` time to finish its own report first.
            crate::core::blocking::run(move || std::fs::read_to_string(path).unwrap_or_default())
                .await
                .map(|text| entry_lines(&text, TABLE))
                .unwrap_or_default()
        };
        let issues = skipped
            .iter()
            .map(|entry| ConfigIssue {
                file: FILE_NAME.to_owned(),
                line: lines.get(entry.index).copied(),
                message: entry.message.clone(),
            })
            .collect();
        self.health.set_file_issues(FILE_NAME, issues);
    }

    // ---- the sampler ----

    /// Starts or ends the sampler so that it exists exactly while it is needed.
    fn ensure_sampler(self: &Arc<Self>) {
        let wanted = !self.shutdown.is_cancelled()
            && !self.refusing()
            && self.lock().defs.iter().any(|def| def.enabled);
        let mut slot = self.sampler.lock().unwrap_or_else(PoisonError::into_inner);
        match (wanted, slot.is_some()) {
            (true, false) => {
                let token = self.shutdown.child_token();
                *slot = Some(token.clone());
                self.sampler_starts.fetch_add(1, Ordering::SeqCst);
                info!("idle sampler started");
                spawn_named("idle-sampler", sample_loop(Arc::clone(self), token));
            }
            (false, true) => {
                if let Some(token) = slot.take() {
                    token.cancel();
                }
                self.health.clear_source("idle-sampler");
                info!("idle sampler stopped");
            }
            _ => {}
        }
    }

    /// One look at the machine and what follows from it.
    async fn tick(self: &Arc<Self>, now: SystemTime, idle: Duration, user_present: bool) {
        let actions = {
            let state = self.lock();
            let mut book: Vec<JobSim> = state
                .defs
                .iter()
                .filter_map(|def| state.jobs.get(&def.id).map(|runtime| sim_of(def, runtime)))
                .collect();
            decide::decide(
                &mut book,
                &Inputs {
                    now,
                    idle,
                    user_present,
                },
                &crate::platform::time::local_date,
            )
        };
        for action in actions {
            match action {
                Action::Requeue(id) => self.requeue(&id),
                Action::Start(id) => {
                    let def = self.lock().defs.iter().find(|d| d.id == id).cloned();
                    if let Some(def) = def {
                        let _ = self.start_run(&def, false, Some(idle));
                    }
                }
                Action::Suspend(id) => self.suspend(&id),
                Action::Resume(id) => self.resume(&id, idle),
            }
        }
        self.publish_if_progress_moved();
    }

    fn requeue(&self, id: &str) {
        {
            let mut state = self.lock();
            let Some(runtime) = state.jobs.get_mut(id) else {
                return;
            };
            if !matches!(runtime.state, IdleState::Success | IdleState::Failed) {
                return;
            }
            runtime.state = IdleState::Queued;
            runtime.progress = 0.0;
            runtime.eta_clear();
        }
        self.publish();
    }

    // ---- runs ----

    /// Starts a run of `def`. Fails when a run of this job exists already, or when another job
    /// holds the same resource. Both are checked under the one lock that also records the new
    /// run, so a start by hand and a start by the sampler cannot both take the resource.
    fn start_run(
        self: &Arc<Self>,
        def: &Arc<IdleDef>,
        manual: bool,
        after_idle: Option<Duration>,
    ) -> Result<(), StartRefused> {
        let now = self.clock.now();
        let run_id = run_id(
            "irun",
            now,
            self.next_run_number.fetch_add(1, Ordering::SeqCst),
        );
        let cancel = CancellationToken::new();
        let (freeze, frozen) = watch::channel(false);
        let sink;
        {
            let mut state = self.lock();
            let holder = state.defs.iter().find(|other| {
                other.id != def.id
                    && other.resource == def.resource
                    && state.jobs.get(&other.id).is_some_and(|runtime| {
                        runtime.live.is_some()
                            && matches!(runtime.state, IdleState::Running | IdleState::Paused)
                    })
            });
            if let Some(holder) = holder {
                return Err(StartRefused::ResourceBusy(holder.name.clone()));
            }
            let runtime = state
                .jobs
                .entry(def.id.clone())
                .or_insert_with(|| JobRt::new(IdleState::Queued));
            if runtime.live.is_some() {
                return Err(StartRefused::AlreadyRunning);
            }
            sink = self.logs.open(&run_id);
            runtime.state = IdleState::Running;
            runtime.progress = 0.0;
            runtime.reported = false;
            runtime.reported_eta = None;
            runtime.last_started = Some(now);
            runtime.last_run_id = Some(run_id.clone());
            runtime.live = Some(Live {
                manual,
                cancel: cancel.clone(),
                job: None,
                freeze,
                started: now,
                frozen_since: None,
                frozen_total: Duration::ZERO,
                why: None,
                sink: Arc::clone(&sink),
            });
            state.live_count += 1;
            self.set_running_count(&state);
        }
        self.publish();
        let run = Run {
            inner: Arc::clone(self),
            def: Arc::clone(def),
            run_id,
            manual,
            after_idle,
            cancel,
            frozen,
            sink,
            busy: self.activity.guard(),
        };
        spawn_named("idle-run", run.execute());
        Ok(())
    }

    /// Ends the run of a job on purpose.
    fn stop_run(&self, id: &str, why: StopWhy) {
        if let Some(runtime) = self.lock().jobs.get_mut(id) {
            stop_live(runtime, why);
        }
    }

    fn cancel_all(&self, why: StopWhy) -> Vec<String> {
        let mut state = self.lock();
        let names: HashMap<String, String> = state
            .defs
            .iter()
            .map(|def| (def.id.clone(), def.name.clone()))
            .collect();
        let mut stopped = Vec::new();
        for (id, runtime) in &mut state.jobs {
            if runtime.live.is_some() {
                stop_live(runtime, why);
                stopped.push(names.get(id).cloned().unwrap_or_else(|| id.clone()));
            }
        }
        stopped
    }

    async fn wait_until_no_run(&self) {
        let mut running = self.running.subscribe();
        let _ = running.wait_for(|count| *count == 0).await;
    }

    /// Freezes a running job because the PC is in use.
    fn suspend(&self, id: &str) {
        let (job, sink) = {
            let state = self.lock();
            let Some(runtime) = state.jobs.get(id) else {
                return;
            };
            let Some(live) = &runtime.live else {
                return;
            };
            match (&live.job, runtime.state) {
                (Some(job), IdleState::Running) if !live.manual => {
                    (Arc::clone(job), Arc::clone(&live.sink))
                }
                _ => return,
            }
        };
        match job.suspend() {
            Ok(()) => {
                let now = self.clock.now();
                {
                    let mut state = self.lock();
                    let def = state.defs.iter().find(|d| d.id == id).cloned();
                    if let (Some(def), Some(runtime)) = (def, state.jobs.get_mut(id)) {
                        let (progress, _) = Self::progress_of(&def, runtime, now);
                        runtime.progress = runtime.progress.max(progress);
                        runtime.state = IdleState::Paused;
                        if let Some(live) = &mut runtime.live {
                            live.frozen_since = Some(now);
                            live.freeze.send_replace(true);
                        }
                    }
                }
                sink.warn("Paused: the PC is in use");
                self.publish();
            }
            Err(_) => {
                // A run that cannot be frozen cannot be left running while the user works.
                sink.warn("Could not pause; the run was stopped");
                self.stop_run(id, StopWhy::PauseFailed);
            }
        }
    }

    /// Lets a frozen job go on.
    fn resume(&self, id: &str, after_idle: Duration) {
        let (job, sink) = {
            let state = self.lock();
            let Some(live) = state.jobs.get(id).and_then(|runtime| runtime.live.as_ref()) else {
                return;
            };
            match &live.job {
                Some(job) => (Arc::clone(job), Arc::clone(&live.sink)),
                None => return,
            }
        };
        match job.resume() {
            Ok(()) => {
                let now = self.clock.now();
                {
                    let mut state = self.lock();
                    if let Some(runtime) = state.jobs.get_mut(id) {
                        runtime.state = IdleState::Running;
                        if let Some(live) = &mut runtime.live {
                            if let Some(since) = live.frozen_since.take() {
                                live.frozen_total += now.duration_since(since).unwrap_or_default();
                            }
                            live.freeze.send_replace(false);
                        }
                    }
                }
                sink.info(format!(
                    "Resumed after {} min idle",
                    after_idle.as_secs() / 60
                ));
                self.publish();
            }
            Err(_) => {
                sink.warn("Could not pause; the run was stopped");
                self.stop_run(id, StopWhy::PauseFailed);
            }
        }
    }

    /// A `rain-progress:` or `rain-eta:` line of a running program.
    fn report(&self, id: &str, line: ProgressLine) {
        {
            let mut state = self.lock();
            let Some(runtime) = state.jobs.get_mut(id) else {
                return;
            };
            match line {
                ProgressLine::Progress(value) => {
                    runtime.reported = true;
                    runtime.progress = runtime.progress.max(value);
                }
                ProgressLine::Eta(minutes) => runtime.reported_eta = Some(minutes),
            }
        }
        self.publish_if_progress_moved();
    }
}

impl JobRt {
    fn eta_clear(&mut self) {
        self.reported_eta = None;
        self.reported = false;
    }
}

/// Ends the run of a job: the cancel token fires and the run task does the rest.
fn stop_live(runtime: &mut JobRt, why: StopWhy) {
    if let Some(live) = &mut runtime.live {
        live.why.get_or_insert(why);
        live.cancel.cancel();
    }
}

fn sim_of(def: &IdleDef, runtime: &JobRt) -> JobSim {
    JobSim {
        id: def.id.clone(),
        enabled: def.enabled,
        resource: def.resource,
        idle_after: Duration::from_secs(u64::from(def.idle_after_minutes) * 60),
        state: runtime.state,
        // A run that is starting already holds its resource, before its program exists.
        has_process: runtime.live.is_some(),
        manual: runtime.live.as_ref().is_some_and(|live| live.manual),
        rerun: def.rerun,
        min_gap: Duration::from_secs(u64::from(def.min_gap_minutes) * 60),
        last_started: runtime.last_started,
        last_ended: runtime.last_ended,
    }
}

// ---- progress lines ----

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressLine {
    /// 0.0 to 1.0.
    Progress(f64),
    /// Minutes left.
    Eta(u32),
}

/// `rain-progress: 42%`, `rain-progress: 0.42`, `rain-eta: 12` (case does not matter, the whole
/// line must be one of them). Anything else is not a report.
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    let line = line.trim().to_ascii_lowercase();
    if let Some(value) = line.strip_prefix("rain-progress:") {
        let value = value.trim();
        let (number, percent) = match value.strip_suffix('%') {
            Some(number) => (number.trim(), true),
            None => (value, false),
        };
        let number: f64 = number.parse().ok().filter(|n: &f64| n.is_finite())?;
        let fraction = if percent { number / 100.0 } else { number };
        return Some(ProgressLine::Progress(fraction.clamp(0.0, 1.0)));
    }
    if let Some(value) = line.strip_prefix("rain-eta:") {
        return value.trim().parse().ok().map(ProgressLine::Eta);
    }
    None
}

// ---- the sampler task ----

async fn sample_loop(inner: Arc<Inner>, token: CancellationToken) {
    let mut last_sample = inner.clock.now();
    loop {
        let now = inner.clock.now();
        if inner.refusing() {
            // Game mode: nothing is sampled. `pause(false)` starts a new sampler, so the slot
            // must not go on saying that one exists (no sampler is made while this holds).
            inner
                .sampler
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            break;
        }
        let (idle, user_present) = match inner.machine.idle_ms() {
            None => {
                // The safe reading: the user is present, no run starts, a running run freezes.
                inner.health.set_source(
                    "idle-sampler",
                    SourceState::Failed,
                    Some("Idle detection is not available".to_owned()),
                );
                (Duration::ZERO, true)
            }
            Some(ms) => {
                inner.health.clear_source("idle-sampler");
                let idle = Duration::from_millis(ms);
                // Input since the last sample: the idle time is shorter than the time between
                // the samples.
                let since_last = now.duration_since(last_sample).unwrap_or_default();
                let touched = idle + SAMPLE_SLACK < since_last;
                let present =
                    touched || idle < RECENT_INPUT || inner.games.any_game_running().await;
                (idle, present)
            }
        };
        last_sample = now;
        inner.tick(now, idle, user_present).await;

        let busy = inner.lock().live_count > 0;
        let delay = if busy {
            inner.tuning.idle_fast
        } else {
            inner.tuning.idle_slow
        };
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = token.cancelled() => break,
        }
    }
    debug!("the idle sampler ended");
}

// ---- one run ----

struct Run {
    inner: Arc<Inner>,
    def: Arc<IdleDef>,
    run_id: String,
    manual: bool,
    after_idle: Option<Duration>,
    cancel: CancellationToken,
    frozen: watch::Receiver<bool>,
    sink: Arc<LogSink>,
    busy: ActivityGuard,
}

/// How a run came out.
enum Outcome {
    Success,
    Failed(Option<i64>),
    Stopped(StopWhy),
}

impl Run {
    async fn execute(self) {
        let inner = Arc::clone(&self.inner);
        let started = inner.clock.now();
        if let Err(e) = store::insert_run(
            &inner.db,
            self.run_id.clone(),
            self.def.id.clone(),
            iso_from_system_time(started),
            self.manual,
        )
        .await
        {
            warn!(error = %e, "could not write the start of an idle run");
        }
        self.sink.info(match self.after_idle {
            Some(idle) => format!("Started after {} min idle", idle.as_secs() / 60),
            None => "Started by Run now".to_owned(),
        });
        self.sink
            .info(format!("Using the {}", resource_name(self.def.resource)));

        let outcome = self.run_process().await;
        self.conclude(outcome).await;
    }

    async fn run_process(&self) -> Outcome {
        let def = &self.def;
        let sink = &self.sink;
        let cwd = match self.working_dir().await {
            Ok(cwd) => cwd,
            Err(message) => {
                sink.error(format!("Could not start: {message}"));
                return Outcome::Failed(None);
            }
        };
        let spec = command::command_spec(&def.command, &cwd, &self.run_id, &def.id, &def.env);
        let started = match runner::start(&spec) {
            Ok(started) => started,
            Err(e) => {
                sink.error(format!("Could not start: {e}"));
                return Outcome::Failed(None);
            }
        };
        // From now on the job can be frozen and ended through its job object.
        if let Some(live) = self
            .inner
            .lock()
            .jobs
            .get_mut(&def.id)
            .and_then(|runtime| runtime.live.as_mut())
        {
            live.job = Some(Arc::clone(&started.job));
        }

        let timeout = self.inner.tuning.minute * def.timeout_minutes;
        let end = runner::drive(
            started,
            timeout,
            &self.cancel,
            Some(self.frozen.clone()),
            |line| self.follow(line),
        )
        .await;
        match end {
            End::TimedOut => {
                sink.error(format!(
                    "Stopped after the time limit of {} minutes",
                    def.timeout_minutes
                ));
                Outcome::Failed(None)
            }
            End::Cancelled => {
                let why = self
                    .inner
                    .lock()
                    .jobs
                    .get(&def.id)
                    .and_then(|runtime| runtime.live.as_ref())
                    .and_then(|live| live.why)
                    .unwrap_or(StopWhy::Shutdown);
                // A run that could not be frozen already said so in the log.
                if why != StopWhy::PauseFailed {
                    sink.warn(why.line());
                }
                Outcome::Stopped(why)
            }
            End::Exited(Some(0)) => {
                sink.info("Finished");
                sink.info("Process exited with code 0");
                Outcome::Success
            }
            End::Exited(code) => {
                match code {
                    Some(code) => sink.error(format!("Process exited with code {code}")),
                    None => sink.error("The program ended without an exit code"),
                }
                Outcome::Failed(code.map(i64::from))
            }
        }
    }

    /// One line of output: the log, and a progress report when it is one.
    fn follow(&self, line: OutputLine) {
        if line.stream == Stream::Stderr {
            self.sink.warn(&line.text);
            return;
        }
        if let Some(report) = parse_progress_line(&line.text) {
            self.inner.report(&self.def.id, report);
        }
        self.sink.info(&line.text);
    }

    async fn working_dir(&self) -> Result<PathBuf, String> {
        if let Some(dir) = &self.def.working_dir {
            let check = dir.clone();
            let exists = crate::core::blocking::run(move || check.is_dir())
                .await
                .unwrap_or(false);
            return if exists {
                Ok(dir.clone())
            } else {
                Err(format!(
                    "the working folder {} does not exist",
                    dir.display()
                ))
            };
        }
        let folder = self
            .inner
            .local_dir
            .join("cache")
            .join("idle")
            .join(&self.def.id);
        let made = folder.clone();
        match crate::core::blocking::run(move || std::fs::create_dir_all(&made)).await {
            Ok(Ok(())) => Ok(folder),
            _ => Err(format!(
                "the working folder {} could not be created",
                folder.display()
            )),
        }
    }

    async fn conclude(self, outcome: Outcome) {
        let inner = &self.inner;
        self.sink.finish().await;
        let ended = inner.clock.now();
        let (result, exit_code) = match &outcome {
            Outcome::Success => ("success", Some(0)),
            Outcome::Failed(code) => ("failed", *code),
            Outcome::Stopped(_) => ("stopped", None),
        };
        match store::finish_run(
            &inner.db,
            self.run_id.clone(),
            self.def.id.clone(),
            iso_from_system_time(ended),
            result,
            exit_code,
        )
        .await
        {
            Ok(old) => {
                for run in old {
                    inner.logs.delete(&run).await;
                }
            }
            Err(e) => warn!(error = %e, "could not write the end of an idle run"),
        }
        inner.logs.trim_total().await;

        {
            let mut state = inner.lock();
            if let Some(runtime) = state.jobs.get_mut(&self.def.id) {
                runtime.live = None;
                match outcome {
                    Outcome::Success => {
                        runtime.state = IdleState::Success;
                        runtime.progress = 1.0;
                        runtime.last_ended = Some(ended);
                    }
                    Outcome::Failed(_) => {
                        runtime.state = IdleState::Failed;
                        runtime.last_ended = Some(ended);
                    }
                    // Switched off: kept as paused, with its progress, for display. Anything
                    // else goes back to the queue to start again from the beginning.
                    Outcome::Stopped(StopWhy::Disabled) => runtime.state = IdleState::Paused,
                    Outcome::Stopped(_) => {
                        runtime.state = IdleState::Queued;
                        runtime.progress = 0.0;
                    }
                }
                runtime.reported_eta = None;
            }
            state.live_count = state.live_count.saturating_sub(1);
            inner.set_running_count(&state);
        }
        // The backend is no longer busy once the state is final, before anyone hears of it.
        drop(self.busy);
        inner.publish();
        info!(job = %self.def.id, result, "an idle run ended");
    }
}

// ---- the public service ----

impl Service {
    /// Called by `init`: opens `idle-jobs.toml` and builds the service. Nothing runs yet.
    pub async fn open(state: &AppState) -> ApiResult<Self> {
        if state.hub.data_dir().is_dir() {
            file_edit::ensure_default_file(&state.hub, FILE_NAME, store::DEFAULT_FILE).await;
        }
        let file = store::open(&state.hub).await?;
        let tuning = state.jobs().tuning();
        let machine = state
            .seam::<dyn MachineIdle>()
            .unwrap_or_else(|| Arc::new(SystemIdle(state.downgrade())));
        let games = state
            .seam::<dyn GamesState>()
            .unwrap_or_else(|| Arc::new(SystemGames(state.downgrade())));
        let (running, _) = watch::channel(0);
        let inner = Arc::new(Inner {
            weak: state.downgrade(),
            file,
            hub: state.hub.clone(),
            db: state.db.clone(),
            clock: Arc::clone(&state.clock),
            health: Arc::clone(&state.health),
            local_dir: state.config.local_dir.clone(),
            tuning,
            logs: state.jobs().log_store(),
            bus: state.bus.clone(),
            publisher: Throttle::new(state.bus.clone(), TOPIC_RATE),
            state: Mutex::default(),
            machine,
            games,
            paused: AtomicBool::new(false),
            running,
            sampler: Mutex::new(None),
            sampler_starts: AtomicU64::new(0),
            shutdown: state.shutdown.token().clone(),
            activity: state.activity.clone(),
            next_run_number: AtomicU64::new(0),
        });
        let first = inner.file.get();
        inner.apply_file(&first).await;
        Ok(Self { inner })
    }

    /// Called by `start`: closes runs that were cut off, rebuilds the state of every job from
    /// the history, and starts the watcher of the file and (when needed) the sampler.
    pub async fn start(&self) -> anyhow::Result<()> {
        let inner = &self.inner;
        let now = inner.clock.now();
        let loaded = store::load(&inner.db, iso_from_system_time(now)).await?;
        for (run, _) in &loaded.interrupted {
            inner
                .logs
                .append(
                    run,
                    LogLevel::Warn,
                    "Rain stopped while this run was in progress",
                )
                .await;
        }
        {
            let mut state = inner.lock();
            let defs = Arc::clone(&state.defs);
            for def in defs.iter() {
                let remembered = loaded.remembered.get(&def.id).cloned().unwrap_or_default();
                let started = remembered.last_started_at.as_deref().and_then(parse_iso);
                let ended = remembered.last_ended_at.as_deref().and_then(parse_iso);
                let interrupted = loaded.interrupted.iter().any(|(_, job)| *job == def.id);
                let mut runtime = JobRt::new(IdleState::Queued);
                runtime.last_started = started;
                runtime.last_ended = ended;
                runtime.last_run_id = remembered.last_run_id.clone();
                let base = match remembered.last_result.as_deref() {
                    Some("success") => IdleState::Success,
                    Some("failed") => IdleState::Failed,
                    _ => IdleState::Queued,
                };
                runtime.state = if interrupted || remembered.last_result.is_none() {
                    IdleState::Queued
                } else {
                    let mut sim = sim_of(def, &runtime);
                    sim.state = base;
                    if def.enabled
                        && decide::rerun_due(&sim, now, &crate::platform::time::local_date)
                    {
                        IdleState::Queued
                    } else {
                        base
                    }
                };
                runtime.progress = if runtime.state == IdleState::Success {
                    1.0
                } else {
                    0.0
                };
                state.jobs.insert(def.id.clone(), runtime);
            }
        }
        spawn_named("idle-reload", reload_loop(Arc::clone(inner)));
        inner.publish();
        inner.ensure_sampler();
        Ok(())
    }

    /// Called by `stop`: ends the runs (marked as stopped on purpose) and the sampler.
    pub async fn stop(&self) {
        let inner = &self.inner;
        inner.cancel_all(StopWhy::Shutdown);
        let _ = tokio::time::timeout(STOP_WAIT, inner.wait_until_no_run()).await;
        if let Some(token) = inner
            .sampler
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            token.cancel();
        }
    }

    /// The same list as `GET /idle-jobs`, from memory, no I/O.
    ///
    /// The launcher maps this list to search items of the type `idleJob`.
    pub fn list(&self) -> Vec<IdleJob> {
        self.inner.list()
    }

    fn def(&self, id: &str) -> ApiResult<Arc<IdleDef>> {
        self.inner
            .lock()
            .defs
            .iter()
            .find(|def| def.id == id)
            .cloned()
            .ok_or_else(unknown_job)
    }

    fn job(&self, id: &str) -> ApiResult<IdleJob> {
        let now = self.inner.clock.now();
        let state = self.inner.lock();
        let def = state
            .defs
            .iter()
            .find(|def| def.id == id)
            .ok_or_else(unknown_job)?;
        let runtime = state.jobs.get(id).ok_or_else(unknown_job)?;
        Ok(self.inner.snapshot(def, runtime, now))
    }

    /// `PATCH /idle-jobs/{id}`.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> ApiResult<IdleJob> {
        self.def(id)?;
        let wanted = id.to_owned();
        let file = self
            .inner
            .file
            .edit(move |doc| {
                file_edit::set_enabled(doc, FILE_NAME, TABLE, ID_PREFIX, &wanted, enabled)
            })
            .await
            .map_err(saved_reason)?;
        self.inner.apply_file(&file).await;
        self.job(id)
    }

    /// `POST /idle-jobs/{id}/run`: "ignore idle". Works for a disabled job too.
    pub async fn run_now(&self, id: &str) -> ApiResult<IdleJob> {
        let def = self.def(id)?;
        let inner = &self.inner;
        // The job already runs: it is returned. A frozen run is resumed as a manual run.
        let frozen_job = {
            let mut state = inner.lock();
            let runtime = state.jobs.get_mut(id).ok_or_else(unknown_job)?;
            match (&mut runtime.live, runtime.state) {
                (Some(live), IdleState::Running) => {
                    live.manual = true;
                    None
                }
                (Some(live), IdleState::Paused) => {
                    live.manual = true;
                    live.job.as_ref().map(Arc::clone)
                }
                _ => {
                    drop(state);
                    return self.start_by_hand(&def);
                }
            }
        };
        if frozen_job.is_some() {
            inner.resume(id, Duration::ZERO);
        }
        self.job(id)
    }

    fn start_by_hand(&self, def: &Arc<IdleDef>) -> ApiResult<IdleJob> {
        let inner = &self.inner;
        if inner.shutdown.is_cancelled() {
            return Err(
                ApiError::unavailable("idle", "Rain is stopping").with_reason("shuttingDown")
            );
        }
        if inner.refusing() {
            return Err(crate::modules::jobs::game_mode_refusal());
        }
        match inner.start_run(def, true, None) {
            Ok(()) | Err(StartRefused::AlreadyRunning) => self.job(&def.id),
            Err(StartRefused::ResourceBusy(holder)) => Err(ApiError::conflict(
                "resourceBusy",
                format!(
                    "{holder} is using the {} right now",
                    resource_name(def.resource)
                ),
            )),
        }
    }

    /// `GET /idle-jobs/{id}/log`: the log of the newest run; empty when the job never ran.
    pub async fn log(&self, id: &str) -> ApiResult<Vec<LogLine>> {
        self.def(id)?;
        let run = self
            .inner
            .lock()
            .jobs
            .get(id)
            .and_then(|runtime| runtime.last_run_id.clone());
        match run {
            Some(run) => self.inner.logs.read(&run).await,
            None => {
                // After a restart the newest run is in the database only.
                match store::newest_run(&self.inner.db, id.to_owned())
                    .await
                    .map_err(|e| e.into_api("Could not read the log"))?
                {
                    Some(run) => self.inner.logs.read(&run).await,
                    None => Ok(Vec::new()),
                }
            }
        }
    }

    /// Whether the sampler task exists. For the test that it exists only while it is needed.
    #[doc(hidden)]
    pub fn sampler_active(&self) -> bool {
        self.inner
            .sampler
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// How many times a sampler was started.
    #[doc(hidden)]
    pub fn sampler_starts(&self) -> u64 {
        self.inner.sampler_starts.load(Ordering::SeqCst)
    }

    /// How game mode ends the runs of idle jobs (section 06 6.7.1).
    pub fn wind_down(&self) -> &dyn WindDownPort {
        self
    }
}

/// Follows `idle-jobs.toml`: a hand edit or Rain's own write reaches the state at once.
async fn reload_loop(inner: Arc<Inner>) {
    let mut changes = inner.file.subscribe();
    loop {
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                let file = Arc::clone(&changes.borrow_and_update());
                debug!("idle-jobs.toml changed");
                inner.apply_file(&file).await;
            }
            () = inner.shutdown.cancelled() => return,
        }
    }
}

impl WindDownPort for Service {
    fn counts(&self) -> Counts {
        let state = self.inner.lock();
        Counts {
            enabled: state.defs.iter().filter(|def| def.enabled).count() as u32,
            running: state.live_count,
        }
    }

    fn running_watch(&self) -> watch::Receiver<u32> {
        self.inner.running.subscribe()
    }

    /// `true`: the sampler ends, so no run starts or resumes. A run that is going on is left
    /// alone (it keeps running, or stays frozen). `false`: the sampler starts again, and
    /// resumes frozen runs when the PC is quiet.
    fn pause(&self, paused: bool) {
        self.inner.paused.store(paused, Ordering::SeqCst);
        self.inner.ensure_sampler();
    }

    fn stop_all(&self, budget: Duration) -> BoxFuture<'_, Vec<Stopped>> {
        Box::pin(async move {
            let names = self.inner.cancel_all(StopWhy::GameMode);
            let done = tokio::time::timeout(budget, self.inner.wait_until_no_run())
                .await
                .is_ok();
            let outcome = if done {
                StopOutcome::Stopped
            } else {
                StopOutcome::Forced
            };
            names
                .into_iter()
                .map(|name| Stopped { name, outcome })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        let cases = [
            ("rain-progress: 42%", Some(ProgressLine::Progress(0.42))),
            ("rain-progress: 0.42", Some(ProgressLine::Progress(0.42))),
            (
                "  RAIN-Progress:   7.5 %  ",
                Some(ProgressLine::Progress(0.075)),
            ),
            ("rain-progress: 250%", Some(ProgressLine::Progress(1.0))),
            ("rain-progress: -3", Some(ProgressLine::Progress(0.0))),
            ("rain-eta: 12", Some(ProgressLine::Eta(12))),
            ("rain-eta: soon", None),
            ("rain-progress: lots", None),
            ("rain-progress:", None),
            ("the rain-progress: 50% is here", None),
            ("rain-progress: NaN", None),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_progress_line(line), expected, "{line:?}");
        }
    }

    #[test]
    fn the_resource_names_are_for_people() {
        assert_eq!(resource_name(IdleResource::Cpu), "CPU");
        assert_eq!(resource_name(IdleResource::Gpu), "GPU");
    }

    fn runtime_at(progress: f64, reported: bool) -> JobRt {
        let mut runtime = JobRt::new(IdleState::Running);
        runtime.progress = progress;
        runtime.reported = reported;
        runtime
    }

    fn def(expected: Option<u32>) -> IdleDef {
        let table: toml::Table = toml::from_str("name = \"x\"\ncommand = \"x\"").unwrap();
        let mut loaded = model::load(&IdleFile {
            idle_job: vec![table],
        });
        let mut def = loaded.defs.remove(0);
        def.expected_minutes = expected;
        def
    }

    #[tokio::test]
    async fn progress_is_the_report_or_an_estimate_and_never_one_before_the_end() {
        let started = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut running = runtime_at(0.0, false);
        let (tx, _) = watch::channel(false);
        let sink = LogStore::new(
            Db::new(std::env::temp_dir().join("rain-idle-unit-unused.db")),
            Arc::new(crate::core::clock::SystemClock),
            Duration::from_millis(10),
        )
        .open("irun-unit");
        running.live = Some(Live {
            manual: false,
            cancel: CancellationToken::new(),
            job: None,
            freeze: tx,
            started,
            frozen_since: None,
            frozen_total: Duration::ZERO,
            why: None,
            sink,
        });

        // No report, no expectation: 0.
        let now = started + Duration::from_secs(600);
        assert_eq!(Inner::progress_of(&def(None), &running, now), (0.0, None));
        // An expectation of 30 minutes: 10 minutes in is a third, with 20 minutes left.
        let (progress, eta) = Inner::progress_of(&def(Some(30)), &running, now);
        assert!((progress - 1.0 / 3.0).abs() < 0.001);
        assert_eq!(eta, Some(20));
        // Far past the expectation: never above 0.95, and the eta is 0.
        let late = started + Duration::from_secs(3 * 3600);
        assert_eq!(
            Inner::progress_of(&def(Some(30)), &running, late),
            (0.95, Some(0))
        );
        // A report wins over the estimate, and the reported eta over the computed one.
        running.progress = 0.6;
        running.reported = true;
        running.reported_eta = Some(7);
        assert_eq!(
            Inner::progress_of(&def(Some(30)), &running, now),
            (0.6, Some(7))
        );
        // Frozen time does not count towards the estimate.
        let mut frozen = runtime_at(0.0, false);
        if let Some(live) = running.live.take() {
            frozen.live = Some(Live {
                frozen_total: Duration::from_secs(300),
                ..live
            });
        }
        let (progress, _) = Inner::progress_of(&def(Some(30)), &frozen, now);
        assert!((progress - 10.0 / 30.0 * 0.5).abs() < 0.001, "{progress}");
    }
}
