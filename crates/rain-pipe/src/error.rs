//! How a failed call becomes an `ApiError` (foundation section 4.5, client side).

use std::error::Error;
use std::io;

use hyper::StatusCode;
use rain_types::{ApiError, ErrorCode};

// Windows error codes that tell what is wrong with the pipe. Spelled out here so that this crate
// needs no Windows bindings.
pub(crate) const ERROR_FILE_NOT_FOUND: i32 = 2;
pub(crate) const ERROR_ACCESS_DENIED: i32 = 5;
pub(crate) const ERROR_PIPE_BUSY: i32 = 231;

/// An error that Rain's client code creates itself (not one the backend sent).
pub(crate) fn client_error(code: ErrorCode, reason: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        code,
        message: message.into(),
        source: Some("backend".into()),
        reason: Some(reason.into()),
    }
}

/// A non-2xx answer becomes an `ApiError`.
///
/// Normal case: the body already is the backend's error body, so it is used as it is.
/// Otherwise (a proxy page, an empty body) the code comes from the status.
pub fn error_from_response(status: StatusCode, body: &[u8]) -> ApiError {
    if let Ok(error) = serde_json::from_slice::<ApiError>(body) {
        return error;
    }
    let code = match status {
        StatusCode::NOT_FOUND => ErrorCode::NotFound,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => {
            ErrorCode::Unavailable
        }
        _ => ErrorCode::Failed,
    };
    client_error(
        code,
        "badResponse",
        format!("The Rain backend gave an unexpected answer ({status})"),
    )
}

/// The call could not be made at all (pipe missing, access denied, connection broken).
/// Always `unavailable`, with the reason of foundation section 6.6.
pub fn transport_error(error: &(dyn Error + 'static)) -> ApiError {
    match os_error_code(error) {
        Some(ERROR_FILE_NOT_FOUND) => client_error(
            ErrorCode::Unavailable,
            "backendDown",
            "Rain backend is not running",
        ),
        Some(ERROR_ACCESS_DENIED) => client_error(
            ErrorCode::Unavailable,
            "pipeDenied",
            "Windows refused access to the Rain backend pipe. Another program or user may own its name.",
        ),
        _ => client_error(
            ErrorCode::Unavailable,
            "transport",
            "Lost contact with the Rain backend",
        ),
    }
}

/// A hyper error wraps the `io::Error` that carries the Windows error code, a few levels down.
/// Walks the chain of causes and returns the first operating system error code.
fn os_error_code(error: &(dyn Error + 'static)) -> Option<i32> {
    let mut current = Some(error);
    while let Some(err) = current {
        if let Some(code) = err
            .downcast_ref::<io::Error>()
            .and_then(io::Error::raw_os_error)
        {
            return Some(code);
        }
        current = err.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_error(code: i32) -> io::Error {
        io::Error::from_raw_os_error(code)
    }

    #[test]
    fn a_body_that_is_an_error_body_is_used_as_it_is() {
        let body = br#"{"code":"conflict","message":"Busy","reason":"busy"}"#;
        let error = error_from_response(StatusCode::CONFLICT, body);
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.reason.as_deref(), Some("busy"));
    }

    #[test]
    fn other_bodies_take_the_code_from_the_status() {
        let cases = [
            (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            (StatusCode::BAD_GATEWAY, ErrorCode::Unavailable),
            (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable),
            (StatusCode::GATEWAY_TIMEOUT, ErrorCode::Unavailable),
            (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Failed),
            (StatusCode::BAD_REQUEST, ErrorCode::Failed),
        ];
        for (status, code) in cases {
            let error = error_from_response(status, b"<html>oops</html>");
            assert_eq!(error.code, code, "status {status}");
            assert_eq!(error.reason.as_deref(), Some("badResponse"));
        }
    }

    #[test]
    fn a_missing_pipe_means_the_backend_is_not_running() {
        let error = transport_error(&os_error(ERROR_FILE_NOT_FOUND));
        assert_eq!(error.code, ErrorCode::Unavailable);
        assert_eq!(error.reason.as_deref(), Some("backendDown"));
        assert_eq!(error.message, "Rain backend is not running");
    }

    #[test]
    fn access_denied_means_someone_else_owns_the_pipe() {
        let error = transport_error(&os_error(ERROR_ACCESS_DENIED));
        assert_eq!(error.reason.as_deref(), Some("pipeDenied"));
    }

    #[test]
    fn any_other_failure_is_a_transport_error() {
        let error = transport_error(&os_error(ERROR_PIPE_BUSY));
        assert_eq!(error.reason.as_deref(), Some("transport"));
    }

    #[test]
    fn the_os_code_is_found_inside_a_wrapped_error() {
        #[derive(Debug)]
        struct Wrapper(io::Error);
        impl std::fmt::Display for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "connect failed")
            }
        }
        impl Error for Wrapper {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }
        let error = transport_error(&Wrapper(os_error(ERROR_FILE_NOT_FOUND)));
        assert_eq!(error.reason.as_deref(), Some("backendDown"));
    }
}
