//! The place of the user's TOML files, the one file watcher, and where their problems are
//! reported. Foundation section 10.4.
//!
//! Two kinds of watching live here, both on `notify` (`ReadDirectoryChangesW`, no polling):
//!
//! - the data folder: any event for a path that ends in `.toml` makes the `TomlFile` of that path
//!   read it again. The event kind is not interpreted (the spike saw `Create`, `Modify`, `Remove`
//!   and renames for the same user action), and Rain's own writes are recognised by their hash,
//!   so they cause no second load;
//! - `watch_dir`: any other folder that a module wants to follow (the Start Menu, the folder of
//!   the Claude sessions, a notes folder). Sections 01, 03 and 04 use it; none builds its own
//!   debouncer.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use rain_types::SourceState;
use rain_types::limits::TOML_DEBOUNCE;
use tokio::sync::Notify;
use tracing::{debug, info, warn};

use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::health::Health;

/// What a `TomlFile` gives the hub to call when its file may have changed. It must return at
/// once: the watcher thread calls it.
pub(super) type Reloader = Arc<dyn Fn() + Send + Sync>;

type FileDebouncer = Debouncer<RecommendedWatcher, RecommendedCache>;

/// The name under which a broken watcher shows in `GET /health`.
const WATCH_SOURCE: &str = "config-watch";

/// Cloning is cheap. Every module's `store.rs` opens its files through the hub.
#[derive(Clone)]
pub struct ConfigHub {
    shared: Arc<Shared>,
}

struct Shared {
    data_dir: PathBuf,
    health: Arc<Health>,
    /// The files to reload, by lower-case path (Windows file names ignore case).
    reloaders: Mutex<HashMap<String, Reloader>>,
    watcher: Mutex<Option<FileDebouncer>>,
}

impl ConfigHub {
    pub fn new(data_dir: PathBuf, health: Arc<Health>) -> Self {
        Self {
            shared: Arc::new(Shared {
                data_dir,
                health,
                reloaders: Mutex::default(),
                watcher: Mutex::new(None),
            }),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.shared.data_dir
    }

    /// The path of a file in the data folder. The file need not exist.
    pub fn file_path(&self, file_name: &str) -> PathBuf {
        self.shared.data_dir.join(file_name)
    }

    /// Where a `TomlFile` reports the problems it finds (`GET /health`, topic `health`).
    pub(super) fn health(&self) -> &Arc<Health> {
        &self.shared.health
    }

    /// First run: makes the data folder (and its parents) when it is missing. Nothing is written
    /// into it; every file is created by its first write (foundation 10.1).
    pub async fn ensure_data_dir(&self) -> ApiResult<()> {
        let folder = self.shared.data_dir.clone();
        blocking::run(move || {
            std::fs::create_dir_all(&folder).map_err(|e| {
                ApiError::failed(format!(
                    "Rain could not create its data folder {} ({e})",
                    folder.display()
                ))
            })
        })
        .await??;
        info!(folder = %self.shared.data_dir.display(), "data folder ready");
        Ok(())
    }

    // ---- the data folder watcher ----

    /// Called by `TomlFile::open`: `reload` runs after a debounced event for `path`.
    pub(super) fn register(&self, path: &Path, reload: Reloader) {
        self.reloaders().insert(key_of(path), reload);
    }

    /// Starts watching the data folder (not its sub-folders: a non-recursive watch reports
    /// nothing from them). Needs a tokio runtime. A failure is not fatal: `TomlFile::refresh`
    /// still notices a hand edit when Settings reads the file, and `GET /health` says so.
    pub fn start_watching(&self) {
        let reloaders = Arc::clone(&self.shared);
        let handler = move |result: DebounceEventResult| reloaders.on_events(result);
        let started = new_debouncer(TOML_DEBOUNCE, None, handler).and_then(|mut debouncer| {
            debouncer.watch(&self.shared.data_dir, RecursiveMode::NonRecursive)?;
            Ok(debouncer)
        });
        match started {
            Ok(debouncer) => {
                *self.watcher() = Some(debouncer);
                self.shared.health.clear_source(WATCH_SOURCE);
                info!("watching the data folder");
            }
            Err(e) => {
                warn!(error = %e, "the data folder cannot be watched");
                self.shared.health.set_source(
                    WATCH_SOURCE,
                    SourceState::Failed,
                    Some("Rain cannot watch your data files; it notices changes when you open Settings".to_owned()),
                );
            }
        }
    }

    /// Ends the watcher thread. Called as one of the last steps of a stop.
    pub fn stop_watching(&self) {
        self.watcher().take();
    }

    fn reloaders(&self) -> MutexGuard<'_, HashMap<String, Reloader>> {
        self.shared
            .reloaders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn watcher(&self) -> MutexGuard<'_, Option<FileDebouncer>> {
        self.shared
            .watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    // ---- other folders ----

    /// Follows a folder that is not a TOML file of the user: `quiet` is how long the folder must
    /// be still before a change is reported (a burst of events becomes one). The watch ends when
    /// the returned `DirWatch` is dropped. An error means the folder cannot be watched (it does
    /// not exist, or the system refuses); the caller then falls back to refreshing on request.
    ///
    /// A recursive watch of a large tree is unproven (foundation appendix C); the fallback is one
    /// non-recursive watch per sub-folder.
    pub fn watch_dir(
        &self,
        folder: &Path,
        recursive: bool,
        quiet: Duration,
    ) -> notify::Result<DirWatch> {
        let pending = Arc::new(Pending::default());
        let sink = Arc::clone(&pending);
        let mut debouncer = new_debouncer(quiet, None, move |result: DebounceEventResult| {
            sink.take_events(result);
        })?;
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        debouncer.watch(folder, mode)?;
        Ok(DirWatch {
            pending,
            _debouncer: debouncer,
        })
    }
}

fn key_of(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

impl Shared {
    /// Runs on the debouncer's thread.
    fn on_events(&self, result: DebounceEventResult) {
        let events = match result {
            Ok(events) => events,
            Err(errors) => {
                for error in errors {
                    warn!(error = %error, "a file watch error");
                }
                return;
            }
        };
        let mut seen = BTreeSet::new();
        for path in events.iter().flat_map(|event| event.paths.iter()) {
            if is_toml(path) {
                seen.insert(key_of(path));
            }
        }
        let reloaders = self
            .reloaders
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for key in seen {
            if let Some(reload) = reloaders.get(&key) {
                debug!("a data file changed; reloading it");
                reload();
            }
        }
    }
}

/// The rule of foundation 10.4: a path that ends in `.toml`. Rain's own temporary files
/// (`.<name>.rain-tmp-<pid>-<n>`), the `.bak` copies and editor files that end in `~` never match.
fn is_toml(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
}

// ---- watching other folders ----

/// A watch of one folder. Dropping it stops the watch.
pub struct DirWatch {
    pending: Arc<Pending>,
    _debouncer: FileDebouncer,
}

/// What happened in a watched folder since the last call of `changed`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirChange {
    /// The paths that changed, sorted, without repeats.
    pub paths: Vec<PathBuf>,
    /// More paths changed than are kept (a big copy or a branch switch): the list is empty and
    /// anything in the folder may have changed.
    pub overflowed: bool,
}

/// The most paths one `DirChange` carries.
const MAX_PATHS: usize = 1000;

#[derive(Default)]
struct Pending {
    changes: Mutex<DirChange>,
    wake: Notify,
}

impl Pending {
    fn take_events(&self, result: DebounceEventResult) {
        let Ok(events) = result else {
            return; // an error of the watcher itself; the next event still arrives
        };
        {
            let mut changes = self.changes.lock().unwrap_or_else(PoisonError::into_inner);
            if !changes.overflowed {
                let mut all: BTreeSet<PathBuf> = changes.paths.drain(..).collect();
                all.extend(events.into_iter().flat_map(|event| event.paths.clone()));
                if all.len() > MAX_PATHS {
                    changes.overflowed = true;
                } else {
                    changes.paths = all.into_iter().collect();
                }
            }
        }
        // `notify_one` keeps a permit while nobody waits, so a change is never lost.
        self.wake.notify_one();
    }
}

impl DirWatch {
    /// Waits for the next change in the folder and returns everything that changed since the
    /// last call. It costs nothing while the folder is quiet.
    pub async fn changed(&self) -> DirChange {
        loop {
            self.pending.wake.notified().await;
            let mut changes = self
                .pending
                .changes
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if changes.overflowed || !changes.paths.is_empty() {
                return std::mem::take(&mut *changes);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn health() -> Arc<Health> {
        Arc::new(Health::new(
            "start".into(),
            "0.1.0",
            crate::core::bus::EventBus::new(),
        ))
    }

    #[tokio::test]
    async fn the_data_folder_and_its_parents_are_created_and_nothing_is_written() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("a").join("data");
        let hub = ConfigHub::new(folder.clone(), health());

        hub.ensure_data_dir().await.unwrap();
        hub.ensure_data_dir().await.unwrap(); // a second call is fine

        assert!(folder.is_dir());
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    }

    #[test]
    fn a_file_path_is_inside_the_data_folder() {
        let hub = ConfigHub::new("data".into(), health());
        assert_eq!(
            hub.file_path("settings.toml"),
            Path::new("data").join("settings.toml")
        );
    }

    #[test]
    fn only_paths_that_end_in_toml_count() {
        assert!(is_toml(Path::new(r"D:\data\settings.toml")));
        assert!(is_toml(Path::new(r"D:\data\SETTINGS.TOML")));
        assert!(!is_toml(Path::new(r"D:\data\.settings.toml.rain-tmp-123")));
        assert!(!is_toml(Path::new(r"D:\data\settings.toml.bak")));
        assert!(!is_toml(Path::new(r"D:\data\settings.toml~")));
        assert!(!is_toml(Path::new(r"D:\data\media")));
    }

    /// Waits until `check` is true. A watcher needs real time, so the tests poll briefly.
    async fn eventually(check: impl Fn() -> bool) -> bool {
        for _ in 0..100 {
            if check() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        check()
    }

    #[tokio::test]
    async fn a_changed_toml_file_calls_its_reloader_and_another_file_does_not() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("data");
        let hub = ConfigHub::new(folder.clone(), health());
        hub.ensure_data_dir().await.unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        hub.register(
            &hub.file_path("settings.toml"),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        hub.start_watching();

        std::fs::write(folder.join("pins.toml"), "x = 1\n").unwrap();
        std::fs::write(folder.join("settings.toml.bak"), "x = 1\n").unwrap();
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "no reloader for these files"
        );

        std::fs::write(folder.join("settings.toml"), "x = 2\n").unwrap();
        assert!(eventually(|| hits.load(Ordering::SeqCst) >= 1).await);
        hub.stop_watching();
    }

    #[tokio::test]
    async fn a_burst_of_writes_is_one_reload() {
        let temp = tempfile::tempdir().unwrap();
        let hub = ConfigHub::new(temp.path().join("data"), health());
        hub.ensure_data_dir().await.unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        hub.register(
            &hub.file_path("a.toml"),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        hub.start_watching();

        for n in 0..20 {
            std::fs::write(hub.file_path("a.toml"), format!("x = {n}\n")).unwrap();
        }
        assert!(eventually(|| hits.load(Ordering::SeqCst) >= 1).await);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        hub.stop_watching();
    }

    #[tokio::test]
    async fn watch_dir_reports_the_paths_that_changed() {
        let temp = tempfile::tempdir().unwrap();
        let hub = ConfigHub::new(temp.path().join("data"), health());
        let watched = temp.path().join("watched");
        std::fs::create_dir_all(watched.join("sub")).unwrap();

        let watch = hub
            .watch_dir(&watched, true, Duration::from_millis(100))
            .unwrap();
        std::fs::write(watched.join("sub").join("a.txt"), "1").unwrap();

        let change = tokio::time::timeout(Duration::from_secs(5), watch.changed())
            .await
            .expect("a change was reported");
        assert!(!change.overflowed);
        assert!(change.paths.iter().any(|path| path.ends_with("a.txt")));
    }

    #[tokio::test]
    async fn a_folder_that_does_not_exist_cannot_be_watched() {
        let temp = tempfile::tempdir().unwrap();
        let hub = ConfigHub::new(temp.path().join("data"), health());
        assert!(
            hub.watch_dir(
                &temp.path().join("missing"),
                false,
                Duration::from_millis(50)
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn too_many_changed_paths_become_the_overflow_flag() {
        let pending = Pending::default();
        let events = (0..=MAX_PATHS)
            .map(|n| {
                notify_debouncer_full::DebouncedEvent::new(
                    notify::Event::new(notify::EventKind::Any)
                        .add_path(PathBuf::from(format!("f{n}"))),
                    std::time::Instant::now(),
                )
            })
            .collect();
        pending.take_events(Ok(events));
        let change = {
            pending.wake.notified().await;
            std::mem::take(&mut *pending.changes.lock().unwrap())
        };
        assert!(change.overflowed);
        assert!(change.paths.is_empty());
    }
}
