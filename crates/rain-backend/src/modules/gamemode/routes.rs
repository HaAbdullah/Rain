//! The routes of game mode. Section 06 2.1.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use rain_types::{GameModeChoiceBody, GameModeState, OtherApps, OtherAppsPatch, StartGameModeBody};
use serde::Deserialize;

use crate::core::error::{ApiJson, ApiQuery, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/game-mode", get(read).post(start))
        .route("/game-mode/choice", post(choose))
        .route("/game-mode/recap/seen", post(recap_seen))
        .route("/game-mode/other-apps", patch(other_apps))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadQuery {
    /// `true`: add the list of the programs that would be closed (this scans the processes).
    #[serde(default)]
    other_apps: bool,
}

async fn read(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ReadQuery>,
) -> Json<GameModeState> {
    if query.other_apps {
        Json(state.gamemode().read_with_apps().await)
    } else {
        Json(state.gamemode().read())
    }
}

/// The master switch and the choice for one program.
async fn other_apps(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<OtherAppsPatch>,
) -> ApiResult<Json<OtherApps>> {
    state.gamemode().patch_other_apps(body).await.map(Json)
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
