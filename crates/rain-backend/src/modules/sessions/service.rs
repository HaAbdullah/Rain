//! The logic and the in-memory state of this module. Other modules call it, and only it.

use futures_util::future::BoxFuture;
use rain_types::MonitorClaudeState;

use crate::core::error::{ApiError, ApiResult};

/// Stub. Section 03 builds the sessions and the hooks installer.
#[derive(Debug, Default)]
pub struct Service;

impl Service {
    /// The number of open Claude sessions that are not runs of Rain's own jobs. Game mode lists
    /// them and never stops them (decision of 2026-11-22).
    pub fn open_user_sessions(&self) -> u32 {
        0
    }

    /// The state of the switch "Monitor Claude sessions" (section 06 6.1.1).
    pub fn hooks_state(&self) -> MonitorClaudeState {
        MonitorClaudeState::Off
    }

    /// Puts Rain's hooks into the Claude settings. Safe to repeat. The real one refuses a
    /// target outside a throwaway place in a sandbox run (foundation 13.3, rule 2).
    pub fn install_hooks(&self) -> BoxFuture<'_, ApiResult<()>> {
        Box::pin(async { Err(not_built()) })
    }

    /// Removes Rain's hooks again. Safe to repeat.
    pub fn remove_hooks(&self) -> BoxFuture<'_, ApiResult<()>> {
        Box::pin(async { Err(not_built()) })
    }
}

fn not_built() -> ApiError {
    ApiError::unavailable("sessions", "This setting is not available yet.").with_reason("notBuilt")
}
