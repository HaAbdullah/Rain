//! The launcher on the test harness: apps, ids, pins, pools, search, recent, launching, icons.
//! Feature section 01, tests of section 10.2.
//!
//! No test starts a program: the harness installs `FakeOpener`, which only records what it was
//! asked. The installed apps come from `FakeAppSource` (made-up apps of
//! `tests/fixtures/launcher/apps-raw.json`).

mod common;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::fakes::{
    FakeAppSource, FakeClock, FakeIconRenderer, FakeOpener, FakePathProbe, FakeProcesses, raw_app,
};
use common::{PATIENCE, StreamReader, TestBackend, TestBackendBuilder};
use hyper::{Method, StatusCode};
use rain_backend::core::launch_contract::{OpenError, OpenRequest};
use rain_backend::modules::launcher::{AppSource, IconRenderer, PathProbe, SourceError};
use rain_backend::platform::process::ProcessLister;
use rain_types::limits::API_PREFIX;
use rain_types::{AppEntry, Health, LaunchRun, Pool, RunStatus, SearchItem, SourceState, Status};
use serde_json::{Value, json};

// ---- helpers ----

struct Setup {
    backend: TestBackend,
    source: Arc<FakeAppSource>,
    opener: Arc<FakeOpener>,
}

fn builder() -> (
    TestBackendBuilder,
    Arc<FakeAppSource>,
    Arc<FakeOpener>,
    Arc<FakeClock>,
) {
    let source = Arc::new(FakeAppSource::from_fixture());
    let opener = Arc::new(FakeOpener::default());
    let clock = Arc::new(FakeClock::new(SystemTime::now()));
    let builder = TestBackend::builder()
        .fixture("launcher")
        .with::<dyn AppSource>(source.clone())
        .opener(opener.clone())
        .clock(clock.clone());
    (builder, source, opener, clock)
}

async fn setup() -> (Setup, Arc<FakeClock>) {
    let (builder, source, opener, clock) = builder();
    let backend = builder.start().await;
    (
        Setup {
            backend,
            source,
            opener,
        },
        clock,
    )
}

async fn apps(backend: &TestBackend) -> Vec<AppEntry> {
    backend.get("/apps").await
}

fn find<'a>(list: &'a [AppEntry], id: &str) -> &'a AppEntry {
    list.iter()
        .find(|app| app.id == id)
        .unwrap_or_else(|| panic!("{id} is in the list"))
}

fn ids(list: &[AppEntry]) -> Vec<&str> {
    list.iter().map(|app| app.id.as_str()).collect()
}

/// Waits until `check` is true, at most `PATIENCE` (a hand edit reaches the backend after the
/// 300 ms debounce of the config hub).
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !check().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn launch_run(backend: &TestBackend, body: Value) -> (StatusCode, Value) {
    backend.send("POST", "/launches", Some(body)).await
}

/// Reads `launch` frames until the run with this id has ended.
async fn final_run(stream: &mut StreamReader, id: &str) -> LaunchRun {
    loop {
        let frame = stream.expect_topic("launch").await;
        let run: LaunchRun = serde_json::from_value(frame.payload).expect("a LaunchRun frame");
        if run.id == id && run.status != RunStatus::Running {
            return run;
        }
    }
}

/// Polls `GET /launches/{id}` until the run has ended.
async fn wait_for_end(backend: &TestBackend, id: &str) -> LaunchRun {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let run: LaunchRun = backend.get(&format!("/launches/{id}")).await;
        if run.status != RunStatus::Running {
            return run;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the run did not end"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn read_data(backend: &TestBackend, name: &str) -> String {
    std::fs::read_to_string(backend.data_dir.join(name)).expect("the data file exists")
}

fn write_data(backend: &TestBackend, name: &str, text: &str) {
    std::fs::write(backend.data_dir.join(name), text).expect("the data file was written");
}

fn reason(body: &Value) -> &str {
    body["reason"].as_str().unwrap_or_default()
}

// ---- the app list ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_app_list_has_stable_readable_ids_kinds_paths_and_icon_addresses() {
    let (s, _) = setup().await;
    let list = apps(&s.backend).await;
    let names: Vec<&str> = list.iter().map(|app| app.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Control Panel",
            "Fake Calculator",
            "Fake Chat",
            "Fake Editor",
            "Fake Music",
            "Fake Notes",
            "Fake Paint",
            "Fake Terminal",
            "Installer Helper",
            "Launcher",
            "Launcher",
            "Launcher",
            "Photos",
            "Photos",
            "Services",
        ]
    );
    let editor = find(&list, "app-fake-editor");
    assert_eq!(editor.kind, rain_types::AppKind::Program);
    assert_eq!(
        editor.exe_path.as_deref(),
        Some(r"C:\Fake\Editor\editor.exe")
    );
    assert!(editor.keywords.contains(&"code".to_owned()), "{editor:?}");
    assert!(editor.pinned && !editor.running && editor.last_launched_at.is_none());
    let calculator = find(&list, "app-fake-calculator");
    assert_eq!(calculator.kind, rain_types::AppKind::Store);
    assert_eq!(calculator.exe_path, None);
    assert_eq!(
        find(&list, "app-control-panel").kind,
        rain_types::AppKind::Other
    );
    for app in &list {
        let value = &app.icon.value;
        let (path, version) = value
            .split_once("?v=")
            .expect("an icon address with a version");
        assert_eq!(path, format!("media/app-icon/{}", app.id));
        assert_eq!(version.len(), 8);
        assert_eq!(app.icon.fallback.as_deref(), Some(app.name.as_str()));
    }
    assert_eq!(
        ids(&list)[9..12],
        ["app-launcher", "app-launcher-2", "app-launcher-3"]
    );
    assert_eq!(&ids(&list)[12..14], ["app-photos", "app-photos-2"]);
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_id_file_is_written_once_and_a_second_scan_does_not_rewrite_it() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let first = read_data(&s.backend, "app-ids.toml");
    assert!(first.contains("# app-ids.toml: made-up ids for the tests."));
    assert_eq!(first.matches("[[app]]").count(), 15, "2 old and 13 new");
    let (status, _) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_data(&s.backend, "app-ids.toml"), first);
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ids_survive_renames_reordering_and_removal_and_a_new_app_gets_a_new_id() {
    let (s, _) = setup().await;
    let before = apps(&s.backend).await;
    let file_before = read_data(&s.backend, "app-ids.toml");

    // Rename one app, reverse the list, drop one app, add a new one.
    let mut items = s.source.items();
    items.reverse();
    for item in &mut items {
        if item.name == "Fake Paint" {
            item.name = "Fake Paint Pro".into();
        }
    }
    items.retain(|item| item.name != "Fake Music");
    items.push(raw_app("Brand New", "Fake.Brand_abc123!App", "", ""));
    s.source.set_items(items);
    let (status, body) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!(status, StatusCode::OK);
    let after: Vec<AppEntry> = serde_json::from_value(body).unwrap();

    assert_eq!(find(&after, "app-fake-paint").name, "Fake Paint Pro");
    assert!(!ids(&after).contains(&"app-fake-music"));
    assert!(ids(&after).contains(&"app-brand-new"));
    for app in &before {
        if app.id != "app-fake-music" {
            assert!(
                ids(&after).contains(&app.id.as_str()),
                "{} kept its id",
                app.id
            );
        }
    }
    let file_after = read_data(&s.backend, "app-ids.toml");
    assert!(
        file_after.starts_with(&file_before),
        "only new entries are appended"
    );
    assert_eq!(file_after.matches("[[app]]").count(), 16);

    // The removed app comes back with its old id.
    let mut items = s.source.items();
    items.push(raw_app("Fake Music", "Fake.Music_abc123!Player", "", ""));
    s.source.set_items(items);
    let (_, body) = s.backend.send("POST", "/apps/scan", None).await;
    let back: Vec<AppEntry> = serde_json::from_value(body).unwrap();
    assert!(ids(&back).contains(&"app-fake-music"));
    assert_eq!(
        read_data(&s.backend, "app-ids.toml")
            .matches("[[app]]")
            .count(),
        16
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_id_file_still_gives_ids_and_shows_in_health_and_is_not_rewritten() {
    let source = Arc::new(FakeAppSource::from_fixture());
    let backend = TestBackend::builder()
        .file("app-ids.toml", "id = = broken\n")
        .with::<dyn AppSource>(source)
        .start()
        .await;
    let list = apps(&backend).await;
    assert!(
        ids(&list).contains(&"app-fake-editor"),
        "provisional ids are made the same way"
    );
    let health: Health = backend.get("/health").await;
    assert!(
        health
            .config_issues
            .iter()
            .any(|issue| issue.file == "app-ids.toml")
    );
    assert_eq!(read_data(&backend, "app-ids.toml"), "id = = broken\n");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_read_waits_for_a_slow_first_scan() {
    let (builder, source, _, _) = builder();
    source.delay_reads(Duration::from_millis(300));
    let backend = builder.start().await;
    let list = apps(&backend).await;
    assert_eq!(list.len(), 15, "the full list, not an empty one");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_scan_keeps_the_last_list_and_tells_health() {
    let (s, _) = setup().await;
    assert_eq!(apps(&s.backend).await.len(), 15);
    s.source.fail_with(Some(SourceError::Failed));
    let (status, body) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reason(&body), "scanFailed");
    assert_eq!(apps(&s.backend).await.len(), 15, "the cache still answers");
    let health: Health = s.backend.get("/health").await;
    let apps_source = health
        .sources
        .iter()
        .find(|source| source.name == "apps")
        .expect("apps is reported");
    assert_eq!(apps_source.state, SourceState::Failed);
    assert_eq!(
        apps_source.message.as_deref(),
        Some("Showing the last known app list")
    );

    s.source.fail_with(None);
    let (status, _) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!(status, StatusCode::OK);
    let health: Health = s.backend.get("/health").await;
    assert!(
        health.sources.iter().all(|source| source.name != "apps"),
        "a source that works leaves the report"
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_any_cache_a_failing_scan_is_a_503_until_the_source_works() {
    let source = Arc::new(FakeAppSource::from_fixture());
    source.fail_with(Some(SourceError::Failed));
    let backend = TestBackend::builder()
        .with::<dyn AppSource>(source.clone())
        .start()
        .await;
    let (status, body) = backend.send("GET", "/apps", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reason(&body), "scanFailed");
    let (status, _) = backend.send("POST", "/apps/scan", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    source.fail_with(Some(SourceError::ComBusy));
    let (_, body) = backend.send("GET", "/apps", None).await;
    assert_eq!(reason(&body), "comBusy");

    source.fail_with(None);
    assert_eq!(apps(&backend).await.len(), 15);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_scan_that_finds_nothing_is_a_valid_empty_list() {
    let backend = TestBackend::start().await;
    assert!(apps(&backend).await.is_empty());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_old_list_is_scanned_again_by_a_read_and_not_before() {
    let (s, clock) = setup().await;
    apps(&s.backend).await;
    // Let the scan of the start end: a read joins a scan that is still running.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let calls = s.source.calls();
    apps(&s.backend).await;
    assert_eq!(s.source.calls(), calls, "a fresh list is not scanned again");

    clock.advance(Duration::from_secs(11 * 60));
    // A read that finds the list old starts a scan in the background. One that is still ending
    // from before is joined, so the read is repeated until a new scan has been made.
    let backend = &s.backend;
    let source = &s.source;
    eventually("a background rescan", || async {
        apps(backend).await;
        source.calls() > calls
    })
    .await;
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_apps_come_from_the_process_snapshot_and_a_launch_counts_for_eight_seconds() {
    let processes = Arc::new(FakeProcesses::default());
    let (builder, _, _, clock) = builder();
    let backend = builder
        .with::<dyn ProcessLister>(processes.clone())
        .start()
        .await;
    let list = apps(&backend).await;
    assert!(list.iter().all(|app| !app.running));

    // A snapshot is reused for 2 seconds.
    processes
        .0
        .lock()
        .unwrap()
        .push(r"c:\fake\editor\EDITOR.exe".into());
    let list = apps(&backend).await;
    assert!(
        !find(&list, "app-fake-editor").running,
        "the old snapshot is still used"
    );
    clock.advance(Duration::from_secs(3));
    let list = apps(&backend).await;
    assert!(find(&list, "app-fake-editor").running);
    assert!(!find(&list, "app-fake-paint").running);

    // A launched app counts as running for 8 seconds although no process shows.
    let (_, body) = launch_run(&backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    let run: LaunchRun = serde_json::from_value(body).unwrap();
    wait_for_end(&backend, &run.id).await;
    assert!(find(&apps(&backend).await, "app-fake-paint").running);
    clock.advance(Duration::from_secs(9));
    assert!(!find(&apps(&backend).await, "app-fake-paint").running);
    backend.stop().await;
}

// ---- pins ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pins_are_listed_in_file_order_and_pinning_is_idempotent_and_keeps_comments() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let pinned: Vec<SearchItem> = s.backend.get("/pinned").await;
    let ids_pinned: Vec<&str> = pinned.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        ids_pinned,
        ["app-fake-editor", "pool-web-dev", "app-fake-notes"]
    );
    assert!(pinned.iter().all(|item| item.score == 0));

    let original = read_data(&s.backend, "pins.toml");
    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-fake-paint",
            Some(json!({ "pinned": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let with_paint = read_data(&s.backend, "pins.toml");
    assert!(with_paint.starts_with(&original) && with_paint.contains("app-fake-paint"));
    assert!(find(&apps(&s.backend).await, "app-fake-paint").pinned);

    // Pinning again writes nothing.
    let modified = std::fs::metadata(s.backend.data_dir.join("pins.toml"))
        .unwrap()
        .modified()
        .unwrap();
    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-fake-paint",
            Some(json!({ "pinned": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(read_data(&s.backend, "pins.toml"), with_paint);
    assert_eq!(
        std::fs::metadata(s.backend.data_dir.join("pins.toml"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );

    // A new pin goes last; unpinning gives the original text back, byte for byte.
    let pinned: Vec<SearchItem> = s.backend.get("/pinned").await;
    assert_eq!(
        pinned.last().map(|item| item.id.as_str()),
        Some("app-fake-paint")
    );
    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-fake-paint",
            Some(json!({ "pinned": false })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(read_data(&s.backend, "pins.toml"), original);
    // Unpinning what is not pinned is fine too.
    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-nothing",
            Some(json!({ "pinned": false })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinning_an_unknown_item_or_with_a_bad_request_is_refused() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let (status, body) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-nothing",
            Some(json!({ "pinned": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(reason(&body), "unknownItem");
    // No project has this id, so it cannot be pinned.
    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/pinned/project/anything",
            Some(json!({ "pinned": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = s
        .backend
        .send("PUT", "/pinned/robot/x", Some(json!({ "pinned": true })))
        .await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::BAD_REQUEST, "unknownType")
    );
    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-fake-paint",
            Some(json!({ "pinned": true, "x": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pin_of_an_item_that_is_gone_is_skipped_in_the_list_but_stays_in_the_file() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let mut text = read_data(&s.backend, "pins.toml");
    text.push_str("\n[[pin]]\ntype = \"app\"\nid = \"app-gone\"\n");
    write_data(&s.backend, "pins.toml", &text);
    // The hand edit is seen after the debounce: the list still has the same three pins.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let pinned: Vec<SearchItem> = s.backend.get("/pinned").await;
    assert_eq!(pinned.len(), 3);
    // The next write keeps the dangling pin.
    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-fake-paint",
            Some(json!({ "pinned": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(read_data(&s.backend, "pins.toml").contains("app-gone"));
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pins_file_with_an_error_refuses_writes_and_keeps_the_last_good_list() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    write_data(
        &s.backend,
        "pins.toml",
        "[[pin]]\ntype = \"job\"\nid = \"x\"\n",
    );
    let backend = &s.backend;
    eventually("the issue shows", || async {
        let health: Health = backend.get("/health").await;
        health
            .config_issues
            .iter()
            .any(|issue| issue.file == "pins.toml")
    })
    .await;
    let pinned: Vec<SearchItem> = s.backend.get("/pinned").await;
    assert_eq!(pinned.len(), 3, "the last good value is used");
    let (status, body) = s
        .backend
        .send(
            "PUT",
            "/pinned/app/app-fake-paint",
            Some(json!({ "pinned": true })),
        )
        .await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::CONFLICT, "configBroken")
    );
    s.backend.stop().await;
}

// ---- pools ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pools_are_listed_in_file_order_with_their_pin() {
    let (s, _) = setup().await;
    let pools: Vec<Pool> = s.backend.get("/pools").await;
    let names: Vec<&str> = pools.iter().map(|pool| pool.name.as_str()).collect();
    assert_eq!(names, ["Web dev", "Reading"]);
    assert!(pools[0].pinned && !pools[1].pinned);
    assert_eq!(pools[0].icon.value, "Web dev");
    assert_eq!(pools[0].members[1].delay_ms, Some(200));
    assert_eq!(pools[0].members[1].args.as_deref(), Some("--here"));
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pool_is_created_updated_and_deleted_with_its_pin_and_history() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let draft = json!({ "name": "  Night shift ", "members": [{ "appId": "app-fake-editor" }, { "appId": "app-fake-paint" }] });
    let (status, body) = s.backend.send("POST", "/pools", Some(draft)).await;
    assert_eq!(status, StatusCode::CREATED);
    let created: Pool = serde_json::from_value(body).unwrap();
    assert_eq!(created.id, "pool-night-shift");
    assert_eq!(created.name, "Night shift");
    assert!(!created.pinned);
    assert!(
        read_data(&s.backend, "pools.toml").contains("# my first pool"),
        "comments stay"
    );

    // The same name gets a suffix.
    let draft = json!({ "name": "Night shift", "members": [{ "appId": "app-fake-editor" }] });
    let (_, body) = s.backend.send("POST", "/pools", Some(draft)).await;
    assert_eq!(body["id"], "pool-night-shift-2");

    // Pin it, then update: the id and the pin stay, a rename does not change the id.
    s.backend
        .send(
            "PUT",
            "/pinned/pool/pool-night-shift",
            Some(json!({ "pinned": true })),
        )
        .await;
    let draft = json!({ "id": "pool-night-shift", "name": "Late shift", "members": [{ "appId": "app-fake-notes" }] });
    let (status, body) = s
        .backend
        .send("PUT", "/pools/pool-night-shift", Some(draft))
        .await;
    assert_eq!(status, StatusCode::OK);
    let updated: Pool = serde_json::from_value(body).unwrap();
    assert_eq!(
        (updated.id.as_str(), updated.name.as_str()),
        ("pool-night-shift", "Late shift")
    );
    assert!(updated.pinned);
    assert_eq!(updated.members.len(), 1);

    // Run it once so that it has history, then delete it.
    let (_, run) = launch_run(
        &s.backend,
        json!({ "type": "pool", "id": "pool-night-shift" }),
    )
    .await;
    wait_for_end(&s.backend, run["id"].as_str().unwrap()).await;
    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    assert!(recent.iter().any(|item| item.id == "pool-night-shift"));

    let (status, _) = s
        .backend
        .send("DELETE", "/pools/pool-night-shift", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let pools: Vec<Pool> = s.backend.get("/pools").await;
    assert!(pools.iter().all(|pool| pool.id != "pool-night-shift"));
    assert!(!read_data(&s.backend, "pins.toml").contains("pool-night-shift"));
    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    assert!(recent.iter().all(|item| item.id != "pool-night-shift"));
    let rows: i64 = s
        .backend
        .state
        .db
        .call(|db| {
            db.query_row(
                "SELECT COUNT(*) FROM launcher_launch WHERE target_id = 'pool-night-shift'",
                [],
                |row| row.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(rows, 0);
    // Deleting twice, or an id that never existed, is fine.
    for id in ["pool-night-shift", "pool-never"] {
        let (status, _) = s
            .backend
            .send("DELETE", &format!("/pools/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_invalid_pool_is_refused_with_the_reason_and_text_of_the_rule() {
    let (s, _) = setup().await;
    let member = json!({ "appId": "app-fake-editor" });
    let cases = [
        (
            json!({ "name": " ", "members": [member] }),
            "emptyName",
            "A pool needs a name",
        ),
        (
            json!({ "name": "x".repeat(81), "members": [member] }),
            "nameTooLong",
            "A pool name can have at most 80 characters",
        ),
        (
            json!({ "name": "A", "members": [] }),
            "noMembers",
            "A pool needs at least one app",
        ),
        (
            json!({ "name": "A", "members": vec![member.clone(); 41] }),
            "tooManyMembers",
            "A pool can hold at most 40 apps",
        ),
        (
            json!({ "name": "A", "members": [{ "appId": "bad id" }] }),
            "badMember",
            "A pool member is not valid",
        ),
        (
            json!({ "name": "A", "members": [{ "appId": "a", "delayMs": 60001 }] }),
            "badMember",
            "A pool member is not valid",
        ),
        (
            json!({ "name": "A", "members": vec![json!({ "appId": "a", "delayMs": 60000 }); 11] }),
            "delaysTooLong",
            "The delays of a pool add up to more than 10 minutes",
        ),
        (
            json!({ "id": "pool-x", "name": "A", "members": [member] }),
            "idMismatch",
            "The pool id does not match",
        ),
    ];
    for (draft, expected_reason, text) in cases {
        let (status, body) = s.backend.send("POST", "/pools", Some(draft.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{draft}");
        assert_eq!(reason(&body), expected_reason, "{draft}");
        assert_eq!(body["message"], text);
    }
    // A PUT with another id in the body, and a PUT for a pool that does not exist.
    let draft = json!({ "id": "pool-other", "name": "A", "members": [member] });
    let (status, body) = s
        .backend
        .send("PUT", "/pools/pool-reading", Some(draft))
        .await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::BAD_REQUEST, "idMismatch")
    );
    let draft = json!({ "name": "A", "members": [member] });
    let (status, body) = s
        .backend
        .send("PUT", "/pools/pool-nothing", Some(draft))
        .await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::NOT_FOUND, "unknownPool")
    );
    // Unknown fields are refused.
    let (status, _) = s
        .backend
        .send(
            "POST",
            "/pools",
            Some(json!({ "name": "A", "members": [member], "pinned": true })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_of_the_pools_file_is_seen_and_a_broken_one_keeps_the_last_good_list() {
    let (s, _) = setup().await;
    let backend = &s.backend;
    write_data(
        backend,
        "pools.toml",
        "schema_version = 1\n\n[[pool]]\nid = \"pool-by-hand\"\nname = \"By hand\"\nmembers = [{ app = \"app-fake-editor\" }]\n",
    );
    eventually("the hand edit is seen", || async {
        let pools: Vec<Pool> = backend.get("/pools").await;
        pools.len() == 1 && pools[0].id == "pool-by-hand"
    })
    .await;

    write_data(
        backend,
        "pools.toml",
        "[[pool]]\nid = \"nope\"\nname = \"x\"\nmembers = []\n",
    );
    eventually("the issue shows", || async {
        let health: Health = backend.get("/health").await;
        health
            .config_issues
            .iter()
            .any(|issue| issue.file == "pools.toml")
    })
    .await;
    let pools: Vec<Pool> = backend.get("/pools").await;
    assert_eq!(pools[0].id, "pool-by-hand", "the last good list");
    let draft = json!({ "name": "New", "members": [{ "appId": "app-fake-editor" }] });
    let (status, body) = backend.send("POST", "/pools", Some(draft)).await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::CONFLICT, "configBroken")
    );
    let (status, body) = backend.send("DELETE", "/pools/pool-by-hand", None).await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::CONFLICT, "configBroken")
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pools_file_with_windows_line_ends_keeps_them() {
    let (s, _) = setup().await;
    let backend = &s.backend;
    let crlf = read_data(backend, "pools.toml")
        .replace("\r\n", "\n")
        .replace('\n', "\r\n");
    write_data(backend, "pools.toml", &crlf);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let draft = json!({ "name": "Windows", "members": [{ "appId": "app-fake-editor" }] });
    let (status, _) = backend.send("POST", "/pools", Some(draft)).await;
    assert_eq!(status, StatusCode::CREATED);
    let text = read_data(backend, "pools.toml");
    assert!(text.contains("Windows"));
    assert!(
        !text.replace("\r\n", "").contains('\n'),
        "every line end is still CRLF"
    );
    s.backend.stop().await;
}

// ---- launching ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launching_an_app_runs_one_step_publishes_frames_and_updates_history_before_the_end() {
    let (s, clock) = setup().await;
    apps(&s.backend).await;
    let mut stream = s.backend.stream().await;
    clock.advance(Duration::from_secs(1));

    let (status, body) = launch_run(
        &s.backend,
        json!({ "type": "app", "id": "app-fake-editor" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let run: LaunchRun = serde_json::from_value(body).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.steps.len(), 1);
    assert_eq!(
        (run.steps[0].label.as_str(), run.steps[0].status),
        ("Fake Editor", Status::Running)
    );
    assert!(run.id.starts_with("launch-"));

    let end = final_run(&mut stream, &run.id).await;
    assert_eq!(end.status, RunStatus::Success);
    assert_eq!(end.steps[0].status, Status::Success);

    // The history was written before the final frame, so a refetch already sees it.
    let list = apps(&s.backend).await;
    assert!(find(&list, "app-fake-editor").last_launched_at.is_some());
    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    assert_eq!(recent[0].id, "app-fake-editor");

    // A shortcut item of a classic app starts through the AppsFolder item.
    assert_eq!(
        s.opener.requests(),
        [OpenRequest::ShellExec {
            file: r"shell:AppsFolder\{FAKE-PF}\Fake Editor\editor.exe".into(),
            params: None,
            dir: None
        }]
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_kind_of_app_is_started_the_way_the_decision_table_says() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    for body in [
        json!({ "type": "app", "id": "app-fake-terminal", "args": "--here" }),
        json!({ "type": "app", "id": "app-fake-paint", "args": "-x" }),
        json!({ "type": "app", "id": "app-control-panel", "args": "ignored" }),
    ] {
        let (_, run) = launch_run(&s.backend, body).await;
        wait_for_end(&s.backend, run["id"].as_str().unwrap()).await;
    }
    assert_eq!(
        s.opener.requests(),
        [
            OpenRequest::Activate {
                aumid: "Fake.Terminal_abc123!App".into(),
                args: Some("--here".into())
            },
            OpenRequest::ShellExec {
                file: r"C:\Fake\System\mspaint.exe".into(),
                params: Some("-x".into()),
                dir: Some(r"C:\Fake\System".into())
            },
            OpenRequest::ShellExec {
                file: "shell:AppsFolder\\Microsoft.Windows.ControlPanel".into(),
                params: None,
                dir: None
            },
        ]
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pool_runs_its_members_in_order_with_the_delay_and_records_the_pool() {
    let (s, clock) = setup().await;
    apps(&s.backend).await;
    let mut stream = s.backend.stream().await;
    clock.advance(Duration::from_secs(1));

    let started = tokio::time::Instant::now();
    let (status, body) =
        launch_run(&s.backend, json!({ "type": "pool", "id": "pool-web-dev" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let run: LaunchRun = serde_json::from_value(body).unwrap();
    let statuses: Vec<Status> = run.steps.iter().map(|step| step.status).collect();
    assert_eq!(statuses, [Status::Running, Status::Idle]);
    let labels: Vec<&str> = run.steps.iter().map(|step| step.label.as_str()).collect();
    assert_eq!(labels, ["Fake Editor", "Fake Terminal"]);

    let end = final_run(&mut stream, &run.id).await;
    assert_eq!(end.status, RunStatus::Success);
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "the member's delay was waited"
    );
    let requests = s.opener.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        matches!(&requests[1], OpenRequest::Activate { args: Some(args), .. } if args == "--here")
    );

    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    let recent_ids: Vec<&str> = recent.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        recent_ids[0], "pool-web-dev",
        "the pool comes first: {recent_ids:?}"
    );
    assert!(recent_ids.contains(&"app-fake-editor") && recent_ids.contains(&"app-fake-terminal"));
    let pools: Vec<Pool> = s.backend.get("/pools").await;
    assert!(pools[0].last_launched_at.is_some());
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_member_does_not_stop_the_pool_and_the_pool_is_not_recorded() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    write_data(
        &s.backend,
        "pools.toml",
        "schema_version = 1\n\n[[pool]]\nid = \"pool-mixed\"\nname = \"Mixed\"\nmembers = [{ app = \"app-fake-paint\" }, { app = \"app-gone\" }, { app = \"app-fake-editor\" }]\n",
    );
    let backend = &s.backend;
    eventually("the pool is seen", || async {
        let pools: Vec<Pool> = backend.get("/pools").await;
        pools.iter().any(|pool| pool.id == "pool-mixed")
    })
    .await;
    // The first call fails without a code, so there is no fallback to a second call.
    s.opener.fail_next(OpenError {
        ui_text: "Failed to start",
        code: None,
        log_reason: "a test".into(),
    });
    let (_, body) = launch_run(&s.backend, json!({ "type": "pool", "id": "pool-mixed" })).await;
    let run = wait_for_end(&s.backend, body["id"].as_str().unwrap()).await;
    assert_eq!(run.status, RunStatus::Failed);
    let steps: Vec<(&str, Status, Option<&str>)> = run
        .steps
        .iter()
        .map(|step| (step.label.as_str(), step.status, step.error.as_deref()))
        .collect();
    assert_eq!(
        steps,
        [
            ("Fake Paint", Status::Failed, Some("Failed to start")),
            ("Unknown app", Status::Failed, Some("App not found")),
            ("Fake Editor", Status::Success, None),
        ]
    );
    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    let recent_ids: Vec<&str> = recent.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        recent_ids,
        ["app-fake-editor"],
        "what started is recent, the pool is not"
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_program_file_that_is_gone_fails_with_its_text_and_makes_no_shell_call() {
    let probe = Arc::new(FakePathProbe::default());
    probe.set_missing(&[r"c:\fake\editor\editor.exe"]);
    let (builder, _, opener, _) = builder();
    let backend = builder.with::<dyn PathProbe>(probe).start().await;
    apps(&backend).await;
    let (_, body) = launch_run(&backend, json!({ "type": "app", "id": "app-fake-editor" })).await;
    let run = wait_for_end(&backend, body["id"].as_str().unwrap()).await;
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.steps[0].error.as_deref(), Some("Executable not found"));
    assert!(opener.requests().is_empty());
    let recent: Vec<SearchItem> = backend.get("/recent").await;
    assert!(recent.is_empty(), "failed launches are not stored");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_launch_of_a_running_target_returns_that_run() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    s.opener.delay_calls(Duration::from_millis(300));
    let (first_status, first) =
        launch_run(&s.backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    let (second_status, second) =
        launch_run(&s.backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    assert_eq!(
        (first_status, second_status),
        (StatusCode::CREATED, StatusCode::OK)
    );
    assert_eq!(first["id"], second["id"]);
    wait_for_end(&s.backend, first["id"].as_str().unwrap()).await;
    let (third_status, third) =
        launch_run(&s.backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    assert_eq!(third_status, StatusCode::CREATED);
    assert_ne!(third["id"], first["id"]);
    s.backend.stop().await;
}

/// An opener whose call panics: the one way to make a run task die.
struct PanickingOpener;

impl rain_backend::core::launch_contract::Opener for PanickingOpener {
    fn open(
        &self,
        _request: OpenRequest,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<rain_backend::core::launch_contract::Started, OpenError>,
    > {
        panic!("the opener of this test always panics");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_task_that_panics_ends_its_run_as_failed_and_does_not_block_the_target() {
    let (builder, _source, _opener, _clock) = builder();
    let backend = builder.opener(Arc::new(PanickingOpener)).start().await;
    apps(&backend).await;
    let (status, first) =
        launch_run(&backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let ended = wait_for_end(&backend, first["id"].as_str().unwrap()).await;
    assert_eq!(ended.status, RunStatus::Failed);
    assert_eq!(ended.steps[0].status, Status::Failed);
    // The dead run is not handed back: a new launch makes a new run.
    let (again, second) =
        launch_run(&backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    assert_eq!(again, StatusCode::CREATED);
    assert_ne!(first["id"], second["id"]);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_requests_that_cannot_work_are_refused_with_their_reasons() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let cases = [
        (
            json!({ "type": "app", "id": "app-nothing" }),
            StatusCode::NOT_FOUND,
            "unknownApp",
        ),
        (
            json!({ "type": "pool", "id": "pool-nothing" }),
            StatusCode::NOT_FOUND,
            "unknownPool",
        ),
        (
            json!({ "type": "pool", "id": "pool-web-dev", "args": "-x" }),
            StatusCode::BAD_REQUEST,
            "argsNotAllowed",
        ),
        (
            json!({ "type": "app", "id": "app-fake-paint", "args": "a\nb" }),
            StatusCode::BAD_REQUEST,
            "badArgs",
        ),
        (
            json!({ "type": "app", "id": "app-fake-paint", "args": "x".repeat(2001) }),
            StatusCode::BAD_REQUEST,
            "badArgs",
        ),
        // Projects and games are planned by their own modules (section 02). With no project and
        // no game on this machine, an id is unknown to them.
        (
            json!({ "type": "project", "id": "anything" }),
            StatusCode::NOT_FOUND,
            "unknownId",
        ),
        (
            json!({ "type": "game", "id": "anything" }),
            StatusCode::NOT_FOUND,
            "unknownId",
        ),
    ];
    for (body, expected_status, expected_reason) in cases {
        let (status, answer) = launch_run(&s.backend, body.clone()).await;
        assert_eq!(status, expected_status, "{body}");
        assert_eq!(reason(&answer), expected_reason, "{body}");
    }
    let (status, answer) = s
        .backend
        .send("GET", "/launches/launch-nothing", None)
        .await;
    assert_eq!(
        (status, reason(&answer)),
        (StatusCode::NOT_FOUND, "unknownRun")
    );
    assert!(s.opener.requests().is_empty());
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_run_can_be_read_for_ten_minutes_and_the_registry_holds_sixty_four() {
    let (s, clock) = setup().await;
    apps(&s.backend).await;
    let mut first_id = String::new();
    let mut second_id = String::new();
    for n in 0..65 {
        clock.advance(Duration::from_secs(1));
        let (_, body) =
            launch_run(&s.backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
        let id = body["id"].as_str().unwrap().to_owned();
        wait_for_end(&s.backend, &id).await;
        match n {
            0 => first_id = id,
            1 => second_id = id,
            _ => {}
        }
    }
    let (status, _) = s
        .backend
        .send("GET", &format!("/launches/{first_id}"), None)
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the 65th run evicted the oldest finished one"
    );
    let (status, _) = s
        .backend
        .send("GET", &format!("/launches/{second_id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    clock.advance(Duration::from_secs(11 * 60));
    let (status, _) = s
        .backend
        .send("GET", &format!("/launches/{second_id}"), None)
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "after ten minutes it is gone"
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_mode_refuses_launches_and_scans_but_not_reads() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    s.backend.state.set_game_mode_active(true);
    let (status, body) =
        launch_run(&s.backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    assert_eq!((status, reason(&body)), (StatusCode::CONFLICT, "gameMode"));
    let (status, body) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!((status, reason(&body)), (StatusCode::CONFLICT, "gameMode"));
    assert_eq!(apps(&s.backend).await.len(), 15);
    s.backend.state.set_game_mode_active(false);
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_that_ended_while_nobody_listened_is_still_there_and_a_new_stream_resyncs() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let (_, body) = launch_run(&s.backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    let id = body["id"].as_str().unwrap().to_owned();
    wait_for_end(&s.backend, &id).await;
    let mut stream = s.backend.stream().await;
    let (event, frame) = stream.expect_event().await;
    assert_eq!((event.as_str(), frame.topic.as_str()), ("hello", "hello"));
    let (_, frame) = stream.expect_event().await;
    assert_eq!(frame.topic, "resync");
    let run: LaunchRun = s.backend.get(&format!("/launches/{id}")).await;
    assert_eq!(run.status, RunStatus::Success);
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pool_with_a_slow_member_sends_frames_between_the_steps() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    write_data(
        &s.backend,
        "pools.toml",
        "schema_version = 1\n\n[[pool]]\nid = \"pool-slow\"\nname = \"Slow\"\nmembers = [{ app = \"app-fake-editor\" }, { app = \"app-fake-paint\", delay_ms = 400 }]\n",
    );
    let backend = &s.backend;
    eventually("the pool is seen", || async {
        let pools: Vec<Pool> = backend.get("/pools").await;
        pools.iter().any(|pool| pool.id == "pool-slow")
    })
    .await;
    let mut stream = s.backend.stream().await;
    let (_, body) = launch_run(&s.backend, json!({ "type": "pool", "id": "pool-slow" })).await;
    let id = body["id"].as_str().unwrap().to_owned();
    // First frame: the editor is done, the second step is running (its delay is being waited).
    let middle = loop {
        let frame = stream.expect_topic("launch").await;
        let run: LaunchRun = serde_json::from_value(frame.payload).unwrap();
        if run.id == id {
            break run;
        }
    };
    assert_eq!(middle.status, RunStatus::Running);
    assert_eq!(
        middle
            .steps
            .iter()
            .map(|step| step.status)
            .collect::<Vec<_>>(),
        [Status::Success, Status::Running]
    );
    let end = final_run(&mut stream, &id).await;
    assert_eq!(end.status, RunStatus::Success);
    s.backend.stop().await;
}

// ---- search, recent, properties ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_ranks_titles_above_keywords_filters_types_and_ignores_a_blank_query() {
    let (s, _) = setup().await;
    apps(&s.backend).await;
    let found: Vec<SearchItem> = s.backend.get("/search?q=web").await;
    assert_eq!(
        found[0].id, "pool-web-dev",
        "the pool's title beats keywords of apps: {found:?}"
    );
    let found: Vec<SearchItem> = s.backend.get("/search?q=web&types=app").await;
    assert!(
        found
            .iter()
            .all(|item| item.r#type == rain_types::SearchType::App)
    );
    let found: Vec<SearchItem> = s.backend.get("/search?q=editor&types=app,pool").await;
    assert_eq!(found[0].id, "app-fake-editor");
    assert!(
        found[0].score > 0 && found[0].subtitle.as_deref() == Some(r"C:\Fake\Editor\editor.exe")
    );
    // A pool is also found by the name of one of its apps.
    let found: Vec<SearchItem> = s.backend.get("/search?q=terminal&types=pool").await;
    assert_eq!(found[0].id, "pool-web-dev");
    for blank in ["/search", "/search?q=", "/search?q=%20%20"] {
        let found: Vec<SearchItem> = s.backend.get(blank).await;
        assert!(found.is_empty(), "{blank}");
    }
    let (status, body) = s.backend.send("GET", "/search?q=a&types=robot", None).await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::BAD_REQUEST, "unknownType")
    );
    let (status, body) = s
        .backend
        .send("GET", &format!("/search?q={}", "x".repeat(201)), None)
        .await;
    assert_eq!(
        (status, reason(&body)),
        (StatusCode::BAD_REQUEST, "queryTooLong")
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_returns_at_most_fifty_items() {
    let (s, _) = setup().await;
    s.source.set_items(
        (0..60)
            .map(|n| {
                raw_app(
                    &format!("Match {n:02}"),
                    &format!("Fake.Match{n:02}_abc!App"),
                    "",
                    "",
                )
            })
            .collect(),
    );
    s.backend.send("POST", "/apps/scan", None).await;
    let found: Vec<SearchItem> = s.backend.get("/search?q=match").await;
    assert_eq!(found.len(), 50);
    assert_eq!(
        found[0].id, "app-match-00",
        "equal scores keep a steady order"
    );
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recent_is_newest_first_limited_and_checks_its_limit() {
    let (s, clock) = setup().await;
    apps(&s.backend).await;
    for id in ["app-fake-paint", "app-fake-editor", "app-fake-notes"] {
        clock.advance(Duration::from_secs(5));
        let (_, body) = launch_run(&s.backend, json!({ "type": "app", "id": id })).await;
        wait_for_end(&s.backend, body["id"].as_str().unwrap()).await;
    }
    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    let order: Vec<&str> = recent.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        order,
        ["app-fake-notes", "app-fake-editor", "app-fake-paint"]
    );
    let one: Vec<SearchItem> = s.backend.get("/recent?limit=1").await;
    assert_eq!(one.len(), 1);
    for bad in ["0", "51"] {
        let (status, body) = s
            .backend
            .send("GET", &format!("/recent?limit={bad}"), None)
            .await;
        assert_eq!(
            (status, reason(&body)),
            (StatusCode::BAD_REQUEST, "badLimit")
        );
    }
    s.backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_survives_a_restart() {
    let (mut s, clock) = setup().await;
    apps(&s.backend).await;
    clock.advance(Duration::from_secs(5));
    let (_, body) = launch_run(&s.backend, json!({ "type": "app", "id": "app-fake-paint" })).await;
    wait_for_end(&s.backend, body["id"].as_str().unwrap()).await;
    s.backend.restart().await;
    let list = apps(&s.backend).await;
    assert!(find(&list, "app-fake-paint").last_launched_at.is_some());
    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    assert_eq!(recent[0].id, "app-fake-paint");
    s.backend.stop().await;
}

/// A small generator, so the property test needs no extra crate and is the same on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % below
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_id_that_search_pinned_and_recent_return_exists_in_its_own_list() {
    let (s, clock) = setup().await;
    let list = apps(&s.backend).await;
    let pools: Vec<Pool> = s.backend.get("/pools").await;
    let mut random = Lcg(7);

    // Random pins and launches.
    for _ in 0..6 {
        let app = &list[random.next(list.len())];
        s.backend
            .send(
                "PUT",
                &format!("/pinned/app/{}", app.id),
                Some(json!({ "pinned": random.next(2) == 0 })),
            )
            .await;
        clock.advance(Duration::from_secs(1));
        let (_, body) = launch_run(&s.backend, json!({ "type": "app", "id": app.id })).await;
        wait_for_end(&s.backend, body["id"].as_str().unwrap()).await;
    }

    let known = |item: &SearchItem| match item.r#type {
        rain_types::SearchType::App => list.iter().any(|app| app.id == item.id),
        rain_types::SearchType::Pool => pools.iter().any(|pool| pool.id == item.id),
        other => panic!("a launcher without projects returned a {other:?}"),
    };
    let words = [
        "fake", "pho", "web", "dev", "launch", "e", "ed", "term", "pa", "zz", "n o", "x",
    ];
    for _ in 0..300 {
        let query = format!(
            "{}{}",
            words[random.next(words.len())],
            if random.next(3) == 0 {
                words[random.next(words.len())]
            } else {
                ""
            }
        );
        let found: Vec<SearchItem> = s
            .backend
            .get(&format!("/search?q={}", query.replace(' ', "%20")))
            .await;
        assert!(found.iter().all(known), "search {query}: {found:?}");
        let mut seen = std::collections::HashSet::new();
        assert!(
            found.iter().all(|item| seen.insert(&item.id)),
            "an id twice in {query}"
        );
    }
    let pinned: Vec<SearchItem> = s.backend.get("/pinned").await;
    let recent: Vec<SearchItem> = s.backend.get("/recent").await;
    assert!(pinned.iter().all(known) && recent.iter().all(known));
    // No id is used by two types.
    let all_ids: Vec<&str> = list
        .iter()
        .map(|a| a.id.as_str())
        .chain(pools.iter().map(|p| p.id.as_str()))
        .collect();
    let unique: std::collections::HashSet<&&str> = all_ids.iter().collect();
    assert_eq!(unique.len(), all_ids.len());
    s.backend.stop().await;
}

// ---- icons ----

struct Answer {
    status: StatusCode,
    headers: hyper::HeaderMap,
    body: hyper::body::Bytes,
}

async fn get_media(backend: &TestBackend, path: &str, if_none_match: Option<&str>) -> Answer {
    let mut headers = Vec::new();
    if let Some(tag) = if_none_match {
        headers.push(("if-none-match", tag));
    }
    let raw = backend
        .client
        .request(Method::GET, &format!("{API_PREFIX}{path}"), None, &headers)
        .await
        .expect("the backend answered");
    Answer {
        status: raw.status,
        headers: raw.headers,
        body: raw.body,
    }
}

fn header<'a>(answer: &'a Answer, name: &str) -> &'a str {
    answer
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn icon_path(list: &[AppEntry], id: &str) -> String {
    format!("/{}", find(list, id).icon.value)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_icon_is_served_with_its_etag_cached_on_disk_and_not_drawn_again() {
    let renderer = Arc::new(FakeIconRenderer::default());
    let (builder, _, _, _) = builder();
    let mut backend = builder
        .with::<dyn IconRenderer>(renderer.clone())
        .start()
        .await;
    let list = apps(&backend).await;
    let path = icon_path(&list, "app-fake-editor");

    let first = get_media(&backend, &path, None).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(&first.body[..], b"png");
    assert_eq!(header(&first, "content-type"), "image/png");
    assert_eq!(header(&first, "x-content-type-options"), "nosniff");
    assert!(header(&first, "cache-control").contains("immutable"));
    let etag = header(&first, "etag").to_owned();
    assert!(!etag.is_empty());
    let drawn = renderer.calls.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(drawn, 1);

    // The version in the address decides: a matching tag is a 304 without any drawing.
    let again = get_media(&backend, &path, Some(&etag)).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    assert_eq!(
        renderer.calls.load(std::sync::atomic::Ordering::SeqCst),
        drawn
    );

    // The file is in the cache folder, and a restart serves it without drawing.
    let folder = backend
        .local_dir
        .join("cache")
        .join("launcher")
        .join("icons");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    backend.restart().await;
    let after = get_media(&backend, &path, None).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(
        renderer.calls.load(std::sync::atomic::Ordering::SeqCst),
        drawn
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_icon_that_cannot_be_drawn_is_404_and_remembered_until_the_next_scan() {
    let renderer = Arc::new(FakeIconRenderer::default());
    renderer.fail_for(&["Fake Paint"]);
    let (builder, _, _, _) = builder();
    let backend = builder
        .with::<dyn IconRenderer>(renderer.clone())
        .start()
        .await;
    let list = apps(&backend).await;
    let path = icon_path(&list, "app-fake-paint");
    let calls = || renderer.calls.load(std::sync::atomic::Ordering::SeqCst);

    let first = get_media(&backend, &path, None).await;
    assert_eq!(first.status, StatusCode::NOT_FOUND);
    let body: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(reason(&body), "noIcon");
    assert_eq!(calls(), 1);
    get_media(&backend, &path, None).await;
    assert_eq!(calls(), 1, "not drawn again");

    backend.send("POST", "/apps/scan", None).await;
    get_media(&backend, &path, None).await;
    assert_eq!(calls(), 2, "a scan gives it another try");

    let unknown = get_media(&backend, "/media/app-icon/app-nothing?v=1", None).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let body: Value = serde_json::from_slice(&unknown.body).unwrap();
    assert_eq!(reason(&body), "unknownApp");
    backend.stop().await;
}

// ---- the dev-only fault route ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fault_route_makes_the_source_empty_or_failing_in_a_sandbox_only() {
    let (s, _) = setup().await;
    assert_eq!(apps(&s.backend).await.len(), 15);

    let (status, _) = s
        .backend
        .send(
            "PUT",
            "/internal/test/apps-source",
            Some(json!({ "mode": "empty" })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!(
        (status, body),
        (StatusCode::OK, json!([])),
        "the empty screen can be seen"
    );

    s.backend
        .send(
            "PUT",
            "/internal/test/apps-source",
            Some(json!({ "mode": "fail" })),
        )
        .await;
    let (status, _) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    s.backend
        .send(
            "PUT",
            "/internal/test/apps-source",
            Some(json!({ "mode": "normal" })),
        )
        .await;
    let (_, body) = s.backend.send("POST", "/apps/scan", None).await;
    assert_eq!(body.as_array().map(Vec::len), Some(15));
    s.backend.stop().await;

    let (builder, _, _, _) = builder();
    let real = builder.real_mode().start().await;
    let (status, body) = real
        .send(
            "PUT",
            "/internal/test/apps-source",
            Some(json!({ "mode": "empty" })),
        )
        .await;
    assert_eq!((status, reason(&body)), (StatusCode::NOT_FOUND, "noRoute"));
    real.stop().await;
}
