//! The routes of this module (section 03 2.1):
//!
//! | Method | Path | Does |
//! |---|---|---|
//! | GET | `/sessions` | the list: live sessions and the ones that ended in the last 24 hours |
//! | POST | `/sessions/{id}/permission` | answers an open permission request |
//! | POST | `/sessions/{id}/focus` | brings the window of the session to the front |
//! | POST | `/sessions/{id}/stop` | ends the process of a live terminal session |
//! | GET | `/sessions/{id}/transcript` | the readable log |
//! | POST | `/internal/hook` | the events of `rain-hook` (never forwarded by the shell) |

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use rain_types::{
    ClaudeSession, FocusResult, LogLine, PermissionAnswerBody, PermissionAnswerResult,
};

use super::hook_wire::HookEnvelope;
use super::service::HookReply;
use crate::core::error::{ApiError, ApiJson, ApiPath, ApiResult};
use crate::core::state::AppState;

/// The hook route takes a larger body than the others: a `Write` tool call may carry a whole
/// file (foundation 3.4 lets a route say so).
const HOOK_BODY_LIMIT: usize = 4 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/sessions", get(list))
        .route("/sessions/{id}/permission", post(answer))
        .route("/sessions/{id}/focus", post(focus))
        .route("/sessions/{id}/stop", post(stop))
        .route("/sessions/{id}/transcript", get(transcript))
        .route(
            "/internal/hook",
            post(hook).layer(DefaultBodyLimit::max(HOOK_BODY_LIMIT)),
        )
}

async fn list(State(state): State<AppState>) -> Json<Vec<ClaudeSession>> {
    Json(state.sessions().list_refreshed().await)
}

async fn answer(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<PermissionAnswerBody>,
) -> ApiResult<Json<PermissionAnswerResult>> {
    state.sessions().answer(&id, body).await.map(Json)
}

async fn focus(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<FocusResult>> {
    state.sessions().focus(&id).await.map(Json)
}

async fn stop(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<StatusCode> {
    state.sessions().stop_session(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn transcript(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Vec<LogLine>>> {
    state.sessions().transcript(&id).await.map(Json)
}

/// For `mode: permission` the answer is the JSON that `rain-hook` prints verbatim for Claude Code
/// (`200`); in every other case `204`. The body is parsed here and not by the usual extractor, so
/// that a bad envelope has its own reason, `badEnvelope`.
async fn hook(
    State(state): State<AppState>,
    body: Result<Bytes, BytesRejection>,
) -> ApiResult<Response> {
    // A body over the limit is refused with the one error body, like on every other route.
    let body = body.map_err(|_| {
        ApiError::invalid("The hook request was too large").with_reason("bodyTooLarge")
    })?;
    let envelope = HookEnvelope::parse(&body).ok_or_else(|| {
        ApiError::invalid("The hook request was not valid").with_reason("badEnvelope")
    })?;
    Ok(match state.sessions().handle_hook(envelope).await {
        HookReply::Empty => StatusCode::NO_CONTENT.into_response(),
        HookReply::Decision(json) => {
            let mut response = (StatusCode::OK, json).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            response
        }
    })
}
