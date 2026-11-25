//! The routes of the VS Code themes module:
//!
//! - `GET /themes/vscode` lists the themes of the installed extensions;
//! - `GET /themes/vscode/theme?id=` returns the Rain colour tokens of one theme;
//! - `GET /themes/vscode/current` says which theme VS Code uses now.
//!
//! Each call reads the files again, on a blocking thread.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use rain_types::{VsCodeCurrentTheme, VsCodeTheme, VsCodeThemeTokens};
use serde::Deserialize;

use super::reader::{self, SystemThemeSource, ThemeError, ThemeSource};
use super::tokens::map_tokens;
use crate::core::error::{ApiError, ApiQuery, ApiResult};
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/themes/vscode", get(list_themes))
        .route("/themes/vscode/theme", get(get_theme))
        .route("/themes/vscode/current", get(get_current))
}

#[derive(Deserialize)]
struct ThemeQuery {
    id: String,
}

/// A test's fake folders, else the real ones.
fn source(state: &AppState) -> Arc<dyn ThemeSource> {
    state
        .seam::<dyn ThemeSource>()
        .unwrap_or_else(|| Arc::new(SystemThemeSource::new()))
}

/// Runs a blocking read off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ApiError::failed("Could not read the VS Code themes"))
}

async fn list_themes(State(state): State<AppState>) -> ApiResult<Json<Vec<VsCodeTheme>>> {
    let source = source(&state);
    Ok(Json(blocking(move || reader::list(source.as_ref())).await?))
}

async fn get_current(State(state): State<AppState>) -> ApiResult<Json<VsCodeCurrentTheme>> {
    let source = source(&state);
    Ok(Json(
        blocking(move || reader::current(source.as_ref())).await?,
    ))
}

async fn get_theme(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ThemeQuery>,
) -> ApiResult<Json<VsCodeThemeTokens>> {
    let source = source(&state);
    let result = blocking(move || {
        let found = reader::find(source.as_ref(), &query.id)?;
        let colors = reader::read_colors(&found)?;
        Ok::<_, ThemeError>(VsCodeThemeTokens {
            id: found.theme.id,
            label: found.theme.label,
            dark: found.theme.dark,
            tokens: map_tokens(&colors, found.theme.dark),
        })
    })
    .await?;
    match result {
        Ok(theme) => Ok(Json(theme)),
        Err(ThemeError::NotFound) => Err(ApiError::not_found("There is no such VS Code theme")),
        Err(ThemeError::Unreadable(why)) => Err(ApiError::failed(format!(
            "Could not read that VS Code theme: {why}"
        ))),
    }
}
