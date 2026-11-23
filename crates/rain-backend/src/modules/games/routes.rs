//! The routes of this module (section 02, 2):
//! `GET /games`, `POST /games/refresh`, `POST /games`, `PATCH /games/{id}`, `DELETE /games/{id}`.
//! Starting a game is `POST /launches` of the launcher.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use rain_types::{CustomGameDraft, Game, GameLibrary, GamePatch};

use crate::core::error::{ApiError, ApiJson, ApiPath, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/games", get(list).post(create))
        .route("/games/refresh", post(refresh))
        .route("/games/{id}", patch(update).delete(remove))
}

/// A refresh rescans the disk, which game mode does not want.
fn refuse_in_game_mode(state: &AppState) -> ApiResult<()> {
    if state.game_mode_active() {
        return Err(ApiError::conflict("gameMode", "Game mode is on").with_source("games"));
    }
    Ok(())
}

async fn list(State(state): State<AppState>) -> ApiResult<Json<GameLibrary>> {
    Ok(Json(state.games().list().await?))
}

async fn refresh(State(state): State<AppState>) -> ApiResult<Json<GameLibrary>> {
    refuse_in_game_mode(&state)?;
    Ok(Json(state.games().refresh().await?))
}

async fn create(
    State(state): State<AppState>,
    ApiJson(draft): ApiJson<CustomGameDraft>,
) -> ApiResult<(StatusCode, Json<Game>)> {
    let game = state.games().save_custom(draft).await?;
    Ok((StatusCode::CREATED, Json(game)))
}

async fn update(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(change): ApiJson<GamePatch>,
) -> ApiResult<Json<Game>> {
    Ok(Json(state.games().patch(&id, change).await?))
}

async fn remove(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<StatusCode> {
    state.games().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
