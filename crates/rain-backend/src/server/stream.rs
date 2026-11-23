//! `GET /stream`: the live stream. Foundation sections 5.1 to 5.4 and 5.7.
//! The handler is the one proven in the spike `types-stream` (`routes.rs`, `events`).

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{Stream, StreamExt, stream};
use rain_types::{Hello, Lifecycle, LifecycleKind, Resync, ResyncReason, StopReason};
use serde::Serialize;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::API_VERSION;
use crate::core::bus::{FrameBytes, envelope_json};
use crate::core::state::AppState;
use std::sync::Arc;

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
    let keepalive = state.config.limits.sse_keepalive;

    let head = stream::iter([
        hello_event(&state),
        resync_event(&state, ResyncReason::Connect, None),
    ]);
    let live = live_events(state.clone(), receiver);
    // The guard tells the stop that the last frame is ready. It is released when that frame is
    // produced (or earlier, together with the stream, when the client leaves); hyper writes the
    // frame a moment later, and the stop waits for that write in step 4, when `serve` drains the
    // open connections.
    let tail = stream::once(async move {
        let event = lifecycle_event(&state);
        drop(guard);
        event
    });

    Sse::new(head.chain(live).chain(tail).map(Ok))
        .keep_alive(KeepAlive::new().interval(keepalive).text("ka"))
}

/// The frames of the bus, as events, until the stop token is cancelled.
///
/// A stop does not drop what was already published: when the token fires, the frames that wait in
/// this client's buffer are still written (the last frame of a game mode wind-down, `done`, is
/// published a moment before the stop), and only then does the stream end. `try_recv` returns
/// `Empty` exactly when the buffer holds nothing more, so the drain is short and cannot wait.
fn live_events(state: AppState, receiver: Receiver<Arc<FrameBytes>>) -> impl Stream<Item = Event> {
    let stop = state.shutdown.token().clone();
    stream::unfold((receiver, false), move |(mut receiver, mut stopping)| {
        let (state, stop) = (state.clone(), stop.clone());
        async move {
            loop {
                if !stopping {
                    // `biased`: when the stop and a frame are both ready, the stop is seen first
                    // and the rest of the buffer is drained below, in order.
                    tokio::select! {
                        biased;
                        () = stop.cancelled() => {
                            stopping = true;
                            continue;
                        }
                        item = receiver.recv() => match item {
                            Ok(frame) => return Some((frame_event(&state, &frame), (receiver, stopping))),
                            Err(RecvError::Lagged(missed)) => {
                                let event = resync_event(&state, ResyncReason::Lagged, Some(missed));
                                return Some((event, (receiver, stopping)));
                            }
                            Err(RecvError::Closed) => return None,
                        },
                    }
                }
                return match receiver.try_recv() {
                    Ok(frame) => Some((frame_event(&state, &frame), (receiver, stopping))),
                    Err(TryRecvError::Lagged(missed)) => {
                        let event = resync_event(&state, ResyncReason::Lagged, Some(missed));
                        Some((event, (receiver, stopping)))
                    }
                    Err(TryRecvError::Empty | TryRecvError::Closed) => None,
                };
            }
        }
    })
}

/// The event of a frame the bus published. A frame that was too big for the stream is replaced by
/// a `resync`: the client refetches what it shows.
fn frame_event(state: &AppState, frame: &FrameBytes) -> Event {
    if frame.too_big {
        resync_event(state, ResyncReason::Lagged, None)
    } else {
        topic_event(frame)
    }
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
