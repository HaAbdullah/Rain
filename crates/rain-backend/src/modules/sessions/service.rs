//! The logic and the in-memory state of this module. Other modules call it, and only it.
//! Section 03 (the sessions half).
//!
//! The service owns the session book behind one mutex that is never held across an await. Every
//! change ends with `publish`, which sends the whole list through a throttle (at most four frames
//! a second, newest wins). Where the facts come from:
//!
//! - hook events from `rain-hook` (the main source: status, permission requests, recent events);
//! - the registry of Claude Code, read when its folder changes (discovery, liveness, host, name);
//! - the transcript, read from its end at first sight, after a `Stop`, and whenever the file
//!   grew (a watch on the folder of each live session; this is what feeds a session that no hook
//!   speaks for);
//! - the process table, for liveness, focus and stop.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use rain_types::limits::HOOK_SERVER_WAIT;
use rain_types::time::iso_from_system_time;
use rain_types::{
    ClaudeSession, FocusKind, FocusResult, LogLevel, LogLine, MonitorClaudeState, PermissionAnswer,
    PermissionAnswerBody, PermissionAnswerResult, SessionEventKind, SessionHost, Sessions,
    SourceState, Status,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::book::{Applied, Book, FileSig, Pending, SessionRec, Turn, fit_frame, same_start};
use super::claude_paths::ClaudePaths;
use super::focus::{self, FocusRequest};
use super::hook_wire::{HookEnvelope, HookMode, text};
use super::hooks_install::{
    self, HookState, HooksStatus, InstallError, Spec, allowed, backup_dir, forwarder_path,
};
use super::registry::{self, RegistryInfo};
use super::seams::{
    ExitSource, ForwarderProgram, ProcessProbe, SystemExits, SystemForwarder, SystemProbe,
    SystemWindows, WindowApi,
};
use super::store::{self, EndedRow};
use super::summary::{
    AlwaysChoice, TYPED_ANSWER_TOOLS, choose_always, decision_json, detail, input_hash, summary,
};
use super::transcript::{self, TAIL_BYTES};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::core::throttle::Throttle;
use crate::store::hub::DirWatch;

/// The registry folder is read this long after its last change event.
const REGISTRY_QUIET: Duration = Duration::from_millis(300);
/// A transcript folder is read this long after its last change event, and then at most once in
/// this long again: Claude writes the file all the time while it works, and a tail read every few
/// seconds is all the list needs. Neither time runs while no transcript changes (foundation 11.5).
const TRANSCRIPT_QUIET: Duration = Duration::from_secs(2);
const TRANSCRIPT_GAP: Duration = Duration::from_secs(2);
/// The program of a session that `stopSession` may end (the image name, lower case).
const STOPPABLE_IMAGES: [&str; 2] = ["claude.exe", "node.exe"];
/// How long `stopSession` waits for the process to be gone.
const STOP_WAIT: Duration = Duration::from_secs(2);
const STOP_POLL: Duration = Duration::from_millis(20);
/// A copy of the hook program that fails because another program holds the file is tried again.
const COPY_RETRIES: u32 = 8;
const COPY_DELAY: Duration = Duration::from_millis(25);
/// The health sources of this module (section 03 8).
const SOURCE_REGISTRY: &str = "claude-registry";
const SOURCE_HOOKS: &str = "claude-hooks";

/// How long a permission request is held for the hook program (`HOOK_SERVER_WAIT`, 19 seconds).
/// A test shortens it with `TestBackend::builder().with::<HookHold>(..)` so that it does not
/// wait 19 seconds; nothing in the product sets it.
#[derive(Debug, Clone, Copy)]
pub struct HookHold(pub Duration);

/// How patient the transcript watch is. A test shortens both times with
/// `TestBackend::builder().with::<TranscriptTiming>(..)`; nothing in the product sets it.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptTiming {
    pub quiet: Duration,
    pub gap: Duration,
}

impl Default for TranscriptTiming {
    fn default() -> Self {
        Self {
            quiet: TRANSCRIPT_QUIET,
            gap: TRANSCRIPT_GAP,
        }
    }
}

/// What the hook route answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookReply {
    /// `204`: nothing to say.
    Empty,
    /// `200` with this JSON text, which the hook program prints for Claude Code.
    Decision(String),
}

#[derive(Clone)]
pub struct Service {
    inner: Arc<Inner>,
}

struct Inner {
    state: WeakAppState,
    paths: ClaudePaths,
    book: Mutex<Book>,
    throttle: Throttle<Sessions>,
    probe: Arc<dyn ProcessProbe>,
    exits: Arc<dyn ExitSource>,
    windows: Arc<dyn WindowApi>,
    forwarder: Arc<dyn ForwarderProgram>,
    /// The exit watches send the pid of an ended process here.
    exit_tx: mpsc::UnboundedSender<u32>,
    exit_rx: Mutex<Option<mpsc::UnboundedReceiver<u32>>>,
    last_event_at: Mutex<Option<SystemTime>>,
    registry_watching: Mutex<bool>,
    /// One watch per folder that holds the transcript of a live session, shared by the sessions in
    /// it. The token ends the task that owns the watch.
    transcript_watches: Mutex<HashMap<PathBuf, CancellationToken>>,
    transcript_timing: TranscriptTiming,
    /// How many tails were really read (a quiet file is not counted). For the tests.
    transcript_reads: AtomicU32,
    cancel: CancellationToken,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    request_counter: AtomicU32,
    /// How long a permission request is held for the hook program.
    hold: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every change of the guarded data is complete before the lock is released, so the data of
    // a poisoned lock is still consistent.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Service {
    /// Builds the service. A test replaces the parts that touch the PC through the seams of
    /// `AppState` (`with::<dyn ProcessProbe>(..)` and so on).
    pub fn new(state: &AppState) -> Self {
        let (exit_tx, exit_rx) = mpsc::unbounded_channel();
        let inner = Inner {
            state: state.downgrade(),
            paths: ClaudePaths::resolve(&state.config),
            book: Mutex::new(Book::default()),
            throttle: Throttle::new(state.bus.clone(), rain_types::limits::TOPIC_RATE_MAX),
            probe: state
                .seam::<dyn ProcessProbe>()
                .unwrap_or_else(|| Arc::new(SystemProbe)),
            exits: state
                .seam::<dyn ExitSource>()
                .unwrap_or_else(|| Arc::new(SystemExits)),
            windows: state
                .seam::<dyn WindowApi>()
                .unwrap_or_else(|| Arc::new(SystemWindows)),
            forwarder: state
                .seam::<dyn ForwarderProgram>()
                .unwrap_or_else(|| Arc::new(SystemForwarder)),
            exit_tx,
            exit_rx: Mutex::new(Some(exit_rx)),
            last_event_at: Mutex::new(None),
            registry_watching: Mutex::new(false),
            transcript_watches: Mutex::new(HashMap::new()),
            transcript_reads: AtomicU32::new(0),
            transcript_timing: state
                .seam::<TranscriptTiming>()
                .map_or_else(TranscriptTiming::default, |timing| *timing),
            cancel: CancellationToken::new(),
            tasks: Mutex::new(Vec::new()),
            request_counter: AtomicU32::new(0),
            hold: state
                .seam::<HookHold>()
                .map_or(HOOK_SERVER_WAIT, |hold| hold.0),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    fn app(&self) -> Option<AppState> {
        self.inner.state.upgrade()
    }

    /// Whether anybody can see the window. While nobody does, the transcripts (up to 256 KB a
    /// read, and Claude writes to them all the time) are not read; `catch_up` reads them on show.
    fn looking(&self) -> bool {
        self.app()
            .is_none_or(|app| app.shell_facts.window_visible())
    }

    /// The window was shown: read the transcripts that moved while it was hidden (a file whose
    /// size and time are as before costs one `metadata` call) and publish what changed.
    async fn catch_up(&self) {
        self.sync_transcript_watches();
        let live = self.live_transcripts();
        futures_util::future::join_all(
            live.iter()
                .map(|(id, _)| self.read_transcript_tail(id, true)),
        )
        .await;
    }

    async fn run_visibility(self, mut watch: tokio::sync::watch::Receiver<bool>) {
        loop {
            tokio::select! {
                () = self.inner.cancel.cancelled() => break,
                changed = watch.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    if *watch.borrow_and_update() {
                        self.catch_up().await;
                    }
                }
            }
        }
    }

    fn now(&self) -> SystemTime {
        self.app()
            .map_or_else(SystemTime::now, |app| app.clock.now())
    }

    fn book(&self) -> MutexGuard<'_, Book> {
        lock(&self.inner.book)
    }

    // ---- what other modules call ----

    /// The sessions of `GET /sessions`, from memory, no I/O.
    pub fn list(&self) -> Vec<ClaudeSession> {
        self.book()
            .list(self.now(), &|cwd| self.project_id_for(cwd))
    }

    /// The number of open Claude sessions that are not runs of Rain's own jobs (a job run is
    /// never in the book). Game mode lists them and never stops them (decision of 2026-11-22).
    pub fn open_user_sessions(&self) -> u32 {
        u32::try_from(self.book().live_count()).unwrap_or(u32::MAX)
    }

    /// The process ids of the open Claude sessions. Game mode keeps the terminal of each.
    pub fn live_pids(&self) -> Vec<u32> {
        self.book().live_pids()
    }

    /// How many transcript tails were read since the start. The tests use it to see that a file
    /// that did not change costs no read.
    pub fn transcript_reads(&self) -> u32 {
        self.inner.transcript_reads.load(Ordering::Relaxed)
    }

    /// How many transcript folders are watched now. For the tests.
    pub fn transcript_watch_count(&self) -> usize {
        lock(&self.inner.transcript_watches).len()
    }

    /// The state of the switch "Monitor Claude sessions" (section 06 6.1.1). Reads one small
    /// file; no timer and no watch.
    pub fn hooks_state(&self) -> MonitorClaudeState {
        let Some(app) = self.app() else {
            return MonitorClaudeState::Off;
        };
        let status = self.hooks_status(&app, app.settings().resolved().monitor_claude);
        self.report_hooks_source(&app, &status);
        match status.state {
            HookState::Off => MonitorClaudeState::Off,
            HookState::Ok if status.last_event_at.is_some() => MonitorClaudeState::On,
            HookState::Ok => MonitorClaudeState::OnInactive,
            HookState::Broken | HookState::Blocked | HookState::Unavailable => {
                MonitorClaudeState::NeedsRepair
            }
        }
    }

    /// Puts Rain's hooks into the Claude settings, behind the switch. Safe to repeat; a repair is
    /// this call again. A sandbox run refuses the real settings file (foundation 13.3, rule 2).
    pub fn install_hooks(&self) -> BoxFuture<'_, ApiResult<()>> {
        Box::pin(self.install_hooks_inner())
    }

    /// Removes Rain's hooks again. Safe to repeat.
    pub fn remove_hooks(&self) -> BoxFuture<'_, ApiResult<()>> {
        Box::pin(self.remove_hooks_inner())
    }

    /// The project that holds the folder of a session (the longest project folder that is a
    /// prefix, case ignored). `None` once the backend is ending.
    fn project_id_for(&self, cwd: &str) -> Option<String> {
        self.app()?.projects().project_id_for_path(cwd)
    }

    // ---- start and stop ----

    /// Loads the sessions that ended in the last 24 hours, reads the registry, starts the exit
    /// watch task and checks the hooks.
    pub async fn start(&self) {
        let Some(app) = self.app() else { return };
        let now = app.clock.now();
        match store::load(&app.db, now).await {
            Ok(rows) => {
                let mut book = self.book();
                for row in rows {
                    if let Some(rec) = store::rec_of(row) {
                        book.recs.insert(rec.id.clone(), rec);
                    }
                }
            }
            Err(e) => warn!(error = %e, "the sessions that ended could not be read"),
        }
        self.ensure_registry_watch(&app);
        self.scan_registry().await;
        let status = self.hooks_status(&app, app.settings().resolved().monitor_claude);
        self.report_hooks_source(&app, &status);

        let visible = app.shell_facts.watch_visible();
        let task = spawn_named("sessions-visible", self.clone().run_visibility(visible));
        lock(&self.inner.tasks).push(task);

        if let Some(receiver) = lock(&self.inner.exit_rx).take() {
            let service = self.clone();
            let task = spawn_named("sessions-exits", service.run_exits(receiver));
            lock(&self.inner.tasks).push(task);
        }
    }

    /// Ends the background tasks and the exit watches. Publishes nothing.
    pub async fn stop(&self) {
        self.inner.cancel.cancel();
        let tasks: Vec<_> = lock(&self.inner.tasks).drain(..).collect();
        for task in tasks {
            let _ = task.await;
        }
        lock(&self.inner.transcript_watches).clear();
        for rec in self.book().recs.values_mut() {
            rec.watch = None;
        }
    }

    async fn run_exits(self, mut receiver: mpsc::UnboundedReceiver<u32>) {
        loop {
            tokio::select! {
                () = self.inner.cancel.cancelled() => break,
                pid = receiver.recv() => {
                    let Some(pid) = pid else { break };
                    debug!(pid, "a watched session process ended");
                    self.refresh().await;
                }
            }
        }
    }

    async fn run_registry(self, watch: DirWatch) {
        loop {
            tokio::select! {
                () = self.inner.cancel.cancelled() => break,
                _change = watch.changed() => self.scan_registry().await,
            }
        }
    }

    /// Follows the registry folder. Called at start and again whenever the list is read, because
    /// the folder may only appear later (Claude Code has not run yet). No timer: the check is one
    /// `is_dir` call per read.
    /// Returns true when the watch was started by this call.
    fn ensure_registry_watch(&self, app: &AppState) -> bool {
        let mut watching = lock(&self.inner.registry_watching);
        if *watching {
            return false;
        }
        let dir = self.inner.paths.registry_dir();
        if !dir.is_dir() {
            app.health.set_source(
                SOURCE_REGISTRY,
                SourceState::Off,
                Some("Claude Code has not run on this PC".to_owned()),
            );
            return false;
        }
        match app.hub.watch_dir(&dir, false, REGISTRY_QUIET) {
            Ok(watch) => {
                *watching = true;
                let task = spawn_named("sessions-registry", self.clone().run_registry(watch));
                lock(&self.inner.tasks).push(task);
                true
            }
            Err(e) => {
                warn!(error = %e, "the registry folder of Claude Code cannot be watched");
                false
            }
        }
    }

    // ---- the registry ----

    /// Reads the registry and updates the book: new sessions appear, known ones get their facts,
    /// ones whose process is gone end.
    pub async fn scan_registry(&self) {
        let Some(app) = self.app() else { return };
        let dir = self.inner.paths.registry_dir();
        let scan = blocking::run(move || registry::scan_dir(&dir))
            .await
            .unwrap_or_default();
        self.report_registry_source(&app, &scan);

        let mut to_read = Vec::new();
        let mut rows = Vec::new();
        {
            let now = app.clock.now();
            let mut book = self.book();
            for info in scan.infos {
                if let Some(id) = self.take_registry_info(&mut book, info, now) {
                    to_read.push(id);
                }
            }
            rows.extend(self.check_liveness_locked(&mut book, now));
        }
        self.persist(&app, rows).await;
        for id in to_read {
            self.spawn_transcript_read(id);
        }
        self.sync_transcript_watches();
        self.publish();
    }

    fn report_registry_source(&self, app: &AppState, scan: &registry::Scan) {
        if !self.inner.paths.registry_dir().is_dir() {
            app.health.set_source(
                SOURCE_REGISTRY,
                SourceState::Off,
                Some("Claude Code has not run on this PC".to_owned()),
            );
        } else if scan.files > 0 && scan.infos.is_empty() {
            warn!("the registry files of Claude Code are not understood");
            app.health.set_source(
                SOURCE_REGISTRY,
                SourceState::Failed,
                Some("Claude session files are in a format Rain does not know".to_owned()),
            );
        } else {
            app.health
                .set_source(SOURCE_REGISTRY, SourceState::Ok, None);
        }
    }

    /// Applies one registry file to the book. Returns the id of a session that is new and whose
    /// transcript should be read.
    fn take_registry_info(
        &self,
        book: &mut Book,
        info: RegistryInfo,
        now: SystemTime,
    ) -> Option<String> {
        // A run of one of Rain's own Claude jobs is not a session of the user (a hook told us).
        if book.job_sessions.contains(&info.session_id) {
            return None;
        }
        // Only a process that really runs makes a session. The registry file of a process that
        // died stays behind until the next start of `claude`.
        let start = self.inner.probe.creation_filetime(info.pid);
        let running = self.inner.probe.is_running(info.pid);
        let same_process = match (start, info.proc_start) {
            (Some(actual), Some(stated)) => same_start(actual, stated),
            (Some(_), None) => true,
            (None, _) => false,
        };
        if !running || !same_process {
            return None;
        }

        // The pid belongs to another session id now (`/clear` without a `SessionEnd`): the old
        // record is dropped silently.
        let stale: Vec<String> = book
            .recs
            .values()
            .filter(|rec| {
                rec.ended.is_none() && rec.pid == Some(info.pid) && rec.id != info.session_id
            })
            .map(|rec| rec.id.clone())
            .collect();
        for id in stale {
            book.recs.remove(&id);
        }

        let is_new = book
            .recs
            .get(&info.session_id)
            .is_none_or(|rec| rec.ended.is_some());
        if is_new {
            // A session that ended and runs again (`--resume`) starts a new record.
            book.recs.insert(
                info.session_id.clone(),
                SessionRec::new(&info.session_id, now),
            );
        }
        let id = info.session_id.clone();
        let rec = book.recs.get_mut(&id)?;
        if rec.proc_start.is_none() {
            rec.proc_start = start;
        }
        rec.apply_registry(info);
        self.watch_exit(rec);
        is_new.then_some(id)
    }

    /// Registers the exit watch of the process of a record, once.
    fn watch_exit(&self, rec: &mut SessionRec) {
        if rec.watch.is_some() || rec.ended.is_some() {
            return;
        }
        let (Some(pid), Some(start)) = (rec.pid, rec.proc_start) else {
            return;
        };
        let sender = self.inner.exit_tx.clone();
        rec.watch = self.inner.exits.watch(
            pid,
            start,
            Box::new(move || {
                let _ = sender.send(pid);
            }),
        );
    }

    // ---- liveness and the end of a session ----

    /// Checks every live record that has a process, and ends the ones whose process is gone.
    /// Cost: about 20 microseconds a session.
    fn check_liveness_locked(&self, book: &mut Book, now: SystemTime) -> Vec<EndedRow> {
        let mut gone: Vec<String> = Vec::new();
        for rec in book.recs.values_mut() {
            if rec.ended.is_none() && !self.is_alive(rec) {
                gone.push(rec.id.clone());
            }
        }
        let rows: Vec<EndedRow> = gone
            .iter()
            .filter_map(|id| {
                let ok = book.recs.get(id).is_none_or(|rec| {
                    let (status, _) = rec.status();
                    rec.stopped_by_rain
                        || !matches!(status, Status::Running | Status::Waiting | Status::Failed)
                });
                self.finish_locked(book, id, ok, now)
            })
            .collect();
        book.prune_ended(now);
        rows
    }

    /// True when the process of the record runs and is the same process (the pid may have been
    /// reused). A record without a pid cannot be checked and counts as alive.
    fn is_alive(&self, rec: &mut SessionRec) -> bool {
        let Some(pid) = rec.pid else { return true };
        if !self.inner.probe.is_running(pid) {
            return false;
        }
        let Some(actual) = self.inner.probe.creation_filetime(pid) else {
            return false;
        };
        match rec.proc_start {
            Some(stated) => same_start(actual, stated),
            None => {
                // First sight of a process that only a hook told about: remember its start.
                rec.proc_start = Some(actual);
                true
            }
        }
    }

    /// Ends a record. Returns the row to store.
    fn finish_locked(
        &self,
        book: &mut Book,
        id: &str,
        ok: bool,
        now: SystemTime,
    ) -> Option<EndedRow> {
        let rec = book.recs.get_mut(id)?;
        if rec.ended.is_some() {
            return None;
        }
        rec.ended = Some(super::book::Ended { at: now, ok });
        // The forwarders of open requests are gone or will be told: dropping the senders ends
        // their wait.
        rec.pending.clear();
        rec.watch = None;
        let project = self.project_id_for(&rec.cwd);
        store::row_of(rec, project)
    }

    async fn persist(&self, app: &AppState, rows: Vec<EndedRow>) {
        let now = app.clock.now();
        for row in rows {
            if let Err(e) = store::insert(&app.db, row, now).await {
                warn!(error = %e, "an ended session could not be stored");
            }
        }
    }

    /// Re-checks liveness, publishes, and returns the list: what `GET /sessions` answers.
    pub async fn list_refreshed(&self) -> Vec<ClaudeSession> {
        if let Some(app) = self.app() {
            // The folder may have appeared since the start: read what is in it now.
            if self.ensure_registry_watch(&app) {
                self.scan_registry().await;
            }
        }
        self.refresh().await;
        // A change event may have been missed (or not come yet): the first paint after a summon
        // looks at the size and the time of every live transcript, and reads the ones that moved.
        let live = self.live_transcripts();
        futures_util::future::join_all(
            live.iter()
                .map(|(id, _)| self.read_transcript_tail(id, true)),
        )
        .await;
        self.list()
    }

    /// The check at read time (section 03 6.3.3): a process that died without a hook ends its
    /// record.
    async fn refresh(&self) {
        let Some(app) = self.app() else { return };
        let rows = {
            let now = app.clock.now();
            let mut book = self.book();
            self.check_liveness_locked(&mut book, now)
        };
        let changed = !rows.is_empty();
        self.persist(&app, rows).await;
        // A session that ended no longer needs its folder watch.
        self.sync_transcript_watches();
        if changed {
            self.publish();
        }
    }

    // ---- publishing ----

    fn publish(&self) {
        let Some(app) = self.app() else { return };
        if !app.bus.has_receivers() {
            return;
        }
        match serde_json::to_value(fit_frame(self.list())) {
            Ok(payload) => self.inner.throttle.publish(&payload),
            Err(e) => warn!(error = %e, "the session list could not be written as JSON"),
        }
    }

    // ---- the transcript ----

    /// Reads the end of the transcript of a session and adds what it says (model, tokens,
    /// branch, title, last message), without blocking the caller.
    fn spawn_transcript_read(&self, id: String) {
        if !self.looking() {
            // Nobody looks: the tail is read when the window is shown (`catch_up`).
            return;
        }
        let service = self.clone();
        let task = spawn_named("sessions-transcript", async move {
            service.read_transcript_tail(&id, false).await;
        });
        let mut tasks = lock(&self.inner.tasks);
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }

    /// Reads the end of one transcript and publishes when something shown changed. With
    /// `only_if_changed` the file is not read when its size and modified time are what the last
    /// read saw: the caller asks "did it grow?" and a quiet file costs one `metadata` call.
    async fn read_transcript_tail(&self, id: &str, only_if_changed: bool) {
        let Some((path, known)) = self.transcript_state_of(id) else {
            return;
        };
        let inner = Arc::clone(&self.inner);
        let read = blocking::run(move || {
            // The signature is taken before the read: a file that grows during the read looks
            // changed at the next look, so no write is ever missed.
            let meta = std::fs::metadata(&path).ok()?;
            let sig = FileSig {
                len: meta.len(),
                modified: meta.modified().ok()?,
            };
            if only_if_changed && known == Some(sig) {
                return None;
            }
            inner.transcript_reads.fetch_add(1, Ordering::Relaxed);
            let tail = transcript::read_tail(&path, TAIL_BYTES).ok()?;
            let lines =
                transcript::parse_lines(&tail, &["user", "assistant", "custom-title", "ai-title"]);
            Some((sig, transcript::facts(&lines)))
        })
        .await
        .ok()
        .flatten();
        let Some((sig, facts)) = read else { return };
        // `stop` publishes nothing: a read that was in flight when it began ends silently.
        if self.inner.cancel.is_cancelled() {
            return;
        }
        let changed = self
            .book()
            .recs
            .get_mut(id)
            .is_some_and(|rec| rec.apply_tail(facts, Some(sig)));
        if changed {
            self.publish();
        }
    }

    /// The path of the transcript of a live session and what was seen of the file at the last
    /// read. `None` for an unknown or ended session.
    fn transcript_state_of(&self, id: &str) -> Option<(PathBuf, Option<FileSig>)> {
        let book = self.book();
        let rec = book.recs.get(id)?;
        if rec.ended.is_some() {
            return None;
        }
        Some((self.transcript_path_for(rec), rec.transcript_sig))
    }

    /// The path that a hook gave, else the one that Claude Code would use.
    fn transcript_path_for(&self, rec: &SessionRec) -> PathBuf {
        rec.transcript_path
            .clone()
            .unwrap_or_else(|| self.inner.paths.guess_transcript(&rec.cwd, &rec.id))
    }

    /// The live sessions with the path of their transcript. A session whose working folder is not
    /// known yet and whose path no hook gave has none (the guess would be wrong).
    fn live_transcripts(&self) -> Vec<(String, PathBuf)> {
        let book = self.book();
        book.recs
            .values()
            .filter(|rec| {
                rec.ended.is_none() && (rec.transcript_path.is_some() || !rec.cwd.is_empty())
            })
            .map(|rec| (rec.id.clone(), self.transcript_path_for(rec)))
            .collect()
    }

    /// Makes the set of folder watches equal to the folders of the live sessions: one watch per
    /// folder, none for a folder that no live session uses. A folder that does not exist yet gets
    /// its watch at the next call (a registry change, `GET /sessions`, a hook event); nothing
    /// polls for it. A new watch starts with one look at its sessions, because the file may have
    /// grown before the watch existed.
    fn sync_transcript_watches(&self) {
        let Some(app) = self.app() else { return };
        if self.inner.cancel.is_cancelled() {
            return;
        }
        let wanted: HashSet<PathBuf> = self
            .live_transcripts()
            .into_iter()
            .filter_map(|(_, path)| path.parent().map(Path::to_path_buf))
            .collect();
        let mut watches = lock(&self.inner.transcript_watches);
        watches.retain(|folder, token| {
            // A folder that was deleted leaves a watch that never speaks again: drop it, so it
            // is made anew if the folder comes back.
            let keep = wanted.contains(folder) && folder.is_dir();
            if !keep {
                token.cancel();
            }
            keep
        });
        for folder in wanted {
            if watches.contains_key(&folder) || !folder.is_dir() {
                continue;
            }
            let quiet = self.inner.transcript_timing.quiet;
            match app.hub.watch_dir(&folder, false, quiet) {
                Ok(watch) => {
                    let token = self.inner.cancel.child_token();
                    watches.insert(folder.clone(), token.clone());
                    let task = spawn_named(
                        "sessions-transcripts",
                        self.clone().run_transcript_folder(folder, watch, token),
                    );
                    let mut tasks = lock(&self.inner.tasks);
                    tasks.retain(|task| !task.is_finished());
                    tasks.push(task);
                }
                Err(e) => {
                    debug!(error = %e, "a transcript folder cannot be watched; it is read at each list");
                }
            }
        }
    }

    /// Owns the watch of one transcript folder until its token is cancelled. After a change it
    /// reads the sessions of the folder, then waits `gap` before it looks again: Claude writes
    /// all the time while it works, and the list does not need more than a read every few seconds.
    async fn run_transcript_folder(
        self,
        folder: PathBuf,
        watch: DirWatch,
        token: CancellationToken,
    ) {
        self.read_folder(&folder).await;
        loop {
            tokio::select! {
                () = token.cancelled() => break,
                _change = watch.changed() => {}
            }
            self.read_folder(&folder).await;
            tokio::select! {
                () = token.cancelled() => break,
                () = tokio::time::sleep(self.inner.transcript_timing.gap) => {}
            }
        }
    }

    /// Reads again the transcripts of the live sessions in `folder` that changed.
    async fn read_folder(&self, folder: &Path) {
        if !self.looking() {
            return;
        }
        for (id, path) in self.live_transcripts() {
            if path.parent() == Some(folder) {
                self.read_transcript_tail(&id, true).await;
            }
        }
    }

    /// `GET /sessions/{id}/transcript`.
    pub async fn transcript(&self, id: &str) -> ApiResult<Vec<LogLine>> {
        let (path, status_line) = {
            let book = self.book();
            let rec = book.recs.get(id).ok_or_else(unknown_session)?;
            let path = rec
                .transcript_path
                .clone()
                .unwrap_or_else(|| self.inner.paths.guess_transcript(&rec.cwd, &rec.id));
            (path, status_line(rec, self.now()))
        };
        let mut lines = blocking::run(move || transcript::read_log(&path))
            .await?
            .map_err(|_| {
                ApiError::not_found("The transcript file was not found").with_reason("noTranscript")
            })?;
        lines.push(status_line);
        Ok(lines)
    }

    // ---- hook events ----

    /// What `POST /internal/hook` does (section 03 6.2.3).
    pub async fn handle_hook(&self, envelope: HookEnvelope) -> HookReply {
        let Some(app) = self.app() else {
            return HookReply::Empty;
        };
        // With the switch off, or while Rain gets ready for game mode, every event is ignored.
        if !app.settings().resolved().monitor_claude || app.game_mode_active() {
            return HookReply::Empty;
        }
        let Some(session_id) = text(&envelope.event, "session_id").map(str::to_owned) else {
            return HookReply::Empty;
        };
        // A run of one of Rain's own Claude jobs is never shown and never asked.
        if let Some(run_id) = &envelope.env.run_id {
            // For a `SessionStart` of a run, the Claude session id is stored with the run.
            if text(&envelope.event, "hook_event_name") == Some("SessionStart") {
                app.jobs().attach_session(run_id, &session_id).await;
            }
            // The registry file of the same run may have been read before this hook came: the
            // session is taken out again and the registry scan skips it from now on.
            let removed = self.book().mark_job_session(&session_id);
            if removed {
                self.publish();
            }
            return HookReply::Empty;
        }
        *lock(&self.inner.last_event_at) = Some(app.clock.now());
        match envelope.mode {
            HookMode::Event => {
                self.apply_hook_event(&app, &session_id, &envelope).await;
                HookReply::Empty
            }
            HookMode::Permission => self.hold_permission(&app, &session_id, envelope).await,
        }
    }

    async fn apply_hook_event(&self, app: &AppState, session_id: &str, envelope: &HookEnvelope) {
        let now = app.clock.now();
        let (applied, rows) = {
            let mut book = self.book();
            let rec = self.rec_for_event(&mut book, session_id, now);
            let applied = rec.apply_event(&envelope.event, &envelope.env, now);
            self.watch_pid_of(rec);
            let mut rows = Vec::new();
            if applied.drop_record {
                book.recs.remove(session_id);
            } else if let Some(ok) = applied.finish {
                rows.extend(self.finish_locked(&mut book, session_id, ok, now));
            }
            (applied, rows)
        };
        self.persist(app, rows).await;
        self.after_event(session_id, &applied);
        self.publish();
    }

    fn after_event(&self, session_id: &str, applied: &Applied) {
        if applied.read_transcript {
            self.spawn_transcript_read(session_id.to_owned());
        }
        // A hook may tell the path of the transcript, and `SessionEnd` ends the session.
        self.sync_transcript_watches();
    }

    /// The record that an event belongs to. An event for an unknown session creates it (a session
    /// that started before Rain, or in a folder that was never trusted). An event for a session
    /// that ended starts a new record: the session runs again.
    fn rec_for_event<'a>(
        &self,
        book: &'a mut Book,
        id: &str,
        now: SystemTime,
    ) -> &'a mut SessionRec {
        let needs_new = book.recs.get(id).is_none_or(|rec| rec.ended.is_some());
        if needs_new {
            book.recs.insert(id.to_owned(), SessionRec::new(id, now));
        }
        book.recs
            .get_mut(id)
            .unwrap_or_else(|| unreachable!("the record was just inserted"))
    }

    /// A record that only a hook told about gets its process start from Windows at first sight,
    /// and an exit watch.
    fn watch_pid_of(&self, rec: &mut SessionRec) {
        if let (Some(pid), None) = (rec.pid, rec.proc_start) {
            rec.proc_start = self.inner.probe.creation_filetime(pid);
        }
        self.watch_exit(rec);
    }

    // ---- permission requests ----

    fn new_request_id(&self, now: SystemTime) -> String {
        let counter = self.inner.request_counter.fetch_add(1, Ordering::Relaxed);
        let millis = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u32);
        // Unique inside this run through the counter, and not guessable by a neighbour.
        let mixed = counter.wrapping_mul(0x9e37_79b1) ^ millis.rotate_left(13);
        format!("req-{mixed:08x}")
    }

    async fn hold_permission(
        &self,
        app: &AppState,
        session_id: &str,
        envelope: HookEnvelope,
    ) -> HookReply {
        let now = app.clock.now();
        let event = &envelope.event;
        let tool = text(event, "tool_name").unwrap_or("").to_owned();
        let input = event.get("tool_input").cloned().unwrap_or(Value::Null);
        let cwd = text(event, "cwd").unwrap_or("").to_owned();
        let typed = TYPED_ANSWER_TOOLS.contains(&tool.as_str());
        // Nobody can answer from Rain when no client listens (the window was never opened, or the
        // shell is gone). Holding the hook would only delay the prompt of Claude Code by the whole
        // wait, so the request is shown like a typed one: Rain says that the session waits, and
        // the hook returns at once.
        let holdable = !typed && app.bus.has_receivers();
        let request_id = self.new_request_id(now);
        let (sender, receiver) = oneshot::channel();

        {
            let mut book = self.book();
            let rec = self.rec_for_event(&mut book, session_id, now);
            rec.last_hook_at = now;
            rec.hooked = true;
            if !cwd.is_empty() {
                rec.cwd.clone_from(&cwd);
            }
            if let Some(path) = text(event, "transcript_path") {
                rec.transcript_path = Some(path.into());
            }
            if rec.pid.is_none() {
                rec.pid = envelope.env.process_id;
            }
            if rec.env_entrypoint.is_none() {
                rec.env_entrypoint.clone_from(&envelope.env.entrypoint);
            }
            self.watch_pid_of(rec);
            // A request means the session is working, whatever the hooks said before.
            if rec.turn != Turn::Failed {
                rec.turn = Turn::Working;
            }
            rec.pending.push(Pending {
                id: request_id.clone(),
                tool: tool.clone(),
                input_hash: input_hash(&input),
                summary: if typed {
                    tool.clone()
                } else {
                    summary(&tool, &input, &cwd)
                },
                detail: if typed {
                    String::new()
                } else {
                    detail(&tool, &input, &cwd)
                },
                requested_at: now,
                answerable: holdable,
                reply: holdable.then_some(sender),
                input,
                suggestions: event.get("permission_suggestions").cloned(),
            });
        }
        self.publish();
        if !holdable {
            // Claude Code shows its own dialog at once; Rain only says that the session waits.
            return HookReply::Empty;
        }

        // If this future is dropped (the forwarder was killed, or Claude Code cancelled the hook
        // because the user answered in the terminal first), the guard marks the request as one
        // that Rain can no longer answer.
        let mut guard = HoldGuard {
            service: self.clone(),
            session_id: session_id.to_owned(),
            request_id: request_id.clone(),
            armed: true,
        };
        // Timer T4 of foundation 11.5: one sleep for this one request, dropped with it.
        match tokio::time::timeout(self.inner.hold, receiver).await {
            Ok(Ok(decision)) => {
                guard.armed = false;
                HookReply::Decision(decision)
            }
            // The sender is gone: an event cleared the request. Claude Code moved on.
            Ok(Err(_)) => {
                guard.armed = false;
                HookReply::Empty
            }
            // No answer in time: the hook program hands over to the prompt of Claude Code.
            Err(_) => HookReply::Empty,
        }
    }

    /// The request can no longer be answered from Rain, but is still open in Claude Code.
    fn expire_request(&self, session_id: &str, request_id: &str) {
        {
            let mut book = self.book();
            let Some(rec) = book.recs.get_mut(session_id) else {
                return;
            };
            if let Some(request) = rec.pending.iter_mut().find(|p| p.id == request_id) {
                request.answerable = false;
                request.reply = None;
            }
        }
        self.publish();
    }

    /// `POST /sessions/{id}/permission`.
    pub async fn answer(
        &self,
        session_id: &str,
        body: PermissionAnswerBody,
    ) -> ApiResult<PermissionAnswerResult> {
        let app = self.app().ok_or_else(shutting_down)?;
        if !app.settings().resolved().monitor_claude {
            return Err(ApiError::conflict(
                "monitoringOff",
                "Monitoring is off. Turn on Monitor Claude sessions in Settings.",
            ));
        }
        let now = app.clock.now();
        let result = {
            let mut book = self.book();
            let rec = book.recs.get_mut(session_id).ok_or_else(unknown_session)?;
            let at = rec
                .pending
                .iter()
                .position(|p| p.id == body.request_id && p.answerable && p.reply.is_some())
                .ok_or_else(request_expired)?;

            // What Claude Code is told, and what really happened.
            let request = &rec.pending[at];
            let (answer, json, rule_text) = match body.answer {
                PermissionAnswer::Deny => (
                    PermissionAnswer::Deny,
                    decision_json(PermissionAnswer::Deny, None),
                    None,
                ),
                PermissionAnswer::Once => (
                    PermissionAnswer::Once,
                    decision_json(PermissionAnswer::Once, None),
                    None,
                ),
                PermissionAnswer::Always => {
                    match choose_always(&request.tool, &request.input, request.suggestions.as_ref())
                    {
                        AlwaysChoice::Rule { update, rule_text } => (
                            PermissionAnswer::Always,
                            decision_json(PermissionAnswer::Always, Some(&update)),
                            Some(rule_text),
                        ),
                        AlwaysChoice::Degrade => (
                            PermissionAnswer::Once,
                            decision_json(PermissionAnswer::Once, None),
                            None,
                        ),
                    }
                }
            };
            let label = format!("{}: {}", request.tool, request.summary);
            let Some(sender) = rec.pending[at].reply.take() else {
                return Err(request_expired());
            };
            if sender.send(json).is_err() {
                // The hook program already gave up: the request is no longer answerable.
                rec.pending[at].answerable = false;
                drop(book);
                self.publish();
                return Err(request_expired());
            }
            rec.pending.remove(at);
            let verb = if answer == PermissionAnswer::Deny {
                "Denied"
            } else {
                "Allowed"
            };
            rec.note(
                now,
                SessionEventKind::Permission,
                &format!("{verb}: {label}"),
            );
            PermissionAnswerResult {
                applied: answer,
                rule_text,
            }
        };
        self.publish();
        Ok(result)
    }

    // ---- focus and stop ----

    /// `POST /sessions/{id}/focus`. Never an error for "could not": the result says `none`.
    pub async fn focus(&self, id: &str) -> ApiResult<FocusResult> {
        // A session whose process ended a moment ago has nothing to focus.
        self.refresh().await;
        let request = {
            let book = self.book();
            let rec = book.recs.get(id).ok_or_else(unknown_session)?;
            let (_, alive) = rec.status();
            match (alive, rec.pid) {
                (true, Some(pid)) => Some(FocusRequest {
                    host: rec.host(),
                    pid,
                    title: rec.title.resolve(&rec.cwd),
                    folder: super::book::folder_name(&rec.cwd),
                }),
                _ => None,
            }
        };
        let Some(request) = request else {
            return Ok(FocusResult {
                focused: FocusKind::None,
            });
        };
        let (probe, windows) = (
            Arc::clone(&self.inner.probe),
            Arc::clone(&self.inner.windows),
        );
        let focused = blocking::run(move || focus::focus(&request, &probe, &windows)).await?;
        Ok(FocusResult { focused })
    }

    /// `POST /sessions/{id}/stop` (section 03 6.7). Only a live terminal session whose process
    /// is verified to be Claude. There is no graceful stop for an interactive session.
    pub async fn stop_session(&self, id: &str) -> ApiResult<()> {
        let pid = {
            let book = self.book();
            let rec = book.recs.get(id).ok_or_else(unknown_session)?;
            if rec.ended.is_some() {
                return Ok(());
            }
            if rec.host() == SessionHost::Desktop {
                return Err(not_stoppable(
                    "This session runs inside the Claude desktop app. Close it there.",
                ));
            }
            let verified = rec.pid.filter(|pid| {
                self.inner.probe.is_running(*pid)
                    && match (self.inner.probe.creation_filetime(*pid), rec.proc_start) {
                        (Some(actual), Some(stated)) => same_start(actual, stated),
                        _ => false,
                    }
            });
            verified.ok_or_else(|| {
                not_stoppable(
                    "Rain cannot verify this session's process. Close it in its terminal.",
                )
            })?
        };

        let probe = Arc::clone(&self.inner.probe);
        let (image, descendants) = blocking::run({
            let probe = Arc::clone(&probe);
            move || (probe.image_name(pid), probe.descendants(pid))
        })
        .await?;
        if !image.is_some_and(|name| STOPPABLE_IMAGES.contains(&name.as_str())) {
            return Err(not_stoppable(
                "That process is not Claude. Nothing was stopped.",
            ));
        }

        // Mark the end as Rain's own BEFORE the process dies: the exit watch fires at once, and the
        // record must then end as a normal end and not as a failure.
        let now = self.now();
        if let Some(rec) = self.book().recs.get_mut(id) {
            rec.stopped_by_rain = true;
            rec.note(now, SessionEventKind::Permission, "Stopped from Rain");
        }

        // The root first, then each descendant that was found.
        let ended = blocking::run({
            let probe = Arc::clone(&probe);
            move || {
                let root = probe.terminate(pid);
                for child in descendants {
                    let _ = probe.terminate(child);
                }
                root
            }
        })
        .await?;
        if ended.is_err() {
            if let Some(rec) = self.book().recs.get_mut(id) {
                rec.stopped_by_rain = false;
            }
            return Err(ApiError::failed("Could not stop the session").with_reason("stopFailed"));
        }
        let mut waited = Duration::ZERO;
        while self.inner.probe.is_running(pid) && waited < STOP_WAIT {
            tokio::time::sleep(STOP_POLL).await;
            waited += STOP_POLL;
        }

        self.refresh().await;
        self.publish();
        Ok(())
    }

    // ---- the hooks installer ----

    fn settings_file(&self) -> PathBuf {
        hooks_install::settings_file(&self.inner.paths)
    }

    fn safe_folders(app: &AppState) -> Vec<PathBuf> {
        vec![
            std::env::temp_dir(),
            app.config.data_dir.clone(),
            app.config.local_dir.clone(),
        ]
    }

    fn check_target_allowed(&self, app: &AppState) -> ApiResult<()> {
        let safe = Self::safe_folders(app);
        let safe: Vec<&Path> = safe.iter().map(PathBuf::as_path).collect();
        if allowed(&self.settings_file(), app.config.sandbox, &safe) {
            Ok(())
        } else {
            Err(ApiError::conflict(
                "sandbox",
                "A development build never changes your real Claude settings.",
            ))
        }
    }

    /// The program of the hook that is installed, whether it is there and whether it is the same
    /// size as the one next to the backend.
    fn forwarder_ok(&self, app: &AppState) -> bool {
        let installed = forwarder_path(&app.config.local_dir);
        let Ok(installed_meta) = std::fs::metadata(&installed) else {
            return false;
        };
        match self
            .inner
            .forwarder
            .source()
            .and_then(|source| std::fs::metadata(source).ok())
        {
            Some(source_meta) => source_meta.len() == installed_meta.len(),
            None => true,
        }
    }

    fn hooks_status(&self, app: &AppState, expected_on: bool) -> HooksStatus {
        let mut status = hooks_install::status(
            &self.settings_file(),
            expected_on,
            self.forwarder_ok(app),
            self.inner.paths.exists(),
        );
        status.last_event_at = lock(&self.inner.last_event_at).map(iso_from_system_time);
        status
    }

    fn report_hooks_source(&self, app: &AppState, status: &HooksStatus) {
        debug!(
            switch = status.enabled,
            installed = status.events_installed,
            expected = status.events_expected,
            forwarder_ok = status.forwarder_ok,
            "the state of Rain's Claude hooks"
        );
        match status.state {
            HookState::Off => app.health.set_source(SOURCE_HOOKS, SourceState::Off, None),
            HookState::Ok => app.health.set_source(SOURCE_HOOKS, SourceState::Ok, None),
            HookState::Broken | HookState::Blocked | HookState::Unavailable => app
                .health
                .set_source(SOURCE_HOOKS, SourceState::Failed, status.message.clone()),
        }
    }

    async fn install_hooks_inner(&self) -> ApiResult<()> {
        let app = self.app().ok_or_else(shutting_down)?;
        self.check_target_allowed(&app)?;
        if !self.inner.paths.exists() {
            return Err(
                ApiError::unavailable("sessions", "Claude Code was not found on this PC.")
                    .with_reason("noClaude"),
            );
        }
        let forwarder = self.ensure_forwarder(&app).await?;

        let settings = self.settings_file();
        let backups = backup_dir(&app.config.local_dir);
        let now_ms = app
            .clock
            .now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let stamp = hooks_install::stamp_of(&iso_from_system_time(app.clock.now()));
        let pipe = app.config.pipe_name.clone();
        let forwarder_text = forwarder.to_string_lossy().into_owned();

        blocking::run(move || {
            // A repair keeps the install id that is already in the file.
            let existing = std::fs::read(&settings)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(strip_bom(&bytes)).ok())
                .and_then(|value| hooks_install::existing_install_id(&value));
            let spec = Spec {
                forwarder: forwarder_text,
                install_id: existing.unwrap_or_else(|| hooks_install::new_install_id(now_ms)),
                // Always written: the hook program picks its own default pipe by its build kind
                // (debug or release), which may differ from the backend that installs it.
                pipe: Some(pipe),
            };
            hooks_install::install(&settings, &backups, &spec, &stamp)
        })
        .await?
        .map_err(InstallError::into_api)?;

        let status = self.hooks_status(&app, true);
        self.report_hooks_source(&app, &status);
        if status.state != HookState::Ok {
            return Err(
                ApiError::failed("Could not write the Claude settings file.")
                    .with_reason("installFailed"),
            );
        }
        info!("Rain's hooks are installed in the Claude settings");
        Ok(())
    }

    async fn remove_hooks_inner(&self) -> ApiResult<()> {
        let app = self.app().ok_or_else(shutting_down)?;
        self.check_target_allowed(&app)?;
        // Nothing to remove when Claude Code was never there.
        if !self.inner.paths.exists() {
            return Ok(());
        }
        let settings = self.settings_file();
        let backups = backup_dir(&app.config.local_dir);
        let stamp = hooks_install::stamp_of(&iso_from_system_time(app.clock.now()));
        blocking::run(move || hooks_install::uninstall(&settings, &backups, &stamp))
            .await?
            .map_err(InstallError::into_api)?;

        let status = self.hooks_status(&app, false);
        self.report_hooks_source(&app, &status);
        if status.events_installed != 0 {
            return Err(
                ApiError::failed("Could not write the Claude settings file.")
                    .with_reason("installFailed"),
            );
        }
        info!("Rain's hooks are removed from the Claude settings");
        Ok(())
    }

    /// Copies the hook program next to the data of Rain, and proves that the copy starts. The
    /// path is stable and outside any folder that Claude Code or Rain updates.
    async fn ensure_forwarder(&self, app: &AppState) -> ApiResult<PathBuf> {
        let missing = || {
            ApiError::failed("The Rain hook program was not found or did not start.")
                .with_reason("hookMissing")
        };
        let source = self.inner.forwarder.source().ok_or_else(missing)?;
        let target = forwarder_path(&app.config.local_dir);
        let copy_target = target.clone();
        let copied = blocking::run(move || copy_forwarder(&source, &copy_target)).await?;
        // A copy that failed but left an older program in place is not fatal: the status then
        // says the program is out of date.
        if copied.is_err() && !target.is_file() {
            return Err(missing());
        }
        if !self.inner.forwarder.selftest(&target).await {
            return Err(missing());
        }
        Ok(target)
    }
}

/// Copies `source` to `target` unless they are equal: through a temporary file in the same
/// folder, then a rename. A running forwarder makes the rename fail with a sharing error, so it
/// is tried again a few times.
fn copy_forwarder(source: &Path, target: &Path) -> std::io::Result<()> {
    let wanted = std::fs::read(source)?;
    if std::fs::read(target).is_ok_and(|current| current == wanted) {
        return Ok(());
    }
    let folder = target.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(folder)?;
    let temp = folder.join(".rain-hook.exe.tmp");
    std::fs::write(&temp, &wanted)?;
    let mut last = None;
    for attempt in 0..COPY_RETRIES {
        match std::fs::rename(&temp, target) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        if attempt + 1 < COPY_RETRIES {
            // Runs on a blocking thread (`core::blocking::run`).
            #[allow(clippy::disallowed_methods)]
            std::thread::sleep(COPY_DELAY);
        }
    }
    let _ = std::fs::remove_file(&temp);
    Err(last.unwrap_or_else(|| std::io::Error::other("no attempt was made")))
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

/// Marks an open request as no longer answerable when the handler that holds it is dropped.
struct HoldGuard {
    service: Service,
    session_id: String,
    request_id: String,
    armed: bool,
}

impl Drop for HoldGuard {
    fn drop(&mut self) {
        if self.armed {
            self.service
                .expire_request(&self.session_id, &self.request_id);
        }
    }
}

// ---- errors ----

fn unknown_session() -> ApiError {
    ApiError::not_found("Session not found").with_reason("unknownSession")
}

fn request_expired() -> ApiError {
    ApiError::conflict(
        "requestExpired",
        "That request is no longer open. Answer it in the terminal.",
    )
}

fn not_stoppable(message: &str) -> ApiError {
    ApiError::conflict("notStoppable", message)
}

fn shutting_down() -> ApiError {
    ApiError::unavailable("sessions", "Rain is stopping").with_reason("shuttingDown")
}

/// The last line of a transcript log: how the session stands now.
fn status_line(rec: &SessionRec, now: SystemTime) -> LogLine {
    let (status, _) = rec.status();
    let (level, text) = match status {
        Status::Waiting => {
            let request = rec.shown_request();
            let what = request.map_or_else(String::new, |r| format!("{} {}", r.tool, r.summary));
            (LogLevel::Warn, format!("Waiting for your answer: {what}"))
        }
        Status::Failed => (LogLevel::Error, "Session stopped after an error".to_owned()),
        Status::Success => (LogLevel::Info, "Session ended".to_owned()),
        Status::Idle => (LogLevel::Info, "Waiting for your next prompt".to_owned()),
        Status::Running => (LogLevel::Info, "Working".to_owned()),
    };
    LogLine {
        ts: iso_from_system_time(now),
        level,
        text,
    }
}
