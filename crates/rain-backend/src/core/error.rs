//! The one error every route returns, and the extractors that make a malformed request produce it.
//! Foundation section 4.5. Copied in shape from the spike `types-stream` (`api_error.rs`).
//!
//! On the wire every non-2xx answer has the body `{ code, message, source?, reason? }`, also for
//! an unknown route, a wrong method and a panic in a handler.
//!
//! There is deliberately no `From<std::io::Error>`: a handler must say what failed ("Could not
//! read the Steam library"), because a bare operating system message means nothing to the user.

use std::any::Any;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rain_types::{ApiError as ErrorBody, ErrorCode};

/// The source written on errors that the backend itself raises (not a feature module).
const CORE_SOURCE: &str = "backend";

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: ErrorBody,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorBody {
                code,
                message: message.into(),
                source: None,
                reason: None,
            },
        }
    }

    /// Status 400: The request itself is wrong: bad JSON, an unknown field, a value that is not allowed.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::Invalid, message)
    }

    /// Status 404: An id or a route that does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, message)
    }

    /// Status 409: The request is fine, but the current state forbids it.
    pub fn conflict(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, message).with_reason(reason)
    }

    /// Status 503: A source or a part of the backend is off or not ready (Steam is not installed, the
    /// backend is stopping, a request took too long).
    pub fn unavailable(source: &'static str, message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            message,
        )
        .with_source(source)
    }

    /// Status 500: The operation was understood but did not work.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Failed,
            message,
        )
    }

    /// Names the module or data source: `games`, `steam`, `vault`, `backend`.
    pub fn with_source(mut self, source: &'static str) -> Self {
        self.body.source = Some(source.to_owned());
        self
    }

    /// A stable machine tag in camelCase, for tests and the few places where the UI branches.
    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.body.reason = Some(reason.to_owned());
        self
    }

    /// An error of the backend itself: source `backend` and a reason.
    fn core(self, reason: &'static str) -> Self {
        self.with_source(CORE_SOURCE).with_reason(reason)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, axum::Json(self.body)).into_response()
    }
}

/// The text of a rejection is serde's own wording ("invalid value: 45, expected one of: 30, 60,
/// 300"). It is shown to the user as it is, so it gets a lead sentence and never the body itself.
fn not_valid(rejection_text: &str) -> String {
    format!("The request was not valid: {rejection_text}")
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection.status() {
            StatusCode::PAYLOAD_TOO_LARGE => {
                Self::invalid("The request was too large").core("bodyTooLarge")
            }
            StatusCode::UNSUPPORTED_MEDIA_TYPE => {
                Self::invalid(not_valid(&rejection.body_text())).core("badContentType")
            }
            // 400 (bad syntax) and 422 (wrong shape or value) are one case for a client.
            _ => Self::invalid(not_valid(&rejection.body_text())).core("badBody"),
        }
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::invalid(not_valid(&rejection.body_text())).core("badQuery")
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::invalid(not_valid(&rejection.body_text())).core("badPath")
    }
}

/// Use `ApiJson<T>`, `ApiQuery<T>` and `ApiPath<T>` instead of `Json<T>`, `Query<T>` and `Path<T>`
/// as handler arguments, so that a malformed request is answered with the one error body.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

// `Query` reads only the URL, so its extractor derives `FromRequestParts`, not `FromRequest`.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct ApiQuery<T>(pub T);

// The same for the `{id}` parts of a route path.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct ApiPath<T>(pub T);

/// Router fallback: an unknown path.
pub async fn fallback_404() -> ApiError {
    ApiError::not_found("There is no such route").core("noRoute")
}

/// Router fallback: a known path with a method it does not have. This is a bug in the client.
pub async fn fallback_405() -> ApiError {
    method_not_allowed_error()
}

/// Status 405: The body is `failed`, not `invalid`: the request is fine, the client is wrong.
pub fn method_not_allowed_error() -> ApiError {
    let mut error =
        ApiError::failed("This method is not allowed on this route").core("methodNotAllowed");
    error.status = StatusCode::METHOD_NOT_ALLOWED;
    error
}

/// For `CatchPanicLayer::custom`: a panic in a handler becomes an ordinary 500, and the
/// connection stays usable. The panic itself is logged by the panic hook (`core::logging`).
pub fn panic_response(_panic: Box<dyn Any + Send + 'static>) -> Response {
    ApiError::failed("Rain had an internal error")
        .core("panic")
        .into_response()
}

/// A request took longer than `Limits::request_timeout`.
pub fn timeout_error() -> ApiError {
    ApiError::unavailable(CORE_SOURCE, "Rain took too long to answer").with_reason("timeout")
}

/// A URL longer than `URL_MAX`.
pub fn url_too_long_error() -> ApiError {
    ApiError::invalid("The address of the request is too long").core("urlTooLong")
}

/// New work is refused while the backend stops.
pub fn shutting_down_error() -> ApiError {
    ApiError::unavailable(CORE_SOURCE, "Rain is stopping").with_reason("shuttingDown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_constructor_has_its_status_and_code() {
        let cases = [
            (
                ApiError::invalid("x"),
                StatusCode::BAD_REQUEST,
                ErrorCode::Invalid,
            ),
            (
                ApiError::not_found("x"),
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
            ),
            (
                ApiError::conflict("busy", "x"),
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
            ),
            (
                ApiError::unavailable("steam", "x"),
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::Unavailable,
            ),
            (
                ApiError::failed("x"),
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Failed,
            ),
        ];
        for (error, status, code) in cases {
            assert_eq!(error.status, status);
            assert_eq!(error.body.code, code);
        }
    }

    #[test]
    fn source_and_reason_are_added_by_the_builders() {
        let error = ApiError::conflict("busy", "x").with_source("jobs");
        assert_eq!(error.body.source.as_deref(), Some("jobs"));
        assert_eq!(error.body.reason.as_deref(), Some("busy"));
    }

    #[test]
    fn the_timeout_error_is_unavailable_with_its_reason() {
        let error = timeout_error();
        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.body.reason.as_deref(), Some("timeout"));
    }
}
