//! The logic and the in-memory state of scheduled jobs. Section 03 6.11.
//!
//! What lives here: the list of jobs (`jobs.toml` plus the history of runs), turning `enabled` on
//! and off, starting a run by hand or by schedule, and the end of a run. The scheduler task is in
//! `scheduler.rs`, the child process in `runner.rs`, the log in `log.rs`.
//!
//! Two kinds of state are kept together under one lock, so that the UI never sees one without the
//! other: `running` of a job is true exactly when its newest run is `running`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use rain_types::time::iso_from_system_time;
use rain_types::{ConfigIssue, JobRun, Jobs, LogLine, ScheduledJob, SourceState, Status};
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::claude_stream::{self, ClaudeResult};
use super::command::{self, SpawnSpec};
use super::entry::entry_lines;
use super::file_edit::{self, saved_reason};
use super::log::{LogSink, LogStore, unknown_run};
use super::model::{self, Action, FILE_NAME, JobDef, JobsFile, TABLE};
use super::runner::{self, End};
use super::schedule::{self, parse_iso};
use super::scheduler;
use super::store::{self, EndedRun, NewRun};
use super::tuning::{ResumeSignal, Tuning};
use crate::core::bus::EventBus;
use crate::core::clock::Clock;
use crate::core::error::{ApiError, ApiResult};
use crate::core::health::Health;
use crate::core::shutdown::{Activity, ActivityGuard};
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::core::throttle::Throttle;
use crate::core::winddown::{Counts, StopOutcome, Stopped, WindDownPort};
use crate::platform::job::{OutputLine, Stream};
use crate::platform::power::{ResumeWatch, on_resume};
use crate::store::db::Db;
use crate::store::hub::ConfigHub;
use crate::store::toml_file::TomlFile;

/// The most runs a job shows (`ScheduledJob.recent_runs`).
const RECENT_MAX: usize = 20;
/// The frames of the topic `jobs` per second at most.
const TOPIC_RATE: u32 = 4;
/// Runs of jobs that are not in the file any more are deleted when older than this.
const REMOVED_JOB_RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);
/// How long `stop` waits for the runs to end.
const STOP_WAIT: Duration = Duration::from_secs(4);

/// Why a run was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Schedule,
    Manual,
    CatchUp,
}

impl Trigger {
    fn column(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Manual => "manual",
            Self::CatchUp => "catchup",
        }
    }

    fn first_line(self) -> &'static str {
        match self {
            Self::Schedule => "Started by schedule",
            Self::Manual => "Started by Run now",
            Self::CatchUp => "Started after a missed run",
        }
    }
}

/// Why a run was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopWhy {
    GameMode,
    Shutdown,
}

impl StopWhy {
    fn line(self) -> &'static str {
        match self {
            Self::GameMode => "Stopped by game mode",
            Self::Shutdown => "Stopped because Rain stopped",
        }
    }
}

/// The run that is in progress for a job.
pub(super) struct Live {
    cancel: CancellationToken,
    why: Option<StopWhy>,
}

#[derive(Default)]
pub(super) struct Runtime {
    /// Newest first, at most `RECENT_MAX`.
    pub recent: VecDeque<JobRun>,
    pub live: Option<Live>,
    pub handled_until: Option<SystemTime>,
}

#[derive(Default)]
pub(super) struct State {
    pub defs: Arc<Vec<Arc<JobDef>>>,
    pub runtime: HashMap<String, Runtime>,
    /// While true, occurrences before `floor` are left for the catch-up.
    pub catch_up_pending: bool,
    pub floor: Option<SystemTime>,
    pub live_count: u32,
}

impl State {
    pub fn floor(&self) -> Option<SystemTime> {
        if self.catch_up_pending {
            self.floor
        } else {
            None
        }
    }
}

pub(super) struct Inner {
    weak: WeakAppState,
    file: TomlFile<JobsFile>,
    hub: ConfigHub,
    db: Db,
    pub clock: Arc<dyn Clock>,
    health: Arc<Health>,
    local_dir: PathBuf,
    pub tuning: Tuning,
    logs: LogStore,
    bus: EventBus,
    publisher: Throttle<Jobs>,
    state: Mutex<State>,
    /// Something changed that the scheduler must look at again.
    pub wake: Notify,
    /// The PC woke from sleep.
    pub resumed: Arc<Notify>,
    paused: AtomicBool,
    pub degraded: AtomicBool,
    running: watch::Sender<u32>,
    /// How many times the scheduler loop ran. A test reads it to see that the loop does not spin.
    pub wakeups: AtomicU64,
    pub shutdown: CancellationToken,
    activity: Activity,
    resume_watch: Mutex<Option<ResumeWatch>>,
    next_run_number: AtomicU64,
}

/// The `jobs` module's service. Other modules call it, and only it.
pub struct Service {
    inner: Arc<Inner>,
}

impl Inner {
    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn app(&self) -> Option<AppState> {
        self.weak.upgrade()
    }

    /// True while no new run may start: game mode is on, or the backend is stopping.
    pub(super) fn refusing(&self) -> bool {
        self.paused.load(Ordering::SeqCst) || self.app().is_some_and(|app| app.game_mode_active())
    }

    pub(super) async fn save_handled(&self, job_id: &str, until: SystemTime) {
        let result = store::save_handled(
            &self.db,
            job_id.to_owned(),
            iso_from_system_time(until),
            iso_from_system_time(self.clock.now()),
        )
        .await;
        if let Err(e) = result {
            warn!(error = %e, "could not save what the scheduler has handled");
        }
    }

    // ---- the list ----

    fn snapshot(&self, state: &State, def: &JobDef, now: SystemTime) -> ScheduledJob {
        let runtime = state.runtime.get(&def.id);
        let next_run_at = def
            .enabled
            .then(|| schedule::next_after(&def.cron, now))
            .flatten()
            .map(iso_from_system_time);
        ScheduledJob {
            id: def.id.clone(),
            name: def.name.clone(),
            kind: def.kind,
            cron: def.schedule.clone(),
            schedule_label: def
                .label
                .clone()
                .unwrap_or_else(|| schedule::label(&def.schedule)),
            enabled: def.enabled,
            running: runtime.is_some_and(|r| r.live.is_some()),
            next_run_at,
            recent_runs: runtime
                .map(|r| r.recent.iter().cloned().collect())
                .unwrap_or_default(),
        }
    }

    fn list(&self) -> Vec<ScheduledJob> {
        let now = self.clock.now();
        let state = self.lock();
        state
            .defs
            .iter()
            .map(|def| self.snapshot(&state, def, now))
            .collect()
    }

    pub(super) fn publish(&self) {
        // Nobody listens: no frame, and no timer of the throttle. A client that connects later
        // fetches the list again (the `resync` of the stream).
        if !self.bus.has_receivers() {
            return;
        }
        // The topic carries plain JSON; the list is built from the typed `ScheduledJob`s.
        let payload = serde_json::to_value(self.list()).unwrap_or_default();
        self.publisher.publish(&payload);
    }

    fn set_running_count(&self, state: &State) {
        self.running.send_replace(state.live_count);
    }

    // ---- the file ----

    /// Takes a (re)loaded `jobs.toml` into the state: the runtime of every id that still exists
    /// is kept, the runtime of removed ids is dropped (a live run of a removed job finishes
    /// normally; its log stays reachable by run id).
    async fn apply_file(&self, file: &JobsFile) {
        let loaded = model::load(file);
        {
            let mut state = self.lock();
            let defs: Vec<Arc<JobDef>> = loaded.defs.into_iter().map(Arc::new).collect();
            let ids: Vec<&str> = defs.iter().map(|def| def.id.as_str()).collect();
            state
                .runtime
                .retain(|id, runtime| ids.contains(&id.as_str()) || runtime.live.is_some());
            for def in &defs {
                state.runtime.entry(def.id.clone()).or_default();
            }
            state.defs = Arc::new(defs);
        }
        self.report_issues(&loaded.skipped).await;
        self.publish();
        self.wake.notify_one();
    }

    /// Names every skipped entry in `GET /health`, with its line when it can be found.
    async fn report_issues(&self, skipped: &[model::Skipped]) {
        let path = self.hub.file_path(FILE_NAME);
        let lines = if skipped.is_empty() {
            Vec::new()
        } else {
            // A blocking read: also gives `TomlFile` time to finish its own report first, so
            // that ours is the last word about this file.
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

    // ---- runs ----

    fn new_run_id(&self, now: SystemTime) -> String {
        run_id(
            "run",
            now,
            self.next_run_number.fetch_add(1, Ordering::SeqCst),
        )
    }

    /// Starts a run unless the job already runs. Returns the new run, or `None` when one is
    /// in progress. The caller has checked that starting is allowed.
    pub(super) fn begin_run(
        self: &Arc<Self>,
        def: &Arc<JobDef>,
        trigger: Trigger,
        due: Option<SystemTime>,
    ) -> Option<JobRun> {
        let now = self.clock.now();
        let run = JobRun {
            id: self.new_run_id(now),
            job_id: def.id.clone(),
            status: Status::Running,
            started_at: iso_from_system_time(now),
            duration_ms: None,
            exit_code: None,
        };
        let cancel = CancellationToken::new();
        {
            let mut state = self.lock();
            let runtime = state.runtime.entry(def.id.clone()).or_default();
            if runtime.live.is_some() {
                return None;
            }
            runtime.live = Some(Live {
                cancel: cancel.clone(),
                why: None,
            });
            runtime.recent.push_front(run.clone());
            runtime.recent.truncate(RECENT_MAX);
            state.live_count += 1;
            self.set_running_count(&state);
        }
        self.publish();
        let task = Run {
            inner: Arc::clone(self),
            def: Arc::clone(def),
            run: run.clone(),
            trigger,
            due,
            cancel,
            busy: self.activity.guard(),
        };
        spawn_named("job-run", task.execute());
        Some(run)
    }

    /// The reason the run of `job_id` was cancelled, if it was.
    fn stop_reason(&self, job_id: &str) -> Option<StopWhy> {
        self.lock().runtime.get(job_id)?.live.as_ref()?.why
    }

    /// Cancels the run of every job, for `why`. Returns the names of the jobs that were running.
    fn cancel_all(&self, why: StopWhy) -> Vec<String> {
        let mut state = self.lock();
        let names: HashMap<String, String> = state
            .defs
            .iter()
            .map(|def| (def.id.clone(), def.name.clone()))
            .collect();
        let mut stopped = Vec::new();
        for (id, runtime) in &mut state.runtime {
            if let Some(live) = &mut runtime.live {
                live.why = Some(why);
                live.cancel.cancel();
                stopped.push(names.get(id).cloned().unwrap_or_else(|| id.clone()));
            }
        }
        stopped
    }

    async fn wait_until_no_run(&self) {
        let mut running = self.running.subscribe();
        let _ = running.wait_for(|count| *count == 0).await;
    }
}

/// `run-20261121-093000-ab12`: the UTC time and four hex characters that differ from run to run.
/// `prefix` is `run` for scheduled runs and `irun` for idle runs.
pub fn run_id(prefix: &str, now: SystemTime, number: u64) -> String {
    use std::hash::{BuildHasher, RandomState};
    let iso = iso_from_system_time(now); // 2026-11-21T09:30:00.000Z
    let compact: String = iso.chars().filter(char::is_ascii_digit).take(14).collect();
    let (date, time) = compact.split_at(8);
    let nonce = RandomState::new().hash_one((number, iso_len_seed(&iso))) & 0xFFFF;
    format!("{prefix}-{date}-{time}-{nonce:04x}")
}

fn iso_len_seed(iso: &str) -> u64 {
    iso.bytes().fold(0u64, |sum, b| {
        sum.wrapping_mul(31).wrapping_add(u64::from(b))
    })
}

// ---- one run ----

/// Everything a run task needs.
struct Run {
    inner: Arc<Inner>,
    def: Arc<JobDef>,
    run: JobRun,
    trigger: Trigger,
    due: Option<SystemTime>,
    cancel: CancellationToken,
    /// Keeps the backend "busy" (no upgrade or restart) while the run lives.
    busy: ActivityGuard,
}

/// What a run came to.
struct Finished {
    status: Status,
    exit_code: Option<i64>,
    session_id: Option<String>,
}

impl Finished {
    fn failed() -> Self {
        Self {
            status: Status::Failed,
            exit_code: None,
            session_id: None,
        }
    }
}

/// The Claude job state that the output lines fill in.
#[derive(Default)]
struct ClaudeProgress {
    session_id: Option<String>,
    result: Option<ClaudeResult>,
}

impl Run {
    async fn execute(self) {
        let started = tokio::time::Instant::now();
        let inner = Arc::clone(&self.inner);
        let sink = inner.logs.open(&self.run.id);
        let new_run = NewRun {
            id: self.run.id.clone(),
            job_id: self.def.id.clone(),
            trigger: self.trigger.column(),
            started_at: self.run.started_at.clone(),
        };
        if let Err(e) = store::insert_run(&inner.db, new_run).await {
            warn!(error = %e, "could not write the start of a run");
        }
        sink.info(self.trigger.first_line());
        if let Action::Claude(claude) = &self.def.action
            && let Some(skill) = &claude.skill
        {
            // The prompt is never logged; the name of the skill says what runs.
            sink.info(format!("Skill {skill}"));
        }

        let finished = self.run_process(&sink, started).await;
        self.conclude(finished, &sink, started).await;
    }

    /// Starts the program, follows it, and writes the closing lines of the log.
    async fn run_process(&self, sink: &Arc<LogSink>, began: tokio::time::Instant) -> Finished {
        let inner = &self.inner;
        let def = &self.def;
        let cwd = match self.working_dir().await {
            Ok(cwd) => cwd,
            Err(message) => {
                sink.error(format!("Could not start: {message}"));
                return Finished::failed();
            }
        };
        let spec = self.spec(&cwd);
        let started = match runner::start(&spec) {
            Ok(started) => started,
            Err(e) => {
                sink.error(format!("Could not start: {e}"));
                return Finished::failed();
            }
        };

        let is_claude = matches!(def.action, Action::Claude(_));
        let mut progress = ClaudeProgress::default();
        let timeout = inner.tuning.minute * def.timeout_minutes;
        let end = runner::drive(started, timeout, &self.cancel, None, |line| {
            follow_line(sink, is_claude, &mut progress, line);
        })
        .await;

        let elapsed = began.elapsed();
        match end {
            End::TimedOut => {
                sink.error(format!(
                    "Stopped after the time limit of {} minutes",
                    def.timeout_minutes
                ));
                Finished {
                    session_id: progress.session_id,
                    ..Finished::failed()
                }
            }
            End::Cancelled => {
                let why = inner.stop_reason(&def.id).unwrap_or(StopWhy::Shutdown);
                sink.warn(why.line());
                Finished {
                    status: Status::Idle,
                    exit_code: None,
                    session_id: progress.session_id,
                }
            }
            End::Exited(code) => finish_exited(sink, is_claude, code, progress, elapsed),
        }
    }

    /// The folder the program runs in. The default one is made now (the user may have deleted
    /// the cache folder); a folder the user named must exist.
    async fn working_dir(&self) -> Result<PathBuf, String> {
        let wanted = match &self.def.working_dir {
            Some(dir) => {
                let dir = dir.clone();
                let exists = {
                    let dir = dir.clone();
                    crate::core::blocking::run(move || dir.is_dir())
                        .await
                        .unwrap_or(false)
                };
                if !exists {
                    return Err(format!(
                        "the working folder {} does not exist",
                        dir.display()
                    ));
                }
                return Ok(dir);
            }
            None => self
                .inner
                .local_dir
                .join("cache")
                .join("jobs")
                .join(&self.def.id),
        };
        let folder = wanted.clone();
        match crate::core::blocking::run(move || std::fs::create_dir_all(&folder)).await {
            Ok(Ok(())) => Ok(wanted),
            _ => Err(format!(
                "the working folder {} could not be created",
                wanted.display()
            )),
        }
    }

    fn spec(&self, cwd: &std::path::Path) -> SpawnSpec {
        let def = &self.def;
        match &def.action {
            Action::Command(line) => {
                command::command_spec(line, cwd, &self.run.id, &def.id, &def.env)
            }
            Action::Claude(claude) => {
                let path_var = std::env::var("PATH").ok();
                let profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
                let program = command::resolve_claude(
                    &claude.program_path,
                    path_var.as_deref(),
                    profile.as_deref(),
                    |path| path.is_file(),
                );
                command::claude_spec(
                    claude,
                    program,
                    &def.name,
                    cwd,
                    &self.run.id,
                    &def.id,
                    &def.env,
                )
            }
        }
    }

    /// The end of a run: the log is closed, the history written, the state updated, the topic
    /// published (section 03 6.11.7).
    async fn conclude(
        self,
        finished: Finished,
        sink: &Arc<LogSink>,
        started: tokio::time::Instant,
    ) {
        let inner = &self.inner;
        let counts = sink.finish().await;
        let duration_ms = started.elapsed().as_millis() as u64;
        let ended_at = iso_from_system_time(inner.clock.now());

        let ended = EndedRun {
            id: self.run.id.clone(),
            job_id: self.def.id.clone(),
            status: finished.status,
            ended_at,
            duration_ms,
            exit_code: finished.exit_code,
            session_id: finished.session_id.clone(),
            log_lines: counts.stored,
            log_omitted: counts.omitted,
        };
        if let Err(e) = store::finish_run(&inner.db, ended).await {
            warn!(error = %e, "could not write the end of a run");
        }
        inner.logs.trim_total().await;

        // A run that Rain stopped on purpose is made up at the next start: its occurrence is
        // marked as not handled. A manual run never touches the scheduler's memory.
        let set_back = (finished.status == Status::Idle && self.trigger != Trigger::Manual)
            .then_some(self.due)
            .flatten()
            .map(scheduler::set_back);
        {
            let mut state = inner.lock();
            if let Some(runtime) = state.runtime.get_mut(&self.def.id) {
                runtime.live = None;
                if let Some(entry) = runtime.recent.iter_mut().find(|r| r.id == self.run.id) {
                    entry.status = finished.status;
                    entry.duration_ms = Some(duration_ms);
                    entry.exit_code = finished.exit_code;
                }
                if let Some(until) = set_back {
                    runtime.handled_until = Some(until);
                }
            }
            state.live_count = state.live_count.saturating_sub(1);
            inner.set_running_count(&state);
        }
        if let Some(until) = set_back {
            inner.save_handled(&self.def.id, until).await;
        }
        // The backend is no longer busy once the state is final, before anyone hears of it.
        drop(self.busy);
        inner.publish();
        inner.wake.notify_one();
        info!(job = %self.def.id, status = ?finished.status, "a job run ended");
    }
}

/// One line of output of a running program, into the log. Stderr is always a warning; stdout of
/// a Claude job is read as the stream of `claude -p`.
fn follow_line(sink: &LogSink, is_claude: bool, progress: &mut ClaudeProgress, line: OutputLine) {
    if line.stream == Stream::Stderr {
        sink.warn(&line.text);
    } else if is_claude {
        let mapped = claude_stream::map_stdout_line(&line.text);
        for (level, text) in mapped.lines {
            sink.line(level, text);
        }
        if let Some(id) = mapped.session_id {
            progress.session_id = Some(id);
        }
        if let Some(result) = mapped.result {
            progress.result = Some(result);
        }
    } else {
        sink.info(&line.text);
    }
}

/// The table of section 03 6.11.7 for a program that ended by itself.
fn finish_exited(
    sink: &LogSink,
    is_claude: bool,
    code: Option<u32>,
    progress: ClaudeProgress,
    elapsed: Duration,
) -> Finished {
    let took = format_duration(elapsed);
    let exit_code = code.map(i64::from);
    let session = progress.session_id;
    let done = |status, exit_code| Finished {
        status,
        exit_code,
        session_id: session.clone(),
    };
    let Some(code) = code else {
        sink.error("The program ended without an exit code");
        return done(Status::Failed, None);
    };
    if code != 0 {
        sink.error(format!("Process exited with code {code}"));
        return done(Status::Failed, exit_code);
    }
    if !is_claude {
        sink.info(format!("Finished in {took}"));
        sink.info("Process exited with code 0");
        return done(Status::Success, exit_code);
    }
    match progress.result {
        Some(result) if result.success => {
            let turns = result
                .turns
                .map(|n| format!(", {n} turns"))
                .unwrap_or_default();
            let cost = result
                .cost_usd
                .map(|usd| format!(", cost ${usd:.2}"))
                .unwrap_or_default();
            sink.info(format!("Finished in {took}{turns}{cost}"));
            done(Status::Success, exit_code)
        }
        Some(result) => {
            sink.error(format!("Claude reported an error: {}", result.subtype));
            for line in &result.errors {
                sink.error(line);
            }
            done(Status::Failed, exit_code)
        }
        None => {
            sink.error("Claude ended without a result");
            done(Status::Failed, exit_code)
        }
    }
}

/// "850 ms", "4.2 s", "3 min 5 s".
pub fn format_duration(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000)
    }
}

// ---- the public service ----

impl Service {
    /// Called by `init`: opens `jobs.toml` and builds the service. Nothing runs yet.
    pub async fn open(state: &AppState) -> ApiResult<Self> {
        // The first start writes the commented file, so the format can be seen. Only when the
        // data folder exists: making it is the start-up's job.
        if state.hub.data_dir().is_dir() {
            file_edit::ensure_default_file(&state.hub, FILE_NAME, store::DEFAULT_FILE).await;
        }
        let file = store::open(&state.hub).await?;
        let tuning = state
            .seam::<Tuning>()
            .map_or_else(Tuning::default, |t| (*t).clone());
        let resumed = state
            .seam::<ResumeSignal>()
            .map_or_else(|| Arc::new(Notify::new()), |signal| Arc::clone(&signal.0));
        let (running, _) = watch::channel(0);
        let inner = Arc::new(Inner {
            weak: state.downgrade(),
            file,
            hub: state.hub.clone(),
            db: state.db.clone(),
            clock: Arc::clone(&state.clock),
            health: Arc::clone(&state.health),
            local_dir: state.config.local_dir.clone(),
            logs: LogStore::new(state.db.clone(), Arc::clone(&state.clock), tuning.log_flush),
            tuning,
            bus: state.bus.clone(),
            publisher: Throttle::new(state.bus.clone(), TOPIC_RATE),
            state: Mutex::default(),
            wake: Notify::new(),
            resumed,
            paused: AtomicBool::new(false),
            degraded: AtomicBool::new(false),
            running,
            wakeups: AtomicU64::new(0),
            shutdown: state.shutdown.token().clone(),
            activity: state.activity.clone(),
            resume_watch: Mutex::new(None),
            next_run_number: AtomicU64::new(0),
        });
        let first = inner.file.get();
        inner.apply_file(&first).await;
        Ok(Self { inner })
    }

    /// Called by `start`: loads the history, closes runs that were cut off, and starts the
    /// scheduler and the watcher of the file.
    pub async fn start(&self, state: &AppState) -> anyhow::Result<()> {
        let inner = &self.inner;
        let now = inner.clock.now();
        let closed = store::close_running(&inner.db, iso_from_system_time(now)).await?;
        if !closed.is_empty() {
            info!(
                runs = closed.len(),
                "closed runs that were cut off by a stop"
            );
        }
        let recent = store::load_recent(&inner.db).await?;
        let handled = store::load_handled(&inner.db).await?;
        {
            let mut guard = inner.lock();
            let present: Vec<String> = guard.defs.iter().map(|def| def.id.clone()).collect();
            let st = &mut *guard;
            for id in present {
                let runtime = st.runtime.entry(id.clone()).or_default();
                if let Some(runs) = recent.get(&id) {
                    runtime.recent = runs.iter().cloned().collect();
                }
                runtime.handled_until = handled.get(&id).and_then(|text| parse_iso(text));
            }
            // Occurrences before this start belong to the catch-up (after its delay).
            st.catch_up_pending = true;
            st.floor = Some(now);
        }
        let present: Vec<String> = inner.lock().defs.iter().map(|d| d.id.clone()).collect();
        let cutoff = now - REMOVED_JOB_RETENTION;
        if let Err(e) =
            store::delete_removed(&inner.db, present, iso_from_system_time(cutoff)).await
        {
            warn!(error = %e, "could not delete the old runs of removed jobs");
        }

        // The notice that the PC woke from sleep. A test's own signal is used as it is.
        if state.seam::<ResumeSignal>().is_none() {
            let resumed = Arc::clone(&inner.resumed);
            match on_resume(move || resumed.notify_one()) {
                Some(watch) => {
                    *inner
                        .resume_watch
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = Some(watch);
                }
                None => {
                    warn!("no notice of a resume from sleep; the scheduler checks every 5 minutes");
                    inner.degraded.store(true, Ordering::SeqCst);
                    inner.health.set_source(
                        "scheduler",
                        SourceState::Ok,
                        Some(
                            "Sleep wake-up detection is not available; checking every 5 minutes"
                                .to_owned(),
                        ),
                    );
                }
            }
        }

        spawn_named("jobs-scheduler", scheduler::run(Arc::clone(inner)));
        spawn_named("jobs-reload", reload_loop(Arc::clone(inner)));
        inner.publish();
        Ok(())
    }

    /// Called by `stop`: ends the runs in progress (marked as stopped on purpose) and lets go
    /// of the resume notice. The runs write their end into the database first.
    pub async fn stop(&self) {
        let inner = &self.inner;
        inner.cancel_all(StopWhy::Shutdown);
        let _ = tokio::time::timeout(STOP_WAIT, inner.wait_until_no_run()).await;
        inner
            .resume_watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    /// The same list as `GET /jobs`, from memory, no I/O. The launcher maps it to search items
    /// of the type `job`.
    pub fn list(&self) -> Vec<ScheduledJob> {
        self.inner.list()
    }

    /// The Claude session id of a run that a hook told us about (`SessionStart` of a session that
    /// the run's own Claude program started). It is stored with the run, so the run can be tied
    /// to its session later. An unknown run id changes nothing and answers false.
    pub async fn attach_session(&self, run_id: &str, session_id: &str) -> bool {
        match store::attach_session(&self.inner.db, run_id.to_owned(), session_id.to_owned()).await
        {
            Ok(changed) => changed,
            Err(e) => {
                warn!(error = %e, "could not store the session id of a run");
                false
            }
        }
    }

    /// Makes the scheduler look at the clock again. Free: no timer.
    pub fn nudge(&self) {
        self.inner.wake.notify_one();
    }

    fn def(&self, id: &str) -> ApiResult<Arc<JobDef>> {
        self.inner
            .lock()
            .defs
            .iter()
            .find(|def| def.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("Job not found").with_reason("unknownJob"))
    }

    /// `PATCH /jobs/{id}`: writes `enabled` into `jobs.toml` and returns the job as it is then.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> ApiResult<ScheduledJob> {
        let before = self.def(id)?;
        if enabled && !before.enabled {
            // Switching a daily 06:00 job on at 10:00 must not run it at once. This is set before
            // the file changes: once the scheduler sees the job enabled it must already have it.
            let now = self.inner.clock.now();
            if let Some(runtime) = self.inner.lock().runtime.get_mut(id) {
                runtime.handled_until = Some(now);
            }
            self.inner.save_handled(id, now).await;
        }
        let wanted = id.to_owned();
        let file = self
            .inner
            .file
            .edit(move |doc| {
                file_edit::set_enabled(doc, FILE_NAME, TABLE, "job-", &wanted, enabled)
            })
            .await
            .map_err(saved_reason)?;
        self.inner.apply_file(&file).await;
        self.inner.publish();
        self.inner.wake.notify_one();
        let now = self.inner.clock.now();
        let state = self.inner.lock();
        state
            .defs
            .iter()
            .find(|def| def.id == id)
            .map(|def| self.inner.snapshot(&state, def, now))
            .ok_or_else(|| {
                ApiError::conflict(
                    "configChanged",
                    "jobs.toml changed while saving. Try again.",
                )
            })
    }

    /// `POST /jobs/{id}/runs`. The flag says whether a new run started (201) or the run in
    /// progress is returned (200). A disabled job may be run: pressing the button is explicit.
    pub async fn run_now(&self, id: &str) -> ApiResult<(JobRun, bool)> {
        let def = self.def(id)?;
        {
            let state = self.inner.lock();
            if let Some(runtime) = state.runtime.get(id)
                && runtime.live.is_some()
                && let Some(current) = runtime.recent.front()
            {
                return Ok((current.clone(), false));
            }
        }
        if self.inner.shutdown.is_cancelled() {
            return Err(
                ApiError::unavailable("jobs", "Rain is stopping").with_reason("shuttingDown")
            );
        }
        if self.inner.refusing() {
            return Err(game_mode_refusal());
        }
        match self.inner.begin_run(&def, Trigger::Manual, None) {
            Some(run) => Ok((run, true)),
            // Another request started it between the check and now.
            None => {
                let state = self.inner.lock();
                state
                    .runtime
                    .get(id)
                    .and_then(|r| r.recent.front().cloned())
                    .map(|run| (run, false))
                    .ok_or_else(|| ApiError::failed("The job could not be started"))
            }
        }
    }

    /// `GET /runs/{id}/log`.
    pub async fn run_log(&self, run_id: &str) -> ApiResult<Vec<LogLine>> {
        let known = self.inner.logs.is_live(run_id)
            || store::run_exists(&self.inner.db, run_id.to_owned())
                .await
                .map_err(|e| e.into_api("Could not read the log"))?;
        if !known {
            return Err(unknown_run());
        }
        self.inner.logs.read(run_id).await
    }

    // ---- for the idle module (section 03 1.3) ----

    /// The shared log store: `idle` writes its logs here and never touches `jobs_*` itself.
    pub fn log_store(&self) -> LogStore {
        self.inner.logs.clone()
    }

    /// The lines of any run, scheduled or idle, in the form of the API.
    pub async fn read_log(&self, run_id: &str) -> ApiResult<Vec<LogLine>> {
        self.inner.logs.read(run_id).await
    }

    /// The times in force (a test may have shortened them).
    pub fn tuning(&self) -> Tuning {
        self.inner.tuning.clone()
    }

    /// How many times the scheduler loop ran. For the test that it does not spin in game mode.
    #[doc(hidden)]
    pub fn scheduler_wakeups(&self) -> u64 {
        self.inner.wakeups.load(Ordering::SeqCst)
    }

    /// How game mode ends the runs of scheduled jobs (section 06 6.7.1).
    pub fn wind_down(&self) -> &dyn WindDownPort {
        self
    }
}

pub fn game_mode_refusal() -> ApiError {
    ApiError::conflict(
        "gameMode",
        "Rain is getting ready for game mode. Nothing new starts.",
    )
}

/// Follows `jobs.toml`: a hand edit or Rain's own write reaches the state at once.
async fn reload_loop(inner: Arc<Inner>) {
    let mut changes = inner.file.subscribe();
    loop {
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                let file = Arc::clone(&changes.borrow_and_update());
                debug!("jobs.toml changed");
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

    /// `true`: the scheduler starts nothing and advances nothing. `false`: it goes on, and what
    /// came due meanwhile runs once (the caller has cleared `game_mode_active` first, or the
    /// runs would still be refused).
    fn pause(&self, paused: bool) {
        let was = self.inner.paused.swap(paused, Ordering::SeqCst);
        self.inner.wake.notify_one();
        if was && !paused {
            let inner = Arc::clone(&self.inner);
            spawn_named("jobs-resume-catch-up", async move {
                inner.plan_catch_up().await;
            });
        }
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
    use rain_types::{JobKind, LogLevel};

    #[test]
    fn a_run_id_has_the_utc_time_and_four_hex_characters() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let id = run_id("run", now, 7);
        assert!(id.starts_with("run-20231114-221320-"), "{id}");
        assert_eq!(id.len(), "run-20231114-221320-".len() + 4);
        assert!(id[id.len() - 4..].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(run_id("irun", now, 7).starts_with("irun-20231114-221320-"));
    }

    #[test]
    fn durations_are_written_for_people() {
        assert_eq!(format_duration(Duration::from_millis(850)), "850 ms");
        assert_eq!(format_duration(Duration::from_millis(4200)), "4.2 s");
        assert_eq!(format_duration(Duration::from_secs(185)), "3 min 5 s");
    }

    fn sink_lines(f: impl FnOnce(&LogSink)) -> Vec<(LogLevel, String)> {
        // A sink needs a runtime and a database only for storing; the snapshot is from memory.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let store = LogStore::new(
                Db::new(std::env::temp_dir().join("rain-status-mapping-unused.db")),
                Arc::new(crate::core::clock::SystemClock),
                Duration::from_millis(10),
            );
            let sink = store.open("run-x");
            f(&sink);
            sink.snapshot()
                .into_iter()
                .map(|l| (l.level, l.text))
                .collect()
        })
    }

    #[test]
    fn status_mapping() {
        let took = Duration::from_millis(1500);
        let result = |success: bool, subtype: &str| ClaudeResult {
            success,
            subtype: subtype.to_owned(),
            turns: Some(3),
            cost_usd: Some(0.25),
            errors: vec!["Ran out of turns".to_owned()],
        };
        let progress = |result: Option<ClaudeResult>| ClaudeProgress {
            session_id: Some("sess".into()),
            result,
        };

        // cron, exit 0 and not 0
        let lines = sink_lines(|sink| {
            let f = finish_exited(sink, false, Some(0), progress(None), took);
            assert_eq!((f.status, f.exit_code), (Status::Success, Some(0)));
        });
        assert_eq!(lines[0].1, "Finished in 1.5 s");
        assert_eq!(lines[1].1, "Process exited with code 0");
        let lines = sink_lines(|sink| {
            let f = finish_exited(sink, false, Some(3), progress(None), took);
            assert_eq!((f.status, f.exit_code), (Status::Failed, Some(3)));
        });
        assert_eq!(
            lines,
            [(LogLevel::Error, "Process exited with code 3".to_owned())]
        );

        // Claude, exit 0 and a good result
        let lines = sink_lines(|sink| {
            let f = finish_exited(
                sink,
                true,
                Some(0),
                progress(Some(result(true, "success"))),
                took,
            );
            assert_eq!(f.status, Status::Success);
            assert_eq!(f.session_id.as_deref(), Some("sess"));
        });
        assert_eq!(lines[0].1, "Finished in 1.5 s, 3 turns, cost $0.25");
        // Claude, exit 0 but the result says error, or no result
        let lines = sink_lines(|sink| {
            let f = finish_exited(
                sink,
                true,
                Some(0),
                progress(Some(result(false, "error_max_turns"))),
                took,
            );
            assert_eq!((f.status, f.exit_code), (Status::Failed, Some(0)));
        });
        assert_eq!(lines[0].1, "Claude reported an error: error_max_turns");
        assert_eq!(lines[1].1, "Ran out of turns");
        let lines = sink_lines(|sink| {
            let f = finish_exited(sink, true, Some(0), progress(None), took);
            assert_eq!(f.status, Status::Failed);
        });
        assert_eq!(lines[0].1, "Claude ended without a result");
        // Claude, exit not 0
        let lines = sink_lines(|sink| {
            let f = finish_exited(sink, true, Some(2), progress(None), took);
            assert_eq!((f.status, f.exit_code), (Status::Failed, Some(2)));
        });
        assert_eq!(lines[0].1, "Process exited with code 2");
    }

    #[test]
    fn the_kind_of_a_job_is_in_the_listing() {
        // `JobKind` is used by the snapshot; a compile-time reminder that both kinds exist.
        assert_ne!(JobKind::Cron, JobKind::Claude);
    }
}
