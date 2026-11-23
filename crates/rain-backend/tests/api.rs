//! `GET /ping`, `GET /health`, and the error body of every failure path (foundation section 4.5),
//! with the limits of section 3.4.

mod common;

use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::{get, post};
use common::{PATIENCE, TestBackend, path_of};
use hyper::StatusCode;
use rain_backend::core::error::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResult};
use rain_backend::core::state::AppState;
use rain_types::{Health, Ping};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ClientOptions;

// ---- routes that fail on purpose; a test adds them to the API ----

#[derive(Deserialize)]
struct SleepQuery {
    ms: u64,
}

async fn sleep(ApiQuery(query): ApiQuery<SleepQuery>) -> ApiResult<()> {
    tokio::time::sleep(Duration::from_millis(query.ms)).await;
    Ok(())
}

async fn item(ApiPath(id): ApiPath<u32>) -> ApiResult<axum::Json<u32>> {
    Ok(axum::Json(id))
}

async fn panics() -> ApiResult<()> {
    panic!("this panic is deliberate: a test of the panic catcher");
}

async fn fails() -> ApiResult<()> {
    Err(ApiError::conflict("busy", "A job is running").with_source("jobs"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    #[allow(dead_code)] // read by serde, never by the handler
    reset_after_seconds: u32,
}

async fn takes_body(ApiJson(_body): ApiJson<Body>) {}

fn failing_routes() -> Router<AppState> {
    Router::new()
        .route("/test/sleep", get(sleep))
        .route("/test/item/{id}", get(item))
        .route("/test/panic", get(panics))
        .route("/test/fails", get(fails))
        .route("/test/body", post(takes_body))
}

async fn backend_with_failing_routes() -> TestBackend {
    TestBackend::builder()
        .extra_routes(failing_routes())
        .start()
        .await
}

/// Checks that an answer has exactly the error body of section 4.5.
fn assert_error(
    (status, body): (StatusCode, Value),
    expected_status: StatusCode,
    code: &str,
    reason: &str,
) {
    assert_eq!(status, expected_status, "body: {body}");
    assert_eq!(body["code"], code, "body: {body}");
    assert_eq!(body["reason"], reason, "body: {body}");
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    let keys: Vec<&String> = body.as_object().expect("an object").keys().collect();
    assert!(
        keys.iter()
            .all(|k| ["code", "message", "source", "reason"].contains(&k.as_str())),
        "unexpected fields in {body}"
    );
}

// ---- ping and health ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ping_describes_the_running_backend() {
    let backend = TestBackend::start().await;
    let ping: Ping = backend.get("/ping").await;

    assert_eq!(ping.app, "rain-backend");
    assert_eq!(ping.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(ping.api_version, 1);
    assert_eq!(ping.pid, std::process::id());
    assert_eq!(ping.start_id, backend.state.start_id);
    assert!(ping.sandbox, "a test backend is a sandbox");
    assert!(!ping.busy);
    assert!(!ping.exe_path.is_empty());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ping_reports_busy_while_an_activity_runs() {
    let backend = TestBackend::start().await;
    let guard = backend.state.activity.guard();
    assert!(backend.get::<Ping>("/ping").await.busy);
    drop(guard);
    assert!(!backend.get::<Ping>("/ping").await.busy);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_starts_empty_and_shows_a_reported_source() {
    let backend = TestBackend::start().await;
    let health: Health = backend.get("/health").await;
    assert_eq!(health.start_id, backend.state.start_id);
    assert!(health.config_issues.is_empty());
    // A module reports its sources while it starts (the sessions module reports the Claude
    // registry), so the list is not empty; a source that nobody reported is not in it.
    assert!(health.sources.iter().all(|source| source.name != "steam"));

    backend.state.health.set_source(
        "steam",
        rain_types::SourceState::Off,
        Some("not installed".into()),
    );
    let health: Health = backend.get("/health").await;
    assert!(health.sources.iter().any(|source| source.name == "steam"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_answer_has_the_standard_headers() {
    let backend = TestBackend::start().await;
    let raw = backend
        .client
        .request(hyper::Method::GET, "/api/v1/ping", None, &[])
        .await
        .unwrap();
    assert_eq!(raw.headers["cache-control"], "no-store");
    assert_eq!(
        raw.headers["content-type"],
        "application/json; charset=utf-8"
    );

    // An error answer has them too.
    let raw = backend
        .client
        .request(hyper::Method::GET, "/api/v1/nope", None, &[])
        .await
        .unwrap();
    assert_eq!(raw.headers["cache-control"], "no-store");
    assert_eq!(
        raw.headers["content-type"],
        "application/json; charset=utf-8"
    );
    backend.stop().await;
}

// ---- the error body of each failure path ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_route_is_not_found() {
    let backend = TestBackend::start().await;
    assert_error(
        backend.send("GET", "/nope", None).await,
        StatusCode::NOT_FOUND,
        "notFound",
        "noRoute",
    );
    // Outside the prefix too.
    let raw = backend
        .client
        .request(hyper::Method::GET, "/ping", None, &[])
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&raw.body).unwrap();
    assert_error(
        (raw.status, body),
        StatusCode::NOT_FOUND,
        "notFound",
        "noRoute",
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_method_is_405_with_the_error_body() {
    let backend = TestBackend::start().await;
    assert_error(
        backend.send("POST", "/ping", Some(json!({}))).await,
        StatusCode::METHOD_NOT_ALLOWED,
        "failed",
        "methodNotAllowed",
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_and_options_are_not_supported() {
    let backend = TestBackend::start().await;
    for method in ["HEAD", "OPTIONS"] {
        assert_eq!(
            backend.send(method, "/ping", None).await.0,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
    }
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_that_is_not_json_is_invalid() {
    let backend = backend_with_failing_routes().await;
    let answer = backend
        .send_bytes("POST", "/test/body", Some("{ not json".into()))
        .await;
    assert_error(answer, StatusCode::BAD_REQUEST, "invalid", "badBody");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_with_an_unknown_field_or_a_missing_field_is_invalid() {
    let backend = backend_with_failing_routes().await;
    let unknown = json!({ "reset_after_seconds": 30, "surprise": true });
    assert_error(
        backend.send("POST", "/test/body", Some(unknown)).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badBody",
    );
    assert_error(
        backend.send("POST", "/test/body", Some(json!({}))).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badBody",
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_value_outside_its_allowed_set_is_invalid() {
    // The real shutdown route: `reason` is an enum, and "explode" is not one of its values.
    let backend = TestBackend::start().await;
    let (status, body) = backend
        .send(
            "POST",
            "/internal/shutdown",
            Some(json!({ "reason": "explode" })),
        )
        .await;
    assert_error(
        (status, body.clone()),
        StatusCode::BAD_REQUEST,
        "invalid",
        "badBody",
    );
    let message = body["message"].as_str().unwrap();
    assert!(
        message.starts_with("The request was not valid: "),
        "{message}"
    );
    backend.get::<Ping>("/ping").await; // and the backend did not stop
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_without_the_json_content_type_is_invalid() {
    let backend = TestBackend::start().await;
    // No body means that the client sets no content type.
    let answer = backend.send("POST", "/internal/shutdown", None).await;
    assert_error(answer, StatusCode::BAD_REQUEST, "invalid", "badContentType");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_query_is_invalid() {
    let backend = backend_with_failing_routes().await;
    assert_error(
        backend.send("GET", "/test/sleep?ms=soon", None).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badQuery",
    );
    assert_error(
        backend.send("GET", "/test/sleep", None).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badQuery",
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_path_value_is_invalid() {
    let backend = backend_with_failing_routes().await;
    assert_eq!(
        backend.send("GET", "/test/item/42", None).await,
        (StatusCode::OK, json!(42))
    );
    assert_error(
        backend.send("GET", "/test/item/forty-two", None).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badPath",
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handler_error_keeps_its_status_source_and_reason() {
    let backend = backend_with_failing_routes().await;
    let (status, body) = backend.send("GET", "/test/fails", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body,
        json!({ "code": "conflict", "message": "A job is running", "source": "jobs", "reason": "busy" })
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_a_handler_is_a_500_and_the_connection_survives() {
    let backend = backend_with_failing_routes().await;
    assert_error(
        backend.send("GET", "/test/panic", None).await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "failed",
        "panic",
    );
    // The same pooled connection (and the backend) still work.
    backend.get::<Ping>("/ping").await;
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_over_the_size_limit_is_invalid_and_the_backend_goes_on() {
    // A small limit keeps the test fast and its outcome certain: the whole request fits into the
    // pipe, so the client has finished writing when the server answers. (With a body of several
    // MiB the server may answer and close before the client has finished; the client then sees a
    // broken connection and not the answer.)
    let backend = TestBackend::builder()
        .extra_routes(failing_routes())
        .limits(|limits| limits.body_limit = 1000)
        .start()
        .await;
    let too_big = json!({ "reset_after_seconds": 30, "padding": "x".repeat(5000) });
    let answer = backend.send("POST", "/test/body", Some(too_big)).await;
    assert_error(answer, StatusCode::BAD_REQUEST, "invalid", "bodyTooLarge");
    backend.get::<Ping>("/ping").await;
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_over_the_time_limit_is_unavailable() {
    let backend = TestBackend::builder()
        .extra_routes(failing_routes())
        .limits(|limits| limits.request_timeout = Duration::from_millis(200))
        .start()
        .await;
    let started = Instant::now();
    let answer = backend.send("GET", "/test/sleep?ms=5000", None).await;
    assert_error(
        answer,
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "timeout",
    );
    assert!(started.elapsed() < Duration::from_secs(3));

    // A request that is quick enough is not touched by the limit.
    let (status, _) = backend.send("GET", "/test/sleep?ms=10", None).await;
    assert_eq!(status, StatusCode::OK);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_url_that_is_too_long_is_invalid() {
    let backend = TestBackend::start().await;
    let long_path = format!("/ping?{}", "a".repeat(2100));
    assert_error(
        backend.send("GET", &long_path, None).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "urlTooLong",
    );
    backend.stop().await;
}

// ---- limits of the connection itself (foundation appendix C: "a test for each") ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_does_not_finish_its_request_head_is_disconnected() {
    let backend = TestBackend::builder()
        .limits(|limits| limits.header_read_timeout = Duration::from_millis(300))
        .start()
        .await;
    let mut pipe = ClientOptions::new()
        .open(path_of(&backend.pipe_name))
        .unwrap();
    // A request head that never ends: no blank line.
    pipe.write_all(b"GET /api/v1/ping HTTP/1.1\r\nHost: rain\r\n")
        .await
        .unwrap();

    let started = Instant::now();
    let mut buffer = [0u8; 256];
    let read = tokio::time::timeout(PATIENCE, pipe.read(&mut buffer))
        .await
        .expect("the server closed the connection in time");
    let ended = matches!(read, Ok(0) | Err(_));
    assert!(ended, "no answer bytes were expected, got {read:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
    backend.stop().await;
}

/// One complete request on a raw connection; returns the start of the answer.
async fn raw_ping(pipe: &mut tokio::net::windows::named_pipe::NamedPipeClient) -> String {
    let request = b"GET /api/v1/ping HTTP/1.1\r\nHost: rain\r\n\r\n";
    // A server that closed the connection makes the write fail; that also counts as "no answer".
    if pipe.write_all(request).await.is_err() {
        return String::new();
    }
    let mut buffer = [0u8; 1024];
    let read = tokio::time::timeout(PATIENCE, pipe.read(&mut buffer))
        .await
        .expect("an answer arrived in time")
        .unwrap_or(0);
    String::from_utf8_lossy(&buffer[..read]).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_over_the_cap_is_closed_at_once() {
    let backend = TestBackend::builder()
        .limits(|limits| limits.max_connections = 2)
        .start()
        .await;
    let open = || {
        ClientOptions::new()
            .open(path_of(&backend.pipe_name))
            .unwrap()
    };

    let (mut first, mut second) = (open(), open());
    assert!(raw_ping(&mut first).await.starts_with("HTTP/1.1 200"));
    assert!(raw_ping(&mut second).await.starts_with("HTTP/1.1 200"));

    // Two connections are open and idle. The third one gets no answer.
    let mut third = open();
    assert_eq!(
        raw_ping(&mut third).await,
        "",
        "the third connection was closed"
    );

    // When one connection leaves, there is room again.
    drop(first);
    let mut fourth = loop {
        let mut candidate = open();
        if raw_ping(&mut candidate).await.starts_with("HTTP/1.1 200") {
            break candidate;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(raw_ping(&mut fourth).await.starts_with("HTTP/1.1 200"));
    drop((second, third, fourth));
    // The server notices a closed connection a moment later. Until it has, the cap of two still
    // applies and would refuse the stop request of the harness.
    let deadline = Instant::now() + PATIENCE;
    while backend
        .client
        .request(hyper::Method::GET, "/api/v1/ping", None, &[])
        .await
        .is_err()
    {
        assert!(
            Instant::now() < deadline,
            "the connections were not released"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    backend.stop().await;
}
