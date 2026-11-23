//! `GET /stream`: the live stream. Foundation sections 5.1 to 5.4 and 5.7.
//! The handler is the one proven in the spike `types-stream` (`routes.rs`, `events`).

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{Stream, StreamExt, stream};
use rain_types::{Hello, Lifecycle, LifecycleKind, Resync, ResyncReason, StopReason};
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::API_VERSION;
use crate::core::bus::{FrameBytes, envelope_json};
use crate::core::state::AppState;

/// How long a browser waits before it reconnects (the SSE `retry` field).
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// What a client sees, in this order:
///
/// 1. `hello`: which backend process this is. Written to this client only.
/// 2. `resync` (reason `connect`): the client must refetch everything it shows.
/// 3. every frame that is published, as it is published. A client that reads too slowly gets a
///    `resync` (reason `lagged`) instead of the frames it lost.
/// 4. when the backend stops: a `lifecycle` frame with the reason, and the stream ends. Its
///    absence tells the client that the backend crashed.
///
/// A `: ka` comment line is sent when nothing else was sent for `Limits::sse_keepalive`
/// (`SSE_KEEPALIVE`, 25 s, by default).
pub async fn stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe BEFORE the head frames are built: nothing published after this line is missed.
    let receiver = state.bus.subscribe();
    let guard = state.shutdown.stream_guard();
    let stop = state.shutdown.token().clone();
    let keepalive = state.config.limits.sse_keepalive;

    let head = stream::iter([
        hello_event(&state),
        resync_event(&state, ResyncReason::Connect, None),
    ]);
    let live = {
        let state = state.clone();
        BroadcastStream::new(receiver).map(move |item| match item {
            Ok(frame) => topic_event(&frame),
            Err(BroadcastStreamRecvError::Lagged(missed)) => {
                resync_event(&state, ResyncReason::Lagged, Some(missed))
            }
        })
    };
    // The guard tells the stop that the last frame is ready. It is released when that frame is
    // produced (or earlier, together with the stream, when the client leaves); hyper writes the
    // frame a moment later, and the stop waits for that write in step 4, when `serve` drains the
    // open connections.
    let tail = stream::once(async move {
        let event = lifecycle_event(&state);
        drop(guard);
        event
    });

    Sse::new(
        head.chain(live.take_until(stop.cancelled_owned()))
            .chain(tail)
            .map(Ok),
    )
    .keep_alive(KeepAlive::new().interval(keepalive).text("ka"))
}

/// Builds an event from the frame of a topic that the bus published.
fn topic_event(frame: &FrameBytes) -> Event {
    Event::default()
        .event(frame.topic)
        .id(frame.seq.to_string())
        .data(&frame.json)
}

/// Builds an event for an internal frame. These frames are written to one client, with the
/// current number of the bus, which is not advanced.
fn internal_event(state: &AppState, topic: &str, payload: &impl Serialize) -> Event {
    let seq = state.bus.seq();
    // The payloads are small plain structs, which always serialise.
    let json = envelope_json(topic, seq, payload).expect("an internal frame is always valid JSON");
    Event::default().event(topic).id(seq.to_string()).data(json)
}

fn hello_event(state: &AppState) -> Event {
    let hello = Hello {
        start_id: state.start_id.clone(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        api_version: API_VERSION,
        seq: state.bus.seq(),
    };
    // `retry` only matters to the `EventSource` of a browser; the Rust reader has its own backoff.
    internal_event(state, "hello", &hello).retry(RECONNECT_DELAY)
}

fn resync_event(state: &AppState, reason: ResyncReason, missed: Option<u64>) -> Event {
    internal_event(state, "resync", &Resync { reason, missed })
}

fn lifecycle_event(state: &AppState) -> Event {
    // The stream ends only because the stop token was cancelled, and the token is cancelled only
    // by `ShutdownHandle::request`, which sets the reason first. `Stop` is a safe default for a
    // case that cannot happen.
    let reason = state.shutdown.reason().unwrap_or(StopReason::Stop);
    let lifecycle = Lifecycle {
        kind: LifecycleKind::Stopping,
        reason,
    };
    internal_event(state, "lifecycle", &lifecycle)
}
