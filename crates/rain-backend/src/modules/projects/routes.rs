//! The routes of the projects module. Section 02 2.
//!
//! `GET /projects`, `POST /projects/refresh`, and `GET` and `PUT /projects/{id}/layout`. Opening a project is `POST /launches` of the
//! launcher, which asks this module for the plan.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use rain_types::{LayoutSave, Project, ProjectLayout};

use super::service::Refresh;
use crate::core::error::{ApiError, ApiJson, ApiPath, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects", get(list))
        .route("/projects/refresh", post(refresh))
        .route("/projects/{id}/layout", get(layout).put(save_layout))
}

async fn layout(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<ProjectLayout>> {
    Ok(Json(state.projects().layout(&id).await?))
}

async fn save_layout(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(save): ApiJson<LayoutSave>,
) -> ApiResult<Json<ProjectLayout>> {
    if state.game_mode_active() {
        return Err(ApiError::conflict("gameMode", "Game mode is on").with_source("projects"));
    }
    Ok(Json(state.projects().save_layout(&id, save).await?))
}

async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<Project>>> {
    let projects = state.projects().list(Refresh::Normal).await?;
    Ok(Json(projects.as_ref().clone()))
}

/// Computes every git fact again, waits a few seconds for the answers, and publishes the topic.
async fn refresh(State(state): State<AppState>) -> ApiResult<Json<Vec<Project>>> {
    if state.game_mode_active() {
        return Err(ApiError::conflict("gameMode", "Game mode is on").with_source("projects"));
    }
    let projects = state.projects().list(Refresh::Force).await?;
    state.projects().publish(&projects);
    Ok(Json(projects.as_ref().clone()))
}
