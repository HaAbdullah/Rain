//! The logic and the in-memory state of the launcher. Other modules call it, and only it.
//! Section 01 6.
//!
//! What the `Service` holds:
//! - the three TOML files (pins, pools, app ids), read from memory and written with `edit`;
//! - the raw rows of the last scan and the app list built from them (memory, rebuilt when a
//!   scan or a hand edit of `app-ids.toml` changes an input);
//! - the history map ("last launched"), read from SQLite once at start;
//! - the registry of launch runs and the task tracker that owns the run tasks.
//!
//! Nothing here polls: scans run at start, on request, after a Start Menu change and when a read
//! finds the list old; a run task lives as long as its steps.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use rain_types::time::iso_from_system_time;
use rain_types::{
    AppEntry, AppKind, IconRef, Launch, LaunchRun, LaunchTarget, LaunchType, PinnableType, Pool,
    PoolDraft, PoolMember, RunStatus, SearchItem, SearchType, SourceState, Status,
};
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};

use super::apps::{self, AppRecord, AppSource, Row, SourceError, WindowsAppSource};
use super::icons::{AppIcons, IconRenderer, Icons, ShellIconRenderer};
use super::launch::{
    DiskProbe, EngineStep, PathProbe, Registry, Source, StepEnv, is_launcher_history, new_run,
    run_step,
};
use super::limits::{
    APPS_SCAN_MAX_AGE, ARGS_MAX, FIRST_SCAN_WAIT, HISTORY_DAYS, POOL_MAX, PROCESS_TTL, QUERY_MAX,
    RECENT_DEFAULT, RECENT_MAX, START_GRACE, START_MENU_QUIET,
};
use super::opener::WindowsOpener;
use super::other_items;
use super::pools::{check_draft, new_pool_id};
use super::running::RunningIndex;
use super::search::{self, Candidate};
use super::store::{self, AppIdsFile, PinsFile, PoolRow, PoolsFile, id_map};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult, shutting_down_error};
use crate::core::launch_contract::{ItemSource, LaunchPlanner};
use crate::core::state::{AppState, WeakAppState};
use crate::core::throttle::Throttle;
use crate::platform::process::{ProcessLister, SystemProcessLister};
use crate::store::hub::DirWatch;
use crate::store::toml_file::TomlFile;

/// Names this module on the errors it raises.
const SOURCE: &str = "launcher";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every change under these locks is a plain assignment, so a poisoned lock is still whole.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- scanning ----

/// Why a scan failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanError {
    Failed,
    ComBusy,
}

impl ScanError {
    fn into_api(self) -> ApiError {
        match self {
            Self::ComBusy => {
                ApiError::unavailable(SOURCE, "Windows is not answering. Try again in a moment")
                    .with_reason("comBusy")
            }
            Self::Failed => ApiError::unavailable(SOURCE, "Could not read the installed apps")
                .with_reason("scanFailed"),
        }
    }
}

impl From<SourceError> for ScanError {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::ComBusy => Self::ComBusy,
            SourceError::Failed => Self::Failed,
        }
    }
}

type ScanFlight = Shared<BoxFuture<'static, Result<(), ScanError>>>;

/// What the dev-only fault route can make the app source do (section 01 11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultMode {
    Normal,
    Empty,
    Fail,
}

/// The source that the service reads: the real (or a test's) one, with a switch that the
/// sandbox-only fault route turns.
struct SwitchableSource {
    inner: Arc<dyn AppSource>,
    mode: Mutex<FaultMode>,
}

impl AppSource for SwitchableSource {
    fn enumerate(&self) -> BoxFuture<'_, Result<Vec<crate::platform::shell::RawApp>, SourceError>> {
        let mode = *lock(&self.mode);
        Box::pin(async move {
            match mode {
                FaultMode::Normal => self.inner.enumerate().await,
                FaultMode::Empty => Ok(Vec::new()),
                FaultMode::Fail => Err(SourceError::Failed),
            }
        })
    }
}

#[derive(Default)]
struct ListState {
    /// The rows of the last scan, or of the cache in SQLite until the first scan ends.
    rows: Vec<Row>,
    /// True once the list is usable: the cache had rows, or a scan ended well.
    filled: bool,
    /// When the last scan ended (well or not); a read older than `APPS_SCAN_MAX_AGE` rescans.
    last_attempt: Option<SystemTime>,
    built: Option<Built>,
}

/// The list built from `rows` and one state of `app-ids.toml`.
struct Built {
    ids_doc: Arc<AppIdsFile>,
    apps: Arc<Vec<AppRecord>>,
}

// ---- the service ----

pub struct Service {
    state: WeakAppState,
    pins: TomlFile<PinsFile>,
    pools: TomlFile<PoolsFile>,
    ids: TomlFile<AppIdsFile>,
    source: SwitchableSource,
    external_source: bool,
    processes: Arc<dyn ProcessLister>,
    icons: Icons,
    probe: Arc<dyn PathProbe>,
    list: Mutex<ListState>,
    flight: Mutex<Option<ScanFlight>>,
    /// The newest use of every app and pool, from `launcher_launch`.
    history: Mutex<HashMap<(LaunchType, String), String>>,
    /// Apps that Rain started lately: they count as running for `START_GRACE`.
    grace: Mutex<HashMap<String, SystemTime>>,
    snapshot: Mutex<Option<(SystemTime, Arc<RunningIndex>)>>,
    runs: Mutex<Registry>,
    run_counter: AtomicU64,
    throttle: Throttle<Launch>,
    tracker: TaskTracker,
}

impl Service {
    /// Opens the files, loads the cache and the history, and puts the parts in place. A test
    /// replaces the app source, the icon renderer, the process lister and the file probe with
    /// `with::<dyn ...>` (foundation 12.2).
    pub async fn new(state: &AppState) -> anyhow::Result<Self> {
        let open_error = |e: ApiError| anyhow::anyhow!("{}", e.body.message);
        let pins = TomlFile::open(&state.hub, store::PINS_FILE, store::validate_pins)
            .await
            .map_err(open_error)?;
        let pools = TomlFile::open(&state.hub, store::POOLS_FILE, store::validate_pools)
            .await
            .map_err(open_error)?;
        // Lenient on purpose: see the top of store.rs.
        let ids = TomlFile::open(&state.hub, store::APP_IDS_FILE, |_: &AppIdsFile| Ok(()))
            .await
            .map_err(open_error)?;

        let external = state.seam::<dyn AppSource>();
        let renderer = state
            .seam::<dyn IconRenderer>()
            .unwrap_or_else(|| Arc::new(ShellIconRenderer));
        let (rows, history) = load_database(state).await;
        let filled = !rows.is_empty();

        Ok(Self {
            state: state.downgrade(),
            pins,
            pools,
            ids,
            external_source: external.is_some(),
            source: SwitchableSource {
                inner: external.unwrap_or_else(|| Arc::new(WindowsAppSource)),
                mode: Mutex::new(FaultMode::Normal),
            },
            processes: state
                .seam::<dyn ProcessLister>()
                .unwrap_or_else(|| Arc::new(SystemProcessLister)),
            icons: Icons::new(&state.config.local_dir, renderer),
            probe: state
                .seam::<dyn PathProbe>()
                .unwrap_or_else(|| Arc::new(DiskProbe)),
            list: Mutex::new(ListState {
                rows,
                filled,
                ..ListState::default()
            }),
            flight: Mutex::default(),
            history: Mutex::new(history),
            grace: Mutex::default(),
            snapshot: Mutex::default(),
            runs: Mutex::default(),
            run_counter: AtomicU64::new(0),
            throttle: Throttle::new(state.bus.clone(), rain_types::limits::TOPIC_RATE_MAX),
            tracker: TaskTracker::new(),
        })
    }

    /// The registration that `init` does besides building the service.
    pub fn register(state: &AppState) {
        state.install_default_opener(Arc::new(WindowsOpener));
        state.media.register(
            "app-icon",
            Arc::new(AppIcons {
                state: state.downgrade(),
            }),
        );
    }

    fn app(&self) -> ApiResult<AppState> {
        self.state.upgrade().ok_or_else(shutting_down_error)
    }

    fn now(&self) -> SystemTime {
        self.state
            .upgrade()
            .map_or_else(SystemTime::now, |state| state.clock.now())
    }

    // ---- start and stop ----

    /// Starts the first scan (unless game mode is on) and the Start Menu watch.
    pub fn start(self: &Arc<Self>, state: &AppState) {
        if !state.game_mode_active() {
            drop(self.scan());
        }
        // A test that replaced the source has no business watching the real Start Menu.
        if !self.external_source {
            for folder in start_menu_folders() {
                for watch in watch_folder(state, &folder) {
                    self.follow(watch, state.clone());
                }
            }
        }
    }

    /// One rescan per quiet burst of changes in a Start Menu folder; no timer of our own.
    fn follow(self: &Arc<Self>, watch: DirWatch, state: AppState) {
        let me = Arc::clone(self);
        let cancel = state.shutdown.token().clone();
        self.tracker.spawn(async move {
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    _ = watch.changed() => {
                        if !state.game_mode_active() {
                            drop(me.scan());
                        }
                    }
                }
            }
        });
    }

    /// Ends the tasks. A run that is still going stops without a final frame: stopping
    /// publishes nothing, and the programs that were started stay.
    pub async fn stop(&self) {
        self.tracker.close();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.tracker.wait()).await;
    }

    // ---- the app list ----

    /// The app list, built from the rows of the last scan and the id map of the file.
    fn current_apps(&self) -> Arc<Vec<AppRecord>> {
        let ids_doc = self.ids.get();
        let mut list = lock(&self.list);
        if let Some(built) = &list.built
            && Arc::ptr_eq(&built.ids_doc, &ids_doc)
        {
            return Arc::clone(&built.apps);
        }
        let apps = Arc::new(apps::build(&list.rows, &id_map(&ids_doc)).apps);
        list.built = Some(Built {
            ids_doc,
            apps: Arc::clone(&apps),
        });
        apps
    }

    fn find_app(&self, id: &str) -> Option<AppRecord> {
        self.current_apps().iter().find(|app| app.id == id).cloned()
    }

    /// Reads the installed apps now. A scan that is running is shared: every caller gets its
    /// result. The scan runs in its own task, so it ends even when every caller gave up.
    pub fn scan(self: &Arc<Self>) -> ScanFlight {
        let mut slot = lock(&self.flight);
        if let Some(running) = slot.as_ref()
            && running.peek().is_none()
        {
            return running.clone();
        }
        let me = Arc::clone(self);
        let flight: ScanFlight = async move { me.do_scan().await }.boxed().shared();
        let driver = flight.clone();
        self.tracker.spawn(async move {
            let _ = driver.await;
        });
        *slot = Some(flight.clone());
        flight
    }

    /// A scan that a person asked for ("Scan again"). One that is already running may have read
    /// the list before the change that the person made, so it is awaited and a fresh one follows.
    async fn scan_fresh(self: &Arc<Self>) -> Result<(), ScanError> {
        let running = lock(&self.flight)
            .as_ref()
            .filter(|flight| flight.peek().is_none())
            .cloned();
        if let Some(running) = running {
            // Its result does not matter: the new scan decides.
            let _ = running.await;
        }
        self.scan().await
    }

    async fn do_scan(&self) -> Result<(), ScanError> {
        let state = self.app().map_err(|_| ScanError::Failed)?;
        let raw = match self.source.enumerate().await {
            Ok(raw) => raw,
            Err(e) => return Err(self.scan_failed(&state, e.into())),
        };
        // Windows always lists apps: an empty answer next to a full cache means the shell did
        // not answer properly. A first scan that finds nothing is a valid empty list. The dev-only
        // fault "empty" is the exception: it exists to show the empty screen.
        let faked_empty = *lock(&self.source.mode) == FaultMode::Empty;
        if raw.is_empty() && !faked_empty && !lock(&self.list).rows.is_empty() {
            return Err(self.scan_failed(&state, ScanError::Failed));
        }
        let targets = apps::file_targets(&raw);
        let mtimes = blocking::run(move || modified_times(&targets))
            .await
            .unwrap_or_default();
        let rows = apps::rows_from_scan(raw, &mtimes);
        let now = state.clock.now();

        // The ids are written BEFORE the list is published, so a reader that sees the new list
        // finds the ids in the file too.
        let issued = apps::build(&rows, &id_map(&self.ids.get())).issued;
        if !issued.is_empty() {
            // Not written when the file has an error: the ids are then provisional (the same
            // function makes them) and the issue shows in /health.
            if let Err(e) = self
                .ids
                .edit(move |doc| store::append_app_ids(doc, &issued))
                .await
            {
                warn!(reason = %e.body.message, "new app ids were not written");
            }
        }
        let stored = rows.clone();
        let scanned_at = iso_from_system_time(now);
        if let Err(e) = state
            .db
            .call(move |db| store::replace_rows(db, &stored, &scanned_at))
            .await
        {
            // The cache is only a faster start; the list in memory is complete.
            warn!(error = %e, "could not store the app cache");
        }
        let apps = apps::build(&rows, &id_map(&self.ids.get())).apps;
        self.icons.clean(&apps).await;
        {
            let mut list = lock(&self.list);
            list.rows = rows;
            list.filled = true;
            list.last_attempt = Some(now);
            list.built = None;
        }
        state.health.clear_source("apps");
        info!(apps = apps.len(), "the app list was read");
        Ok(())
    }

    fn scan_failed(&self, state: &AppState, why: ScanError) -> ScanError {
        let had_list = {
            let mut list = lock(&self.list);
            list.last_attempt = Some(state.clock.now());
            list.filled
        };
        let message = if had_list {
            "Showing the last known app list"
        } else {
            "Could not read the installed apps"
        };
        state
            .health
            .set_source("apps", SourceState::Failed, Some(message.to_owned()));
        why
    }

    /// The names of the known programs by the lower-case path of their program file, from the
    /// list in memory (never a scan). Game mode names the programs it lists with it.
    pub fn app_names(&self) -> HashMap<String, String> {
        self.current_apps()
            .iter()
            .filter(|app| app.kind == AppKind::Program && !app.exe_path.is_empty())
            .map(|app| {
                (
                    apps::normalise_path(&app.exe_path).to_lowercase(),
                    app.name.clone(),
                )
            })
            .collect()
    }

    /// `GET /apps`.
    pub async fn list_apps(self: &Arc<Self>) -> ApiResult<Vec<AppEntry>> {
        let state = self.app()?;
        let (filled, stale) = {
            let list = lock(&self.list);
            let now = state.clock.now();
            let stale = list
                .last_attempt
                .is_none_or(|at| now.duration_since(at).unwrap_or_default() >= APPS_SCAN_MAX_AGE);
            (list.filled, stale)
        };
        if !filled {
            if state.game_mode_active() {
                return Ok(Vec::new());
            }
            match tokio::time::timeout(FIRST_SCAN_WAIT, self.scan()).await {
                Err(_) => {
                    return Err(
                        ApiError::unavailable(SOURCE, "The app list is still loading")
                            .with_reason("scanning"),
                    );
                }
                Ok(Err(e)) => return Err(e.into_api()),
                Ok(Ok(())) => {}
            }
        } else if stale && !state.game_mode_active() {
            drop(self.scan());
        }
        Ok(self.entries(&state).await)
    }

    /// `POST /apps/scan`.
    pub async fn rescan(self: &Arc<Self>) -> ApiResult<Vec<AppEntry>> {
        let state = self.app()?;
        if state.game_mode_active() {
            return Err(game_mode());
        }
        // A scan that a person asked for blocks an upgrade while it runs (foundation 11.7). The
        // scan of the start and the background ones do not: nobody waits for them.
        let _busy = state.activity.guard();
        self.scan_fresh().await.map_err(ScanError::into_api)?;
        Ok(self.entries(&state).await)
    }

    async fn entries(&self, state: &AppState) -> Vec<AppEntry> {
        let apps = self.current_apps();
        let index = self.running_index(state).await;
        let pins = self.pins.get();
        let history = lock(&self.history).clone();
        let now = state.clock.now();
        let mut entries: Vec<AppEntry> = apps
            .iter()
            .map(|app| AppEntry {
                id: app.id.clone(),
                name: app.name.clone(),
                icon: app.icon(),
                kind: app.kind,
                exe_path: (app.kind == AppKind::Program).then(|| app.exe_path.clone()),
                keywords: app.keywords.clone(),
                pinned: pins.contains("app", &app.id),
                running: index.is_running(app) || self.in_grace(&app.id, now),
                last_launched_at: history.get(&(LaunchType::App, app.id.clone())).cloned(),
            })
            .collect();
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        entries
    }

    fn in_grace(&self, id: &str, now: SystemTime) -> bool {
        lock(&self.grace)
            .get(id)
            .is_some_and(|at| now.duration_since(*at).unwrap_or_default() < START_GRACE)
    }

    /// The snapshot of running programs, at most `PROCESS_TTL` old (4.7 ms to take).
    async fn running_index(&self, state: &AppState) -> Arc<RunningIndex> {
        let now = state.clock.now();
        if let Some((at, index)) = lock(&self.snapshot).as_ref()
            && now.duration_since(*at).unwrap_or_default() < PROCESS_TTL
        {
            return Arc::clone(index);
        }
        let lister = Arc::clone(&self.processes);
        let images = blocking::run(move || lister.images())
            .await
            .unwrap_or_default();
        let index = Arc::new(RunningIndex::new(&images));
        *lock(&self.snapshot) = Some((now, Arc::clone(&index)));
        index
    }

    /// `GET /media/app-icon/{id}`.
    pub async fn icon(&self, id: &str) -> ApiResult<crate::core::media::Media> {
        let app = self.find_app(id).ok_or_else(|| {
            ApiError::not_found("No such app")
                .with_source(SOURCE)
                .with_reason("unknownApp")
        })?;
        self.icons.get(&app).await
    }

    /// Swaps what the app source answers (the sandbox-only fault route).
    pub fn set_fault(&self, mode: FaultMode) {
        *lock(&self.source.mode) = mode;
    }

    // ---- items ----

    fn app_item(app: &AppRecord) -> SearchItem {
        let subtitle = match app.kind {
            AppKind::Program => Some(app.exe_path.clone()),
            AppKind::Store | AppKind::Other => apps::package_name(&app.aumid).map(str::to_owned),
        };
        SearchItem {
            r#type: SearchType::App,
            id: app.id.clone(),
            title: app.name.clone(),
            subtitle,
            icon: app.icon(),
            status: None,
            score: 0,
        }
    }

    fn pool_item(row: &PoolRow) -> SearchItem {
        let count = row.members.len();
        SearchItem {
            r#type: SearchType::Pool,
            id: row.id.clone(),
            title: row.name.clone(),
            subtitle: Some(format!(
                "{count} {}",
                if count == 1 { "app" } else { "apps" }
            )),
            icon: IconRef::letter(&row.name),
            status: None,
            score: 0,
        }
    }

    /// The item of a pin or a history row. Gone or hidden items give `None`.
    fn item_for(&self, state: &AppState, kind: &str, id: &str) -> Option<SearchItem> {
        match kind {
            "app" => self.find_app(id).map(|app| Self::app_item(&app)),
            "pool" => self
                .pools
                .get()
                .pool
                .iter()
                .find(|row| row.id == id)
                .map(Self::pool_item),
            "project" => state.projects().item_for(id),
            "game" => state.games().item_for(id),
            _ => None,
        }
    }

    // ---- search, pins, recent ----

    /// `GET /search`. No disk, no SQLite, no process snapshot.
    pub fn search(&self, query: &str, types: Option<&[SearchType]>) -> ApiResult<Vec<SearchItem>> {
        if query.chars().count() > QUERY_MAX {
            return Err(ApiError::invalid("The search text is too long")
                .with_source(SOURCE)
                .with_reason("queryTooLong"));
        }
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let state = self.app()?;
        let wanted = |kind: SearchType| types.is_none_or(|list| list.contains(&kind));
        let mut candidates = Vec::new();
        if wanted(SearchType::App) {
            candidates.extend(self.current_apps().iter().map(|app| Candidate {
                item: Self::app_item(app),
                keywords: app.keywords.clone(),
            }));
        }
        if wanted(SearchType::Pool) {
            let apps = self.current_apps();
            for row in &self.pools.get().pool {
                // A pool is also found by the names of its apps.
                let keywords = row
                    .members
                    .iter()
                    .filter_map(|member| apps.iter().find(|app| app.id == member.app))
                    .map(|app| app.name.clone())
                    .collect();
                candidates.push(Candidate {
                    item: Self::pool_item(row),
                    keywords,
                });
            }
        }
        for (kind, source) in [
            (SearchType::Project, &*state.projects() as &dyn ItemSource),
            (SearchType::Game, &*state.games() as &dyn ItemSource),
        ] {
            if wanted(kind) {
                candidates.extend(
                    source
                        .search_entries()
                        .into_iter()
                        .map(|(item, keywords)| Candidate { item, keywords }),
                );
            }
        }
        // Sessions, jobs and idle jobs are mapped here from the lists of section 03; a type
        // that is not asked for is not listed at all.
        if wanted(SearchType::Session) {
            candidates.extend(
                state
                    .sessions()
                    .list()
                    .iter()
                    .map(other_items::session_candidate),
            );
        }
        if wanted(SearchType::Job) {
            candidates.extend(state.jobs().list().iter().map(other_items::job_candidate));
        }
        if wanted(SearchType::IdleJob) {
            candidates.extend(
                state
                    .idle()
                    .list()
                    .iter()
                    .map(other_items::idle_job_candidate),
            );
        }
        Ok(search::search(query, candidates))
    }

    /// `GET /pinned`: the pins in file order. A pin whose item is gone is skipped, never deleted.
    pub fn pinned(&self) -> ApiResult<Vec<SearchItem>> {
        let state = self.app()?;
        let pins = self.pins.get();
        Ok(pins
            .in_order()
            .into_iter()
            .filter_map(|(kind, id)| self.item_for(&state, kind, id))
            .collect())
    }

    /// The ids of one type that are pinned. Projects and games read it for their own `pinned`.
    pub fn pinned_ids(&self, kind: PinnableType) -> HashSet<String> {
        let name = pin_kind(kind);
        self.pins
            .get()
            .pin
            .iter()
            .filter(|pin| pin.kind == name)
            .map(|pin| pin.id.clone())
            .collect()
    }

    /// Removes a pin (a game that was hidden or deleted). Nothing happens when it is not pinned.
    pub async fn unpin(&self, kind: PinnableType, id: &str) -> ApiResult<()> {
        let name = pin_kind(kind);
        if !self.pins.get().contains(name, id) {
            return Ok(());
        }
        let id = id.to_owned();
        self.pins
            .edit(move |doc| store::remove_pin(doc, name, &id))
            .await
            .map(|_| ())
            .map_err(saved)
    }

    /// `PUT /pinned/{type}/{id}`.
    pub async fn set_pinned(&self, kind: PinnableType, id: &str, pinned: bool) -> ApiResult<()> {
        if !pinned {
            return self.unpin(kind, id).await;
        }
        let name = pin_kind(kind);
        if self.pins.get().contains(name, id) {
            return Ok(()); // already pinned: it keeps its place and nothing is written
        }
        if !self.is_known(kind, id)? {
            return Err(ApiError::not_found("Item not found")
                .with_source(SOURCE)
                .with_reason("unknownItem"));
        }
        let id = id.to_owned();
        self.pins
            .edit(move |doc| store::add_pin(doc, name, &id))
            .await
            .map(|_| ())
            .map_err(saved)
    }

    fn is_known(&self, kind: PinnableType, id: &str) -> ApiResult<bool> {
        let state = self.app()?;
        Ok(match kind {
            PinnableType::App => self.find_app(id).is_some(),
            PinnableType::Pool => self.pools.get().pool.iter().any(|row| row.id == id),
            PinnableType::Project => state.projects().knows(id),
            PinnableType::Game => state.games().knows(id),
        })
    }

    /// `GET /recent`: apps and pools from the history map, projects and games from their own
    /// history, newest first.
    pub fn recent(&self, limit: Option<usize>) -> ApiResult<Vec<SearchItem>> {
        let limit = limit.unwrap_or(RECENT_DEFAULT);
        if limit == 0 || limit > RECENT_MAX {
            return Err(ApiError::invalid("The limit must be between 1 and 50")
                .with_source(SOURCE)
                .with_reason("badLimit"));
        }
        let state = self.app()?;
        let mut used: Vec<(String, SearchItem)> = lock(&self.history)
            .iter()
            .filter_map(|((kind, id), at)| {
                let name = match kind {
                    LaunchType::App => "app",
                    LaunchType::Pool => "pool",
                    LaunchType::Project | LaunchType::Game => return None,
                };
                self.item_for(&state, name, id)
                    .map(|item| (at.clone(), item))
            })
            .collect();
        used.extend(state.projects().recent());
        used.extend(state.games().recent());
        // Newest first. One pool launch stamps the pool and its apps with one time: the pool
        // comes first, then the rest by title, so the order does not depend on a hash map.
        used.sort_by(|(at_a, a), (at_b, b)| {
            at_b.cmp(at_a)
                .then_with(|| type_rank(a.r#type).cmp(&type_rank(b.r#type)))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.id.cmp(&b.id))
        });
        used.truncate(limit);
        Ok(used.into_iter().map(|(_, item)| item).collect())
    }

    // ---- pools ----

    fn pool_view(&self, row: &PoolRow) -> Pool {
        let history = lock(&self.history);
        Pool {
            id: row.id.clone(),
            name: row.name.clone(),
            icon: IconRef::letter(&row.name),
            members: row
                .members
                .iter()
                .map(|member| PoolMember {
                    app_id: member.app.clone(),
                    args: member.args.clone(),
                    delay_ms: member.delay_ms,
                })
                .collect(),
            pinned: self.pins.get().contains("pool", &row.id),
            last_launched_at: history.get(&(LaunchType::Pool, row.id.clone())).cloned(),
        }
    }

    /// `GET /pools`, in file order.
    pub fn list_pools(&self) -> Vec<Pool> {
        self.pools
            .get()
            .pool
            .iter()
            .map(|row| self.pool_view(row))
            .collect()
    }

    /// `POST /pools` (`update` is `None`) and `PUT /pools/{id}`. Answers the saved pool and
    /// whether it was created.
    pub async fn save_pool(
        &self,
        update: Option<&str>,
        draft: PoolDraft,
    ) -> ApiResult<(Pool, bool)> {
        let mismatch = || {
            ApiError::invalid("The pool id does not match")
                .with_source(SOURCE)
                .with_reason("idMismatch")
        };
        match (update, draft.id.as_deref()) {
            (None, Some(_)) => return Err(mismatch()),
            (Some(path), Some(body)) if path != body => return Err(mismatch()),
            _ => {}
        }
        let checked = check_draft(&draft)?;
        let (name, members) = (checked.name, checked.members);
        let id = match update {
            Some(id) => {
                let (id, name, members) = (id.to_owned(), name.clone(), members);
                let for_edit = id.clone();
                self.pools
                    .edit(move |doc| store::update_pool(doc, &for_edit, &name, &members))
                    .await
                    .map_err(saved)?;
                id
            }
            None => {
                let existing: HashSet<String> = self
                    .pools
                    .get()
                    .pool
                    .iter()
                    .map(|row| row.id.clone())
                    .collect();
                if existing.len() >= POOL_MAX {
                    return Err(ApiError::invalid("Rain can keep at most 100 pools")
                        .with_source(SOURCE)
                        .with_reason("tooManyPools"));
                }
                let id = new_pool_id(&name, &existing);
                let (for_edit, name, members) = (id.clone(), name.clone(), members);
                self.pools
                    .edit(move |doc| store::create_pool(doc, &for_edit, &name, &members))
                    .await
                    .map_err(saved)?;
                id
            }
        };
        let file = self.pools.get();
        let row = file.pool.iter().find(|row| row.id == id).ok_or_else(|| {
            ApiError::failed("Could not save pools.toml").with_reason("saveFailed")
        })?;
        Ok((self.pool_view(row), update.is_none()))
    }

    /// `DELETE /pools/{id}`: the pool, its pin and its history go. An unknown id is fine.
    pub async fn delete_pool(&self, id: &str) -> ApiResult<()> {
        let for_edit = id.to_owned();
        self.pools
            .edit(move |doc| store::delete_pool(doc, &for_edit))
            .await
            .map_err(saved)?;
        if let Err(e) = self.unpin(PinnableType::Pool, id).await {
            // A dangling pin is skipped when listing, so this is only untidy.
            warn!(reason = %e.body.message, "the pin of a deleted pool was not removed");
        }
        if let Ok(state) = self.app() {
            let for_sql = id.to_owned();
            if let Err(e) = state
                .db
                .call(move |db| store::delete_history(db, "pool", &for_sql))
                .await
            {
                warn!(error = %e, "the history of a deleted pool was not removed");
            }
        }
        lock(&self.history).remove(&(LaunchType::Pool, id.to_owned()));
        Ok(())
    }

    // ---- launching ----

    /// `POST /launches`. Answers the run and whether it is new (`false`: the same target is
    /// still running and that run is returned).
    pub async fn launch(self: &Arc<Self>, target: LaunchTarget) -> ApiResult<(LaunchRun, bool)> {
        let state = self.app()?;
        if state.shutdown.shutting_down() {
            return Err(shutting_down_error());
        }
        if state.game_mode_active() {
            return Err(game_mode());
        }
        let target = checked_target(target)?;
        if let Some(run) = lock(&self.runs).running(&target) {
            return Ok((run, false));
        }
        let (steps, planner) = self.plan(&state, &target)?;
        if steps.is_empty() {
            return Err(ApiError::failed("Nothing to launch").with_source(SOURCE));
        }
        let now = state.clock.now();
        let run = new_run(self.next_run_id(now), &target, &steps);
        {
            let mut runs = lock(&self.runs);
            // Another request may have started the same target while this one planned.
            if let Some(existing) = runs.running(&target) {
                return Ok((existing, false));
            }
            runs.insert(run.clone(), now);
        }
        let task = Arc::clone(self);
        let (task_run, guard) = (run.clone(), state.activity.guard());
        let run_id = run.id.clone();
        let recovery_planner = planner.clone();
        self.tracker.spawn(async move {
            let _busy = guard;
            let outcome = AssertUnwindSafe(Arc::clone(&task).run_task(
                state.clone(),
                task_run,
                steps,
                planner,
            ))
            .catch_unwind()
            .await;
            if outcome.is_err() {
                error!("a launch run panicked and ended");
                task.abandon(&state, &run_id, recovery_planner).await;
            }
        });
        Ok((run, true))
    }

    /// A run task that panicked would leave its run `running` for ever, and every later launch of
    /// the same target would get that dead run back. Ends it as failed instead.
    ///
    /// The planner's `finished` hook still runs (a project or game history row is written for a
    /// run that panicked), but not when the panic came from that hook itself.
    async fn abandon(&self, state: &AppState, id: &str, planner: Option<Arc<dyn LaunchPlanner>>) {
        let now = state.clock.now();
        let Some(done) = lock(&self.runs).update(id, |run| {
            run.status = RunStatus::Failed;
            for step in &mut run.steps {
                if matches!(step.status, Status::Running) {
                    step.status = Status::Failed;
                    step.error = Some("Failed to start".to_owned());
                }
            }
        }) else {
            return;
        };
        if let Some(planner) = planner {
            let hook = AssertUnwindSafe(planner.finished(&done))
                .catch_unwind()
                .await;
            if hook.is_err() {
                error!("the planner's finished hook panicked while a run was being ended");
            }
        }
        lock(&self.runs).finish(&done, now);
        self.throttle.publish_final(&done);
    }

    fn next_run_id(&self, now: SystemTime) -> String {
        let millis = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let counter = self.run_counter.fetch_add(1, Ordering::SeqCst);
        format!("launch-{millis:x}-{counter}")
    }

    /// Turns the target into steps. Projects and games plan for themselves through
    /// `LaunchPlanner`; each answers for itself (unknown id, game mode, still loading).
    #[allow(clippy::type_complexity)]
    fn plan(
        &self,
        state: &AppState,
        target: &LaunchTarget,
    ) -> ApiResult<(Vec<EngineStep>, Option<Arc<dyn LaunchPlanner>>)> {
        match target.r#type {
            LaunchType::App => {
                let app = self.find_app(&target.id).ok_or_else(|| {
                    ApiError::not_found("App not found")
                        .with_source(SOURCE)
                        .with_reason("unknownApp")
                })?;
                Ok((
                    vec![EngineStep::for_app(&app, target.args.clone(), 0)],
                    None,
                ))
            }
            LaunchType::Pool => {
                let file = self.pools.get();
                let row = file
                    .pool
                    .iter()
                    .find(|row| row.id == target.id)
                    .ok_or_else(|| {
                        ApiError::not_found("Pool not found")
                            .with_source(SOURCE)
                            .with_reason("unknownPool")
                    })?;
                // One step for each member, in order: the screen reads steps[i] against members[i].
                let steps = row
                    .members
                    .iter()
                    .map(|member| match self.find_app(&member.app) {
                        Some(app) => EngineStep::for_app(
                            &app,
                            member.args.clone(),
                            member.delay_ms.unwrap_or(0),
                        ),
                        None => EngineStep::unknown_app(),
                    })
                    .collect();
                Ok((steps, None))
            }
            LaunchType::Project => self.plan_with(state.projects(), target),
            LaunchType::Game => self.plan_with(state.games(), target),
        }
    }

    #[allow(clippy::type_complexity)]
    fn plan_with<P: LaunchPlanner + 'static>(
        &self,
        planner: Arc<P>,
        target: &LaunchTarget,
    ) -> ApiResult<(Vec<EngineStep>, Option<Arc<dyn LaunchPlanner>>)> {
        let plan = planner.plan(target)?;
        let steps = plan
            .steps
            .into_iter()
            .map(|planned| EngineStep::from_planned(planned, |id| self.find_app(id)))
            .collect();
        Ok((steps, Some(planner as Arc<dyn LaunchPlanner>)))
    }

    /// The task of one run: the steps one after the other, a frame after each, and the end.
    async fn run_task(
        self: Arc<Self>,
        state: AppState,
        run: LaunchRun,
        steps: Vec<EngineStep>,
        planner: Option<Arc<dyn LaunchPlanner>>,
    ) {
        let env = StepEnv {
            opener: state.opener(),
            probe: Arc::clone(&self.probe),
        };
        let cancel = state.shutdown.token().clone();
        let mut used_apps: Vec<String> = Vec::new();
        let count = steps.len();
        for (index, step) in steps.iter().enumerate() {
            let outcome = tokio::select! {
                () = cancel.cancelled() => return, // stopping publishes nothing
                outcome = run_step(&env, step) => outcome,
            };
            let succeeded = outcome.is_ok();
            if succeeded && let Source::App { app, .. } = &step.source {
                used_apps.push(app.id.clone());
                lock(&self.grace).insert(app.id.clone(), state.clock.now());
            }
            let stop = !succeeded && step.stops_run;
            let more = index + 1 < count && !stop;
            let snapshot = lock(&self.runs).update(&run.id, |run| {
                let step = &mut run.steps[index];
                match outcome {
                    Ok(_) => step.status = Status::Success,
                    Err(text) => {
                        step.status = Status::Failed;
                        step.error = Some(text);
                    }
                }
                if more {
                    run.steps[index + 1].status = Status::Running;
                }
            });
            if more && let Some(snapshot) = snapshot {
                self.throttle.publish(&snapshot);
            }
            if stop {
                break;
            }
        }
        self.finish(&state, &run, &used_apps, planner).await;
    }

    /// The end of a run, in the order that keeps what the screen refetches up to date: history
    /// first, the planner's own history next, the final frame last (section 01 2, "Live topics").
    async fn finish(
        &self,
        state: &AppState,
        run: &LaunchRun,
        used_apps: &[String],
        planner: Option<Arc<dyn LaunchPlanner>>,
    ) {
        let now = state.clock.now();
        let Some(mut done) = lock(&self.runs).get(&run.id, now) else {
            return;
        };
        done.status = if done.steps.iter().any(|step| step.status == Status::Failed) {
            RunStatus::Failed
        } else {
            RunStatus::Success
        };
        // 2 and 3: the history rows, then the memory that the next read uses.
        if is_launcher_history(done.target.r#type) {
            self.record_history(state, &done, used_apps, now).await;
        }
        *lock(&self.snapshot) = None;
        // 4: projects and games write their own history.
        if let Some(planner) = planner {
            planner.finished(&done).await;
        }
        // 5 and 6: the run is final now; a waiting throttled frame is dropped and the last one
        // goes out at once.
        lock(&self.runs).finish(&done, state.clock.now());
        self.throttle.publish_final(&done);
    }

    async fn record_history(
        &self,
        state: &AppState,
        run: &LaunchRun,
        used_apps: &[String],
        now: SystemTime,
    ) {
        let mut used: Vec<(&'static str, String)> =
            used_apps.iter().map(|id| ("app", id.clone())).collect();
        if run.target.r#type == LaunchType::Pool && run.status == RunStatus::Success {
            used.push(("pool", run.target.id.clone()));
        }
        if used.is_empty() {
            return;
        }
        let at = iso_from_system_time(now);
        let (rows, at_for_sql, run_id) = (used.clone(), at.clone(), run.id.clone());
        let written = state
            .db
            .call(move |db| {
                let borrowed: Vec<(&str, &str)> =
                    rows.iter().map(|(kind, id)| (*kind, id.as_str())).collect();
                store::insert_history(db, &borrowed, &at_for_sql, &run_id)
            })
            .await;
        if let Err(e) = written {
            // "Recent" will miss this launch after a restart; the screen still shows it now.
            warn!(error = %e, "the launch history was not written");
        }
        let mut history = lock(&self.history);
        for (kind, id) in used {
            let kind = if kind == "pool" {
                LaunchType::Pool
            } else {
                LaunchType::App
            };
            history.insert((kind, id), at.clone());
        }
    }

    /// `GET /launches/{id}`.
    pub fn get_launch(&self, id: &str) -> ApiResult<LaunchRun> {
        lock(&self.runs).get(id, self.now()).ok_or_else(|| {
            ApiError::not_found("Launch not found")
                .with_source(SOURCE)
                .with_reason("unknownRun")
        })
    }
}

// ---- helpers ----

fn game_mode() -> ApiError {
    ApiError::conflict("gameMode", "Game mode is on").with_source(SOURCE)
}

/// The text of a launch target is checked here: arguments belong to apps only, and are short and
/// on one line.
fn checked_target(mut target: LaunchTarget) -> ApiResult<LaunchTarget> {
    target.args = target.args.filter(|args| !args.is_empty());
    if let Some(args) = &target.args {
        if target.r#type != LaunchType::App {
            return Err(ApiError::invalid("Arguments can only be passed to an app")
                .with_source(SOURCE)
                .with_reason("argsNotAllowed"));
        }
        if args.chars().count() > ARGS_MAX || args.contains(['\n', '\r']) {
            return Err(
                ApiError::invalid("The arguments are too long or have a line break")
                    .with_source(SOURCE)
                    .with_reason("badArgs"),
            );
        }
    }
    Ok(target)
}

/// A failed write of a TOML file is told with the reason of this section.
fn saved(error: ApiError) -> ApiError {
    if error.body.reason.as_deref() == Some("writeFailed") {
        return error.with_reason("saveFailed").with_source(SOURCE);
    }
    error
}

pub fn pin_kind(kind: PinnableType) -> &'static str {
    match kind {
        PinnableType::App => "app",
        PinnableType::Pool => "pool",
        PinnableType::Project => "project",
        PinnableType::Game => "game",
    }
}

fn type_rank(kind: SearchType) -> u8 {
    match kind {
        SearchType::Pool => 0,
        SearchType::App => 1,
        SearchType::Project => 2,
        SearchType::Game => 3,
        SearchType::Session => 4,
        SearchType::Job => 5,
        SearchType::IdleJob => 6,
    }
}

/// Reads the cache and the history from SQLite; prunes history older than a year. A database
/// that cannot be read leaves both empty (the module may be out of service; its routes then
/// answer 503 before they reach the service).
async fn load_database(state: &AppState) -> (Vec<Row>, HashMap<(LaunchType, String), String>) {
    let cutoff = iso_from_system_time(
        state
            .clock
            .now()
            .checked_sub(Duration::from_secs(HISTORY_DAYS * 24 * 3600))
            .unwrap_or(UNIX_EPOCH),
    );
    let loaded = state
        .db
        .call(move |db| {
            store::prune_old_history(db, &cutoff)?;
            Ok((store::load_rows(db)?, store::load_history(db)?))
        })
        .await;
    match loaded {
        Ok((rows, history)) => {
            let history = history
                .into_iter()
                .filter_map(|(kind, id, at)| match kind.as_str() {
                    "app" => Some(((LaunchType::App, id), at)),
                    "pool" => Some(((LaunchType::Pool, id), at)),
                    _ => None,
                })
                .collect();
            (rows, history)
        }
        Err(e) => {
            warn!(error = %e, "the launcher tables could not be read");
            (Vec::new(), HashMap::new())
        }
    }
}

/// The modification time of each target that is a file, in seconds since 1970.
fn modified_times(targets: &[String]) -> HashMap<String, i64> {
    targets
        .iter()
        .filter_map(|target| {
            let modified = std::fs::metadata(target).ok()?.modified().ok()?;
            let seconds = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((target.clone(), i64::try_from(seconds).ok()?))
        })
        .collect()
}

/// The two Start Menu folders, from the environment (never a literal path). Only those that exist.
fn start_menu_folders() -> Vec<PathBuf> {
    ["ProgramData", "APPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| {
            PathBuf::from(base)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
        })
        .filter(|folder| folder.is_dir())
        .collect()
}

/// A recursive watch of the folder. The recursive mode on a Start Menu is not proven yet
/// (open point 5 of section 01): when the system refuses it, each sub-folder is watched on its
/// own, which is the mode the foundation proved.
fn watch_folder(state: &AppState, folder: &std::path::Path) -> Vec<DirWatch> {
    match state.hub.watch_dir(folder, true, START_MENU_QUIET) {
        Ok(watch) => vec![watch],
        Err(e) => {
            warn!(error = %e, "a recursive watch of the Start Menu failed; watching its folders one by one");
            let Ok(entries) = std::fs::read_dir(folder) else {
                return Vec::new();
            };
            let mut watches: Vec<DirWatch> = entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| {
                    state
                        .hub
                        .watch_dir(&entry.path(), false, START_MENU_QUIET)
                        .ok()
                })
                .collect();
            watches.extend(state.hub.watch_dir(folder, false, START_MENU_QUIET).ok());
            watches
        }
    }
}
