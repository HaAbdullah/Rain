//! Work that must not run on an async worker. Foundation section 11.6.
//!
//! Reading a file, hashing, anything that may take more than a few milliseconds goes through
//! [`run`]. The two async workers then stay free to answer `/ping` and keep every stream going.

use tracing::error;

use super::error::{ApiError, ApiResult};

/// Runs `work` on a blocking thread and waits for it. A panic inside `work` becomes a `failed`
/// error and is logged; it does not end the backend.
pub async fn run<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(work).await.map_err(|e| {
        error!(error = %e, "a blocking task ended abnormally");
        ApiError::failed("Rain had an internal error").with_reason("taskFailed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_result_of_the_work_comes_back() {
        assert_eq!(run(|| 2 + 2).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn a_panic_in_the_work_is_a_failed_error() {
        let error = run(|| -> u32 { panic!("deliberate: a test of the blocking helper") })
            .await
            .unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("taskFailed"));
    }
}
