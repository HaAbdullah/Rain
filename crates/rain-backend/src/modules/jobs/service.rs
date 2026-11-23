//! The logic and the in-memory state of this module. Other modules call it, and only it.

use crate::core::winddown::{NothingToWindDown, WindDownPort};

/// Stub. Section 03 builds the scheduler and the runs.
#[derive(Default)]
pub struct Service {
    wind_down: NothingToWindDown,
}

impl Service {
    /// How game mode ends the runs of scheduled jobs (section 06 6.7.1).
    pub fn wind_down(&self) -> &dyn WindDownPort {
        &self.wind_down
    }
}
