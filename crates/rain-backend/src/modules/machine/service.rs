//! The machine facts. Other modules call `idle_ms`, and only it.

use std::sync::Arc;

use rain_types::MachineState;
use rain_types::limits::USER_ACTIVE_BELOW_MS;

/// Where the idle time comes from. The seam of the tests.
pub trait IdleSource: Send + Sync {
    /// Milliseconds since the last input of this session, or `None` when Windows refuses.
    fn idle_ms(&self) -> Option<u64>;
}

/// The real source: `GetLastInputInfo`. It works from a process without a window, as long as the
/// process runs in the user's own session (the backend does; a Windows service would not).
pub struct SystemIdle;

impl IdleSource for SystemIdle {
    fn idle_ms(&self) -> Option<u64> {
        crate::platform::idle::idle_ms()
    }
}

pub struct Service {
    source: Arc<dyn IdleSource>,
}

impl Service {
    pub fn new(source: Arc<dyn IdleSource>) -> Self {
        Self { source }
    }

    /// Milliseconds since the last keyboard or mouse input of this session; `None` when Windows
    /// refuses. The idle jobs of section 03 call this instead of the platform function, so that
    /// there is one reading of the idle time. They must read `None` as "not idle".
    pub fn idle_ms(&self) -> Option<u64> {
        self.source.idle_ms()
    }

    /// What `GET /machine` returns. The user counts as active while the last input is younger
    /// than a minute, so "not active" always means at least one whole idle minute.
    pub fn state(&self) -> Option<MachineState> {
        let idle = self.idle_ms()?;
        Some(MachineState {
            user_active: idle < USER_ACTIVE_BELOW_MS,
            idle_minutes: u32::try_from(idle / 60_000).unwrap_or(u32::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Option<u64>);

    impl IdleSource for Fixed {
        fn idle_ms(&self) -> Option<u64> {
            self.0
        }
    }

    fn state_for(idle: Option<u64>) -> Option<MachineState> {
        Service::new(Arc::new(Fixed(idle))).state()
    }

    fn known(idle: u64) -> (bool, u32) {
        let state = state_for(Some(idle)).expect("a reading");
        (state.user_active, state.idle_minutes)
    }

    #[test]
    fn the_user_is_active_for_a_minute_after_the_last_input() {
        assert_eq!(known(0), (true, 0));
        assert_eq!(known(59_999), (true, 0));
        assert_eq!(known(60_000), (false, 1));
        assert_eq!(known(5_999_999), (false, 99));
    }

    #[test]
    fn no_reading_is_no_state_and_no_idle_time() {
        assert!(state_for(None).is_none());
        assert_eq!(Service::new(Arc::new(Fixed(None))).idle_ms(), None);
        assert_eq!(Service::new(Arc::new(Fixed(Some(7)))).idle_ms(), Some(7));
    }

    #[test]
    fn an_absurd_idle_time_does_not_wrap() {
        assert_eq!(known(u64::MAX).1, u32::MAX);
    }

    #[test]
    fn the_real_source_answers() {
        assert!(SystemIdle.idle_ms().is_some());
    }
}
