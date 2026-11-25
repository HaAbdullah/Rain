//! Putting the library together. Section 02, 6.8 and 6.12.
//!
//! Four sources (Steam, Epic, Riot, game folders and hand-added games) each give `Found` games.
//! The user's corrections of `games.toml` are applied on top, the list is sorted, and the result
//! is kept for two seconds so that a burst of requests (Home, Games, game mode) costs one scan.
//! What changes quickly (running, pinned, play time) is added at every answer, in `compose`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rain_types::{
    ConfigIssue, Game, GameLibrary, GameSource, GameSourceStatus, SourceState,
    time::iso_from_system_time,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::warn;

use super::config::{self, Custom, FILE_NAME, Parsed, canonical_key};
use super::covers;
use super::epic;
use super::folders::{self, FolderGame};
use super::guess;
use super::model::{Found, Launch, LocalInfo};
use super::names::{fnv8, norm_path, path_key, slug};
use super::riot;
use super::service::{Service, lock, play_key};
use super::steam;
use super::store::{self, Play, SizeRow};
use crate::core::blocking;
use crate::core::error::ApiResult;
use crate::core::task::spawn_named;
use crate::modules::jobs::entry::entry_lines;

/// How long an assembled library answers without a new scan.
pub const LIBRARY_TTL: Duration = Duration::from_secs(2);
/// How long one list waits for program guesses; the rest finishes in the background.
pub const GUESS_BUDGET: Duration = Duration::from_secs(5);
/// Guesses that run at the same time.
const GUESS_PARALLEL: usize = 4;
const GUESS_VALID_DAYS: u64 = 30;
const SIZE_VALID_DAYS: u64 = 7;
const PRUNE_DAYS: u64 = 90;

/// The assembled library: every game of every source that works, sorted.
pub struct Library {
    pub entries: Vec<Found>,
    /// Always four: steam, epic, riot, custom.
    pub sources: Vec<GameSourceStatus>,
    pub built_at: Instant,
}

impl Library {
    pub fn find(&self, id: &str) -> Option<&Found> {
        self.entries.iter().find(|entry| entry.id == id)
    }
}

/// What the live part of an answer needs.
pub struct Live {
    pub running: Arc<HashSet<String>>,
    pub plays: Arc<HashMap<String, Play>>,
    pub pinned: HashSet<String>,
}

/// What `local_games` makes of the game folders and `games.toml`.
#[derive(Debug, Default)]
pub struct LocalOut {
    pub games: Vec<Found>,
    /// Folders whose size is missing or old: they are measured in the background.
    pub size_wanted: Vec<(PathBuf, String)>,
}

/// The local games: folder games (with their guess and corrections) and hand-added games. Blocking
/// (it looks at the disk for the existence of program files and covers).
///
/// `guesses` maps `path_key(folder)` to the guessed program, relative to the folder.
pub fn local_games(
    folder_games: &[FolderGame],
    guesses: &HashMap<String, Option<String>>,
    parsed: &Parsed,
    data_dir: &Path,
    sizes: &HashMap<String, SizeRow>,
    size_cutoff: &str,
) -> LocalOut {
    let mut out = LocalOut::default();

    // Two folders with the same name (`D:\Games\Foo`, `E:\Games\Foo`) would get one id. The
    // first by path keeps the plain id, the others add a hash of their path.
    let mut by_base: HashMap<String, Vec<&FolderGame>> = HashMap::new();
    for game in folder_games {
        by_base
            .entry(format!("game-dir-{}", slug(&game.name)))
            .or_default()
            .push(game);
    }
    let mut ids: HashMap<String, String> = HashMap::new();
    for (base, mut group) in by_base {
        group.sort_by_key(|game| path_key(&game.path.to_string_lossy()));
        for (index, game) in group.into_iter().enumerate() {
            let key = path_key(&game.path.to_string_lossy());
            let id = if index == 0 {
                base.clone()
            } else {
                format!("{base}-{}", fnv8(&key))
            };
            ids.insert(key, id);
        }
    }

    for game in folder_games {
        let folder_text = game.path.to_string_lossy().into_owned();
        let folder_key = path_key(&folder_text);
        let key = format!("dir:{}", norm_path(&folder_text));
        let correction = canonical_key(&key)
            .and_then(|canonical| parsed.corrections.get(&canonical))
            .cloned()
            .unwrap_or_default();

        let (exe, guessed) = match &correction.exe {
            Some(exe) if config::is_absolute(exe) => (Some(PathBuf::from(norm_path(exe))), false),
            Some(relative) => (Some(game.path.join(relative)), false),
            None => (
                guesses
                    .get(&folder_key)
                    .cloned()
                    .flatten()
                    .map(|rel| game.path.join(rel)),
                true,
            ),
        };
        // A corrected program that is gone makes the game "not installed": the guess is not used
        // behind the user's back. A guess that vanished since it was made is treated the same.
        let installed = exe.as_ref().is_none_or(|exe| exe.is_file());
        let cover = correction
            .cover
            .as_deref()
            .map(|value| covers::cover_path(data_dir, value))
            .or_else(|| covers::find_in_folder(&game.path))
            // No picture of its own: the icon of the program file (drawn when it is first shown).
            .or_else(|| exe.clone());

        let signature = folders::signature(&game.path);
        let known_size = sizes.get(&folder_key);
        if known_size
            .is_none_or(|row| row.signature != signature || row.measured_at.as_str() < size_cutoff)
        {
            out.size_wanted.push((game.path.clone(), signature));
        }

        out.games.push(Found {
            key,
            id: ids.get(&folder_key).cloned().unwrap_or_default(),
            title: correction
                .title
                .clone()
                .unwrap_or_else(|| game.name.clone()),
            source: GameSource::Custom,
            launch: Launch::Local,
            install: Some(game.path.clone()),
            size_bytes: known_size.map(|row| row.size_bytes),
            cover,
            playtime_minutes: 0,
            last_played_unix: None,
            hidden: correction.hidden,
            cover_version: None,
            local: Some(LocalInfo {
                exe_guessed: guessed && exe.is_some(),
                exe,
                args: correction.args.clone(),
                installed,
                removable: false,
            }),
        });
    }

    for custom in &parsed.customs {
        out.games.push(custom_game(custom, data_dir));
    }
    out
}

fn custom_game(custom: &Custom, data_dir: &Path) -> Found {
    let exe = PathBuf::from(norm_path(&custom.exe));
    Found {
        key: format!("custom:{}", custom.id),
        id: custom.id.clone(),
        title: custom.title.clone(),
        source: GameSource::Custom,
        launch: Launch::Local,
        install: None,
        size_bytes: None,
        cover: custom
            .cover
            .as_deref()
            .map(|value| covers::cover_path(data_dir, value))
            .or_else(|| Some(exe.clone())),
        playtime_minutes: 0,
        last_played_unix: None,
        hidden: custom.hidden,
        cover_version: None,
        local: Some(LocalInfo {
            installed: exe.is_file(),
            exe: Some(exe),
            exe_guessed: false,
            args: custom.args.clone(),
            removable: true,
        }),
    }
}

/// Gives the Riot games the size that was measured before and returns the folders that still need
/// measuring (the same background measurement and cache as for game folders).
fn riot_sizes(
    items: &mut [Found],
    sizes: &HashMap<String, SizeRow>,
    size_cutoff: &str,
) -> Vec<(PathBuf, String)> {
    let mut wanted = Vec::new();
    for item in items {
        let Some(install) = item.install.clone() else {
            continue;
        };
        let known = sizes.get(&path_key(&install.to_string_lossy()));
        item.size_bytes = known.map(|row| row.size_bytes);
        let signature = folders::signature(&install);
        if known
            .is_none_or(|row| row.signature != signature || row.measured_at.as_str() < size_cutoff)
        {
            wanted.push((install, signature));
        }
    }
    wanted
}

/// Applies the hide flag of `[[game]]` to Steam, Epic and Riot games, sorts, and keeps only covers
/// that exist (a missing file means "no cover": the tile shows the title). Blocking.
pub fn finish(mut entries: Vec<Found>, parsed: &Parsed) -> Vec<Found> {
    for entry in &mut entries {
        if entry.source != GameSource::Custom
            && let Some(correction) =
                canonical_key(&entry.key).and_then(|key| parsed.corrections.get(&key))
        {
            entry.hidden = correction.hidden;
        }
        entry.cover_version = entry.cover.as_deref().and_then(covers::cover_version);
        if entry.cover_version.is_none() {
            entry.cover = None;
        }
    }
    let rank = |source: GameSource| match source {
        GameSource::Steam => 0,
        GameSource::Epic => 1,
        GameSource::Riot => 2,
        GameSource::Custom => 3,
    };
    entries.sort_by(|a, b| {
        (rank(a.source), a.title.to_lowercase(), &a.id).cmp(&(
            rank(b.source),
            b.title.to_lowercase(),
            &b.id,
        ))
    });
    entries
}

fn iso_days_ago(now: std::time::SystemTime, days: u64) -> String {
    iso_from_system_time(
        now.checked_sub(Duration::from_secs(days * 86_400))
            .unwrap_or(std::time::UNIX_EPOCH),
    )
}

/// The state of the third source: game folders and `games.toml`.
fn custom_status(missing: &[PathBuf], file_broken: bool) -> GameSourceStatus {
    let (state, message) = if let Some(first) = missing.first() {
        let mut text = format!("Game folder not found: {}", first.display());
        if missing.len() > 1 {
            text.push_str(&format!(" and {} more", missing.len() - 1));
        }
        (SourceState::Failed, Some(text))
    } else if file_broken {
        (
            SourceState::Failed,
            Some("games.toml has an error. Showing the last good copy".to_owned()),
        )
    } else {
        (SourceState::Ok, None)
    };
    GameSourceStatus {
        source: GameSource::Custom,
        state,
        message,
    }
}

impl Service {
    pub(super) fn cached_library(&self) -> Option<Arc<Library>> {
        lock(&self.cache).library.clone()
    }

    /// The next `library()` scans again. The old library stays for the callers that cannot
    /// wait (search, pins, planning a launch).
    pub(super) fn invalidate(&self) {
        let mut cache = lock(&self.cache);
        cache.stale = true;
        cache.running = None;
    }

    /// The library. A scan of the last 2 seconds is reused unless `fresh`; in game mode the
    /// cached library is always reused (nothing is rescanned while the PC is being cleared).
    pub(super) async fn library(&self, fresh: bool) -> ApiResult<Arc<Library>> {
        let reusable = |this: &Self| {
            let (library, stale) = {
                let cache = lock(&this.cache);
                (cache.library.clone(), cache.stale)
            };
            library.filter(|library| {
                !fresh
                    && ((!stale && library.built_at.elapsed() < LIBRARY_TTL)
                        || this.app().is_ok_and(|app| app.game_mode_active()))
            })
        };
        if let Some(library) = reusable(self) {
            return Ok(library);
        }
        // One scan at a time; a request that waited takes the result of the one before it.
        let _one_at_a_time = self.build_lock.lock().await;
        if let Some(library) = reusable(self) {
            return Ok(library);
        }
        self.build(true).await
    }

    /// The first load after the start of the backend. It reports nothing to `GET /health`: a
    /// backend that has not been asked anything yet shows an empty health report, as it did
    /// before the games existed. The sources report from the first request on.
    pub(super) async fn preload(&self) -> ApiResult<()> {
        let _one_at_a_time = self.build_lock.lock().await;
        self.build(false).await.map(|_| ())
    }

    /// Names every skipped entry of `games.toml` in `GET /health`, with its line. The file's own
    /// problems (a broken file) are reported by `TomlFile`; this only touches the report when it
    /// has skipped entries, or when it must take back what it reported before.
    async fn report_entry_issues(&self, parsed: &config::Parsed) {
        use std::sync::atomic::Ordering;
        let Ok(app) = self.app() else { return };
        if parsed.skipped_at.is_empty() {
            if self.issues_reported.swap(false, Ordering::SeqCst) {
                app.health.set_file_issues(FILE_NAME, Vec::new());
            }
            return;
        }
        let path = app.hub.file_path(FILE_NAME);
        let text = blocking::run(move || std::fs::read_to_string(path).unwrap_or_default())
            .await
            .unwrap_or_default();
        let (games, customs) = (entry_lines(&text, "game"), entry_lines(&text, "custom"));
        let issues = parsed
            .skipped_at
            .iter()
            .map(|(table, index)| ConfigIssue {
                file: FILE_NAME.to_owned(),
                line: match *table {
                    "game" => games.get(*index),
                    _ => customs.get(*index),
                }
                .copied(),
                message: format!("An entry of [[{table}]] is not valid and was skipped"),
            })
            .collect();
        self.issues_reported.store(true, Ordering::SeqCst);
        app.health.set_file_issues(FILE_NAME, issues);
    }

    async fn build(&self, report: bool) -> ApiResult<Arc<Library>> {
        let app = self.app()?;
        let resolved = app.settings().resolved();
        let parsed = Arc::new(config::parse(&self.file.get()));
        self.report_entry_issues(&parsed).await;
        let data_dir = app.hub.data_dir().to_path_buf();
        let now = app.clock.now();
        let game_mode = app.game_mode_active();

        // Steam and Epic are independent: scan them side by side on the blocking pool.
        let steam_scan = {
            let (resolved, registry) = (Arc::clone(&resolved), Arc::clone(&self.registry));
            blocking::run(move || {
                steam::scan(
                    resolved.steam_enabled,
                    resolved.steam_path.as_deref(),
                    registry.as_ref(),
                )
            })
        };
        let epic_scan = {
            let resolved = Arc::clone(&resolved);
            blocking::run(move || {
                epic::scan(
                    resolved.epic_enabled,
                    resolved.epic_path.as_deref(),
                    resolved.epic_extra_path.as_deref(),
                )
            })
        };
        let riot_scan = {
            let resolved = Arc::clone(&resolved);
            blocking::run(move || riot::scan(resolved.riot_enabled, resolved.riot_path.as_deref()))
        };
        let (steam_result, epic_result, riot_result) =
            tokio::join!(steam_scan, epic_scan, riot_scan);
        let (steam_result, epic_result, riot_result) = (steam_result?, epic_result?, riot_result?);

        // Folders that Steam, Epic and Riot list themselves are not game folders.
        let mut exclude: Vec<PathBuf> = Vec::new();
        if let Some(steam) = &resolved.steam_path {
            exclude.push(steam.join("steamapps"));
        }
        for item in steam_result
            .items
            .iter()
            .chain(&epic_result.items)
            .chain(&riot_result.items)
        {
            match item.launch {
                Launch::Steam { .. } => exclude.extend(
                    item.install
                        .as_ref()
                        .and_then(|install| install.parent()?.parent().map(Path::to_path_buf)),
                ),
                _ => exclude.extend(item.install.clone()),
            }
        }
        let game_folders = resolved.game_folders.clone();
        let discovery = blocking::run(move || folders::discover(&game_folders, &exclude)).await?;

        let guesses = self
            .guesses(&app.db, &discovery.games, &parsed, now, game_mode)
            .await?;
        let sizes = store::sizes_all(&app.db).await?;
        let size_cutoff = iso_days_ago(now, SIZE_VALID_DAYS);

        let (entries, size_wanted) = {
            let (parsed, folder_games) = (Arc::clone(&parsed), discovery.games.clone());
            blocking::run(move || {
                let local = local_games(
                    &folder_games,
                    &guesses,
                    &parsed,
                    &data_dir,
                    &sizes,
                    &size_cutoff,
                );
                let mut all = steam_result.items;
                all.extend(epic_result.items);
                let mut size_wanted = local.size_wanted;
                let mut riot_items = riot_result.items;
                size_wanted.extend(riot_sizes(&mut riot_items, &sizes, &size_cutoff));
                all.extend(riot_items);
                all.extend(local.games);
                (finish(all, &parsed), size_wanted)
            })
            .await?
        };

        let steam_status = GameSourceStatus {
            source: GameSource::Steam,
            state: steam_result.state,
            message: steam_result.message,
        };
        let epic_status = GameSourceStatus {
            source: GameSource::Epic,
            state: epic_result.state,
            message: epic_result.message,
        };
        let riot_status = GameSourceStatus {
            source: GameSource::Riot,
            state: riot_result.state,
            message: riot_result.message,
        };
        let custom = custom_status(&discovery.missing, self.file_is_broken());
        if report {
            for (name, source) in [
                ("steam", &steam_status),
                ("epic", &epic_status),
                ("riot", &riot_status),
                ("gamefolders", &custom),
            ] {
                app.health
                    .set_source(name, source.state, source.message.clone());
            }
        }

        let library = Arc::new(Library {
            entries,
            sources: vec![steam_status, epic_status, riot_status, custom],
            built_at: Instant::now(),
        });
        {
            let mut cache = lock(&self.cache);
            cache.library = Some(Arc::clone(&library));
            cache.stale = false;
        }
        self.reload_plays().await;
        if !game_mode {
            self.enqueue_sizes(size_wanted);
            // Old cache rows go with a library build, at most once a day of real use.
            let _ = store::prune_caches(&app.db, iso_days_ago(now, PRUNE_DAYS)).await;
        }
        Ok(library)
    }

    /// Whether `games.toml` cannot be read now (a syntax error): the last good copy is in use.
    fn file_is_broken(&self) -> bool {
        self.file
            .writable()
            .err()
            .is_some_and(|error| error.body.reason.as_deref() == Some("configBroken"))
    }

    pub(super) async fn reload_plays(&self) {
        let Ok(app) = self.app() else { return };
        match store::load_plays(&app.db).await {
            Ok(plays) => lock(&self.cache).plays = Arc::new(plays),
            Err(error) => warn!(message = %error.body.message, "play history could not be read"),
        }
    }

    // ---- the program guess ----

    /// The guessed program of every folder game that needs one: from the cache when it is still
    /// right, else by walking the folder (4 at once, at most `GUESS_BUDGET` of waiting; a guess
    /// that is not finished by then goes on in the background and publishes when it is done).
    async fn guesses(
        &self,
        db: &crate::store::db::Db,
        games: &[FolderGame],
        parsed: &Parsed,
        now: std::time::SystemTime,
        game_mode: bool,
    ) -> ApiResult<HashMap<String, Option<String>>> {
        let mut result: HashMap<String, Option<String>> = HashMap::new();
        let wanted: Vec<FolderGame> = games
            .iter()
            .filter(|game| {
                let key = canonical_key(&format!("dir:{}", game.path.display()));
                !key.and_then(|key| parsed.corrections.get(&key))
                    .is_some_and(|correction| correction.exe.is_some())
            })
            .cloned()
            .collect();
        let signatures = {
            let wanted = wanted.clone();
            blocking::run(move || {
                wanted
                    .into_iter()
                    .map(|game| {
                        let signature = folders::signature(&game.path);
                        (game, signature)
                    })
                    .collect::<Vec<_>>()
            })
            .await?
        };

        let valid_since = iso_days_ago(now, GUESS_VALID_DAYS);
        let mut needed = Vec::new();
        for (game, signature) in signatures {
            let key = path_key(&game.path.to_string_lossy());
            let cached = store::guess_get(db, key.clone()).await?.filter(|row| {
                row.signature == signature
                    && row.guessed_at.as_str() >= valid_since.as_str()
                    && row
                        .exe_rel
                        .as_ref()
                        .is_none_or(|rel| game.path.join(rel).is_file())
            });
            match cached {
                Some(row) => {
                    result.insert(key, row.exe_rel);
                }
                None if !game_mode => needed.push((game, signature, key)),
                None => {}
            }
        }
        if needed.is_empty() {
            return Ok(result);
        }

        let found: Arc<std::sync::Mutex<HashMap<String, Option<String>>>> = Arc::default();
        let permits = Arc::new(Semaphore::new(GUESS_PARALLEL));
        let stamp = iso_from_system_time(now);
        let mut set = JoinSet::new();
        for (game, signature, key) in needed {
            let (db, permits, found, stamp) = (
                db.clone(),
                Arc::clone(&permits),
                Arc::clone(&found),
                stamp.clone(),
            );
            set.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let folder = game.path.clone();
                let guessed =
                    blocking::run(move || guess::guess(&game.name, &folders::walk(&folder))).await;
                let Ok(guessed) = guessed else { return };
                let (rel, score) = match guessed {
                    Some(guess) => (Some(guess.rel), Some(guess.score)),
                    None => (None, None),
                };
                let _ =
                    store::guess_put(&db, key.clone(), signature, rel.clone(), score, stamp).await;
                super::service::lock(&found).insert(key, rel);
            });
        }
        let finished = tokio::time::timeout(GUESS_BUDGET, async {
            while set.join_next().await.is_some() {}
        })
        .await;
        if finished.is_err()
            && let Ok(app) = self.app()
        {
            // Not everything is guessed yet: let the rest finish, then publish the better list.
            // Dropping a `JoinSet` would cancel its tasks, so it moves into the background task.
            let service = app.games();
            spawn_named("games-guess", async move {
                while set.join_next().await.is_some() {}
                service.invalidate();
                service.publish().await;
            });
        }
        result.extend(super::service::lock(&found).drain());
        Ok(result)
    }

    // ---- the answer ----

    pub(super) fn live(&self, running: Arc<HashSet<String>>) -> Live {
        Live {
            running,
            plays: Arc::clone(&lock(&self.cache).plays),
            pinned: self.pinned_ids(),
        }
    }

    /// The library as the API shows it: the assembled games plus what changes quickly.
    pub(super) async fn compose(&self, library: &Library) -> ApiResult<GameLibrary> {
        let live = self.live(self.running_ids(library).await?);
        Ok(GameLibrary {
            games: library
                .entries
                .iter()
                .map(|entry| game_of(entry, &live))
                .collect(),
            sources: library.sources.clone(),
        })
    }

    /// `GET /games`.
    pub async fn list(&self) -> ApiResult<GameLibrary> {
        let library = self.library(false).await?;
        self.compose(&library).await
    }

    /// `POST /games/refresh`: the caches of guesses and sizes are dropped and every source is
    /// scanned again.
    pub async fn refresh(&self) -> ApiResult<GameLibrary> {
        let app = self.app()?;
        store::clear_caches(&app.db).await?;
        // A program whose icon could not be drawn may be tried again.
        self.program_icons.forget_failures();
        self.invalidate();
        let library = self.library(true).await?;
        let answer = self.compose(&library).await?;
        self.throttle.publish(&answer);
        Ok(answer)
    }

    /// Sends the current library to every connected page (at most once a second).
    ///
    /// Boxed on purpose: a late program guess calls `publish`, and `publish` scans (and so
    /// guesses) again. The box cuts that cycle of future types, which the compiler cannot check
    /// for `Send` otherwise.
    pub fn publish(&self) -> futures_util::future::BoxFuture<'_, ()> {
        Box::pin(self.publish_now())
    }

    async fn publish_now(&self) {
        let Ok(app) = self.app() else { return };
        // Nobody listens: a rescan now would only cost. The next request scans anyway.
        if !app.bus.has_receivers() {
            return;
        }
        match self.library(false).await {
            Ok(library) => match self.compose(&library).await {
                Ok(answer) => self.throttle.publish(&answer),
                Err(error) => {
                    warn!(message = %error.body.message, "the games could not be published")
                }
            },
            Err(error) => warn!(message = %error.body.message, "the games could not be published"),
        }
    }
}

/// One game as the API shows it.
pub fn game_of(entry: &Found, live: &Live) -> Game {
    let play = live.plays.get(&play_key(entry));
    let rain_last = play.and_then(|play| play.last_played_at.clone());
    let rain_minutes = play.map_or(0, |play| {
        (play.play_seconds / 60).min(u64::from(u32::MAX)) as u32
    });
    let launcher_last = entry
        .last_played_unix
        .filter(|seconds| *seconds > 0)
        .map(|seconds| {
            iso_from_system_time(
                std::time::UNIX_EPOCH + Duration::from_secs(seconds.unsigned_abs()),
            )
        });
    let local = entry.local.as_ref();
    Game {
        id: entry.id.clone(),
        title: entry.title.clone(),
        source: entry.source,
        cover_url: entry
            .cover_version
            .as_ref()
            .map(|version| format!("media/game-cover/{}?v={version}", entry.id)),
        cover_is_icon: entry
            .cover
            .as_deref()
            .is_some_and(super::program_icon::is_program)
            .then_some(true),
        installed: local.is_none_or(|local| local.installed),
        running: live.running.contains(&entry.id),
        size_bytes: entry.size_bytes,
        // Steam keeps its own play time. The new Epic launcher keeps one too: it is the starting
        // point, and Rain's own record (it counts nothing for a game that the launcher starts)
        // can only raise it. Everything else is Rain's record of its own launches.
        playtime_minutes: match entry.source {
            GameSource::Steam => entry.playtime_minutes,
            GameSource::Epic => entry.playtime_minutes.max(rain_minutes),
            // Riot keeps no play time that Rain can read, and the client, not Rain, starts
            // the game, so Rain counts nothing either.
            GameSource::Riot | GameSource::Custom => rain_minutes,
        },
        last_played_at: [launcher_last, rain_last].into_iter().flatten().max(),
        pinned: live.pinned.contains(&entry.id),
        hidden: entry.hidden,
        exe_path: local
            .and_then(|local| local.exe.as_ref())
            .map(|exe| exe.to_string_lossy().into_owned()),
        exe_guessed: local.map(|local| local.exe_guessed),
        install_path: entry
            .install
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        removable: local.is_some_and(|local| local.removable).then_some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path, size: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    fn folder_game(root: &Path, name: &str) -> FolderGame {
        FolderGame {
            name: name.to_owned(),
            path: root.join(name),
        }
    }

    fn parsed(text: &str) -> Parsed {
        config::parse(&toml::from_str(text).unwrap())
    }

    #[test]
    fn a_folder_game_uses_the_guess_and_the_title_of_its_folder() {
        let temp = tempfile::tempdir().unwrap();
        touch(
            &temp.path().join("Fixture Quest").join("Game").join("q.exe"),
            5,
        );
        let game = folder_game(temp.path(), "Fixture Quest");
        let guesses = HashMap::from([(
            path_key(&game.path.to_string_lossy()),
            Some("Game\\q.exe".to_owned()),
        )]);
        let out = local_games(
            std::slice::from_ref(&game),
            &guesses,
            &Parsed::default(),
            temp.path(),
            &HashMap::new(),
            "",
        );
        let found = &out.games[0];
        assert_eq!(found.id, "game-dir-fixture-quest");
        assert_eq!(found.title, "Fixture Quest");
        let local = found.local.as_ref().unwrap();
        assert_eq!(
            local.exe.as_deref(),
            Some(game.path.join("Game\\q.exe").as_path())
        );
        assert!(local.exe_guessed && local.installed && !local.removable);
        assert_eq!(out.size_wanted.len(), 1, "no size is known yet");
    }

    #[test]
    fn corrections_replace_title_program_cover_and_hidden() {
        let temp = tempfile::tempdir().unwrap();
        touch(&temp.path().join("G").join("old.exe"), 5);
        touch(&temp.path().join("G").join("bin").join("new.exe"), 5);
        touch(&temp.path().join("G").join("cover.png"), 5);
        let game = folder_game(temp.path(), "G");
        let key = game.path.to_string_lossy().replace('\\', "\\\\");
        let corrections = parsed(&format!(
            "[[game]]\nkey = \"dir:{key}\"\nhidden = true\ntitle = \"Nicer\"\nexe = \"bin\\\\new.exe\"\ncover = \"D:\\\\mine\\\\c.png\"\n"
        ));
        let guesses = HashMap::from([(
            path_key(&game.path.to_string_lossy()),
            Some("old.exe".to_owned()),
        )]);
        let found = &local_games(
            std::slice::from_ref(&game),
            &guesses,
            &corrections,
            temp.path(),
            &HashMap::new(),
            "",
        )
        .games[0];
        assert!(found.hidden);
        assert_eq!(found.title, "Nicer");
        let local = found.local.as_ref().unwrap();
        assert!(!local.exe_guessed);
        assert!(local.exe.as_ref().unwrap().ends_with("bin\\new.exe"));
        assert_eq!(
            found.cover.as_deref(),
            Some(Path::new("D:\\mine\\c.png")),
            "the user's cover wins"
        );
    }

    #[test]
    fn a_corrected_program_that_is_gone_is_not_installed_and_the_guess_is_not_used() {
        let temp = tempfile::tempdir().unwrap();
        touch(&temp.path().join("G").join("old.exe"), 5);
        let game = folder_game(temp.path(), "G");
        let key = game.path.to_string_lossy().replace('\\', "\\\\");
        let corrections = parsed(&format!(
            "[[game]]\nkey = \"dir:{key}\"\nexe = \"missing.exe\"\n"
        ));
        let guesses = HashMap::from([(
            path_key(&game.path.to_string_lossy()),
            Some("old.exe".to_owned()),
        )]);
        let found = &local_games(
            &[game],
            &guesses,
            &corrections,
            temp.path(),
            &HashMap::new(),
            "",
        )
        .games[0];
        assert!(!found.local.as_ref().unwrap().installed);
        assert!(
            found
                .local
                .as_ref()
                .unwrap()
                .exe
                .as_ref()
                .unwrap()
                .ends_with("missing.exe")
        );
    }

    #[test]
    fn a_folder_with_no_program_is_installed_and_has_no_program_file() {
        let temp = tempfile::tempdir().unwrap();
        touch(&temp.path().join("Old Game").join("data.bin"), 5);
        let game = folder_game(temp.path(), "Old Game");
        let found = &local_games(
            &[game],
            &HashMap::new(),
            &Parsed::default(),
            temp.path(),
            &HashMap::new(),
            "",
        )
        .games[0];
        let local = found.local.as_ref().unwrap();
        assert!(local.installed && local.exe.is_none() && !local.exe_guessed);
    }

    #[test]
    fn two_folders_with_one_name_keep_one_plain_id() {
        let temp = tempfile::tempdir().unwrap();
        let a = FolderGame {
            name: "Foo".into(),
            path: temp.path().join("a").join("Foo"),
        };
        let b = FolderGame {
            name: "Foo".into(),
            path: temp.path().join("b").join("Foo"),
        };
        let out = local_games(
            &[b.clone(), a.clone()],
            &HashMap::new(),
            &Parsed::default(),
            temp.path(),
            &HashMap::new(),
            "",
        );
        let id_of = |game: &FolderGame| {
            out.games
                .iter()
                .find(|found| found.install.as_deref() == Some(game.path.as_path()))
                .unwrap()
                .id
                .clone()
        };
        assert_eq!(id_of(&a), "game-dir-foo");
        assert_eq!(
            id_of(&b),
            format!(
                "game-dir-foo-{}",
                fnv8(&path_key(&b.path.to_string_lossy()))
            )
        );
    }

    #[test]
    fn a_known_size_is_used_and_an_old_one_is_measured_again() {
        let temp = tempfile::tempdir().unwrap();
        touch(&temp.path().join("G").join("a.exe"), 5);
        let game = folder_game(temp.path(), "G");
        let key = path_key(&game.path.to_string_lossy());
        let sig = folders::signature(&game.path);
        let fresh = HashMap::from([(
            key.clone(),
            SizeRow {
                size_bytes: 77,
                measured_at: "2026-11-20T00:00:00.000Z".into(),
                signature: sig.clone(),
            },
        )]);
        let out = local_games(
            std::slice::from_ref(&game),
            &HashMap::new(),
            &Parsed::default(),
            temp.path(),
            &fresh,
            "2026-11-15T00:00:00.000Z",
        );
        assert_eq!(out.games[0].size_bytes, Some(77));
        assert!(out.size_wanted.is_empty());
        let old = local_games(
            std::slice::from_ref(&game),
            &HashMap::new(),
            &Parsed::default(),
            temp.path(),
            &fresh,
            "2026-11-21T00:00:00.000Z",
        );
        assert_eq!(old.size_wanted.len(), 1);
        assert_eq!(
            old.games[0].size_bytes,
            Some(77),
            "the old size shows until the new one arrives"
        );
    }

    #[test]
    fn a_local_game_without_a_cover_shows_the_icon_of_its_program_and_a_cover_wins() {
        let temp = tempfile::tempdir().unwrap();
        touch(&temp.path().join("Plain").join("p.exe"), 5);
        touch(&temp.path().join("Plain").join("other.exe"), 7);
        touch(&temp.path().join("Covered").join("c.exe"), 5);
        touch(&temp.path().join("Covered").join("cover.png"), 5);
        let (plain, covered) = (
            folder_game(temp.path(), "Plain"),
            folder_game(temp.path(), "Covered"),
        );
        let guesses = HashMap::from([
            (
                path_key(&plain.path.to_string_lossy()),
                Some("p.exe".to_owned()),
            ),
            (
                path_key(&covered.path.to_string_lossy()),
                Some("c.exe".to_owned()),
            ),
        ]);
        let list = |parsed: &Parsed| {
            finish(
                local_games(
                    &[plain.clone(), covered.clone()],
                    &guesses,
                    parsed,
                    temp.path(),
                    &HashMap::new(),
                    "",
                )
                .games,
                parsed,
            )
        };
        let live = || Live {
            running: Arc::default(),
            plays: Arc::default(),
            pinned: HashSet::new(),
        };
        let found = list(&Parsed::default());
        let (covered_game, plain_game) = (&found[0], &found[1]);
        assert!(covered_game.cover.as_ref().unwrap().ends_with("cover.png"));
        assert!(game_of(covered_game, &live()).cover_is_icon.is_none());
        assert!(plain_game.cover.as_ref().unwrap().ends_with("p.exe"));
        let api = game_of(plain_game, &live());
        assert_eq!(api.cover_is_icon, Some(true));
        assert!(
            api.cover_url
                .unwrap()
                .starts_with("media/game-cover/game-dir-plain?v=")
        );
        let before = plain_game.cover_version.clone();

        // The user corrects the program: the icon, and with it the address, follows.
        let key = plain.path.to_string_lossy().replace('\\', "\\\\");
        let corrected = parsed(&format!(
            "[[game]]\nkey = \"dir:{key}\"\nexe = \"other.exe\"\n"
        ));
        let found = list(&corrected);
        let now = found
            .iter()
            .find(|entry| entry.id == "game-dir-plain")
            .unwrap();
        assert!(now.cover.as_ref().unwrap().ends_with("other.exe"));
        assert_ne!(now.cover_version, before);

        // A program that is gone leaves no picture: the tile shows the title.
        fs::remove_file(plain.path.join("p.exe")).unwrap();
        let found = list(&Parsed::default());
        let gone = found
            .iter()
            .find(|entry| entry.id == "game-dir-plain")
            .unwrap();
        assert!(gone.cover.is_none() && gone.cover_version.is_none());
    }

    #[test]
    fn a_hand_added_game_shows_the_icon_of_its_program_unless_it_has_a_cover() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("x").join("x.exe");
        touch(&exe, 5);
        let text = format!(
            "[[custom]]\nid = \"game-x\"\ntitle = \"X\"\nexe = \"{}\"\n",
            exe.to_string_lossy().replace('\\', "\\\\")
        );
        let parsed = parsed(&text);
        let out = local_games(
            &[],
            &HashMap::new(),
            &parsed,
            temp.path(),
            &HashMap::new(),
            "",
        );
        assert_eq!(out.games[0].cover.as_deref(), Some(exe.as_path()));
    }

    #[test]
    fn hand_added_games_have_no_folder_and_can_be_removed() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("x").join("x.exe");
        touch(&exe, 5);
        let text = format!(
            "[[custom]]\nid = \"game-x\"\ntitle = \"X\"\nexe = \"{}\"\n",
            exe.to_string_lossy().replace('\\', "\\\\")
        );
        let out = local_games(
            &[],
            &HashMap::new(),
            &parsed(&text),
            temp.path(),
            &HashMap::new(),
            "",
        );
        let found = &out.games[0];
        assert_eq!(
            (found.key.as_str(), found.install.as_ref()),
            ("custom:game-x", None)
        );
        let local = found.local.as_ref().unwrap();
        assert!(local.removable && local.installed);
        fs::remove_file(&exe).unwrap();
        let gone = local_games(
            &[],
            &HashMap::new(),
            &parsed(&text),
            temp.path(),
            &HashMap::new(),
            "",
        );
        assert!(!gone.games[0].local.as_ref().unwrap().installed);
    }

    fn found(id: &str, title: &str, source: GameSource) -> Found {
        Found {
            key: format!("k:{id}"),
            id: id.into(),
            title: title.into(),
            source,
            launch: Launch::Local,
            install: None,
            size_bytes: None,
            cover: None,
            playtime_minutes: 0,
            last_played_unix: None,
            hidden: false,
            cover_version: None,
            local: None,
        }
    }

    #[test]
    fn the_list_is_sorted_by_source_then_title_then_id_and_steam_games_can_be_hidden() {
        let parsed = parsed("[[game]]\nkey = \"steam:2\"\nhidden = true\n");
        let mut steam_two = found("game-steam-2", "beta", GameSource::Steam);
        steam_two.key = "steam:2".into();
        let list = finish(
            vec![
                found("c", "Zed", GameSource::Custom),
                steam_two,
                found("a", "alpha", GameSource::Steam),
                found("e", "Epic One", GameSource::Epic),
                found("b", "Able", GameSource::Custom),
            ],
            &parsed,
        );
        let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["a", "game-steam-2", "e", "b", "c"]);
        assert!(list[1].hidden && !list[0].hidden);
    }

    #[test]
    fn a_cover_that_does_not_exist_is_no_cover() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("c.png");
        fs::write(&real, [0x89, b'P', b'N', b'G']).unwrap();
        let mut with = found("a", "A", GameSource::Steam);
        with.cover = Some(real);
        let mut without = found("b", "B", GameSource::Steam);
        without.cover = Some(temp.path().join("gone.png"));
        let list = finish(vec![with, without], &Parsed::default());
        assert!(list[0].cover_version.is_some());
        assert!(list[1].cover.is_none() && list[1].cover_version.is_none());
    }

    #[test]
    fn the_custom_source_reports_missing_folders_and_a_broken_file() {
        assert_eq!(custom_status(&[], false).state, SourceState::Ok);
        let one = custom_status(&[PathBuf::from("D:\\Games")], false);
        assert_eq!(
            one.message.as_deref(),
            Some("Game folder not found: D:\\Games")
        );
        let many = custom_status(
            &[
                PathBuf::from("D:\\A"),
                PathBuf::from("D:\\B"),
                PathBuf::from("D:\\C"),
            ],
            false,
        );
        assert_eq!(
            many.message.as_deref(),
            Some("Game folder not found: D:\\A and 2 more")
        );
        assert!(
            custom_status(&[], true)
                .message
                .unwrap()
                .contains("games.toml")
        );
    }

    #[test]
    fn a_game_shows_the_larger_last_played_of_steam_and_rain() {
        let mut entry = found("game-steam-1", "A", GameSource::Steam);
        entry.key = "steam:1".into();
        entry.last_played_unix = Some(1_700_000_000);
        entry.playtime_minutes = 125;
        let mut plays = HashMap::new();
        plays.insert(
            "steam:1".to_owned(),
            Play {
                last_played_at: Some("2030-01-01T00:00:00.000Z".into()),
                play_seconds: 600,
                launches: 2,
            },
        );
        let live = Live {
            running: Arc::new(HashSet::new()),
            plays: Arc::new(plays),
            pinned: HashSet::from(["game-steam-1".to_owned()]),
        };
        let game = game_of(&entry, &live);
        assert_eq!(
            game.last_played_at.as_deref(),
            Some("2030-01-01T00:00:00.000Z")
        );
        assert_eq!(game.playtime_minutes, 125, "Steam keeps its own play time");
        assert!(game.pinned && game.installed && game.removable.is_none());
    }
}
