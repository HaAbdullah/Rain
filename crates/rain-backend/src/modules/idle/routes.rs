//! The routes of idle jobs. Section 03 2.3.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::{get, patch, post};
use rain_types::{EnabledBody, IdleJob, LogLine};

use crate::core::error::{ApiJson, ApiPath, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/idle-jobs", get(list))
        .route("/idle-jobs/{id}", patch(set_enabled))
        .route("/idle-jobs/{id}/run", post(run_now))
        .route("/idle-jobs/{id}/log", get(log))
}

async fn list(State(state): State<AppState>) -> Json<Vec<IdleJob>> {
    Json(state.idle().list())
}

async fn set_enabled(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<EnabledBody>,
) -> ApiResult<Json<IdleJob>> {
    Ok(Json(state.idle().set_enabled(&id, body.enabled).await?))
}

/// 200 also when the job already runs: the running job is returned.
async fn run_now(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<IdleJob>> {
    Ok(Json(state.idle().run_now(&id).await?))
}

async fn log(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Vec<LogLine>>> {
    Ok(Json(state.idle().log(&id).await?))
}
