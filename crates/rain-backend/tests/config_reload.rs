//! The file watcher on the data folder: a hand edit of `settings.toml` reaches the page without a
//! click. Foundation section 10.4; section 06 1.4.

mod common;

use std::fs;
use std::time::{Duration, Instant};

use common::{StreamReader, TestBackend};
use serde_json::{Value, json};

async fn open_stream(backend: &TestBackend) -> StreamReader {
    let mut stream = backend.stream().await;
    assert_eq!(stream.expect_event().await.0, "hello");
    assert_eq!(stream.expect_event().await.0, "resync");
    stream
}

fn settings_path(backend: &TestBackend) -> std::path::PathBuf {
    backend.data_dir.join("settings.toml")
}

/// Reads frames of `topic` until one satisfies `check`.
async fn until(stream: &mut StreamReader, topic: &str, check: impl Fn(&Value) -> bool) -> Value {
    loop {
        let frame = stream.expect_topic(topic).await;
        if check(&frame.payload) {
            return frame.payload;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_reaches_the_topic_settings_within_a_second() {
    let backend = TestBackend::with_fixture("typical").await;
    let mut stream = open_stream(&backend).await;

    let started = Instant::now();
    fs::write(
        settings_path(&backend),
        "schema_version = 1\nreset_after_seconds = 30\n",
    )
    .unwrap();
    let payload = until(&mut stream, "settings", |s| s["resetAfterSeconds"] == 30).await;

    assert_eq!(
        payload["hideOnBlur"], true,
        "a key that is gone is its default again"
    );
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "took {:?}: the debounce is 300 ms",
        started.elapsed()
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_edit_keeps_the_last_good_value_and_the_issue_clears_when_the_file_is_fixed() {
    let backend = TestBackend::with_fixture("typical").await;
    let mut stream = open_stream(&backend).await;
    let good: Value = backend.get("/settings").await;

    fs::write(settings_path(&backend), "hide_on_blur = [\n").unwrap();
    let health = until(&mut stream, "health", |h| {
        h["configIssues"][0]["file"] == "settings.toml"
    })
    .await;
    assert_eq!(health["configIssues"][0]["line"], 1);
    let still: Value = backend.get("/settings").await;
    assert_eq!(
        still, good,
        "the last good value is shown while the file is broken"
    );

    fs::write(
        settings_path(&backend),
        "schema_version = 1\nreset_after_seconds = 30\n",
    )
    .unwrap();
    until(&mut stream, "health", |h| h["configIssues"] == json!([])).await;
    let fixed: Value = backend.get("/settings").await;
    assert_eq!(fixed["resetAfterSeconds"], 30);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deleted_file_means_the_defaults() {
    let backend = TestBackend::with_fixture("typical").await;
    let mut stream = open_stream(&backend).await;

    fs::remove_file(settings_path(&backend)).unwrap();
    let payload = until(&mut stream, "settings", |s| s["resetAfterSeconds"] == 60).await;
    assert_eq!(payload["hideOnBlur"], true);
    assert_eq!(payload["projectRoots"], json!([]));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_own_write_of_a_patch_is_one_frame_and_not_a_second_one_from_the_watcher() {
    let backend = TestBackend::start().await;
    let mut stream = open_stream(&backend).await;

    let (status, _) = backend
        .send(
            "PATCH",
            "/settings",
            Some(json!({ "resetAfterSeconds": 300 })),
        )
        .await;
    assert_eq!(status, hyper::StatusCode::OK);

    let mut settings_frames = 0;
    let deadline = Instant::now() + Duration::from_millis(1800);
    while Instant::now() < deadline {
        let next = tokio::time::timeout(Duration::from_millis(200), stream.next_event()).await;
        if let Ok(Some((_, frame))) = next
            && frame.topic == "settings"
        {
            settings_frames += 1;
        }
    }
    assert_eq!(
        settings_frames, 1,
        "the echo of Rain's own write is recognised"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_that_other_files_do_not_matter_for_is_not_reloaded() {
    let backend = TestBackend::with_fixture("typical").await;
    let mut stream = open_stream(&backend).await;

    // Rain's own temporary and backup names and an unknown file never matter.
    fs::write(backend.data_dir.join("settings.toml.bak"), "x = 1\n").unwrap();
    fs::write(
        backend.data_dir.join(".settings.toml.rain-tmp-1"),
        "x = 1\n",
    )
    .unwrap();
    fs::write(backend.data_dir.join("notes.txt"), "x").unwrap();

    let quiet = tokio::time::timeout(Duration::from_millis(900), stream.next_event()).await;
    assert!(quiet.is_err(), "no frame for files that are not data files");
    backend.stop().await;
}
