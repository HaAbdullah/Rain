//! Which games run, and how play time is counted. Section 02, 6.13.
//!
//! - `running` is computed when a list is asked for (one process snapshot and one registry read,
//!   kept for 2 seconds). Nothing polls.
//! - Play time is counted for local games that Rain started itself: the program's exit is
//!   awaited through `ProcessWaiter` (the operating system signals it; no thread, no timer).
//!   The time between the start and the exit is added to `games_play`.
//! - A backend that stops while a game runs leaves the game running. At the next start every
//!   open session is attached again, or, when its process is gone, counted up to the last time
//!   the backend saw it alive.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use tokio::time::Instant;
use tracing::warn;

use super::library::Library;
use super::names::path_key;
use super::service::{Service, lock, play_key};
use super::store;
use crate::core::blocking;
use crate::core::error::ApiResult;
use crate::core::task::spawn_named;
use crate::modules::jobs::parse_iso;
use crate::platform::process::ExitWatcher;

/// How long a process snapshot answers `running` questions.
pub const RUNNING_TTL: Duration = Duration::from_secs(2);

/// Waits for a process to end. A trait so that a test says when a "game" ends.
pub trait ProcessWaiter: Send + Sync {
    /// A future that completes when the process `pid`, created at `creation_time` (a Windows
    /// FILETIME), has ended. `None` when no such process runs (it ended, it is protected, or
    /// the pid belongs to another process now).
    fn wait_for(&self, pid: u32, creation_time: u64) -> Option<BoxFuture<'static, ()>>;
}

/// The real thing: `RegisterWaitForSingleObject` through `ExitWatcher`.
pub struct SystemWaiter;

impl ProcessWaiter for SystemWaiter {
    fn wait_for(&self, pid: u32, creation_time: u64) -> Option<BoxFuture<'static, ()>> {
        let mut watcher = ExitWatcher::open_for_wait(pid, creation_time)?;
        Some(Box::pin(async move { watcher.exited().await }))
    }
}

impl Service {
    /// The ids of the games that run now. Cached for `RUNNING_TTL`.
    pub(super) async fn running_ids(&self, library: &Library) -> ApiResult<Arc<HashSet<String>>> {
        if let Some((at, ids)) = &lock(&self.cache).running
            && at.elapsed() < RUNNING_TTL
        {
            return Ok(Arc::clone(ids));
        }
        let (lister, registry) = (Arc::clone(&self.lister), Arc::clone(&self.registry));
        let (images, steam_running) =
            blocking::run(move || (lister.images(), registry.running_app_id())).await?;
        let images: Vec<String> = images.iter().map(|image| path_key(image)).collect();
        let watched: HashSet<String> = lock(&self.sessions).keys().cloned().collect();

        let mut ids = HashSet::new();
        for entry in &library.entries {
            let by_session = watched.contains(&play_key(entry));
            let by_steam = matches!(&entry.launch, super::model::Launch::Steam { appid }
                if steam_running.is_some_and(|running| running != 0 && running.to_string() == *appid));
            let by_image = game_folder(entry).is_some_and(|folder| {
                let prefix = format!("{}\\", path_key(&folder.to_string_lossy()));
                images
                    .iter()
                    .any(|image| image.starts_with(&prefix) && !is_riot_client(image))
            });
            if by_session || by_steam || by_image {
                ids.insert(entry.id.clone());
            }
        }
        let ids = Arc::new(ids);
        lock(&self.cache).running = Some((Instant::now(), Arc::clone(&ids)));
        Ok(ids)
    }

    /// The program of a game that Rain started has a process: count its time.
    pub(super) async fn begin_session(
        self: Arc<Self>,
        key: String,
        pid: u32,
        creation_time: u64,
        folder: Option<String>,
    ) {
        let Ok(app) = self.app() else { return };
        let started = app.clock.now();
        let row = match store::insert_session(
            &app.db,
            key.clone(),
            pid,
            creation_time,
            folder,
            rain_types::time::iso_from_system_time(started),
        )
        .await
        {
            Ok(row) => row,
            Err(error) => {
                warn!(message = %error.body.message, "a running game could not be recorded");
                return;
            }
        };
        lock(&self.sessions).insert(key.clone(), row);
        lock(&self.cache).running = None;
        match self.waiter.wait_for(pid, creation_time) {
            Some(wait) => self.spawn_watch(key, row, wait, started),
            // The process is already gone or cannot be watched (it runs with more rights than
            // Rain): only the launch counts.
            None => self.end_session(&key, row, None).await,
        }
    }

    fn spawn_watch(
        self: Arc<Self>,
        key: String,
        row: i64,
        wait: BoxFuture<'static, ()>,
        started: SystemTime,
    ) {
        spawn_named("games-session", async move {
            let Ok(app) = self.app() else { return };
            tokio::select! {
                () = wait => {}
                // The game goes on without the backend; the next start picks the session up.
                () = app.shutdown.token().cancelled() => return,
            }
            let seconds = app
                .clock
                .now()
                .duration_since(started)
                .map_or(0, |elapsed| elapsed.as_secs());
            self.end_session(&key, row, Some(seconds)).await;
        });
    }

    /// A session is over: add its time (when known), forget it, tell the screens.
    async fn end_session(&self, key: &str, row: i64, seconds: Option<u64>) {
        let Ok(app) = self.app() else { return };
        if let Some(seconds) = seconds
            && let Err(error) = store::add_play_seconds(&app.db, key.to_owned(), seconds).await
        {
            warn!(message = %error.body.message, "play time could not be recorded");
        }
        if let Err(error) = store::delete_session(&app.db, row).await {
            warn!(message = %error.body.message, "a finished game could not be forgotten");
        }
        lock(&self.sessions).remove(key);
        lock(&self.cache).running = None;
        self.reload_plays().await;
        self.publish().await;
    }

    /// At the start of the backend: every open session is attached again or counted.
    pub(super) async fn recover_sessions(self: &Arc<Self>) {
        let Ok(app) = self.app() else { return };
        let rows = match store::open_sessions(&app.db).await {
            Ok(rows) => rows,
            Err(error) => {
                warn!(message = %error.body.message, "running games could not be recovered");
                return;
            }
        };
        for row in rows {
            let started = parse_iso(&row.started_at);
            match self.waiter.wait_for(row.pid, row.process_start) {
                Some(wait) => {
                    lock(&self.sessions).insert(row.game_key.clone(), row.id);
                    Arc::clone(self).spawn_watch(
                        row.game_key,
                        row.id,
                        wait,
                        started.unwrap_or_else(|| app.clock.now()),
                    );
                }
                None => {
                    // The game ended while the backend was away: count up to the last time the
                    // backend saw it alive. After a crash that is the launch time, so only the
                    // launch counts (a documented limit).
                    let seen = parse_iso(&row.last_seen_at);
                    let seconds = match (started, seen) {
                        (Some(started), Some(seen)) => seen
                            .duration_since(started)
                            .map_or(0, |elapsed| elapsed.as_secs()),
                        _ => 0,
                    };
                    let _ = store::add_play_seconds(&app.db, row.game_key, seconds).await;
                    let _ = store::delete_session(&app.db, row.id).await;
                }
            }
        }
        self.reload_plays().await;
    }
}

/// The Riot Client runs in the background most of the time, so it never means that a game runs,
/// wherever it lies. (Teamfight Tactics runs inside the League of Legends client, so it has no
/// process of its own and is never shown as running.) `image` is a `path_key`.
fn is_riot_client(image: &str) -> bool {
    image
        .rsplit('\\')
        .next()
        .is_some_and(|name| name == super::riot::CLIENT_EXE.to_lowercase())
}

/// The folder whose processes make a game "running": the install folder, or for a hand-added
/// game the folder of its program (never a drive root, which would match everything).
fn game_folder(entry: &super::model::Found) -> Option<std::path::PathBuf> {
    let folder = entry.install.clone().or_else(|| {
        entry
            .local
            .as_ref()
            .and_then(|local| local.exe.as_ref())
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
    })?;
    let text = folder.to_string_lossy().into_owned();
    // `D:\` has three characters.
    (text.len() > 3).then_some(folder)
}
