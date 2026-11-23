//! The routes of game mode. Section 06 2.1.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use rain_types::{GameModeChoiceBody, GameModeState, StartGameModeBody};

use crate::core::error::{ApiJson, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/game-mode", get(read).post(start))
        .route("/game-mode/choice", post(choose))
        .route("/game-mode/recap/seen", post(recap_seen))
}

async fn read(State(state): State<AppState>) -> Json<GameModeState> {
    Json(state.gamemode().read())
}

/// Starts game mode. While a run is under way it answers with that run and ignores the body.
async fn start(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<StartGameModeBody>,
) -> ApiResult<Json<GameModeState>> {
    state.gamemode().start(body).await.map(Json)
}

async fn choose(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<GameModeChoiceBody>,
) -> ApiResult<Json<GameModeState>> {
    state.gamemode().choose(body.choice).map(Json)
}

async fn recap_seen(State(state): State<AppState>) -> ApiResult<StatusCode> {
    state.gamemode().mark_recap_seen().await?;
    Ok(StatusCode::NO_CONTENT)
}
