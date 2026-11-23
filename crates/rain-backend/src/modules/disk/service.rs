//! The logic of the disk module: drives, scans, the lazy subtree and the two path routes.
//! Other modules call it, and only it (`cancel_running`, `is_scanning`).
//!
//! The scan runs like this (section 05 6.2, 6.3):
//!
//! 1. `start_scan` registers the scan in the [`ScanTable`] (state `scanning`, one frame).
//! 2. A thread named `rain-scan-<id>` builds a pool of 8 walker threads, walks the drive, drops
//!    the pool and moves the finished tree into an [`Arena`].
//! 3. A small task waits for the thread's result and ends the scan in the table (`done`,
//!    `failed`). A cancel ends it earlier; the table makes sure that only one of them counts.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

use rain_types::limits::{SCAN_BELOW_NORMAL, SCAN_KEEP_FILES_PER_DIR, SCAN_WALK_THREADS};
use rain_types::time::iso_from_system_time;
use rain_types::{DiskNode, DiskScan, Drive, PathCheck, ScanStatus};
use tokio::sync::oneshot;
use tracing::{error, info};

use super::arena::{Arena, CutLimits, Lookup, cut, find, flatten};
use super::drives::{DriveInfo, DriveSource, SystemDrives};
use super::paths::{PathProbe, Paths, Revealer, ShellRevealer, StdProbe};
use super::store::{Begin, Finish, ScanTable};
use super::walk::{Counters, Ctx, DirSource, StdDirs, walk_dir};
use crate::core::blocking;
use crate::core::clock::Clock;
use crate::core::error::{ApiError, ApiResult};
use crate::core::shutdown::ShutdownHandle;
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::platform::thread::set_below_normal;

/// What the walk thread hands back.
enum Outcome {
    Done(Arena, String),
    Failed(String),
    Cancelled,
}

/// The text of a scan that ends in a way the user cannot do anything about.
const SCAN_CRASHED: &str = "The scan stopped unexpectedly";

pub struct Service {
    scans: Arc<ScanTable>,
    drives: Arc<dyn DriveSource>,
    dirs: Arc<dyn DirSource>,
    paths: Paths,
    clock: Arc<dyn Clock>,
    shutdown: Arc<ShutdownHandle>,
    /// For the game mode flag. The state owns this service, so a strong reference would be a
    /// cycle (foundation 11.3).
    state: WeakAppState,
    /// Scan threads that are still running (a hook for the tests).
    threads: Arc<AtomicUsize>,
    /// Whether the walker threads run at below-normal priority (`SCAN_BELOW_NORMAL`).
    below_normal: AtomicBool,
}

impl Service {
    /// Builds the service. A test replaces the parts that touch the PC (`state.seam`).
    pub fn new(state: &AppState) -> Self {
        let drives = state
            .seam::<dyn DriveSource>()
            .unwrap_or_else(|| Arc::new(SystemDrives));
        let dirs = state
            .seam::<dyn DirSource>()
            .unwrap_or_else(|| Arc::new(StdDirs));
        let prober = state
            .seam::<dyn PathProbe>()
            .unwrap_or_else(|| Arc::new(StdProbe));
        let revealer = state
            .seam::<dyn Revealer>()
            .unwrap_or_else(|| Arc::new(ShellRevealer));
        Self {
            scans: ScanTable::new(state.bus.clone(), state.activity.clone()),
            drives,
            dirs,
            paths: Paths::new(prober, revealer),
            clock: Arc::clone(&state.clock),
            shutdown: Arc::clone(&state.shutdown),
            state: state.downgrade(),
            threads: Arc::default(),
            below_normal: AtomicBool::new(SCAN_BELOW_NORMAL),
        }
    }

    // ---- what other modules call ----

    /// Cancels a running disk scan at once and publishes `cancelled`. Game mode calls this
    /// before it winds down (section 06 6.7.5). Does nothing when no scan runs.
    pub fn cancel_running(&self) {
        self.scans.cancel_running();
    }

    pub fn is_scanning(&self) -> bool {
        self.scans.is_scanning()
    }

    /// Scan threads that have not ended yet. Used by the tests to see that a cancel leaves no
    /// thread behind.
    pub fn threads_alive(&self) -> usize {
        self.threads.load(SeqCst)
    }

    /// Switches the below-normal priority of the walker threads for the next scans. The tests
    /// turn it off, because a walker that is starved by a busy PC makes timing tests flaky; it
    /// is also how the priority is compared by hand (section 05, acceptance check 3).
    pub fn set_below_normal(&self, on: bool) {
        self.below_normal.store(on, SeqCst);
    }

    /// The end of the backend: the scan stops, no frame is published, the tree is dropped.
    pub fn stop(&self) {
        self.scans.stop();
    }

    // ---- drives ----

    pub async fn list_drives(&self) -> ApiResult<Vec<Drive>> {
        Ok(self
            .drive_infos()
            .await?
            .into_iter()
            .map(|info| info.drive)
            .collect())
    }

    /// Read live every time: a drive can come and go, and a cache would be stale.
    async fn drive_infos(&self) -> ApiResult<Vec<DriveInfo>> {
        let source = Arc::clone(&self.drives);
        blocking::run(move || source.list()).await
    }

    // ---- scans ----

    /// `POST /scans`. The flag says whether the scan is new (201) or the running scan of this
    /// drive was returned (200).
    pub async fn start_scan(&self, drive_id: &str) -> ApiResult<(DiskScan, bool)> {
        if self
            .state
            .upgrade()
            .is_some_and(|state| state.game_mode_active())
        {
            return Err(ApiError::conflict(
                "gameMode",
                "Game mode is active. Scans start again after it.",
            ));
        }
        if self.shutdown.shutting_down() {
            return Err(
                ApiError::unavailable("disk", "Rain is stopping").with_reason("shuttingDown")
            );
        }
        let info = self
            .drive_infos()
            .await?
            .into_iter()
            .find(|info| info.drive.id == drive_id)
            .ok_or_else(|| ApiError::not_found("Drive not found").with_reason("unknownDrive"))?;
        let now = iso_from_system_time(self.clock.now());
        match self.scans.begin(&info.drive, &now) {
            Begin::Existing(scan) => Ok((scan, false)),
            Begin::New {
                scan,
                cancel,
                counters,
            } => {
                info!(drive = %info.root_name, "disk scan started");
                self.launch_walk(&scan.id, info, cancel, counters);
                Ok((scan, true))
            }
        }
    }

    /// Starts the scan thread and the task that waits for it.
    fn launch_walk(
        &self,
        id: &str,
        info: DriveInfo,
        cancel: Arc<AtomicBool>,
        counters: Arc<Counters>,
    ) {
        let (sender, receiver) = oneshot::channel::<std::thread::Result<Outcome>>();
        let source = Arc::clone(&self.dirs);
        let alive = ThreadCount::new(Arc::clone(&self.threads));
        let below_normal = self.below_normal.load(SeqCst);
        let started = std::thread::Builder::new()
            .name(format!("rain-scan-{id}"))
            .spawn(move || {
                let _alive = alive;
                // A panic in the walker must end the scan as `failed`, not take the thread down
                // silently. The result travels over the channel either way.
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_walk(
                        source,
                        info.root,
                        info.root_name,
                        cancel,
                        counters,
                        below_normal,
                    )
                }));
                let _ = sender.send(result);
            });
        let scans = Arc::clone(&self.scans);
        let id = id.to_owned();
        if let Err(cause) = started {
            error!(error = %cause, "the disk scan thread could not start");
            scans.finish(&id, Finish::Failed(SCAN_CRASHED.to_owned()));
            return;
        }
        spawn_named("disk-scan-finish", async move {
            let how = match receiver.await {
                Ok(Ok(Outcome::Done(arena, root_path))) => Finish::Done(arena, root_path),
                Ok(Ok(Outcome::Failed(message))) => Finish::Failed(message),
                Ok(Ok(Outcome::Cancelled)) => Finish::Cancelled,
                Ok(Err(_)) | Err(_) => {
                    error!("a disk scan thread ended without a result");
                    Finish::Failed(SCAN_CRASHED.to_owned())
                }
            };
            // A scan that was cancelled meanwhile has already ended: then this does nothing and
            // the tree is dropped here.
            scans.finish(&id, how);
        });
    }

    /// `GET /scans/{id}`. `root` is there when the scan is done and its tree is still held.
    pub async fn get_scan(&self, id: &str) -> ApiResult<DiskScan> {
        let read = self.scans.read(id)?;
        let mut scan = read.scan;
        if scan.status == ScanStatus::Done
            && let Some((arena, root_path)) = read.tree
        {
            // Sorting a folder with 100 000 entries can take longer than a few milliseconds.
            scan.root = Some(
                blocking::run(move || cut(&arena, 0, &root_path, &CutLimits::STANDARD)).await?,
            );
        }
        Ok(scan)
    }

    /// `GET /scans/{id}/node?path=`: one folder of a finished scan, cut to the budget.
    pub async fn scan_node(&self, id: &str, path: String) -> ApiResult<DiskNode> {
        let (arena, root_path) = self.scans.tree_for_node(id)?;
        blocking::run(move || match find(&arena, &root_path, &path) {
            Lookup::Dir { id, path } => Ok(cut(&arena, id, &path, &CutLimits::STANDARD)),
            Lookup::NotADirectory => {
                Err(ApiError::invalid("That is not a folder").with_reason("notADirectory"))
            }
            Lookup::Unknown => {
                Err(ApiError::not_found("That folder is not in this scan")
                    .with_reason("unknownPath"))
            }
        })
        .await?
    }

    /// `DELETE /scans/{id}`.
    pub fn cancel_scan(&self, id: &str) -> ApiResult<()> {
        self.scans.cancel(id)
    }

    // ---- paths ----

    pub async fn check_paths(
        &self,
        sent: Vec<String>,
        expanded: Vec<String>,
    ) -> ApiResult<Vec<PathCheck>> {
        self.paths.check(sent, expanded).await
    }

    pub async fn reveal(&self, path: &str) -> ApiResult<()> {
        self.paths.reveal(path).await
    }
}

/// Counts a scan thread for as long as it lives.
struct ThreadCount(Arc<AtomicUsize>);

impl ThreadCount {
    fn new(count: Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, SeqCst);
        Self(count)
    }
}

impl Drop for ThreadCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, SeqCst);
    }
}

/// The body of the scan thread: builds the pool, walks, drops the pool, flattens the tree.
fn run_walk(
    source: Arc<dyn DirSource>,
    root: PathBuf,
    root_name: String,
    cancel: Arc<AtomicBool>,
    counters: Arc<Counters>,
    below_normal: bool,
) -> Outcome {
    let mut builder = rayon::ThreadPoolBuilder::new()
        .num_threads(SCAN_WALK_THREADS)
        // Eight megabytes of stack, because the walk recurses one level per folder level.
        .stack_size(8 << 20)
        .thread_name(|index| format!("rain-walk-{index}"));
    if below_normal {
        // So that scanning a whole drive does not slow down what the user is doing.
        builder = builder.start_handler(|_| set_below_normal());
    }
    let pool = match builder.build() {
        Ok(pool) => pool,
        Err(cause) => {
            error!(error = %cause, "the walker pool could not start");
            return Outcome::Failed(SCAN_CRASHED.to_owned());
        }
    };
    let ctx = Ctx {
        src: &*source,
        cancel: &cancel,
        counters: &counters,
        keep: SCAN_KEEP_FILES_PER_DIR,
    };
    let (tree, _) = pool.install(|| walk_dir(&root, &root_name, 0, &ctx));
    // The eight threads end here: nothing stays resident after a scan.
    drop(pool);
    info!(
        files = counters.files.load(SeqCst),
        folders = counters.dirs.load(SeqCst),
        not_accessible = counters.denied.load(SeqCst),
        links_skipped = counters.reparse.load(SeqCst),
        "disk walk finished"
    );
    if cancel.load(SeqCst) {
        return Outcome::Cancelled;
    }
    if tree.denied {
        // The root itself was refused or is offline.
        return Outcome::Failed(format!("Drive {root_name} could not be read"));
    }
    Outcome::Done(flatten(tree), format!("{root_name}\\"))
}
