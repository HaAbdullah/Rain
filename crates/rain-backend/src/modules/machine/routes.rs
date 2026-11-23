//! The route of the machine module: `GET /machine`.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use rain_types::MachineState;

use crate::core::error::{ApiError, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/machine", get(get_machine))
}

async fn get_machine(State(state): State<AppState>) -> ApiResult<Json<MachineState>> {
    state.machine().state().map(Json).ok_or_else(|| {
        ApiError::unavailable("machine", "Could not read the idle time")
            .with_reason("idleUnavailable")
    })
}
