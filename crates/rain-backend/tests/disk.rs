//! The disk module on a real backend: drives, scans, the lazy subtree, reveal and the path check
//! (feature section 05, section 10.3).
//!
//! Nothing here touches the real PC: the drives, the folder reads, the probes and the Explorer
//! call are fakes, or a temp folder that the test builds. No test opens a window.

mod common;

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{StreamReader, TestBackend};
use futures_util::future::BoxFuture;
use hyper::StatusCode;
use rain_backend::modules::disk::{
    DirSource, DriveInfo, DriveSource, PathProbe, Probe, RawEntry, Revealer,
};
use rain_backend::platform::com::ComError;
use rain_types::{DiskNode, DiskScan, Drive, NodeKind, PathCheck, PathKind, ScanStatus};
use serde_json::{Value, json};

// ---- fakes ----

/// Waits for `time` on this thread. `park_timeout` is a sleep that clippy allows, but it can return
/// early (a park token may be left over), so it is repeated until the time is up.
fn nap(time: Duration) {
    let end = Instant::now() + time;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        std::thread::park_timeout(left);
    }
}

struct FakeDrives(Vec<DriveInfo>);

impl DriveSource for FakeDrives {
    fn list(&self) -> Vec<DriveInfo> {
        self.0.clone()
    }
}

/// A fake drive `letter:` whose files are in `root` (a temp folder or a folder of `FakeDirs`).
fn drive(letter: char, root: &Path, label: &str, total: u64, used: u64) -> DriveInfo {
    DriveInfo {
        drive: Drive {
            id: format!("drive-{}", letter.to_ascii_lowercase()),
            letter: letter.to_ascii_uppercase().to_string(),
            label: if label.is_empty() {
                "Local Disk".to_owned()
            } else {
                label.to_owned()
            },
            total_bytes: total,
            used_bytes: used,
        },
        root: root.to_path_buf(),
        root_name: format!("{}:", letter.to_ascii_uppercase()),
    }
}

/// Folders in memory. A folder can answer an error, and every call can be slow.
#[derive(Default)]
struct FakeDirs {
    dirs: HashMap<PathBuf, Result<Vec<RawEntry>, io::ErrorKind>>,
    delay: Duration,
}

impl FakeDirs {
    fn dir(mut self, path: &str, entries: Vec<RawEntry>) -> Self {
        self.dirs.insert(PathBuf::from(path), Ok(entries));
        self
    }

    fn failing(mut self, path: &str, kind: io::ErrorKind) -> Self {
        self.dirs.insert(PathBuf::from(path), Err(kind));
        self
    }

    fn slow(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

impl DirSource for FakeDirs {
    fn list(&self, dir: &Path) -> io::Result<Vec<RawEntry>> {
        if !self.delay.is_zero() {
            nap(self.delay);
        }
        match self.dirs.get(dir) {
            Some(Ok(entries)) => Ok(entries.clone()),
            Some(Err(kind)) => Err(io::Error::from(*kind)),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }
}

fn file(name: &str, len: u64) -> RawEntry {
    RawEntry {
        name: name.into(),
        attrs: 0x20,
        len,
    }
}

fn folder(name: &str) -> RawEntry {
    RawEntry {
        name: name.into(),
        attrs: 0x10,
        len: 0,
    }
}

/// 600 folders with 5 files each and 20 ms per folder: a scan that takes a second or more (the
/// walker threads run at below-normal priority, so on a busy PC it takes longer).
fn slow_tree() -> FakeDirs {
    let mut dirs = FakeDirs::default()
        .slow(Duration::from_millis(20))
        .dir("S:\\", (0..600).map(|n| folder(&format!("d{n}"))).collect());
    for n in 0..600 {
        dirs = dirs.dir(
            &format!("S:\\d{n}"),
            (0..5).map(|f| file(&format!("f{f}"), 10)).collect(),
        );
    }
    dirs
}

async fn start_with(drives: Vec<DriveInfo>, dirs: FakeDirs) -> TestBackend {
    let backend = TestBackend::builder()
        .with::<dyn DriveSource>(Arc::new(FakeDrives(drives)))
        .with::<dyn DirSource>(Arc::new(dirs))
        .start()
        .await;
    // Normal priority: a walker that a busy PC starves would make the timing tests flaky.
    backend.state.disk().set_below_normal(false);
    backend
}

async fn start_slow() -> TestBackend {
    start_with(
        vec![drive('s', Path::new("S:\\"), "", 1_000_000, 1_000_000)],
        slow_tree(),
    )
    .await
}

// ---- helpers ----

/// A folder tree on the real disk, built at run time so that no path is ever committed.
/// `a\` with files 100, 200, 300; `b\c\` with 70 files of 10 bytes; `empty\`; `big.bin` 10 000;
/// `notes.TXT` 5. The root holds 11 305 bytes in 75 files.
fn basic_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("a temporary folder");
    let root = temp.path();
    let make = |path: PathBuf, len: u64| {
        File::create(path)
            .expect("a file")
            .set_len(len)
            .expect("a size");
    };
    std::fs::create_dir_all(root.join("a")).expect("a folder");
    std::fs::create_dir_all(root.join("b").join("c")).expect("a folder");
    std::fs::create_dir_all(root.join("empty")).expect("a folder");
    for (name, len) in [("one", 100), ("two", 200), ("three", 300)] {
        make(root.join("a").join(name), len);
    }
    for n in 0..70 {
        make(root.join("b").join("c").join(format!("f{n:02}")), 10);
    }
    make(root.join("big.bin"), 10_000);
    make(root.join("notes.TXT"), 5);
    temp
}

async fn start_basic(root: &Path) -> TestBackend {
    let backend = TestBackend::builder()
        .with::<dyn DriveSource>(Arc::new(FakeDrives(vec![drive(
            't', root, "Test", 100_000, 40_000,
        )])))
        .start()
        .await;
    backend.state.disk().set_below_normal(false);
    backend
}

fn scan_of(value: &Value) -> DiskScan {
    serde_json::from_value(value.clone()).expect("a DiskScan")
}

fn node_of(value: &Value) -> DiskNode {
    serde_json::from_value(value.clone()).expect("a DiskNode")
}

/// Starts a scan and returns its state.
async fn start_scan(backend: &TestBackend, drive_id: &str) -> (StatusCode, DiskScan) {
    let (status, body) = backend
        .send("POST", "/scans", Some(json!({ "driveId": drive_id })))
        .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "{status}: {body}"
    );
    (status, scan_of(&body))
}

/// The next `scan` frame.
async fn next_scan(stream: &mut StreamReader) -> DiskScan {
    let frame = stream.expect_topic("scan").await;
    scan_of(&frame.payload)
}

/// Waits for the final frame of scan `id`.
async fn final_frame(stream: &mut StreamReader, id: &str) -> DiskScan {
    loop {
        let scan = next_scan(stream).await;
        if scan.id == id && scan.status != ScanStatus::Scanning {
            return scan;
        }
    }
}

/// Starts a scan and waits until it has ended, by asking (the walker runs at below-normal
/// priority, so how long it takes depends on how busy the PC is).
async fn scan_to_the_end(backend: &TestBackend, drive_id: &str) -> DiskScan {
    let (_, started) = start_scan(backend, drive_id).await;
    let begun = Instant::now();
    loop {
        let (_, body) = get_scan(backend, &started.id).await;
        let scan = scan_of(&body);
        if scan.status != ScanStatus::Scanning {
            return scan;
        }
        assert!(
            begun.elapsed() < Duration::from_secs(120),
            "the scan ended in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn get_scan(backend: &TestBackend, id: &str) -> (StatusCode, Value) {
    backend.send("GET", &format!("/scans/{id}"), None).await
}

/// The query value of a path, percent-encoded as the REST client does it.
fn enc(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

async fn node(backend: &TestBackend, id: &str, path: &str) -> (StatusCode, Value) {
    backend
        .send("GET", &format!("/scans/{id}/node?path={}", enc(path)), None)
        .await
}

fn find<'a>(node: &'a DiskNode, name: &str) -> &'a DiskNode {
    node.children
        .as_ref()
        .and_then(|children| children.iter().find(|child| child.name == name))
        .unwrap_or_else(|| panic!("{name} is a child of {}", node.path))
}

/// The nodes below and including `node`; the sums of every folder with children must be exact.
fn count_and_check(node: &DiskNode) -> usize {
    let mut count = 1;
    if let Some(children) = &node.children {
        assert!(!children.is_empty());
        let sum: u64 = children.iter().map(|child| child.size_bytes).sum();
        assert_eq!(sum, node.size_bytes, "the children of {} add up", node.path);
        count += children.iter().map(count_and_check).sum::<usize>();
    }
    count
}

async fn wait_for_no_threads(backend: &TestBackend) {
    let started = Instant::now();
    while backend.state.disk().threads_alive() > 0 {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the scan thread ended in time"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---- drives ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drives_are_listed_by_letter_with_a_label_and_a_used_size() {
    let backend = start_with(
        vec![
            drive('d', Path::new("D:\\"), "Data", 1000, 250),
            drive('c', Path::new("C:\\"), "", 2000, 1500),
        ],
        FakeDirs::default(),
    )
    .await;
    // The fake lists in the order it was given; the sorting is the real source's job (unit
    // tested). What the route adds is the plain list of `Drive`.
    let drives: Vec<Drive> = backend.get("/drives").await;
    assert_eq!(drives.len(), 2);
    let c = drives.iter().find(|d| d.id == "drive-c").unwrap();
    assert_eq!((c.letter.as_str(), c.label.as_str()), ("C", "Local Disk"));
    assert_eq!((c.total_bytes, c.used_bytes), (2000, 1500));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_drives_is_an_empty_list() {
    let backend = start_with(vec![], FakeDirs::default()).await;
    let drives: Vec<Drive> = backend.get("/drives").await;
    assert!(drives.is_empty());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_source_lists_this_pcs_fixed_drives() {
    let backend = TestBackend::start().await;
    let drives: Vec<Drive> = backend.get("/drives").await;
    assert!(!drives.is_empty());
    assert!(
        drives
            .iter()
            .all(|d| d.total_bytes > 0 && !d.label.is_empty())
    );
    backend.stop().await;
}

// ---- scans ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scan_ends_done_with_exact_sizes_and_no_tree_in_its_frames() {
    let temp = basic_tree();
    let backend = start_basic(temp.path()).await;
    let mut stream = backend.stream().await;

    let (status, started) = start_scan(&backend, "drive-t").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(started.status, ScanStatus::Scanning);
    assert!(started.root.is_none());
    assert!(started.id.starts_with("scan-"));

    let done = final_frame(&mut stream, &started.id).await;
    assert_eq!(done.status, ScanStatus::Done);
    assert_eq!(done.progress, 1.0);
    assert_eq!(done.files_seen, 75);
    assert!(done.root.is_none(), "a frame never carries the tree");
    assert!(done.not_accessible.is_none());

    let (status, body) = get_scan(&backend, &started.id).await;
    assert_eq!(status, StatusCode::OK);
    let scan = scan_of(&body);
    assert_eq!(
        (scan.status, scan.progress, scan.files_seen),
        (ScanStatus::Done, 1.0, 75)
    );
    let root = scan.root.expect("a done scan has its tree");
    assert_eq!((root.name.as_str(), root.path.as_str()), ("T:", "T:\\"));
    assert_eq!(root.size_bytes, 11_305);
    count_and_check(&root);
    assert_eq!(find(&root, "a").size_bytes, 600);
    assert_eq!(find(&root, "b").size_bytes, 700);
    assert_eq!(find(&root, "big.bin").path, "T:\\big.bin");
    assert_eq!(
        find(&root, "big.bin").category,
        Some(rain_types::FileCategory::Other)
    );
    assert_eq!(
        find(&root, "notes.TXT").category,
        Some(rain_types::FileCategory::Document)
    );
    // Below the first level a folder shows 12 children; the other 58 files are one node.
    let c = find(find(&root, "b"), "c");
    let kids = c.children.as_ref().unwrap();
    assert_eq!(kids.len(), 13);
    let other = kids.last().unwrap();
    assert_eq!(other.kind, NodeKind::Other);
    assert_eq!(
        (other.name.as_str(), other.size_bytes),
        ("other (58 items)", 580)
    );
    assert_eq!(other.path, "T:\\b\\c\\*");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_that_connects_after_the_end_still_finds_the_final_state_by_asking() {
    let temp = basic_tree();
    let backend = start_basic(temp.path()).await;
    let (_, started) = start_scan(&backend, "drive-t").await;
    // Wait without a stream until the scan has ended.
    let begun = Instant::now();
    loop {
        let (_, body) = get_scan(&backend, &started.id).await;
        if scan_of(&body).status == ScanStatus::Done {
            break;
        }
        assert!(begun.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // A late client misses the final frame and reads the same state from the route.
    let mut stream = backend.stream().await;
    let (_, again) = get_scan(&backend, &started.id).await;
    assert_eq!(scan_of(&again).status, ScanStatus::Done);
    let (event, frame) = stream.expect_event().await;
    assert_eq!((event.as_str(), frame.topic.as_str()), ("hello", "hello"));
    let (event, _) = stream.expect_event().await;
    assert_eq!(event, "resync", "the client is told to refetch on connect");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frames_are_small_slow_monotonic_and_the_final_one_comes_once_and_last() {
    let backend = start_slow().await;
    let mut stream = backend.stream().await;
    let (_, started) = start_scan(&backend, "drive-s").await;

    let mut frames: Vec<(Instant, DiskScan)> = Vec::new();
    loop {
        let frame = stream.expect_topic("scan").await;
        assert!(
            frame.payload.to_string().len() < 1024,
            "a frame is about 200 bytes"
        );
        let scan = scan_of(&frame.payload);
        let last = scan.status != ScanStatus::Scanning;
        frames.push((Instant::now(), scan));
        if last {
            break;
        }
    }
    assert!(
        frames.len() >= 4,
        "a scan of several seconds reports progress"
    );
    assert!(frames.iter().all(|(_, scan)| scan.id == started.id));
    let (last, running) = frames.split_last().unwrap();
    assert_eq!(last.1.status, ScanStatus::Done);
    assert!(
        running
            .iter()
            .all(|(_, scan)| scan.status == ScanStatus::Scanning && scan.progress < 1.0)
    );
    assert!(
        frames
            .windows(2)
            .all(|pair| pair[1].1.progress >= pair[0].1.progress),
        "progress never decreases"
    );
    assert!(
        frames
            .windows(2)
            .all(|pair| pair[1].1.files_seen >= pair[0].1.files_seen)
    );
    // At most 4 progress frames a second: any five progress frames in a row span nearly a second.
    // The final frame is not counted: section 05 lets it come at once, right after a progress
    // frame (it has its own assertions above: once, and last).
    for window in running.windows(5) {
        assert!(
            window[4].0 - window[0].0 >= Duration::from_millis(900),
            "at most four frames a second"
        );
    }
    // No second final frame follows: the next topic frame, if any, is not of this scan.
    assert_eq!(last.1.files_seen, 3_000);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_ends_the_scan_at_once_and_a_new_scan_can_start() {
    let backend = start_slow().await;
    let mut stream = backend.stream().await;
    let (_, started) = start_scan(&backend, "drive-s").await;

    let (status, body) = backend
        .send("DELETE", &format!("/scans/{}", started.id), None)
        .await;
    assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
    let cancelled = tokio::time::timeout(
        Duration::from_millis(500),
        final_frame(&mut stream, &started.id),
    )
    .await
    .expect("the cancelled frame arrives at once");
    assert_eq!(cancelled.status, ScanStatus::Cancelled);

    let (status, _) = backend
        .send("DELETE", &format!("/scans/{}", started.id), None)
        .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a second cancel does nothing"
    );
    let (_, body) = get_scan(&backend, &started.id).await;
    let scan = scan_of(&body);
    assert_eq!(scan.status, ScanStatus::Cancelled);
    assert!(scan.root.is_none());

    wait_for_no_threads(&backend).await;
    let (status, second) = start_scan(&backend, "drive-s").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(second.id, started.id);
    let next = next_scan(&mut stream).await;
    assert_eq!(
        next.id, second.id,
        "the first frame after the cancel is of the new scan"
    );
    let (status, _) = backend.send("DELETE", "/scans/scan-nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_scan_of_the_same_drive_is_the_running_one_and_another_drive_cancels_it() {
    let dirs = slow_tree().dir("U:\\", vec![file("x", 1)]);
    let backend = start_with(
        vec![
            drive('s', Path::new("S:\\"), "", 1_000_000, 1_000_000),
            drive('u', Path::new("U:\\"), "", 1_000_000, 1_000_000),
        ],
        dirs,
    )
    .await;
    let mut stream = backend.stream().await;
    let (_, first) = start_scan(&backend, "drive-s").await;
    let (status, again) = start_scan(&backend, "drive-s").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again.id, first.id);

    let (status, other) = start_scan(&backend, "drive-u").await;
    assert_eq!(status, StatusCode::CREATED);
    let old = final_frame(&mut stream, &first.id).await;
    assert_eq!(old.status, ScanStatus::Cancelled);
    assert_eq!(
        final_frame(&mut stream, &other.id).await.status,
        ScanStatus::Done
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_windows_refuses_is_counted_and_a_vanished_one_is_not() {
    let dirs = FakeDirs::default()
        .dir("D:\\", vec![folder("ok"), folder("locked"), folder("gone")])
        .dir("D:\\ok", vec![file("x", 50)])
        .failing("D:\\locked", io::ErrorKind::PermissionDenied)
        .failing("D:\\gone", io::ErrorKind::NotFound);
    let backend = start_with(vec![drive('d', Path::new("D:\\"), "", 1000, 500)], dirs).await;
    let done = scan_to_the_end(&backend, "drive-d").await;
    assert_eq!(done.status, ScanStatus::Done);
    assert_eq!(done.not_accessible, Some(1));
    let (_, body) = get_scan(&backend, &done.id).await;
    let root = scan_of(&body).root.unwrap();
    assert_eq!(root.size_bytes, 50);
    let locked = find(&root, "locked");
    assert_eq!((locked.denied, locked.size_bytes), (Some(true), 0));
    count_and_check(&root);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drive_whose_root_cannot_be_read_ends_failed_with_a_short_text() {
    let dirs = FakeDirs::default().failing("T:\\", io::ErrorKind::PermissionDenied);
    let backend = start_with(vec![drive('t', Path::new("T:\\"), "", 1000, 500)], dirs).await;
    let failed = scan_to_the_end(&backend, "drive-t").await;
    assert_eq!(failed.status, ScanStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("Drive T: could not be read"));
    let (_, body) = get_scan(&backend, &failed.id).await;
    assert!(scan_of(&body).root.is_none());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_junction_is_not_followed() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    File::create(real.join("x")).unwrap().set_len(100).unwrap();
    // Test only: `mklink /J` makes a junction without administrator rights. `hidden_command` is
    // the one allowed way to start a helper program.
    let output = rain_backend::platform::process::hidden_command("cmd.exe")
        .args(["/c", "mklink", "/J"])
        .arg(temp.path().join("link"))
        .arg(&real)
        .output()
        .await;
    if !output.is_ok_and(|out| out.status.success()) {
        eprintln!("ignored: this PC does not allow mklink /J");
        return;
    }
    let backend = start_basic(temp.path()).await;
    let done = scan_to_the_end(&backend, "drive-t").await;
    let (_, body) = get_scan(&backend, &done.id).await;
    let root = scan_of(&body).root.unwrap();
    assert_eq!(root.size_bytes, 100, "the junction target is counted once");
    assert!(
        root.children
            .as_ref()
            .unwrap()
            .iter()
            .all(|child| child.name != "link")
    );
    // Remove the junction itself (never its target) before the temp folder is cleaned.
    std::fs::remove_dir(temp.path().join("link")).unwrap();
    backend.stop().await;
}

// ---- the node route ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_is_fetched_whole_by_its_path() {
    let temp = basic_tree();
    let backend = start_basic(temp.path()).await;
    let done = scan_to_the_end(&backend, "drive-t").await;

    let (status, body) = node(&backend, &done.id, "T:\\b\\c").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let c = node_of(&body);
    assert_eq!((c.path.as_str(), c.size_bytes), ("T:\\b\\c", 700));
    let kids = c.children.as_ref().unwrap();
    assert_eq!(
        kids.len(),
        65,
        "the folder asked for shows 64 files and one other node"
    );
    let other = kids.last().unwrap();
    assert_eq!(
        (other.name.as_str(), other.size_bytes),
        ("other (6 items)", 60)
    );
    count_and_check(&c);

    // The case of the letters does not matter; the answer spells the path as the scan does.
    let (status, body) = node(&backend, &done.id, "t:/B/C/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(node_of(&body).path, "T:\\b\\c");
    let (status, body) = node(&backend, &done.id, "T:\\").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(node_of(&body).size_bytes, 11_305);

    for (path, status, reason) in [
        ("T:\\big.bin", StatusCode::BAD_REQUEST, "notADirectory"),
        ("T:\\b\\c\\*", StatusCode::BAD_REQUEST, "notADirectory"),
        ("T:\\*", StatusCode::BAD_REQUEST, "notADirectory"),
        ("T:\\nope", StatusCode::NOT_FOUND, "unknownPath"),
        ("U:\\b", StatusCode::NOT_FOUND, "unknownPath"),
    ] {
        let (got, body) = node(&backend, &done.id, path).await;
        assert_eq!(
            (got, body["reason"].as_str()),
            (status, Some(reason)),
            "{path}"
        );
    }
    let (status, body) = backend
        .send("GET", &format!("/scans/{}/node", done.id), None)
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("badQuery"))
    );
    let (status, body) = node(&backend, "scan-nope", "T:\\").await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownScan"))
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_of_a_scan_that_is_still_running_is_not_available() {
    let backend = start_slow().await;
    let (_, started) = start_scan(&backend, "drive-s").await;
    let (status, body) = node(&backend, &started.id, "S:\\d1").await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("scanNotDone"))
    );
    backend.stop().await;
}

/// This test pauses the clock, which only works on a single-threaded runtime.
#[tokio::test]
async fn the_tree_goes_ten_minutes_after_the_last_read_and_a_delete_frees_it_at_once() {
    let temp = basic_tree();
    let backend = start_basic(temp.path()).await;
    let done = scan_to_the_end(&backend, "drive-t").await;
    tokio::time::pause();

    // A read at minute 9 moves the deadline: the tree is still there at minute 11.
    tokio::time::advance(Duration::from_secs(9 * 60)).await;
    let (_, body) = get_scan(&backend, &done.id).await;
    assert!(scan_of(&body).root.is_some());
    tokio::time::advance(Duration::from_secs(2 * 60)).await;
    let (status, _) = node(&backend, &done.id, "T:\\b").await;
    assert_eq!(status, StatusCode::OK);

    // Ten minutes without a read: the tree is gone, the record stays.
    tokio::time::advance(Duration::from_secs(10 * 60 + 1)).await;
    let (status, body) = node(&backend, &done.id, "T:\\b").await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("scanExpired"))
    );
    let (status, body) = get_scan(&backend, &done.id).await;
    let scan = scan_of(&body);
    assert_eq!((status, scan.status), (StatusCode::OK, ScanStatus::Done));
    assert!(scan.root.is_none());

    // A delete frees a tree at once and keeps the record.
    tokio::time::resume();
    let again = scan_to_the_end(&backend, "drive-t").await;
    let (status, _) = backend
        .send("DELETE", &format!("/scans/{}", again.id), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = node(&backend, &again.id, "T:\\b").await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("scanExpired"))
    );
    let (_, body) = get_scan(&backend, &again.id).await;
    assert_eq!(scan_of(&body).status, ScanStatus::Done);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_scan_drops_the_old_tree() {
    let temp = basic_tree();
    let backend = start_basic(temp.path()).await;
    let first = scan_to_the_end(&backend, "drive-t").await;
    let second = scan_to_the_end(&backend, "drive-t").await;
    assert_ne!(first.id, second.id);
    let (status, body) = node(&backend, &first.id, "T:\\b").await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("scanExpired"))
    );
    let (status, _) = node(&backend, &second.id, "T:\\b").await;
    assert_eq!(status, StatusCode::OK);
    backend.stop().await;
}

// ---- sizes ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_big_drive_gives_small_answers_with_exact_sums() {
    // 40 000 files in 4 000 folders, in memory (more folders than the budget of 2 000 nodes).
    let mut dirs =
        FakeDirs::default().dir("B:\\", (0..40).map(|g| folder(&format!("g{g}"))).collect());
    for g in 0..40u64 {
        dirs = dirs.dir(
            &format!("B:\\g{g}"),
            (0..100).map(|f| folder(&format!("f{f}"))).collect(),
        );
        for f in 0..100u64 {
            let files = (0..10u64)
                .map(|n| file(&format!("file{n}.dat"), (g * 31 + f * 7 + n) % 977 + 1))
                .collect();
            dirs = dirs.dir(&format!("B:\\g{g}\\f{f}"), files);
        }
    }
    let backend = start_with(
        vec![drive('b', Path::new("B:\\"), "", 1 << 40, 1 << 39)],
        dirs,
    )
    .await;
    let done = scan_to_the_end(&backend, "drive-b").await;
    assert_eq!(done.files_seen, 40_000);

    let (_, body) = get_scan(&backend, &done.id).await;
    assert!(
        body.to_string().len() < 700_000,
        "the tree answer stays small"
    );
    let root = scan_of(&body).root.unwrap();
    assert!(count_and_check(&root) <= 2000);

    // The biggest folder, fetched whole.
    let biggest = &root.children.as_ref().unwrap()[0];
    let (status, body) = node(&backend, &done.id, &biggest.path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.to_string().len() < 700_000);
    let folder_node = node_of(&body);
    assert_eq!(folder_node.size_bytes, biggest.size_bytes);
    assert!(count_and_check(&folder_node) <= 2000);
    backend.stop().await;
}

// ---- game mode and the end of the backend ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_mode_refuses_a_scan_and_cancels_a_running_one() {
    let backend = start_slow().await;
    backend.state.set_game_mode_active(true);
    let (status, body) = backend
        .send("POST", "/scans", Some(json!({ "driveId": "drive-s" })))
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("gameMode"))
    );
    backend.state.set_game_mode_active(false);

    let mut stream = backend.stream().await;
    let (_, started) = start_scan(&backend, "drive-s").await;
    assert!(backend.state.disk().is_scanning());
    backend.state.disk().cancel_running();
    assert_eq!(
        final_frame(&mut stream, &started.id).await.status,
        ScanStatus::Cancelled
    );
    assert!(!backend.state.disk().is_scanning());
    backend.state.disk().cancel_running(); // nothing runs: nothing happens
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_unknown_drive_and_a_bad_body_are_told_apart() {
    let backend = start_slow().await;
    let (status, body) = backend
        .send("POST", "/scans", Some(json!({ "driveId": "drive-z" })))
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownDrive"))
    );
    let (status, body) = backend
        .send(
            "POST",
            "/scans",
            Some(json!({ "driveId": "drive-s", "x": 1 })),
        )
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("badBody"))
    );
    let (status, _) = backend.send("POST", "/scans", Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_the_backend_ends_a_running_scan_without_a_scan_frame_after_the_lifecycle_frame() {
    let mut backend = start_slow().await;
    let mut stream = backend.stream().await;
    let (_, started) = start_scan(&backend, "drive-s").await;
    // One progress frame, so that the stream has carried scan frames before the stop.
    assert_eq!(next_scan(&mut stream).await.id, started.id);

    backend.request_stop("stop").await;
    let rest = stream.rest().await;
    let lifecycle = rest
        .iter()
        .position(|(_, frame)| frame.topic == "lifecycle");
    let lifecycle = lifecycle.expect("the stream ends with a lifecycle frame");
    assert!(
        rest[lifecycle..]
            .iter()
            .all(|(_, frame)| frame.topic != "scan"),
        "no scan frame after the lifecycle frame"
    );
    assert!(
        rest.iter()
            .all(|(_, frame)| scan_status(frame) != Some(ScanStatus::Cancelled)),
        "the stop publishes no cancelled frame"
    );
    backend.wait_exited().await;
}

fn scan_status(frame: &rain_types::Frame) -> Option<ScanStatus> {
    (frame.topic == "scan").then(|| scan_of(&frame.payload).status)
}

// ---- reveal ----

#[derive(Default)]
struct FakeRevealer {
    shown: Mutex<Vec<PathBuf>>,
    fail: Option<ComError>,
}

impl Revealer for FakeRevealer {
    fn reveal(&self, path: PathBuf) -> BoxFuture<'static, Result<(), ComError>> {
        self.shown.lock().expect("the list").push(path);
        Box::pin(std::future::ready(self.fail.clone().map_or(Ok(()), Err)))
    }
}

/// Answers like the file system, except that a path with `slow` in it takes 3 seconds.
struct SlowProbe;

impl PathProbe for SlowProbe {
    fn stat(&self, path: &Path, _follow_links: bool) -> Probe {
        if path.to_string_lossy().contains("slow") {
            nap(Duration::from_secs(3));
        }
        match std::fs::metadata(path) {
            Ok(found) if found.is_dir() => Probe::Dir,
            Ok(_) => Probe::File,
            Err(_) => Probe::Missing,
        }
    }
}

async fn start_with_shell(revealer: Arc<FakeRevealer>) -> TestBackend {
    TestBackend::builder()
        .with::<dyn Revealer>(revealer)
        .with::<dyn PathProbe>(Arc::new(SlowProbe))
        .start()
        .await
}

async fn reveal(backend: &TestBackend, path: &str) -> (StatusCode, Value) {
    backend
        .send("POST", "/shell/reveal", Some(json!({ "path": path })))
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reveal_shows_an_existing_item_and_refuses_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("a.txt");
    std::fs::write(&existing, b"x").unwrap();
    let fake = Arc::new(FakeRevealer::default());
    let backend = start_with_shell(Arc::clone(&fake)).await;

    let (status, body) = reveal(&backend, &existing.to_string_lossy()).await;
    assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
    let shown = fake.shown.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert!(
        shown[0]
            .to_string_lossy()
            .eq_ignore_ascii_case(&existing.to_string_lossy())
    );
    // A folder is shown too.
    let (status, _) = reveal(&backend, &temp.path().to_string_lossy()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let missing = temp.path().join("gone.txt");
    let (status, body) = reveal(&backend, &missing.to_string_lossy()).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("pathMissing"))
    );
    let (status, body) = reveal(&backend, "\\\\srv\\share\\x").await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("networkPath"))
    );
    for bad in ["C:\\a\\..\\b", "relative\\x", "C:\\a*", ""] {
        let (status, body) = reveal(&backend, bad).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("badPath")),
            "{bad:?}"
        );
    }
    let (status, body) = backend
        .send(
            "POST",
            "/shell/reveal",
            Some(json!({ "path": "C:\\", "x": 1 })),
        )
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("badBody"))
    );
    assert_eq!(
        fake.shown.lock().unwrap().len(),
        2,
        "the shell was asked only for the two that exist"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_explorer_call_is_a_500_and_a_busy_one_a_503() {
    let temp = tempfile::tempdir().unwrap();
    let here = temp.path().to_string_lossy().into_owned();
    for (error, status, reason) in [
        (
            ComError::Os { code: 5 },
            StatusCode::INTERNAL_SERVER_ERROR,
            "revealFailed",
        ),
        (ComError::Busy, StatusCode::SERVICE_UNAVAILABLE, "comBusy"),
    ] {
        let fake = Arc::new(FakeRevealer {
            fail: Some(error),
            ..FakeRevealer::default()
        });
        let backend = start_with_shell(fake).await;
        let (got, body) = reveal(&backend, &here).await;
        assert_eq!((got, body["reason"].as_str()), (status, Some(reason)));
        backend.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_location_that_does_not_answer_in_two_seconds_is_a_503() {
    let temp = tempfile::tempdir().unwrap();
    let slow = temp.path().join("slow");
    std::fs::create_dir(&slow).unwrap();
    let backend = start_with_shell(Arc::default()).await;
    let started = Instant::now();
    let (status, body) = reveal(&backend, &slow.to_string_lossy()).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("pathSlow"))
    );
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(1800) && took < Duration::from_millis(2900),
        "{took:?}"
    );
    backend.stop().await;
}

// ---- path check ----

async fn check(backend: &TestBackend, paths: &[&str]) -> (StatusCode, Value) {
    backend
        .send("POST", "/paths/check", Some(json!({ "paths": paths })))
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_path_check_tells_folders_files_and_missing_paths_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let folder_path = temp.path().to_string_lossy().into_owned();
    let file_path = temp.path().join("a.txt");
    std::fs::write(&file_path, b"x").unwrap();
    let file_text = file_path.to_string_lossy().into_owned();
    let missing = format!("{folder_path}\\nope");
    let slow = temp.path().join("slow");
    std::fs::create_dir(&slow).unwrap();
    let slow_text = slow.to_string_lossy().into_owned();
    let backend = start_with_shell(Arc::default()).await;

    let paths = [
        folder_path.as_str(),
        file_text.as_str(),
        missing.as_str(),
        "relative\\path",
        "C:\\wild*card",
        "\\\\no-such-server\\share",
        "  ",
    ];
    let (status, body) = check(&backend, &paths).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer: Vec<PathCheck> = serde_json::from_value(body).unwrap();
    assert_eq!(answer.len(), paths.len());
    for (item, sent) in answer.iter().zip(paths) {
        assert_eq!(item.path, sent, "the sent text comes back unchanged");
    }
    assert_eq!(
        (answer[0].exists, answer[0].kind),
        (true, Some(PathKind::Dir))
    );
    assert_eq!(
        (answer[1].exists, answer[1].kind),
        (true, Some(PathKind::File))
    );
    assert!(
        answer[2..]
            .iter()
            .all(|item| !item.exists && item.kind.is_none())
    );

    // A slow location is simply missing, and does not hold up the others for long.
    let started = Instant::now();
    let (_, body) = check(&backend, &[slow_text.as_str(), folder_path.as_str()]).await;
    let answer: Vec<PathCheck> = serde_json::from_value(body).unwrap();
    assert!(!answer[0].exists && answer[1].exists);
    assert!(started.elapsed() < Duration::from_millis(2900));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_path_check_expands_environment_variables_and_keeps_its_limits() {
    let backend = TestBackend::start().await;
    // `SystemRoot` exists on every Windows PC; the text sent is echoed as it was typed.
    let (status, body) = check(&backend, &["%SystemRoot%"]).await;
    assert_eq!(status, StatusCode::OK);
    let answer: Vec<PathCheck> = serde_json::from_value(body).unwrap();
    assert_eq!(
        (answer[0].path.as_str(), answer[0].exists, answer[0].kind),
        ("%SystemRoot%", true, Some(PathKind::Dir))
    );

    let many = vec!["C:\\"; 33];
    let (status, body) = check(&backend, &many).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("tooMany"))
    );
    let long = "C".repeat(1025);
    let (status, body) = check(&backend, &[long.as_str()]).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("tooLong"))
    );
    let (status, body) = backend
        .send("POST", "/paths/check", Some(json!({ "paths": [], "x": 1 })))
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::BAD_REQUEST, Some("badBody"))
    );
    let (status, body) = check(&backend, &[]).await;
    assert_eq!((status, body), (StatusCode::OK, json!([])));
    backend.stop().await;
}
