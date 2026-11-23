//! `GET /stream` (foundation section 5) and the stop of the backend while streams are open
//! (section 2.6).

mod common;

use std::time::{Duration, Instant};

use common::TestBackend;
use rain_pipe::SseFrame;
use rain_types::limits::BUS_CAPACITY;
use rain_types::{
    Hello, Jobs, Lifecycle, LifecycleKind, Resync, ResyncReason, Sessions, StopReason,
};
use serde_json::json;

/// Reads the two frames that open every connection and checks them.
async fn expect_head(stream: &mut common::StreamReader, backend: &TestBackend) -> Hello {
    let (event, frame) = stream.expect_event().await;
    assert_eq!(event, "hello");
    assert_eq!(frame.topic, "hello");
    let hello: Hello = serde_json::from_value(frame.payload).expect("a hello payload");

    let (event, frame) = stream.expect_event().await;
    assert_eq!(event, "resync");
    let resync: Resync = serde_json::from_value(frame.payload).expect("a resync payload");
    assert_eq!(resync.reason, ResyncReason::Connect);
    assert_eq!(resync.missed, None);

    assert_eq!(hello.start_id, backend.state.start_id);
    hello
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_stream_starts_with_hello_and_then_resync() {
    let backend = TestBackend::start().await;
    let mut stream = backend.stream().await;

    let hello = expect_head(&mut stream, &backend).await;
    assert_eq!(hello.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(hello.api_version, 1);
    assert_eq!(hello.seq, 0, "nothing was published yet");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_stream_answers_with_the_headers_of_an_event_stream() {
    let backend = TestBackend::start().await;
    let stream = backend.stream().await;
    assert_eq!(stream.headers["content-type"], "text/event-stream");
    assert_eq!(stream.headers["cache-control"], "no-store");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idle_stream_gets_a_keep_alive_comment() {
    let backend = TestBackend::builder()
        .limits(|limits| limits.sse_keepalive = Duration::from_millis(200))
        .start()
        .await;
    let mut stream = backend.stream().await;
    expect_head(&mut stream, &backend).await;

    assert_eq!(stream.next_raw().await, SseFrame::Comment("ka".into()));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_connection_gets_its_own_hello_with_the_current_counter() {
    let backend = TestBackend::start().await;
    let mut first = backend.stream().await;
    expect_head(&mut first, &backend).await;

    backend.state.bus.publish::<Jobs>(&json!([]));
    backend.state.bus.publish::<Jobs>(&json!([]));

    let mut second = backend.stream().await;
    let hello = expect_head(&mut second, &backend).await;
    assert_eq!(hello.seq, 2, "the second client sees where the process is");

    // The first client got the two frames; the second one got only its own head.
    assert_eq!(first.expect_event().await.1.seq, 1);
    assert_eq!(first.expect_event().await.1.seq, 2);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_published_topic_arrives_as_a_frame() {
    let backend = TestBackend::start().await;
    let mut stream = backend.stream().await;
    expect_head(&mut stream, &backend).await;

    backend
        .state
        .bus
        .publish::<Sessions>(&json!([{ "id": "s1" }]));

    let (event, frame) = stream.expect_event().await;
    assert_eq!(event, "sessions", "the event name repeats the topic");
    assert_eq!((frame.topic.as_str(), frame.seq), ("sessions", 1));
    assert_eq!(frame.payload, json!([{ "id": "s1" }]));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_of_health_is_published_on_the_health_topic() {
    let backend = TestBackend::start().await;
    let mut stream = backend.stream().await;
    expect_head(&mut stream, &backend).await;

    backend.state.health.set_source(
        "steam",
        rain_types::SourceState::Off,
        Some("not installed".into()),
    );

    let (event, frame) = stream.expect_event().await;
    assert_eq!(event, "health");
    assert_eq!(frame.payload["sources"][0]["name"], "steam");
    assert_eq!(frame.payload["startId"], backend.state.start_id.as_str());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_client_is_told_to_resync_and_then_gets_the_newest_frames() {
    let backend = TestBackend::start().await;
    let mut stream = backend.stream().await;
    expect_head(&mut stream, &backend).await;

    // The client stops reading. About 15 MB are far more than the pipe, the buffers of hyper and
    // the 64 frames of the bus can hold, so the bus must drop the oldest frames.
    let padding = "x".repeat(50_000);
    let published = 300u64;
    for _ in 0..published {
        backend
            .state
            .bus
            .publish::<Sessions>(&json!({ "padding": padding }));
    }
    assert_eq!(backend.state.bus.seq(), published);

    // Reading again: some old frames, then one `resync` that says how many were lost, then the
    // newest frames up to the last one.
    let mut lagged: Option<Resync> = None;
    let mut frames_after_resync = 0;
    loop {
        let (event, frame) = stream.expect_event().await;
        if event == "resync" {
            lagged = Some(serde_json::from_value(frame.payload).expect("a resync payload"));
        } else if lagged.is_some() {
            frames_after_resync += 1;
        }
        if frame.seq == published && event == "sessions" {
            break;
        }
    }
    let lagged = lagged.expect("a resync arrived before the newest frame");
    assert_eq!(lagged.reason, ResyncReason::Lagged);
    assert!(lagged.missed.is_some_and(|missed| missed > 0));
    assert!(
        frames_after_resync <= BUS_CAPACITY,
        "only the newest frames follow"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_ends_every_stream_with_the_lifecycle_frame() {
    let backend = TestBackend::start().await;
    let mut stream = backend.stream().await;
    expect_head(&mut stream, &backend).await;

    backend.request_stop("gameMode").await;
    let rest = stream.rest().await;

    let (event, frame) = rest.last().expect("the stream ended with a frame");
    assert_eq!(event, "lifecycle");
    let lifecycle: Lifecycle = serde_json::from_value(frame.payload.clone()).expect("a payload");
    assert_eq!(lifecycle.kind, LifecycleKind::Stopping);
    assert_eq!(lifecycle.reason, StopReason::GameMode);
    assert_eq!(rest.len(), 1, "nothing else follows the head frames");
    backend.wait_stopped().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_is_not_held_up_by_a_client_that_never_reads() {
    let mut backend = TestBackend::start().await;
    let mut stalled = backend.stream().await;
    expect_head(&mut stalled, &backend).await;

    // Fill the pipe so that the server cannot even write the last frame to this client.
    let padding = "x".repeat(50_000);
    for _ in 0..100 {
        backend
            .state
            .bus
            .publish::<Sessions>(&json!({ "padding": padding }));
    }

    let started = Instant::now();
    backend.request_stop("stop").await;
    backend.wait_exited().await;
    // Streams get 0.5 s to write, open connections 2 s to finish (sections 2.6 and 3.3).
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "took {:?}",
        started.elapsed()
    );
    // The pipe name goes only when the client lets go of its end too.
    drop(stalled);
    backend.wait_pipe_gone().await;
}
