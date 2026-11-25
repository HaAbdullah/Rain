//! The routes of the hardware module: `GET /hardware` reads the PC now, `POST /hardware/{id}/open`
//! starts what a tile of that read opens.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use rain_types::HardwareTile;

use crate::core::error::{ApiPath, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/hardware", get(read_hardware))
        .route("/hardware/{id}/open", post(open_tile))
}

async fn read_hardware(State(state): State<AppState>) -> ApiResult<Json<Vec<HardwareTile>>> {
    state.hardware().read().await.map(Json)
}

async fn open_tile(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<StatusCode> {
    state.hardware().open_tile(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
