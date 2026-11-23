//! Puts the routes of the backend and of all modules together, and wraps them in the layers.
//! Foundation sections 3.4 and 4.5.

use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::DefaultBodyLimit;
use axum::middleware::{from_fn, from_fn_with_state};
use rain_types::limits::API_PREFIX;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;

use super::{builtin, layers};
use crate::core::error::{fallback_404, fallback_405, panic_response};
use crate::core::state::AppState;
use crate::modules::{self, MODULES};

/// The finished router. `extra` is a set of routes added to the API; the product passes an empty
/// router, and a test adds routes that fail on purpose (a panic, a slow answer).
pub fn build(state: AppState, extra: Router<AppState>) -> Router {
    let api = builtin::router()
        .merge(modules::routers(&state, MODULES))
        .merge(extra)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405);

    // Layers, from the outside in. The panic catcher sits inside the logging layer, so that the
    // 500 it makes is logged and gets the same headers as every other answer.
    let layers = ServiceBuilder::new()
        .layer(from_fn(layers::observe))
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(from_fn_with_state(state.clone(), layers::check_request))
        .layer(HandleErrorLayer::new(layers::handle_timeout))
        .timeout(state.config.limits.request_timeout)
        .layer(DefaultBodyLimit::max(state.config.limits.body_limit));

    Router::new()
        .nest(API_PREFIX, api)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(layers)
        .with_state(state)
}
