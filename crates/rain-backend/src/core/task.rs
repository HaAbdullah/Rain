//! Background tasks that report their own panic. Foundation section 11.7.

use std::future::Future;
use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use tokio::task::JoinHandle;
use tracing::error;

/// Starts a background task. A panic inside it is caught and logged at `error` with the task's
/// name, so a bug in a watcher or a timer is not silent. The task is not restarted: its section
/// says whether it should be.
pub fn spawn_named<F>(name: &'static str, task: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        if AssertUnwindSafe(task).catch_unwind().await.is_err() {
            error!(task = name, "a background task panicked and ended");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_task_that_finishes_normally_ends_normally() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let handle = spawn_named("test", async move {
            let _ = sender.send(7);
        });
        handle.await.unwrap();
        assert_eq!(receiver.await.unwrap(), 7);
    }

    #[tokio::test]
    async fn a_panic_in_the_task_does_not_reach_the_handle() {
        let handle = spawn_named("test", async {
            panic!("deliberate: a test of spawn_named");
        });
        // `catch_unwind` turned the panic into a log line, so the join is not an error.
        assert!(handle.await.is_ok());
    }
}
