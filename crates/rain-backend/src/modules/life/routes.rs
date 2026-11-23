//! The routes of Life. Section 04 section 2. No logic here: every handler is one call of
//! `life::Service`. The tick handler is the only caller of `Service::tick`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rain_types::{
    CalendarEvent, Habit, HabitDay, HabitDraft, HabitStats, HabitSummary, HabitTickBody,
    OpenNoteResult, PropertySuggestion, VaultStatus,
};
use serde::Deserialize;

use crate::core::error::{ApiJson, ApiPath, ApiQuery, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vault", get(vault))
        .route("/vault/reread", post(reread))
        .route("/vault/properties", get(properties))
        .route("/habits", get(list_habits).post(create_habit))
        .route("/habits/{id}", put(update_habit).delete(delete_habit))
        .route("/habits/{id}/stats", get(habit_stats))
        .route("/habits/{id}/days/{date}", put(tick))
        .route("/habit-days", get(habit_days))
        .route("/calendar/events", get(calendar_events))
        .route("/daily-notes/{date}/open", post(open_note))
}

async fn vault(State(state): State<AppState>) -> ApiResult<Json<VaultStatus>> {
    state.life().vault_status().await.map(Json)
}

async fn reread(State(state): State<AppState>) -> ApiResult<StatusCode> {
    state.life().reread().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn properties(State(state): State<AppState>) -> ApiResult<Json<Vec<PropertySuggestion>>> {
    state.life().suggestions().await.map(Json)
}

async fn list_habits(State(state): State<AppState>) -> ApiResult<Json<Vec<HabitSummary>>> {
    state.life().list_habits().await.map(Json)
}

async fn create_habit(
    State(state): State<AppState>,
    ApiJson(draft): ApiJson<HabitDraft>,
) -> ApiResult<(StatusCode, Json<Habit>)> {
    let habit = state.life().save_habit(None, draft).await?;
    Ok((StatusCode::CREATED, Json(habit)))
}

async fn update_habit(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(draft): ApiJson<HabitDraft>,
) -> ApiResult<Json<Habit>> {
    state.life().save_habit(Some(&id), draft).await.map(Json)
}

async fn delete_habit(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<StatusCode> {
    state.life().delete_habit(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn habit_stats(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<HabitStats>> {
    state.life().habit_stats(&id).await.map(Json)
}

#[derive(Deserialize)]
struct DaysQuery {
    from: String,
    to: String,
    habit: Option<String>,
}

async fn habit_days(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<DaysQuery>,
) -> ApiResult<Json<Vec<HabitDay>>> {
    state
        .life()
        .habit_days(&query.from, &query.to, query.habit.as_deref())
        .await
        .map(Json)
}

/// The tick: no logic, one call.
async fn tick(
    State(state): State<AppState>,
    ApiPath((id, date)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<HabitTickBody>,
) -> ApiResult<Json<HabitSummary>> {
    state.life().tick(&id, &date, body.done).await.map(Json)
}

#[derive(Deserialize)]
struct RangeQuery {
    from: String,
    to: String,
}

async fn calendar_events(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<RangeQuery>,
) -> ApiResult<Json<Vec<CalendarEvent>>> {
    state
        .life()
        .calendar_events(&query.from, &query.to)
        .map(Json)
}

async fn open_note(
    State(state): State<AppState>,
    ApiPath(date): ApiPath<String>,
) -> ApiResult<Json<OpenNoteResult>> {
    state.life().open_note(&date).await.map(Json)
}
