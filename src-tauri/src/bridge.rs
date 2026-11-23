//! The one door between the page and the backend: the Tauri command `api_request`
//! (foundation section 6.1).
//!
//! The page sends a method, a path and a JSON body. The shell checks them, sends the call down
//! the pipe, and gives back the JSON of the answer, or an error object in the shape of the
//! backend's own errors. The path `/local/backend` is not sent anywhere: the shell answers it
//! itself, because only the shell knows the state of the backend process.
//!
//! `handle` is the whole logic and uses no Tauri type, so tests run it against a real backend.

use std::time::Duration;

use hyper::Method;
use hyper::body::Bytes;
use rain_pipe::PipeClient;
use rain_types::limits::{API_PREFIX, BRIDGE_TIMEOUT, BRIDGE_TIMEOUT_MAX, RESPONSE_MAX, URL_MAX};
use rain_types::{ApiError, ErrorCode};
use serde_json::Value;

use crate::backend::{Backend, backend_down, shell_error, timeout_error};

/// The methods the page may use.
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Paths under this prefix belong to the shell; the backend has no route that starts with it.
const LOCAL_PREFIX: &str = "/local";
/// The one shell-local path that exists.
const LOCAL_BACKEND_PATH: &str = "/local/backend";
/// The live stream. The relay is its only reader: a call that waited for the end of it would
/// collect an endless body in memory.
const STREAM_PATH: &str = "/stream";
/// Routes for Rain's own programs. The page must never reach them (foundation section 13.1).
const INTERNAL_PREFIX: &str = "/internal";

/// What a call is for, once its method and path have been checked.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// `GET /local/backend`: the shell answers.
    LocalBackendState,
    /// Everything else goes to the backend.
    Backend,
}

fn bad_request(message: &str) -> ApiError {
    ApiError {
        code: ErrorCode::Invalid,
        message: message.to_owned(),
        source: Some("shell".to_owned()),
        reason: Some("badRequest".to_owned()),
    }
}

/// The path without its query string.
fn path_only(path: &str) -> &str {
    path.split_once('?').map_or(path, |(before, _)| before)
}

/// `/internal` and everything below it, but not a sibling such as `/internals`.
fn is_below(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Checks a call from the page before anything is sent (foundation section 6.1).
///
/// The page is not trusted: it is a web page that may show anything. So the path may not hold
/// anything that could change where the request goes, and it may not name an internal route.
fn route(method: &str, path: &str) -> Result<Target, ApiError> {
    if !METHODS.contains(&method) {
        return Err(bad_request("The method is not allowed"));
    }
    let tricks = ["\\", "//", "://", "#"];
    // `..` is a trick only as a whole path segment. In the query string it is plain user text
    // (a search for "a..b", a folder named "v1..2"), so only the part before `?` is checked.
    let dot_segment = path_only(path).split('/').any(|segment| segment == "..");
    if !path.starts_with('/')
        || path.len() > URL_MAX
        || path.chars().any(char::is_control)
        || dot_segment
        || tricks.iter().any(|trick| path.contains(trick))
    {
        return Err(bad_request("The path is not allowed"));
    }
    let only = path_only(path);
    if is_below(only, INTERNAL_PREFIX) || is_below(only, STREAM_PATH) {
        return Err(bad_request("The path is not allowed"));
    }
    if is_below(only, LOCAL_PREFIX) {
        return if method == "GET" && only == LOCAL_BACKEND_PATH {
            Ok(Target::LocalBackendState)
        } else {
            Err(ApiError {
                code: ErrorCode::NotFound,
                message: "Unknown shell path".to_owned(),
                source: Some("shell".to_owned()),
                reason: Some("noRoute".to_owned()),
            })
        };
    }
    Ok(Target::Backend)
}

/// The timeout the page asked for, kept between 1 ms and 60 s. The default is 20 s.
fn clamp_timeout(requested_ms: Option<u64>) -> Duration {
    requested_ms.map_or(BRIDGE_TIMEOUT, |ms| {
        Duration::from_millis(ms).clamp(Duration::from_millis(1), BRIDGE_TIMEOUT_MAX)
    })
}

/// Turns the body of an answer into JSON: an empty body (204, 304) is `null`.
fn json_of(body: &[u8]) -> Result<Value, ApiError> {
    if body.len() > RESPONSE_MAX {
        return Err(shell_error(
            ErrorCode::Failed,
            "responseTooLarge",
            "The Rain backend answer is too large to show",
        ));
    }
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(body).map_err(|_| {
        shell_error(
            ErrorCode::Failed,
            "badResponse",
            "The Rain backend gave an answer that is not valid JSON",
        )
    })
}

/// One attempt: send the request and read the answer. A non-2xx answer with a proper error body
/// is returned as that error, unchanged.
async fn send(
    client: &PipeClient,
    method: &Method,
    path: &str,
    body: Option<Bytes>,
) -> Result<Value, ApiError> {
    let raw = client
        .request(
            method.clone(),
            &format!("{API_PREFIX}{path}"),
            body,
            &[("accept", "application/json")],
        )
        .await?
        .into_success()?;
    json_of(&raw.body)
}

/// Sends the call. When the pipe does not exist at all the backend is not running, so it is
/// started and the call is tried once more (foundation section 6.7). A call that was written to
/// the pipe is never repeated: it may have had an effect.
async fn call_backend(
    backend: &Backend,
    method: &Method,
    path: &str,
    body: Option<Bytes>,
) -> Result<Value, ApiError> {
    let client = backend.client();
    let first = send(&client, method, path, body.clone()).await;
    match first {
        Err(error) if error.reason.as_deref() == Some("backendDown") => {
            start_backend(backend).await?;
            send(&client, method, path, body).await
        }
        answered => {
            if answered.is_ok() {
                backend.note_answered();
            }
            answered
        }
    }
}

/// Starts the backend in a task of its own. The page's timeout may drop the call that asked for
/// the start; a dropped start would leave the state at `starting` for good.
async fn start_backend(backend: &Backend) -> Result<(), ApiError> {
    let backend = backend.clone();
    tokio::spawn(async move { backend.ensure().await })
        .await
        .map_err(|_| backend_down())?
}

/// The whole of `api_request`, free of Tauri types.
pub async fn handle(
    backend: &Backend,
    method: &str,
    path: &str,
    body: Option<Value>,
    timeout_ms: Option<u64>,
) -> Result<Value, ApiError> {
    match route(method, path)? {
        Target::LocalBackendState => {
            Ok(serde_json::to_value(backend.status()).expect("a status is always valid JSON"))
        }
        Target::Backend => {
            let method = Method::from_bytes(method.as_bytes()).expect("route accepted the method");
            let body = body.map(|value| Bytes::from(value.to_string()));
            let call = async {
                let _place = backend.acquire_inflight().await;
                call_backend(backend, &method, path, body).await
            };
            tokio::time::timeout(clamp_timeout(timeout_ms), call)
                .await
                .unwrap_or_else(|_| Err(timeout_error()))
        }
    }
}

/// The only command of the shell. The argument names are camelCase on the JavaScript side
/// (`timeoutMs`); Tauri converts them.
#[tauri::command]
pub async fn api_request(
    backend: tauri::State<'_, Backend>,
    method: String,
    path: String,
    body: Option<Value>,
    timeout_ms: Option<u64>,
) -> Result<Value, ApiError> {
    handle(&backend, &method, &path, body, timeout_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(method: &str, path: &str) -> ApiError {
        route(method, path).expect_err(&format!("{method} {path} should be refused"))
    }

    #[test]
    fn an_ordinary_call_goes_to_the_backend() {
        for (method, path) in [
            ("GET", "/settings"),
            ("PATCH", "/settings"),
            ("PUT", "/habits/gym/days/2026-11-22"),
            ("DELETE", "/jobs/abc"),
            ("POST", "/launch"),
            ("GET", "/search?q=code&limit=5"),
        ] {
            assert_eq!(route(method, path), Ok(Target::Backend), "{method} {path}");
        }
    }

    #[test]
    fn methods_outside_the_list_are_refused() {
        for method in ["", "get", "HEAD", "OPTIONS", "TRACE", "CONNECT", "GET "] {
            assert_eq!(
                refused(method, "/settings").reason.as_deref(),
                Some("badRequest")
            );
        }
    }

    #[test]
    fn a_refused_call_is_invalid_with_the_reason_bad_request() {
        let error = refused("GET", "settings");
        assert_eq!(error.code, ErrorCode::Invalid);
        assert_eq!(error.reason.as_deref(), Some("badRequest"));
    }

    #[test]
    fn a_path_must_start_with_a_slash() {
        for path in ["", "settings", "?x=1", " /settings"] {
            refused("GET", path);
        }
    }

    #[test]
    fn paths_that_could_change_the_destination_are_refused() {
        for path in [
            "/../settings",
            "/a/..",
            "/a\\b",
            "//host/share",
            "/a//b",
            "/http://evil.example",
            "/x?next=https://evil.example",
            "/settings#fragment",
            "/set\ttings",
            "/set\nttings",
            "/set\0tings",
            "/set\u{7f}tings",
        ] {
            refused("GET", path);
        }
    }

    #[test]
    fn two_dots_in_the_query_text_are_plain_text() {
        for path in [
            "/search?q=..",
            "/search?q=a..b",
            "/disk/scan?path=D%3A%2Fv1..2",
            "/a/b..c",
        ] {
            assert!(route("GET", path).is_ok(), "{path}");
        }
    }

    #[test]
    fn a_path_is_at_most_2048_bytes() {
        let at_limit = format!("/{}", "a".repeat(URL_MAX - 1));
        assert!(route("GET", &at_limit).is_ok());
        refused("GET", &format!("{at_limit}a"));
    }

    #[test]
    fn internal_routes_are_refused_in_every_spelling_that_reaches_them() {
        for path in [
            "/internal",
            "/internal/",
            "/internal/shutdown",
            "/internal/shell",
            "/internal/shutdown?reason=stop",
            "/internal?x=1",
        ] {
            refused("POST", path);
        }
    }

    #[test]
    fn the_stream_is_read_by_the_relay_only() {
        for path in ["/stream", "/stream/", "/stream?x=1"] {
            refused("GET", path);
        }
        assert_eq!(route("GET", "/streams"), Ok(Target::Backend));
    }

    #[test]
    fn a_route_that_only_starts_like_internal_is_not_internal() {
        assert_eq!(route("GET", "/internals"), Ok(Target::Backend));
        assert_eq!(route("GET", "/internal-notes"), Ok(Target::Backend));
    }

    #[test]
    fn the_local_backend_path_is_answered_by_the_shell() {
        assert_eq!(
            route("GET", "/local/backend"),
            Ok(Target::LocalBackendState)
        );
        assert_eq!(
            route("GET", "/local/backend?x=1"),
            Ok(Target::LocalBackendState)
        );
    }

    #[test]
    fn every_other_local_path_is_not_found() {
        for (method, path) in [
            ("POST", "/local/backend"),
            ("DELETE", "/local/backend"),
            ("GET", "/local/other"),
            ("GET", "/local"),
            ("GET", "/local/"),
            ("GET", "/local/backend/x"),
        ] {
            let error = refused(method, path);
            assert_eq!(error.code, ErrorCode::NotFound, "{method} {path}");
        }
    }

    #[test]
    fn a_route_that_only_starts_like_local_goes_to_the_backend() {
        assert_eq!(route("GET", "/locality"), Ok(Target::Backend));
    }

    #[test]
    fn the_timeout_defaults_to_20_seconds_and_stays_within_1_ms_and_60_s() {
        assert_eq!(clamp_timeout(None), Duration::from_secs(20));
        assert_eq!(clamp_timeout(Some(5_000)), Duration::from_secs(5));
        assert_eq!(clamp_timeout(Some(0)), Duration::from_millis(1));
        assert_eq!(clamp_timeout(Some(60_000)), Duration::from_secs(60));
        assert_eq!(clamp_timeout(Some(u64::MAX)), Duration::from_secs(60));
    }

    #[test]
    fn an_empty_body_is_null() {
        assert_eq!(json_of(b"").unwrap(), Value::Null);
    }

    #[test]
    fn a_json_body_is_parsed() {
        assert_eq!(
            json_of(br#"{"a":[1,2]}"#).unwrap(),
            serde_json::json!({ "a": [1, 2] })
        );
    }

    #[test]
    fn a_body_that_is_not_json_is_a_bad_response() {
        let error = json_of(b"<html>").unwrap_err();
        assert_eq!(error.code, ErrorCode::Failed);
        assert_eq!(error.reason.as_deref(), Some("badResponse"));
    }

    #[test]
    fn a_body_over_16_mib_is_too_large() {
        let error = json_of(&vec![b' '; RESPONSE_MAX + 1]).unwrap_err();
        assert_eq!(error.code, ErrorCode::Failed);
        assert_eq!(error.reason.as_deref(), Some("responseTooLarge"));
    }
}
