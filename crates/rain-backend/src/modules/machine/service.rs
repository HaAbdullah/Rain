//! The logic and the in-memory state of this module. Other modules call it, and only it.

/// Stub. Section 05 builds the machine state (`GET /machine`).
#[derive(Debug, Default)]
pub struct Service;

impl Service {
    /// Milliseconds since the last keyboard or mouse input of this session; `None` when Windows
    /// refuses. The idle jobs of section 03 call this instead of the platform function, so that
    /// there is one reading of the idle time.
    pub fn idle_ms(&self) -> Option<u64> {
        crate::platform::idle::idle_ms()
    }
}
