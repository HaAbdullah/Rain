//! The scans that this backend knows, and the one finished tree it holds. Section 05 5, 6.2.
//!
//! Nothing is written to disk: a scan result has millions of nodes and is wanted for minutes at
//! most, so it lives in memory and is dropped 10 minutes after its last read (timer T6).
//!
//! Every change of a scan's state goes through [`ScanTable::finish`], which runs only once for a
//! scan. That is what makes the rules of the topic true: the final frame is published exactly
//! once, and no progress frame can come after it (the progress task publishes under the same
//! lock and only while the scan is still `scanning`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rain_types::limits::{SCAN_ARENA_TTL, SCAN_PROGRESS_TICK, SCAN_RECORDS_KEPT};
use rain_types::{DiskScan, Drive, Scan, ScanStatus};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::arena::Arena;
use super::walk::Counters;
use crate::core::bus::EventBus;
use crate::core::error::ApiError;
use crate::core::shutdown::{Activity, ActivityGuard};
use crate::core::task::spawn_named;

/// "scan-20261122-093000-1": the UTC time of the start (cut from an ISO text) and a counter of
/// this process, so that two scans in one second differ.
pub fn scan_id(iso: &str, number: u64) -> String {
    let digits: String = iso.chars().take(19).filter(char::is_ascii_digit).collect();
    if digits.len() == 14 {
        format!("scan-{}-{}-{number}", &digits[..8], &digits[8..])
    } else {
        format!("scan-{number}")
    }
}

/// How a scan ends.
pub enum Finish {
    /// The walk is complete: the tree, and the path of its root (`C:\`).
    Done(Arena, String),
    Failed(String),
    Cancelled,
}

/// What `begin` found.
pub enum Begin {
    /// A scan of this drive is already running: here is its state (answer 200).
    Existing(DiskScan),
    /// A new scan: its state, the flag that cancels it and the counters that the walker fills
    /// (answer 201).
    New {
        scan: DiskScan,
        cancel: Arc<AtomicBool>,
        counters: Arc<Counters>,
    },
}

/// What a read of a scan returns.
pub struct Read {
    pub scan: DiskScan,
    /// The tree and the path of its root, when the scan is done and the tree is still held.
    pub tree: Option<(Arc<Arena>, String)>,
}

struct Record {
    id: String,
    drive_id: String,
    status: ScanStatus,
    progress: f64,
    files_seen: u64,
    not_accessible: u64,
    error: Option<String>,
    /// Present while the scan runs.
    live: Option<Live>,
}

struct Live {
    counters: Arc<Counters>,
    /// What the drive reports as used; the progress is the bytes seen divided by it.
    used_bytes: u64,
}

struct Running {
    id: String,
    cancel: Arc<AtomicBool>,
    progress_task: JoinHandle<()>,
    /// Keeps `GET /ping` `busy` true while the scan runs.
    _busy: ActivityGuard,
}

/// The one finished tree.
struct Slot {
    scan_id: String,
    arena: Arc<Arena>,
    root_path: String,
    deadline: Instant,
    expiry_task: JoinHandle<()>,
}

struct Inner {
    /// Oldest first, at most `SCAN_RECORDS_KEPT`.
    records: VecDeque<Record>,
    running: Option<Running>,
    slot: Option<Slot>,
    /// Scans started by this process.
    started: u64,
}

pub struct ScanTable {
    inner: Mutex<Inner>,
    bus: EventBus,
    activity: Activity,
}

impl ScanTable {
    pub fn new(bus: EventBus, activity: Activity) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                records: VecDeque::new(),
                running: None,
                slot: None,
                started: 0,
            }),
            bus,
            activity,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the record of a scan of `drive`, or returns the running scan of that drive. A
    /// running scan of another drive is ended as `cancelled` (its final frame goes out first),
    /// and the old tree is dropped, so that the peak memory is one tree and not two.
    pub fn begin(self: &Arc<Self>, drive: &Drive, iso_now: &str) -> Begin {
        let mut guard = self.lock();
        let inner = &mut *guard;
        if let Some(running) = &inner.running {
            let same_drive = inner
                .records
                .iter()
                .any(|record| record.id == running.id && record.drive_id == drive.id);
            if same_drive {
                let id = running.id.clone();
                let record = inner
                    .records
                    .iter_mut()
                    .find(|record| record.id == id)
                    .expect("the running scan has a record");
                refresh(record);
                return Begin::Existing(snapshot(record));
            }
            let id = running.id.clone();
            self.finish_locked(inner, &id, Finish::Cancelled, true);
        }
        if let Some(old) = inner.slot.take() {
            old.expiry_task.abort();
        }
        inner.started += 1;
        let id = scan_id(iso_now, inner.started);
        let counters = Arc::new(Counters::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let record = Record {
            id: id.clone(),
            drive_id: drive.id.clone(),
            status: ScanStatus::Scanning,
            progress: 0.0,
            files_seen: 0,
            not_accessible: 0,
            error: None,
            live: Some(Live {
                counters: Arc::clone(&counters),
                used_bytes: drive.used_bytes,
            }),
        };
        let scan = snapshot(&record);
        inner.records.push_back(record);
        while inner.records.len() > SCAN_RECORDS_KEPT {
            inner.records.pop_front();
        }
        inner.running = Some(Running {
            id: id.clone(),
            cancel: Arc::clone(&cancel),
            progress_task: self.spawn_progress(id),
            _busy: self.activity.guard(),
        });
        Begin::New {
            scan,
            cancel,
            counters,
        }
    }

    /// Ends a scan that is still `scanning`. Returns false (and does nothing) when it already
    /// ended: only the first caller counts.
    pub fn finish(self: &Arc<Self>, id: &str, how: Finish) -> bool {
        let mut inner = self.lock();
        self.finish_locked(&mut inner, id, how, true)
    }

    fn finish_locked(
        self: &Arc<Self>,
        inner: &mut Inner,
        id: &str,
        how: Finish,
        publish: bool,
    ) -> bool {
        let Inner {
            records,
            running,
            slot,
            ..
        } = inner;
        let Some(record) = records.iter_mut().find(|record| record.id == id) else {
            return false;
        };
        if record.status != ScanStatus::Scanning {
            return false;
        }
        // The last numbers, from the counters, before they are let go.
        refresh(record);
        record.live = None;
        match how {
            Finish::Done(arena, root_path) => {
                record.status = ScanStatus::Done;
                record.progress = 1.0;
                if let Some(old) = slot.take() {
                    old.expiry_task.abort();
                }
                *slot = Some(Slot {
                    scan_id: id.to_owned(),
                    arena: Arc::new(arena),
                    root_path,
                    deadline: Instant::now() + SCAN_ARENA_TTL,
                    expiry_task: self.spawn_expiry(id.to_owned()),
                });
            }
            Finish::Failed(message) => {
                record.status = ScanStatus::Failed;
                record.error = Some(message);
            }
            Finish::Cancelled => record.status = ScanStatus::Cancelled,
        }
        if running.as_ref().is_some_and(|running| running.id == id)
            && let Some(ended) = running.take()
        {
            // Tells the walker to stop (a no-op when it has finished) and ends the progress task.
            ended.cancel.store(true, Relaxed);
            ended.progress_task.abort();
        }
        if publish {
            self.bus.publish::<Scan>(&snapshot(record));
        }
        true
    }

    /// Ends the running scan as `cancelled` and publishes that. Game mode calls this.
    pub fn cancel_running(self: &Arc<Self>) {
        let mut inner = self.lock();
        if let Some(id) = inner.running.as_ref().map(|running| running.id.clone()) {
            self.finish_locked(&mut inner, &id, Finish::Cancelled, true);
        }
    }

    pub fn is_scanning(&self) -> bool {
        self.lock().running.is_some()
    }

    /// `DELETE /scans/{id}`: cancels a running scan, frees the tree of a finished one, and does
    /// nothing for a scan that has neither.
    pub fn cancel(self: &Arc<Self>, id: &str) -> Result<(), ApiError> {
        let mut inner = self.lock();
        if !inner.records.iter().any(|record| record.id == id) {
            return Err(unknown_scan());
        }
        if inner
            .running
            .as_ref()
            .is_some_and(|running| running.id == id)
        {
            self.finish_locked(&mut inner, id, Finish::Cancelled, true);
        } else if inner.slot.as_ref().is_some_and(|slot| slot.scan_id == id)
            && let Some(slot) = inner.slot.take()
        {
            slot.expiry_task.abort();
        }
        Ok(())
    }

    /// `GET /scans/{id}`. Reading a finished scan restarts the 10 minutes of its tree.
    pub fn read(&self, id: &str) -> Result<Read, ApiError> {
        let mut inner = self.lock();
        let Inner { records, slot, .. } = &mut *inner;
        let record = records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(unknown_scan)?;
        refresh(record);
        let scan = snapshot(record);
        let tree = match slot {
            Some(slot) if slot.scan_id == id => {
                slot.deadline = Instant::now() + SCAN_ARENA_TTL;
                Some((Arc::clone(&slot.arena), slot.root_path.clone()))
            }
            _ => None,
        };
        Ok(Read { scan, tree })
    }

    /// The tree for `GET /scans/{id}/node`: an error says why there is none.
    pub fn tree_for_node(&self, id: &str) -> Result<(Arc<Arena>, String), ApiError> {
        let read = self.read(id)?;
        if read.scan.status != ScanStatus::Done {
            return Err(ApiError::conflict(
                "scanNotDone",
                "The scan is not finished yet",
            ));
        }
        read.tree.ok_or_else(|| {
            ApiError::conflict("scanExpired", "This scan is too old. Scan the drive again.")
        })
    }

    /// The end of the backend: the running scan stops without a frame (nobody listens any more)
    /// and the tree is dropped.
    pub fn stop(self: &Arc<Self>) {
        let mut inner = self.lock();
        if let Some(id) = inner.running.as_ref().map(|running| running.id.clone()) {
            self.finish_locked(&mut inner, &id, Finish::Cancelled, false);
        }
        if let Some(slot) = inner.slot.take() {
            slot.expiry_task.abort();
        }
    }

    // ---- timers ----

    /// Timer T5: while the scan runs, one wake-up every 250 ms and a frame when a number moved.
    fn spawn_progress(self: &Arc<Self>, id: String) -> JoinHandle<()> {
        let table = Arc::downgrade(self);
        spawn_named("disk-scan-progress", async move {
            let mut last = None;
            loop {
                tokio::time::sleep(SCAN_PROGRESS_TICK).await;
                let Some(table) = table.upgrade() else { return };
                if !table.report(&id, &mut last) {
                    return;
                }
            }
        })
    }

    /// Publishes the scan's numbers when they changed since `last`. False when the scan is no
    /// longer running (the task then ends).
    fn report(&self, id: &str, last: &mut Option<(u64, u64, u64)>) -> bool {
        let mut inner = self.lock();
        if !inner
            .running
            .as_ref()
            .is_some_and(|running| running.id == id)
        {
            return false;
        }
        let Some(record) = inner.records.iter_mut().find(|record| record.id == id) else {
            return false;
        };
        if record.status != ScanStatus::Scanning {
            return false;
        }
        refresh(record);
        let key = (
            record.files_seen,
            record.not_accessible,
            (record.progress * 1000.0) as u64,
        );
        if *last != Some(key) {
            *last = Some(key);
            self.bus.publish::<Scan>(&snapshot(record));
        }
        true
    }

    /// Timer T6: sleeps until the tree's deadline, which every read moves forward, and then
    /// drops the tree.
    fn spawn_expiry(self: &Arc<Self>, id: String) -> JoinHandle<()> {
        let table = Arc::downgrade(self);
        spawn_named("disk-arena-expiry", async move {
            loop {
                let Some(strong) = table.upgrade() else {
                    return;
                };
                let Some(deadline) = strong.deadline_of(&id) else {
                    return;
                };
                if Instant::now() >= deadline {
                    strong.expire(&id, deadline);
                    return;
                }
                drop(strong);
                tokio::time::sleep_until(deadline).await;
            }
        })
    }

    fn deadline_of(&self, id: &str) -> Option<Instant> {
        let inner = self.lock();
        inner
            .slot
            .as_ref()
            .filter(|slot| slot.scan_id == id)
            .map(|slot| slot.deadline)
    }

    /// Drops the tree when its deadline is still the one the timer saw (a read in between moved
    /// it, and then the timer sleeps again).
    fn expire(&self, id: &str, seen: Instant) {
        let mut inner = self.lock();
        if inner
            .slot
            .as_ref()
            .is_some_and(|slot| slot.scan_id == id && slot.deadline <= seen)
        {
            // The timer is this very task: it ends by itself, so it is not aborted.
            inner.slot = None;
        }
    }
}

fn unknown_scan() -> ApiError {
    ApiError::not_found("Scan not found").with_reason("unknownScan")
}

/// Brings the numbers of a running record up to date from the walker's counters. Progress never
/// goes down, stays at 0.99 or less until the scan is done, and has three digits.
fn refresh(record: &mut Record) {
    let Some(live) = &record.live else { return };
    record.files_seen = live.counters.files.load(Relaxed);
    record.not_accessible = live.counters.denied.load(Relaxed);
    if live.used_bytes > 0 {
        let seen = live.counters.bytes.load(Relaxed) as f64 / live.used_bytes as f64;
        let rounded = (seen.min(0.99) * 1000.0).round() / 1000.0;
        record.progress = record.progress.max(rounded);
    }
}

fn snapshot(record: &Record) -> DiskScan {
    DiskScan {
        id: record.id.clone(),
        drive_id: record.drive_id.clone(),
        status: record.status,
        progress: record.progress,
        files_seen: record.files_seen,
        error: record.error.clone(),
        not_accessible: (record.not_accessible > 0).then_some(record.not_accessible),
        root: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::disk::arena::flatten;
    use crate::modules::disk::walk::DirNode;
    use rain_types::Frame;
    use tokio::sync::broadcast::Receiver;

    fn drive(letter: char, used: u64) -> Drive {
        Drive {
            id: format!("drive-{letter}"),
            letter: letter.to_ascii_uppercase().to_string(),
            label: "Local Disk".into(),
            total_bytes: used * 2,
            used_bytes: used,
        }
    }

    fn table() -> (Arc<ScanTable>, Receiver<Arc<crate::core::bus::FrameBytes>>) {
        let bus = EventBus::new();
        let receiver = bus.subscribe();
        (ScanTable::new(bus, Activity::default()), receiver)
    }

    fn tree() -> Arena {
        flatten(DirNode {
            name: "T:".into(),
            size: 0,
            files: vec![],
            other_files: (0, 0),
            dirs: vec![],
            denied: false,
            dominant: 255,
        })
    }

    fn frames(receiver: &mut Receiver<Arc<crate::core::bus::FrameBytes>>) -> Vec<DiskScan> {
        let mut out = Vec::new();
        while let Ok(frame) = receiver.try_recv() {
            let parsed: Frame = serde_json::from_str(&frame.json).expect("a frame");
            assert_eq!(parsed.topic, "scan");
            out.push(serde_json::from_value(parsed.payload).expect("a scan"));
        }
        out
    }

    fn new_scan(table: &Arc<ScanTable>, drive: &Drive) -> (String, Arc<Counters>) {
        match table.begin(drive, "2026-11-22T09:30:00.123Z") {
            Begin::New { scan, counters, .. } => (scan.id, counters),
            Begin::Existing(_) => panic!("a new scan"),
        }
    }

    #[test]
    fn a_scan_id_has_the_utc_time_and_a_counter() {
        assert_eq!(
            scan_id("2026-11-22T09:30:00.123Z", 1),
            "scan-20261122-093000-1"
        );
        assert_eq!(scan_id("garbage", 7), "scan-7");
    }

    #[tokio::test]
    async fn only_the_first_end_of_a_scan_counts_and_its_frame_is_published_once() {
        let (table, mut frames_in) = table();
        let (id, _) = new_scan(&table, &drive('t', 100));
        assert!(table.finish(&id, Finish::Done(tree(), "T:\\".into())));
        assert!(
            !table.finish(&id, Finish::Cancelled),
            "a second end is ignored"
        );
        assert!(!table.finish(&id, Finish::Failed("x".into())));
        let published = frames(&mut frames_in);
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].status, ScanStatus::Done);
        assert_eq!(published[0].progress, 1.0);
        assert!(published[0].root.is_none());
        assert_eq!(table.read(&id).unwrap().scan.status, ScanStatus::Done);
    }

    #[tokio::test]
    async fn progress_never_goes_down_and_stays_below_one_until_done() {
        let (table, _frames) = table();
        let (id, counters) = new_scan(&table, &drive('t', 1000));
        let progress = |table: &Arc<ScanTable>| table.read(&id).unwrap().scan.progress;
        counters.bytes.store(500, Relaxed);
        counters.files.store(7, Relaxed);
        assert_eq!(progress(&table), 0.5);
        assert_eq!(table.read(&id).unwrap().scan.files_seen, 7);
        counters.bytes.store(100, Relaxed);
        assert_eq!(progress(&table), 0.5, "it does not go back");
        counters.bytes.store(5000, Relaxed);
        assert_eq!(progress(&table), 0.99, "more bytes than the drive reports");
        table.finish(&id, Finish::Done(tree(), "T:\\".into()));
        assert_eq!(progress(&table), 1.0);
    }

    #[tokio::test]
    async fn a_scan_of_the_same_drive_is_returned_and_another_drive_cancels_it() {
        let (table, mut frames_in) = table();
        let (first, _) = new_scan(&table, &drive('t', 100));
        let Begin::Existing(again) = table.begin(&drive('t', 100), "2026-11-22T09:30:01.000Z")
        else {
            panic!("the running scan is returned");
        };
        assert_eq!(again.id, first);
        assert!(frames(&mut frames_in).is_empty());

        let (second, _) = new_scan(&table, &drive('u', 100));
        assert_ne!(first, second);
        assert_eq!(
            table.read(&first).unwrap().scan.status,
            ScanStatus::Cancelled
        );
        let published = frames(&mut frames_in);
        assert_eq!(published.len(), 1);
        assert_eq!(
            (published[0].id.as_str(), published[0].status),
            (first.as_str(), ScanStatus::Cancelled)
        );
    }

    #[tokio::test]
    async fn cancelling_sets_the_flag_publishes_once_and_a_second_cancel_does_nothing() {
        let (table, mut frames_in) = table();
        let flag = match table.begin(&drive('t', 100), "2026-11-22T09:30:00.123Z") {
            Begin::New { cancel, scan, .. } => {
                assert!(table.is_scanning());
                assert_eq!(scan.status, ScanStatus::Scanning);
                cancel
            }
            Begin::Existing(_) => panic!("new"),
        };
        let id = table.read("scan-20261122-093000-1").unwrap().scan.id;
        table.cancel(&id).unwrap();
        assert!(flag.load(Relaxed), "the walker is told to stop");
        assert!(!table.is_scanning());
        table.cancel(&id).unwrap();
        assert_eq!(frames(&mut frames_in).len(), 1);
        assert!(table.cancel("scan-nope").is_err());
    }

    #[tokio::test]
    async fn the_table_keeps_the_last_eight_records() {
        let (table, _frames) = table();
        let mut ids = Vec::new();
        for n in 0..10 {
            let letter = if n % 2 == 0 { 't' } else { 'u' };
            ids.push(new_scan(&table, &drive(letter, 100)).0);
        }
        assert!(table.read(&ids[0]).is_err());
        assert!(table.read(&ids[1]).is_err());
        assert!(table.read(&ids[2]).is_ok());
        assert!(table.read(&ids[9]).is_ok());
    }

    #[tokio::test]
    async fn a_running_scan_keeps_the_backend_busy_until_it_ends() {
        let activity = Activity::default();
        let table = ScanTable::new(EventBus::new(), activity.clone());
        let (id, _) = new_scan(&table, &drive('t', 100));
        assert!(activity.busy());
        table.finish(&id, Finish::Failed("no".into()));
        assert!(!activity.busy());
        let read = table.read(&id).unwrap();
        assert_eq!(read.scan.error.as_deref(), Some("no"));
        assert!(read.tree.is_none());
    }

    #[tokio::test]
    async fn the_tree_is_held_until_it_is_freed_and_then_the_node_route_says_so() {
        let (table, _frames) = table();
        let (id, _) = new_scan(&table, &drive('t', 100));
        assert_eq!(
            table.tree_for_node(&id).unwrap_err().body.reason.as_deref(),
            Some("scanNotDone")
        );
        table.finish(&id, Finish::Done(tree(), "T:\\".into()));
        assert!(table.tree_for_node(&id).is_ok());
        table.cancel(&id).unwrap();
        assert_eq!(
            table.tree_for_node(&id).unwrap_err().body.reason.as_deref(),
            Some("scanExpired")
        );
        let read = table.read(&id).unwrap();
        assert_eq!(read.scan.status, ScanStatus::Done, "the record stays");
        assert!(read.tree.is_none());
    }

    #[tokio::test]
    async fn stopping_ends_the_scan_without_a_frame_and_drops_the_tree() {
        let (table, mut frames_in) = table();
        let (running, _) = new_scan(&table, &drive('t', 100));
        table.stop();
        assert!(frames(&mut frames_in).is_empty());
        assert_eq!(
            table.read(&running).unwrap().scan.status,
            ScanStatus::Cancelled
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_read_moves_the_deadline_and_the_tree_goes_ten_minutes_after_the_last_read() {
        let (table, _frames) = table();
        let (id, _) = new_scan(&table, &drive('t', 100));
        table.finish(&id, Finish::Done(tree(), "T:\\".into()));
        tokio::time::sleep(SCAN_ARENA_TTL - std::time::Duration::from_secs(60)).await;
        assert!(table.read(&id).unwrap().tree.is_some(), "minute 9");
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        assert!(
            table.read(&id).unwrap().tree.is_some(),
            "minute 11, but read at minute 9"
        );
        tokio::time::sleep(SCAN_ARENA_TTL + std::time::Duration::from_secs(1)).await;
        assert!(
            table.read(&id).unwrap().tree.is_none(),
            "gone after 10 idle minutes"
        );
        assert_eq!(table.read(&id).unwrap().scan.status, ScanStatus::Done);
    }
}
