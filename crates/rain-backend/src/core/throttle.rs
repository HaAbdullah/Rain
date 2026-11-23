//! Limits how often a topic is published. Foundation sections 5.3 and 5.4.
//!
//! A topic that can change faster than `TOPIC_RATE_MAX` times a second (scan progress, a launch
//! run, the game mode countdown) goes through a `Throttle`. The newest value always wins: a value
//! that arrives too early waits, and a still newer one replaces it. A run that ends calls
//! `publish_final`, which drops the waiting value and publishes at once, so that no stale
//! progress frame can arrive after the end.

use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rain_types::Topic;
use tokio::time::Instant;

use super::bus::EventBus;

pub struct Throttle<T: Topic> {
    shared: Arc<Shared<T>>,
}

struct Shared<T: Topic> {
    bus: EventBus,
    /// The shortest time between two frames.
    gap: Duration,
    state: Mutex<State<T::Payload>>,
    topic: PhantomData<fn() -> T>,
}

struct State<P> {
    last_sent: Option<Instant>,
    /// The newest value that arrived too early, waiting for the timer.
    pending: Option<P>,
    timer_running: bool,
}

impl<T: Topic> Clone for Throttle<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Throttle<T>
where
    T: Topic + 'static,
    T::Payload: Clone + Send + Sync + 'static,
{
    /// At most `max_per_second` frames a second (a topic of the foundation uses
    /// `TOPIC_RATE_MAX`, or the rate its section names).
    pub fn new(bus: EventBus, max_per_second: u32) -> Self {
        Self {
            shared: Arc::new(Shared {
                bus,
                gap: Duration::from_secs(1) / max_per_second.max(1),
                state: Mutex::new(State {
                    last_sent: None,
                    pending: None,
                    timer_running: false,
                }),
                topic: PhantomData,
            }),
        }
    }

    /// Publishes `payload` now when the gap since the last frame is over, else keeps it as the
    /// waiting value. A timer sends the waiting value when the gap ends. Needs a tokio runtime.
    pub fn publish(&self, payload: &T::Payload) {
        let mut state = self.shared.lock();
        let ready = !state.timer_running && self.shared.gap_is_over(state.last_sent);
        if ready {
            self.shared.send(&mut state, payload);
            return;
        }
        state.pending = Some(payload.clone());
        if !state.timer_running {
            state.timer_running = true;
            let wait_until = state
                .last_sent
                .map_or_else(Instant::now, |at| at + self.shared.gap);
            tokio::spawn(Arc::clone(&self.shared).flush_at(wait_until));
        }
    }

    /// Drops the waiting value and publishes `payload` at once. Call it for the last state of a
    /// run: nothing that was waiting can arrive behind it.
    pub fn publish_final(&self, payload: &T::Payload) {
        let mut state = self.shared.lock();
        state.pending = None;
        self.shared.send(&mut state, payload);
    }
}

impl<T> Shared<T>
where
    T: Topic + 'static,
    T::Payload: Clone + Send + Sync + 'static,
{
    fn lock(&self) -> MutexGuard<'_, State<T::Payload>> {
        // Every change of the state is one assignment, so a poisoned lock is still consistent.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn gap_is_over(&self, last_sent: Option<Instant>) -> bool {
        last_sent.is_none_or(|at| at.elapsed() >= self.gap)
    }

    /// Publishes while the caller holds the lock, so that frames of one topic keep their order.
    fn send(&self, state: &mut State<T::Payload>, payload: &T::Payload) {
        state.last_sent = Some(Instant::now());
        self.bus.publish::<T>(payload);
    }

    /// The timer of a waiting value: one `sleep_until` that exists only while a value waits
    /// (timer T5 of foundation 11.5).
    async fn flush_at(self: Arc<Self>, when: Instant) {
        tokio::time::sleep_until(when).await;
        let mut state = self.lock();
        state.timer_running = false;
        if let Some(payload) = state.pending.take() {
            self.send(&mut state, &payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::Sessions;
    use serde_json::{Value, json};
    use tokio::sync::broadcast::Receiver;

    use crate::core::bus::FrameBytes;

    fn throttle() -> (Throttle<Sessions>, Receiver<Arc<FrameBytes>>) {
        let bus = EventBus::new();
        let receiver = bus.subscribe();
        (Throttle::new(bus, 4), receiver)
    }

    fn payload_of(frame: &FrameBytes) -> Value {
        serde_json::from_str::<rain_types::Frame>(&frame.json)
            .unwrap()
            .payload
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_value_is_published_at_once() {
        let (throttle, mut receiver) = throttle();
        throttle.publish(&json!(1));
        assert_eq!(payload_of(&receiver.try_recv().unwrap()), json!(1));
    }

    #[tokio::test(start_paused = true)]
    async fn values_that_arrive_too_early_are_replaced_by_the_newest() {
        let (throttle, mut receiver) = throttle();
        throttle.publish(&json!(1));
        throttle.publish(&json!(2));
        throttle.publish(&json!(3));
        assert_eq!(payload_of(&receiver.try_recv().unwrap()), json!(1));
        assert!(receiver.try_recv().is_err(), "2 and 3 wait for the gap");

        tokio::time::sleep(Duration::from_millis(260)).await;
        assert_eq!(payload_of(&receiver.try_recv().unwrap()), json!(3));
        assert!(receiver.try_recv().is_err(), "2 was dropped");
    }

    #[tokio::test(start_paused = true)]
    async fn a_value_after_the_gap_goes_out_at_once() {
        let (throttle, mut receiver) = throttle();
        throttle.publish(&json!(1));
        tokio::time::sleep(Duration::from_millis(300)).await;
        throttle.publish(&json!(2));
        assert_eq!(payload_of(&receiver.try_recv().unwrap()), json!(1));
        assert_eq!(payload_of(&receiver.try_recv().unwrap()), json!(2));
    }

    #[tokio::test(start_paused = true)]
    async fn the_final_value_cancels_the_waiting_one_and_is_published_at_once() {
        let (throttle, mut receiver) = throttle();
        throttle.publish(&json!("progress 1"));
        throttle.publish(&json!("progress 2"));
        throttle.publish_final(&json!("done"));
        tokio::time::sleep(Duration::from_secs(1)).await;

        assert_eq!(
            payload_of(&receiver.try_recv().unwrap()),
            json!("progress 1")
        );
        assert_eq!(payload_of(&receiver.try_recv().unwrap()), json!("done"));
        assert!(
            receiver.try_recv().is_err(),
            "no stale progress frame behind the final one"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_clone_shares_the_gap() {
        let (throttle, mut receiver) = throttle();
        let clone = throttle.clone();
        throttle.publish(&json!(1));
        clone.publish(&json!(2));
        assert_eq!(payload_of(&receiver.try_recv().unwrap()), json!(1));
        assert!(receiver.try_recv().is_err());
    }
}
