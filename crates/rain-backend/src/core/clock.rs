//! Time, as a value that a test can replace. Foundation section 11.4.
//!
//! Code in the backend asks the `Clock` for the time and never calls `SystemTime::now()`, so a
//! test can decide what time it is.

use std::time::SystemTime;

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// The real clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
