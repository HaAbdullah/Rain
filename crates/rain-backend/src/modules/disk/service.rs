//! The logic and the in-memory state of this module. Other modules call it, and only it.

/// Stub. Section 05 builds the scans.
#[derive(Debug, Default)]
pub struct Service;

impl Service {
    /// Cancels a running disk scan at once. Game mode calls this before it winds down (section
    /// 06 6.7.5). Nothing runs yet, so there is nothing to cancel.
    pub fn cancel_running(&self) {}
}
