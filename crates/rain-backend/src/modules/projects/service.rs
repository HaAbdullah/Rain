//! The logic and the in-memory state of this module. Other modules call it, and only it.

use rain_types::{LaunchTarget, SearchItem};

use crate::core::error::{ApiError, ApiResult};
use crate::core::launch_contract::{ItemSource, LaunchPlan, LaunchPlanner};

/// Stub. Section 02 builds the project list. It already implements the two traits of the launch
/// contract, with empty answers, so that the launcher compiles and runs before projects exist.
#[derive(Debug, Default)]
pub struct Service;

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
        Err(
            ApiError::unavailable("projects", "Projects are not available yet.")
                .with_reason("notBuilt"),
        )
    }
}
