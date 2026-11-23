//! The routes of this module. None yet.

use axum::Router;

use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
