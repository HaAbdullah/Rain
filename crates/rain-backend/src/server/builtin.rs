//! The routes that belong to the backend itself, not to a feature module.
//! Foundation sections 2.6, 4.7 and 6.9.
//!
//! `GET /ping`, `GET /health`, `GET /stream`, `GET /media/{kind}/{id}`, `PUT /internal/shell` and
//! `POST /internal/shutdown`. Everything else belongs to a module.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rain_types::{Health, Ping, ShellFacts, ShutdownBody};
use tracing::info;

use super::{media, stream};
use crate::core::error::{ApiJson, ApiResult};
use crate::core::state::AppState;
use crate::modules::lifecycle::check_stop;
use crate::{API_VERSION, APP_NAME};

/// The route of the shutdown, without `/api/v1`. The request filter lets it through while the
/// backend is already stopping, so that a second request is answered like the first.
pub const SHUTDOWN_ROUTE: &str = "/internal/shutdown";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ping", get(ping))
        .route("/health", get(health))
        .route("/stream", get(stream::stream))
        .route("/internal/shell", put(put_shell))
        .route(SHUTDOWN_ROUTE, post(shutdown))
        .merge(media::router())
}

/// Cheap and without side effects: it answers while the backend is stopping as well.
async fn ping(State(state): State<AppState>) -> Json<Ping> {
    Json(Ping {
        app: APP_NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        api_version: API_VERSION,
        start_id: state.start_id.clone(),
        pid: std::process::id(),
        exe_path: state.exe_path.clone(),
        sandbox: state.config.sandbox,
        busy: state.activity.busy(),
    })
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(state.health.snapshot())
}

/// The shell reports what only it knows (foundation 6.5). The backend keeps it in memory, tells
/// `/health` and the page, and answers 204. The same report again changes nothing, except that
/// the lifecycle module looks at the autostart entry after every report.
async fn put_shell(
    State(state): State<AppState>,
    ApiJson(facts): ApiJson<ShellFacts>,
) -> StatusCode {
    let hotkey_registered = facts.hotkey_registered;
    if state.shell_facts.set(facts.clone()) {
        state.health.set_shell(hotkey_registered);
        // Publishes `settings` when `hotkeyAvailable` changed.
        state.settings().on_shell_facts_changed().await;
    }
    state.lifecycle().on_shell_facts(&facts).await;
    StatusCode::NO_CONTENT
}

/// Answers 204 at once and then the backend stops (see `start::run_shutdown`).
///
/// The policy of section 06 6.9 comes first: `upgrade` and `restart` are refused with `busy`
/// while work runs, and `restart` while game mode runs. `force` skips the refusals; it is what
/// `rain-backend --stop --force` sends.
async fn shutdown(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ShutdownBody>,
) -> ApiResult<StatusCode> {
    check_stop(
        body.reason,
        body.force.unwrap_or(false),
        state.activity.busy(),
        state.game_mode_active(),
    )?;
    if state.shutdown.request(body.reason) {
        info!(reason = ?body.reason, "stop requested");
    }
    Ok(StatusCode::NO_CONTENT)
}
