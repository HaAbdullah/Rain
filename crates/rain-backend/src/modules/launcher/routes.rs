//! The routes of the launcher. Section 01 2.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rain_types::{
    AppEntry, LaunchRun, LaunchTarget, PinBody, PinnableType, Pool, PoolDraft, SearchItem,
};
use serde::Deserialize;

use super::search::parse_types;
use super::service::FaultMode;
use crate::core::error::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResult, fallback_404};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/search", get(search))
        .route("/pinned", get(pinned))
        .route("/recent", get(recent))
        .route("/pinned/{kind}/{id}", put(set_pinned))
        .route("/apps", get(list_apps))
        .route("/apps/scan", post(rescan_apps))
        .route("/pools", get(list_pools).post(create_pool))
        .route("/pools/{id}", put(update_pool).delete(delete_pool))
        .route("/launches", post(launch))
        .route("/launches/{id}", get(get_launch))
        .route("/internal/test/apps-source", put(set_apps_source))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchParams {
    q: Option<String>,
    types: Option<String>,
}

async fn search(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<SearchParams>,
) -> ApiResult<Json<Vec<SearchItem>>> {
    let types = parse_types(params.types.as_deref()).map_err(|_| {
        ApiError::invalid("Unknown item type")
            .with_source("launcher")
            .with_reason("unknownType")
    })?;
    state
        .launcher()
        .search(params.q.as_deref().unwrap_or_default(), types.as_deref())
        .map(Json)
}

async fn pinned(State(state): State<AppState>) -> ApiResult<Json<Vec<SearchItem>>> {
    state.launcher().pinned().map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecentParams {
    limit: Option<usize>,
}

async fn recent(
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<RecentParams>,
) -> ApiResult<Json<Vec<SearchItem>>> {
    state.launcher().recent(params.limit).map(Json)
}

async fn set_pinned(
    State(state): State<AppState>,
    ApiPath((kind, id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<PinBody>,
) -> ApiResult<StatusCode> {
    let kind: PinnableType =
        serde_json::from_value(serde_json::Value::String(kind)).map_err(|_| {
            ApiError::invalid("Unknown item type")
                .with_source("launcher")
                .with_reason("unknownType")
        })?;
    state.launcher().set_pinned(kind, &id, body.pinned).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_apps(State(state): State<AppState>) -> ApiResult<Json<Vec<AppEntry>>> {
    state.launcher().list_apps().await.map(Json)
}

async fn rescan_apps(State(state): State<AppState>) -> ApiResult<Json<Vec<AppEntry>>> {
    state.launcher().rescan().await.map(Json)
}

async fn list_pools(State(state): State<AppState>) -> Json<Vec<Pool>> {
    Json(state.launcher().list_pools())
}

async fn create_pool(
    State(state): State<AppState>,
    ApiJson(draft): ApiJson<PoolDraft>,
) -> ApiResult<(StatusCode, Json<Pool>)> {
    let (pool, _) = state.launcher().save_pool(None, draft).await?;
    Ok((StatusCode::CREATED, Json(pool)))
}

async fn update_pool(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(draft): ApiJson<PoolDraft>,
) -> ApiResult<Json<Pool>> {
    let (pool, _) = state.launcher().save_pool(Some(&id), draft).await?;
    Ok(Json(pool))
}

async fn delete_pool(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<StatusCode> {
    state.launcher().delete_pool(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 201 for a new run, 200 when the run of the same target is still running and is returned.
async fn launch(
    State(state): State<AppState>,
    ApiJson(target): ApiJson<LaunchTarget>,
) -> ApiResult<(StatusCode, Json<LaunchRun>)> {
    let (run, created) = state.launcher().launch(target).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(run)))
}

async fn get_launch(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<LaunchRun>> {
    state.launcher().get_launch(&id).map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultBody {
    mode: FaultName,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum FaultName {
    Normal,
    Empty,
    Fail,
}

/// Dev only (section 01 11.3): makes the app source answer normally, with nothing, or with an
/// error, so that the empty and failing screens can be seen. A real run has no such route.
async fn set_apps_source(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<FaultBody>,
) -> ApiResult<StatusCode> {
    if !state.config.sandbox {
        return Err(fallback_404().await);
    }
    state.launcher().set_fault(match body.mode {
        FaultName::Normal => FaultMode::Normal,
        FaultName::Empty => FaultMode::Empty,
        FaultName::Fail => FaultMode::Fail,
    });
    Ok(StatusCode::NO_CONTENT)
}
