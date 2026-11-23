//! The logic and the in-memory state of this module. Other modules call it, and only it.

use futures_util::future::BoxFuture;
use rain_types::{LaunchTarget, SearchItem};

use crate::core::error::{ApiError, ApiResult};
use crate::core::launch_contract::{ItemSource, LaunchPlan, LaunchPlanner};

/// Stub. Section 02 builds the game library. It already implements the two traits of the launch
/// contract, with empty answers, so that the launcher compiles and runs before games exist.
#[derive(Debug, Default)]
pub struct Service;

/// What `launch_after_game_mode` tells game mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchedAfterGameMode {
    /// True when the backend itself runs inside a job, so the game may close with Rain.
    pub in_job: bool,
}

impl Service {
    /// Starts a game outside Rain's process tree for game mode, and records "last played"
    /// (section 06 6.7.1). Returns within 10 seconds.
    pub fn launch_after_game_mode(
        &self,
        _id: &str,
    ) -> BoxFuture<'_, ApiResult<LaunchedAfterGameMode>> {
        Box::pin(async { Err(not_built()) })
    }
}

impl ItemSource for Service {
    fn search_entries(&self) -> Vec<(SearchItem, Vec<String>)> {
        Vec::new()
    }

    fn item_for(&self, _id: &str) -> Option<SearchItem> {
        None
    }

    fn recent(&self) -> Vec<(String, SearchItem)> {
        Vec::new()
    }
}

impl LaunchPlanner for Service {
    fn plan(&self, _target: &LaunchTarget) -> ApiResult<LaunchPlan> {
        Err(not_built())
    }
}

fn not_built() -> ApiError {
    ApiError::unavailable("games", "Games are not available yet.").with_reason("notBuilt")
}
