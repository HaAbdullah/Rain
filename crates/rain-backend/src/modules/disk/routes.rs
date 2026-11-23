//! The routes of the disk module. Section 05 2.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use rain_types::{DiskNode, DiskScan, Drive, PathCheck, PathsCheckBody, RevealBody, StartScanBody};
use serde::Deserialize;

use crate::core::error::{ApiJson, ApiPath, ApiQuery, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/drives", get(list_drives))
        .route("/scans", post(start_scan))
        .route("/scans/{id}", get(get_scan).delete(cancel_scan))
        .route("/scans/{id}/node", get(get_scan_node))
        .route("/shell/reveal", post(reveal))
        .route("/paths/check", post(check_paths))
}

async fn list_drives(State(state): State<AppState>) -> ApiResult<Json<Vec<Drive>>> {
    state.disk().list_drives().await.map(Json)
}

/// 201 for a new scan; 200 when a scan of the same drive is already running and is returned.
async fn start_scan(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<StartScanBody>,
) -> ApiResult<(StatusCode, Json<DiskScan>)> {
    let (scan, created) = state.disk().start_scan(&body.drive_id).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(scan)))
}

async fn get_scan(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<DiskScan>> {
    state.disk().get_scan(&id).await.map(Json)
}

/// Cancels a running scan, or frees the tree of a finished one.
async fn cancel_scan(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<StatusCode> {
    state.disk().cancel_scan(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The query of the node route. It is not generated to TypeScript: the REST client builds the
/// query text itself.
#[derive(Deserialize)]
struct ScanNodeQuery {
    path: String,
}

async fn get_scan_node(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiQuery(query): ApiQuery<ScanNodeQuery>,
) -> ApiResult<Json<DiskNode>> {
    state.disk().scan_node(&id, query.path).await.map(Json)
}

async fn reveal(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<RevealBody>,
) -> ApiResult<StatusCode> {
    state.disk().reveal(&body.path).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn check_paths(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<PathsCheckBody>,
) -> ApiResult<Json<Vec<PathCheck>>> {
    // `%VAR%` is expanded by the settings module, the one place that does it, so that the glyph
    // in Settings shows what the projects and games modules will really use.
    let settings = state.settings();
    let expanded = body
        .paths
        .iter()
        .map(|path| settings.expand_env(path))
        .collect();
    state
        .disk()
        .check_paths(body.paths, expanded)
        .await
        .map(Json)
}
