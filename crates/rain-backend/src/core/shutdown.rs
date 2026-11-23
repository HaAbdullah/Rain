//! Stopping the backend, and the gauge that says whether it is busy.
//! Foundation sections 2.6 and 6.9.
//!
//! The order of a stop is run by `start::run_shutdown`; this file holds the pieces it uses.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rain_types::StopReason;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub struct ShutdownHandle {
    /// Cancelled when a stop starts. Open streams see it, write their last frame and end.
    token: CancellationToken,
    /// Why the backend stops. Set once, before the token is cancelled.
    reason: OnceLock<StopReason>,
    /// Streams that have not yet written their last frame.
    streams_open: AtomicUsize,
    streams_changed: Notify,
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownHandle {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            reason: OnceLock::new(),
            streams_open: AtomicUsize::new(0),
            streams_changed: Notify::new(),
        }
    }

    /// Starts a stop. Returns `true` for the first call and `false` when a stop was already
    /// requested (its reason stays). Asking twice is harmless.
    pub fn request(&self, reason: StopReason) -> bool {
        let first = self.reason.set(reason).is_ok();
        self.token.cancel();
        first
    }

    pub fn shutting_down(&self) -> bool {
        self.token.is_cancelled()
    }

    /// The reason of the stop, once one was requested.
    pub fn reason(&self) -> Option<StopReason> {
        self.reason.get().copied()
    }

    /// Cancelled when a stop starts.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// An open stream holds this until it has written its last frame (or is dropped). The guard
    /// owns a share of the handle, so it can live inside the stream, which outlives the request.
    pub fn stream_guard(self: &Arc<Self>) -> StreamGuard {
        self.streams_open.fetch_add(1, Ordering::SeqCst);
        StreamGuard {
            handle: Arc::clone(self),
        }
    }

    /// Waits until every open stream has written its last frame, but at most `limit`. A client
    /// that does not read can keep a stream from writing, and must not hold up the stop.
    pub async fn wait_for_streams(&self, limit: Duration) {
        let all_written = async {
            while self.streams_open.load(Ordering::SeqCst) > 0 {
                // `notify_one` keeps a permit when nobody waits yet, so no wake-up is lost.
                self.streams_changed.notified().await;
            }
        };
        // Running out of time is not an error: the stop goes on.
        let _ = tokio::time::timeout(limit, all_written).await;
    }
}

pub struct StreamGuard {
    handle: Arc<ShutdownHandle>,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.handle.streams_open.fetch_sub(1, Ordering::SeqCst);
        self.handle.streams_changed.notify_one();
    }
}

/// Counts the work that must not be cut off by an upgrade: a scheduled job, an idle job, a scan, a
/// launch. `GET /ping` reports it as `busy`. Each module holds a guard while it works; the guard
/// owns a share of the counter, so a spawned task can carry it.
#[derive(Default, Clone)]
pub struct Activity {
    running: Arc<AtomicUsize>,
}

impl Activity {
    pub fn guard(&self) -> ActivityGuard {
        self.running.fetch_add(1, Ordering::SeqCst);
        ActivityGuard {
            running: Arc::clone(&self.running),
        }
    }

    pub fn busy(&self) -> bool {
        self.running.load(Ordering::SeqCst) > 0
    }
}

pub struct ActivityGuard {
    running: Arc<AtomicUsize>,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_first_request_decides_the_reason() {
        let handle = ShutdownHandle::new();
        assert!(!handle.shutting_down());
        assert_eq!(handle.reason(), None);

        assert!(handle.request(StopReason::GameMode));
        assert!(!handle.request(StopReason::Stop));
        assert!(handle.shutting_down());
        assert_eq!(handle.reason(), Some(StopReason::GameMode));
        assert!(handle.token().is_cancelled());
    }

    #[tokio::test]
    async fn waiting_for_streams_ends_when_the_last_guard_drops() {
        let handle = Arc::new(ShutdownHandle::new());
        let guard_task = {
            let handle = handle.clone();
            tokio::spawn(async move {
                let guard = handle.stream_guard();
                tokio::time::sleep(Duration::from_millis(50)).await;
                drop(guard);
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await; // let the guard be taken
        let started = std::time::Instant::now();
        handle.wait_for_streams(Duration::from_secs(5)).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        guard_task.await.unwrap();
    }

    #[tokio::test]
    async fn waiting_for_a_stream_that_never_finishes_ends_at_the_limit() {
        let handle = Arc::new(ShutdownHandle::new());
        let _guard = handle.stream_guard();
        let started = std::time::Instant::now();
        handle.wait_for_streams(Duration::from_millis(60)).await;
        assert!(started.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn activity_is_busy_while_a_guard_lives() {
        let activity = Activity::default();
        assert!(!activity.busy());
        let first = activity.guard();
        let second = activity.guard();
        drop(first);
        assert!(activity.busy());
        drop(second);
        assert!(!activity.busy());
    }
}
