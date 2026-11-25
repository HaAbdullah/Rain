//! The routes of scheduled jobs. Section 03 2.2.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use rain_types::{EnabledBody, JobRun, LogLine, ScheduledJob};

use crate::core::error::{ApiJson, ApiPath, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/jobs", get(list))
        .route("/jobs/{id}", patch(set_enabled))
        .route("/jobs/{id}/runs", post(run_now))
        .route("/runs/{id}/log", get(run_log))
}

/// Also makes the scheduler look at the clock again: a clock that was changed raises no event on
/// Windows, so every list is a free chance to notice (section 03 6.11.3).
async fn list(State(state): State<AppState>) -> Json<Vec<ScheduledJob>> {
    let jobs = state.jobs();
    jobs.nudge();
    Json(jobs.list_with_claude_app().await)
}

async fn set_enabled(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<EnabledBody>,
) -> ApiResult<Json<ScheduledJob>> {
    Ok(Json(state.jobs().set_enabled(&id, body.enabled).await?))
}

/// 201 for a new run; 200 with the run in progress when the job already runs.
async fn run_now(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Response> {
    let (run, created): (JobRun, bool) = state.jobs().run_now(&id).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(run)).into_response())
}

async fn run_log(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Vec<LogLine>>> {
    Ok(Json(state.jobs().run_log(&id).await?))
}
