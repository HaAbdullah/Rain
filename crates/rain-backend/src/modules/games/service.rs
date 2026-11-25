//! The logic and the in-memory state of this module. Other modules call it, and only it.
//!
//! The work is split over a few files that each add methods to `Service`:
//! `library.rs` (assembling and answering), `running.rs` (running games and play time),
//! `launch.rs` (starting a game, and the launch contract), `edit.rs` (changes the user makes).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rain_types::{Game, Games, IconKind, IconRef, PinnableType, SearchItem, SearchType};
use tracing::warn;

use super::config::{GamesDoc, canonical_key};
use super::folders;
use super::library::{Library, game_of};
use super::model::Found;
use super::running::ProcessWaiter;
use super::steam::SteamRegistry;
use super::store::{self, Play};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::launch_contract::ItemSource;
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::core::throttle::Throttle;
use crate::platform::process::ProcessLister;
use crate::store::toml_file::TomlFile;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The key under which Rain's own record (`games_play`) knows a game: the game key, with the
/// path of a folder game written the way paths are compared.
pub fn play_key(found: &Found) -> String {
    canonical_key(&found.key).unwrap_or_else(|| found.key.clone())
}

/// What is kept in memory between requests.
#[derive(Default)]
pub(super) struct Cache {
    /// The last assembled library. It stays in place when it is stale, so that the synchronous
    /// parts of the launch contract (search, pins, planning) can still answer from it.
    pub library: Option<Arc<Library>>,
    /// Something changed: the next list scans again.
    pub stale: bool,
    pub plays: Arc<HashMap<String, Play>>,
    pub running: Option<(tokio::time::Instant, Arc<HashSet<String>>)>,
}

/// Folders whose size is to be measured, one at a time.
#[derive(Default)]
pub(super) struct SizeQueue {
    pub pending: VecDeque<(PathBuf, String)>,
    pub running: bool,
}

pub struct Service {
    pub(super) state: WeakAppState,
    pub(super) file: TomlFile<GamesDoc>,
    pub(super) registry: Arc<dyn SteamRegistry>,
    pub(super) lister: Arc<dyn ProcessLister>,
    pub(super) waiter: Arc<dyn ProcessWaiter>,
    pub(super) throttle: Throttle<Games>,
    pub(super) cache: Mutex<Cache>,
    /// Held while the library is assembled, so that one scan serves every waiting request.
    pub(super) build_lock: tokio::sync::Mutex<()>,
    pub(super) sizes: Mutex<SizeQueue>,
    /// Raised at the stop of the backend: a size measurement ends at the next folder.
    pub(super) cancel: Arc<AtomicBool>,
    /// The games that Rain started and watches: game key to the row of `games_session`.
    pub(super) sessions: Mutex<HashMap<String, i64>>,
    /// Games whose last plan found them already watched (the step does nothing). Their run must
    /// not count as one more launch: `finished` takes the key out and returns early.
    pub(super) already_running: Mutex<HashSet<String>>,
    /// `GET /health` holds issues of `games.toml` that this module reported (skipped entries).
    pub(super) issues_reported: AtomicBool,
}

/// What `launch_after_game_mode` tells game mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchedAfterGameMode {
    /// True when the backend itself runs inside a job, so the game may close with Rain.
    pub in_job: bool,
}

impl Service {
    pub fn new(state: &AppState, file: TomlFile<GamesDoc>) -> Self {
        // A test replaces each of these with a fake, so that it never reads the real PC.
        Self {
            state: state.downgrade(),
            file,
            registry: state
                .seam::<dyn SteamRegistry>()
                .unwrap_or_else(|| Arc::new(super::steam::SystemSteamRegistry)),
            lister: state
                .seam::<dyn ProcessLister>()
                .unwrap_or_else(|| Arc::new(crate::platform::process::SystemProcessLister)),
            waiter: state
                .seam::<dyn ProcessWaiter>()
                .unwrap_or_else(|| Arc::new(super::running::SystemWaiter)),
            throttle: Throttle::new(state.bus.clone(), 1),
            cache: Mutex::default(),
            build_lock: tokio::sync::Mutex::new(()),
            sizes: Mutex::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            sessions: Mutex::default(),
            already_running: Mutex::default(),
            issues_reported: AtomicBool::new(false),
        }
    }

    /// The application state, or an error once the backend is ending.
    pub(super) fn app(&self) -> ApiResult<AppState> {
        self.state.upgrade().ok_or_else(|| {
            ApiError::unavailable("games", "Rain is stopping").with_reason("shuttingDown")
        })
    }

    // ---- start and stop ----

    /// Starts the background work: takes up the games that run, loads the library once (the
    /// search and the pins answer from it) and follows the settings and `games.toml`.
    pub async fn start(&self, state: AppState) {
        let service = state.games();
        service.recover_sessions().await;
        let loader = Arc::clone(&service);
        spawn_named("games-load", async move {
            if let Err(error) = loader.preload().await {
                warn!(message = %error.body.message, "the game library could not be loaded");
            }
        });
        spawn_named(
            "games-changes",
            Arc::clone(&service).publish_changes(state.shutdown.token().clone()),
        );
    }

    /// Ends size measuring and writes "alive now" into every open session, so that the next
    /// start can count the time of a game that runs on.
    pub async fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        lock(&self.sizes).pending.clear();
        if let Ok(app) = self.app() {
            let now = rain_types::time::iso_from_system_time(app.clock.now());
            if let Err(error) = store::touch_sessions(&app.db, now).await {
                warn!(message = %error.body.message, "open games could not be marked");
            }
        }
    }

    /// Drops the library and publishes the new one whenever the settings or `games.toml` change.
    /// It waits for an event and never wakes by itself.
    async fn publish_changes(self: Arc<Self>, stop: tokio_util::sync::CancellationToken) {
        let Ok(app) = self.app() else { return };
        let mut settings = app.settings().watch();
        let mut file = self.file.subscribe();
        drop(app);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                changed = settings.changed() => if changed.is_err() { return },
                changed = file.changed() => if changed.is_err() { return },
            }
            self.invalidate();
            self.publish().await;
        }
    }

    // ---- sizes ----

    /// Queues folders for measuring. Nothing starts in game mode.
    pub(super) fn enqueue_sizes(&self, wanted: Vec<(PathBuf, String)>) {
        if wanted.is_empty() || self.cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(app) = self.app() else { return };
        if app.game_mode_active() {
            return;
        }
        let start = {
            let mut queue = lock(&self.sizes);
            for item in wanted {
                if !queue.pending.contains(&item) {
                    queue.pending.push_back(item);
                }
            }
            !std::mem::replace(&mut queue.running, true)
        };
        if start {
            let service = app.games();
            spawn_named("games-size", async move { service.measure_queue().await });
        }
    }

    async fn measure_queue(&self) {
        loop {
            let next = {
                let mut queue = lock(&self.sizes);
                let next = queue.pending.pop_front();
                if next.is_none() {
                    queue.running = false;
                }
                next
            };
            let Some((folder, signature)) = next else {
                return;
            };
            let Ok(app) = self.app() else { return };
            if app.game_mode_active() || self.cancel.load(Ordering::Relaxed) {
                let mut queue = lock(&self.sizes);
                queue.pending.clear();
                queue.running = false;
                return;
            }
            let _busy = app.activity.guard();
            let cancel = Arc::clone(&self.cancel);
            let measured = {
                let folder = folder.clone();
                blocking::run(move || folders::measure(&folder, &cancel)).await
            };
            let Ok(Some(size)) = measured else { continue };
            let stored = store::size_put(
                &app.db,
                super::names::path_key(&folder.to_string_lossy()),
                size,
                signature,
                rain_types::time::iso_from_system_time(app.clock.now()),
            )
            .await;
            if stored.is_ok() {
                self.invalidate();
                self.publish().await;
            }
        }
    }

    // ---- pins ----
    // The pins belong to the launcher module (section 01); the game library only asks it.

    pub(super) fn pinned_ids(&self) -> HashSet<String> {
        self.state
            .upgrade()
            .map(|app| app.launcher().pinned_ids(PinnableType::Game))
            .unwrap_or_default()
    }

    /// Removes the pin of a game that was hidden or deleted. A failed write is logged and does
    /// not undo the hide: the pin is skipped by `GET /pinned` while the game is hidden anyway.
    pub(super) async fn unpin(&self, id: &str) {
        let Some(app) = self.state.upgrade() else {
            return;
        };
        if let Err(error) = app.launcher().unpin(PinnableType::Game, id).await {
            tracing::warn!("could not remove the pin of a game: {}", error.body.message);
        }
    }

    // ---- what other modules ask ----

    /// One game, from the last assembled library (game mode shows the next game's cover and
    /// title). It never scans.
    pub fn get(&self, id: &str) -> Option<Game> {
        let library = self.cached_library()?;
        let entry = library.find(id)?;
        let running = lock(&self.cache)
            .running
            .as_ref()
            .map(|(_, ids)| Arc::clone(ids))
            .unwrap_or_default();
        Some(game_of(entry, &self.live(running)))
    }

    /// Whether any game runs now. For the idle jobs, which must not start while a game runs. It
    /// reads the process list at call time (cached for 2 seconds), never a stale `running`.
    pub async fn any_game_running(&self) -> bool {
        // The last assembled library is enough: "running" looks at the process list, and a disk
        // scan at every sample of an idle job would cost for nothing. Only when no library
        // exists yet is one built.
        let library = match self.cached_library() {
            Some(library) => library,
            None => match self.library(false).await {
                Ok(library) => library,
                Err(_) => return false,
            },
        };
        self.running_ids(&library)
            .await
            .is_ok_and(|ids| !ids.is_empty())
    }

    /// The picture file of a game, for the media route.
    pub async fn cover_file(&self, id: &str) -> Option<PathBuf> {
        let library = self.library(false).await.ok()?;
        library.find(id)?.cover.clone()
    }
}

/// The text under a game in search results.
fn source_label(found: &Found) -> &'static str {
    match found.source {
        rain_types::GameSource::Steam => "Steam",
        rain_types::GameSource::Epic => "Epic",
        rain_types::GameSource::Riot => "Riot",
        rain_types::GameSource::Custom => "Local",
    }
}

/// The icon of a game: its cover, or the gamepad of the title tile.
pub fn icon_of(found: &Found) -> IconRef {
    match &found.cover_version {
        Some(version) => IconRef {
            kind: IconKind::Url,
            value: format!("media/game-cover/{}?v={version}", found.id),
            tint: None,
            fallback: None,
        },
        None => IconRef {
            kind: IconKind::Lucide,
            value: "gamepad-2".to_owned(),
            tint: None,
            fallback: None,
        },
    }
}

fn search_item(found: &Found) -> SearchItem {
    SearchItem {
        r#type: SearchType::Game,
        id: found.id.clone(),
        title: found.title.clone(),
        subtitle: Some(source_label(found).to_owned()),
        icon: icon_of(found),
        status: None,
        score: 0,
    }
}

impl ItemSource for Service {
    fn search_entries(&self) -> Vec<(SearchItem, Vec<String>)> {
        let Some(library) = self.cached_library() else {
            return Vec::new();
        };
        library
            .entries
            .iter()
            .filter(|found| !found.hidden)
            .map(|found| (search_item(found), vec![source_label(found).to_owned()]))
            .collect()
    }

    fn item_for(&self, id: &str) -> Option<SearchItem> {
        let library = self.cached_library()?;
        library
            .find(id)
            .filter(|found| !found.hidden)
            .map(search_item)
    }

    fn recent(&self) -> Vec<(String, SearchItem)> {
        let Some(library) = self.cached_library() else {
            return Vec::new();
        };
        let live = self.live(Arc::default());
        let mut recent: Vec<(String, SearchItem)> = library
            .entries
            .iter()
            .filter(|found| !found.hidden)
            .filter_map(|found| {
                let played = game_of(found, &live).last_played_at?;
                Some((played, search_item(found)))
            })
            .collect();
        recent.sort_by(|a, b| b.0.cmp(&a.0));
        recent
    }
}
