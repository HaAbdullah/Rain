//! The routes of the settings. Section 06 2.1.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use rain_types::{Settings, SettingsPatch};

use crate::core::error::{ApiJson, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/settings", get(get_settings).patch(patch_settings))
}

async fn get_settings(State(state): State<AppState>) -> Json<Settings> {
    Json(state.slots.settings.get().read().await)
}

/// Answers with the complete new `Settings`. A patch that equals the current state writes nothing.
async fn patch_settings(
    State(state): State<AppState>,
    ApiJson(patch): ApiJson<SettingsPatch>,
) -> ApiResult<Json<Settings>> {
    state.slots.settings.get().update(patch).await.map(Json)
}
