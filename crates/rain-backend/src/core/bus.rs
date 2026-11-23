//! The event bus: how a module tells every connected client that something changed.
//! Foundation sections 5.3, 5.4 and 11.3.
//!
//! A frame is serialised once and shared (`Arc`) between all clients. The channel holds
//! `BUS_CAPACITY` frames; a client that reads too slowly loses the oldest ones and is told to
//! refetch (the stream turns the `Lagged` error into a `resync` frame).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rain_types::Topic;
use rain_types::limits::{BUS_CAPACITY, FRAME_MAX};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::error;

/// One frame, ready to be written to a client.
#[derive(Debug)]
pub struct FrameBytes {
    /// The topic name: the SSE `event:` field.
    pub topic: &'static str,
    /// The sequence number: the SSE `id:` field.
    pub seq: u64,
    /// The envelope `{ "topic", "seq", "payload" }` as JSON text: the SSE `data:` field.
    pub json: String,
}

/// The same shape as `rain_types::Frame`, but it borrows its parts and serialises the payload
/// directly. A test compares the two.
#[derive(Serialize)]
struct Envelope<'a, P: Serialize> {
    topic: &'a str,
    seq: u64,
    payload: &'a P,
}

/// The JSON text of a frame (foundation section 5.1).
pub fn envelope_json<P: Serialize>(
    topic: &str,
    seq: u64,
    payload: &P,
) -> serde_json::Result<String> {
    serde_json::to_string(&Envelope {
        topic,
        seq,
        payload,
    })
}

/// Cloning an `EventBus` is cheap: all clones are the same bus.
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<Inner>,
}

struct Inner {
    sender: broadcast::Sender<Arc<FrameBytes>>,
    /// The number of the last frame that was published. The lock also makes sure that frames enter
    /// the channel in the order of their numbers.
    seq: Mutex<u64>,
    /// Set by `freeze`: no topic frame is published after it.
    frozen: AtomicBool,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(BUS_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                sender,
                seq: Mutex::new(0),
                frozen: AtomicBool::new(false),
            }),
        }
    }

    /// Publishes a snapshot to every connected client.
    ///
    /// Does nothing while no client is connected: the state of a module still changes, but nobody
    /// needs a frame, and a client that connects later refetches anyway. A payload over
    /// `FRAME_MAX` is dropped and logged: it is a programming error (foundation section 5.3).
    pub fn publish<T: Topic>(&self, payload: &T::Payload) {
        let mut seq = self
            .inner
            .seq
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.inner.sender.receiver_count() == 0 || self.frozen() {
            return;
        }
        let next = *seq + 1;
        let json = match envelope_json(T::NAME, next, payload) {
            Ok(json) => json,
            Err(e) => {
                error!(topic = T::NAME, error = %e, "frame dropped: the payload cannot be written as JSON");
                return;
            }
        };
        if json.len() > FRAME_MAX {
            error!(
                topic = T::NAME,
                bytes = json.len(),
                "frame dropped: the payload is over the frame limit"
            );
            return;
        }
        *seq = next;
        // `send` fails only when there is no receiver, and `receiver_count` was just checked
        // under the same lock. A client that leaves in between does no harm.
        let _ = self.inner.sender.send(Arc::new(FrameBytes {
            topic: T::NAME,
            seq: next,
            json,
        }));
    }

    /// Whether a client listens. A publisher that has to build an expensive payload asks first.
    pub fn has_receivers(&self) -> bool {
        self.inner.sender.receiver_count() > 0
    }

    /// Stops every further topic frame for good. Game mode calls it after `done`, so that `done`
    /// stays the last topic frame even if a module publishes while it stops (foundation 5.4).
    /// The `lifecycle` tail of a stream is not a topic frame and is not affected.
    pub fn freeze(&self) {
        // Taking the lock makes a `publish` that is under way finish first, so it cannot be the
        // frame that comes after the freeze.
        let _order = self
            .inner
            .seq
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.inner.frozen.store(true, Ordering::SeqCst);
    }

    pub fn frozen(&self) -> bool {
        self.inner.frozen.load(Ordering::SeqCst)
    }

    /// A new receiver. Only the stream writer calls this.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<FrameBytes>> {
        self.inner.sender.subscribe()
    }

    /// The number of the last frame that was published; 0 before the first.
    pub fn seq(&self) -> u64 {
        *self
            .inner
            .seq
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::{Frame, Jobs, Sessions};
    use serde_json::{Value, json};
    use tokio::sync::broadcast::error::TryRecvError;

    #[test]
    fn a_published_frame_reaches_a_subscriber_with_the_next_number() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        bus.publish::<Sessions>(&json!([{ "id": "s1" }]));
        bus.publish::<Jobs>(&json!([]));

        let first = receiver.try_recv().unwrap();
        let second = receiver.try_recv().unwrap();
        assert_eq!((first.topic, first.seq), ("sessions", 1));
        assert_eq!((second.topic, second.seq), ("jobs", 2));
        assert_eq!(bus.seq(), 2);
    }

    #[test]
    fn the_json_is_the_frame_envelope_of_rain_types() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        bus.publish::<Sessions>(&json!({ "a": [1, 2] }));

        let frame: Frame = serde_json::from_str(&receiver.try_recv().unwrap().json).unwrap();
        assert_eq!(frame.topic, "sessions");
        assert_eq!(frame.seq, 1);
        assert_eq!(frame.payload, json!({ "a": [1, 2] }));
    }

    #[test]
    fn nothing_is_published_while_no_client_is_connected() {
        let bus = EventBus::new();
        bus.publish::<Sessions>(&json!([]));
        assert_eq!(bus.seq(), 0, "the counter does not move without a receiver");
    }

    #[test]
    fn a_payload_over_the_frame_limit_is_dropped() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        let too_big = Value::String("x".repeat(FRAME_MAX));
        bus.publish::<Sessions>(&too_big);

        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(bus.seq(), 0);
    }

    #[test]
    fn a_slow_receiver_is_told_how_many_frames_it_missed() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        for _ in 0..BUS_CAPACITY + 10 {
            bus.publish::<Sessions>(&json!([]));
        }
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Lagged(10))));
        assert_eq!(
            receiver.try_recv().unwrap().seq,
            11,
            "the oldest 10 are gone"
        );
    }

    #[test]
    fn after_a_freeze_no_topic_frame_is_published() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        bus.publish::<Sessions>(&json!([1]));
        bus.freeze();
        bus.publish::<Jobs>(&json!([2]));

        assert!(bus.frozen());
        assert_eq!(receiver.try_recv().unwrap().topic, "sessions");
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(bus.seq(), 1, "the counter did not move after the freeze");
    }

    #[test]
    fn has_receivers_follows_the_connected_clients() {
        let bus = EventBus::new();
        assert!(!bus.has_receivers());
        let receiver = bus.subscribe();
        assert!(bus.has_receivers());
        drop(receiver);
        assert!(!bus.has_receivers());
    }
}
