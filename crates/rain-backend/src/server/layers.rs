//! The layers around every route: what is checked before a handler runs, and what is added to
//! every answer. Foundation sections 3.4 and 4.2.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rain_types::limits::{API_PREFIX, URL_MAX};
use tracing::debug;

use super::builtin::SHUTDOWN_ROUTE;
use crate::core::error::{
    method_not_allowed_error, shutting_down_error, timeout_error, url_too_long_error,
};
use crate::core::state::AppState;

/// The outermost layer. Logs one line for every request and sets the headers of every answer:
/// `Cache-Control: no-store` (a route that wants caching sets its own), and the charset of JSON.
pub async fn observe(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let mut response = next.run(request).await;

    let headers = response.headers_mut();
    // axum's `Sse` sets `no-cache` on a stream; foundation 5.1 wants `no-store` for it as well.
    let is_stream = headers
        .get(CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"text/event-stream"));
    if is_stream || !headers.contains_key(CACHE_CONTROL) {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if headers
        .get(CONTENT_TYPE)
        .is_some_and(|v| v == "application/json")
    {
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    // The query is not logged: it may hold a search text.
    debug!(
        %method,
        path,
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "request"
    );
    response
}

/// Refuses a request before a handler sees it:
///
/// - a URL longer than `URL_MAX`;
/// - the methods `HEAD` and `OPTIONS`, which the API does not have (axum would answer `HEAD` for
///   every `GET` route by itself);
/// - while the backend stops, everything that is not a read. The shutdown route itself passes, so
///   that a second request gets the same answer as the first.
pub async fn check_request(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let url_length = request
        .uri()
        .path_and_query()
        .map_or(0, |p| p.as_str().len());
    if url_length > URL_MAX {
        return url_too_long_error().into_response();
    }
    let method = request.method();
    if matches!(*method, Method::HEAD | Method::OPTIONS) {
        return method_not_allowed_error().into_response();
    }
    let is_shutdown_route = request.uri().path().strip_prefix(API_PREFIX) == Some(SHUTDOWN_ROUTE);
    if state.shutdown.shutting_down() && *method != Method::GET && !is_shutdown_route {
        return shutting_down_error().into_response();
    }
    next.run(request).await
}

/// The route that holds a request open longer than `request_timeout` on purpose: the hook route
/// waits up to 19 seconds for the user's answer to a permission request (section 03 6.5.1).
/// Everything else has the one time limit of foundation 3.4.
const HOLDING_ROUTE: &str = "/internal/hook";

/// A handler that takes longer than `Limits::request_timeout` is answered with `unavailable`
/// (reason `timeout`). The permission hold of `HOLDING_ROUTE` is the one exception.
pub async fn limit_time(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request.uri().path().strip_prefix(API_PREFIX) == Some(HOLDING_ROUTE) {
        return next.run(request).await;
    }
    match tokio::time::timeout(state.config.limits.request_timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => timeout_error().into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use rain_types::StopReason;
    use tower::ServiceExt;

    use crate::config::Config;
    use crate::core::state::AppState;
    use crate::server::router;

    fn state() -> AppState {
        AppState::builder(Config::for_test("rain-unit", "data".into(), "local".into())).build()
    }

    async fn status_of(state: &AppState, method: &str, path: &str) -> StatusCode {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .expect("a valid request");
        // `oneshot` runs one request through the whole router and its layers, without a pipe.
        router::build(state.clone(), axum::Router::new())
            .oneshot(request)
            .await
            .expect("the router never fails")
            .status()
    }

    #[tokio::test]
    async fn while_the_backend_stops_a_write_is_refused_and_a_read_is_answered() {
        let state = state();
        state.shutdown.request(StopReason::Stop);

        assert_eq!(
            status_of(&state, "POST", "/api/v1/anything").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_of(&state, "GET", "/api/v1/ping").await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn the_shutdown_route_is_still_answered_while_the_backend_stops() {
        let state = state();
        state.shutdown.request(StopReason::Stop);
        // The route is reached, not refused: its body is missing, so the extractor answers 400.
        assert_eq!(
            status_of(&state, "POST", "/api/v1/internal/shutdown").await,
            StatusCode::BAD_REQUEST
        );
    }
}
