//! The one COM thread. Foundation section 11.6.
//!
//! The Windows shell calls (icons, shortcuts, the app list, `ShellExecuteExW`, "show in
//! Explorer", known folders) need COM on a single-threaded apartment. There is exactly one such
//! thread in the process. It starts on first use, initialises COM, and then waits on a channel
//! (zero CPU while idle). Code that needs COM sends a closure to it with [`run`]:
//!
//! ```ignore
//! let png = com::run(move || icons::render(&source, size)).await?;
//! ```
//!
//! Rules for the closure: it returns plain data (`String`, `Vec<u8>`, numbers). No COM
//! interface pointer, string buffer or handle leaves the closure, and none is kept between
//! calls. No other thread may call COM.
//!
//! A call that takes longer than `COM_CALL_TIMEOUT` is abandoned (a thread cannot be killed) and
//! logged. While an abandoned call has not returned, every new call fails at once with
//! `ComError::Busy` instead of queuing behind it forever. The usual cause is a shortcut on a
//! disconnected network drive, so the modules avoid resolving those eagerly.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use rain_types::limits::COM_CALL_TIMEOUT;
use tokio::sync::oneshot;
use tracing::{error, warn};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};

use crate::core::error::ApiError;

/// A job for the COM thread.
type Job = Box<dyn FnOnce() + Send>;

/// What can go wrong with a call on the COM thread.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComError {
    /// An earlier call has not returned (see the module text). Later calls fail until the
    /// process restarts, or until that call finally returns.
    #[error("the Windows shell thread is busy with a call that did not return")]
    Busy,
    /// This call did not return within `COM_CALL_TIMEOUT`.
    #[error("the Windows shell did not answer in time")]
    TimedOut,
    /// The thread could not start or has ended.
    #[error("the Windows shell thread is not running")]
    Gone,
    /// A Windows call failed. `code` is the HRESULT.
    #[error("a Windows call failed (code {code:#010x})")]
    Os { code: i32 },
}

impl From<windows::core::Error> for ComError {
    fn from(error: windows::core::Error) -> Self {
        Self::Os {
            code: error.code().0,
        }
    }
}

impl ComError {
    /// The error for the user. The caller says what it was doing (foundation 4.5).
    pub fn into_api(self, doing: &str) -> ApiError {
        match self {
            Self::Busy | Self::TimedOut => {
                ApiError::unavailable("windows", format!("{doing}: Windows did not answer"))
                    .with_reason("comBusy")
            }
            Self::Gone | Self::Os { .. } => {
                error!(error = %self, "a Windows shell call failed");
                ApiError::failed(doing.to_owned()).with_source("windows")
            }
        }
    }
}

struct ComThread {
    sender: Mutex<Sender<Job>>,
    /// Calls that timed out and have not returned yet.
    abandoned: AtomicUsize,
}

static THREAD: OnceLock<Result<ComThread, ()>> = OnceLock::new();

fn thread() -> Result<&'static ComThread, ComError> {
    THREAD
        .get_or_init(start_thread)
        .as_ref()
        .map_err(|()| ComError::Gone)
}

fn start_thread() -> Result<ComThread, ()> {
    let (sender, jobs) = mpsc::channel::<Job>();
    std::thread::Builder::new()
        .name("rain-com".to_owned())
        .spawn(move || serve(jobs))
        .map_err(|e| error!(error = %e, "could not start the COM thread"))?;
    Ok(ComThread {
        sender: Mutex::new(sender),
        abandoned: AtomicUsize::new(0),
    })
}

fn serve(jobs: mpsc::Receiver<Job>) {
    // SAFETY: called once on this thread, before any other COM call, and matched by
    // `CoUninitialize` when the thread ends. The apartment is single-threaded because the shell
    // objects that this thread uses require it.
    let initialised =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if initialised.is_err() {
        error!("COM could not be initialised; shell calls will fail");
    }
    while let Ok(job) = jobs.recv() {
        job();
    }
    if initialised.is_ok() {
        // SAFETY: balances the successful `CoInitializeEx` above on the same thread.
        unsafe { CoUninitialize() };
    }
}

/// Runs `work` on the COM thread and waits for the answer, at most `COM_CALL_TIMEOUT`.
pub async fn run<T, F>(work: F) -> Result<T, ComError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ComError> + Send + 'static,
{
    run_with_timeout(work, COM_CALL_TIMEOUT).await
}

/// `run` with its own time limit. The tests use a short one.
pub async fn run_with_timeout<T, F>(work: F, limit: Duration) -> Result<T, ComError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ComError> + Send + 'static,
{
    let com = thread()?;
    if com.abandoned.load(Ordering::SeqCst) > 0 {
        return Err(ComError::Busy);
    }
    let (reply, answer) = oneshot::channel();
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job: Job = {
        let timed_out = std::sync::Arc::clone(&timed_out);
        Box::new(move || {
            let result = work();
            // The caller may have given up. Then the call is no longer abandoned.
            if timed_out.load(Ordering::SeqCst) {
                com.abandoned.fetch_sub(1, Ordering::SeqCst);
            }
            let _ = reply.send(result);
        })
    };
    com.sender
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send(job)
        .map_err(|_| ComError::Gone)?;

    match tokio::time::timeout(limit, answer).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(ComError::Gone),
        Err(_) => {
            warn!("a Windows shell call did not return in time and was abandoned");
            com.abandoned.fetch_add(1, Ordering::SeqCst);
            timed_out.store(true, Ordering::SeqCst);
            Err(ComError::TimedOut)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::ThreadId;

    fn current_thread_id() -> ThreadId {
        std::thread::current().id()
    }

    #[tokio::test]
    async fn work_runs_on_one_thread_that_is_not_the_caller() {
        let first = run(|| Ok(current_thread_id())).await.unwrap();
        let second = run(|| Ok(current_thread_id())).await.unwrap();
        assert_eq!(first, second, "there is exactly one COM thread");
        assert_ne!(first, current_thread_id());
    }

    #[tokio::test]
    async fn the_answer_of_the_closure_comes_back_with_its_error() {
        assert_eq!(run(|| Ok(41 + 1)).await, Ok(42));
        let failed: Result<u32, ComError> = run(|| Err(ComError::Os { code: -5 })).await;
        assert_eq!(failed, Err(ComError::Os { code: -5 }));
    }

    #[tokio::test]
    async fn a_windows_error_keeps_its_code() {
        let error = windows::core::Error::from(windows::core::HRESULT(0x8000_4005_u32 as i32));
        assert_eq!(
            ComError::from(error),
            ComError::Os {
                code: 0x8000_4005_u32 as i32
            }
        );
    }

    #[test]
    fn a_failed_call_is_reported_with_what_rain_was_doing() {
        let busy = ComError::Busy.into_api("Could not show the file");
        assert_eq!(busy.body.reason.as_deref(), Some("comBusy"));
        let failed = ComError::Os { code: 1 }.into_api("Could not show the file");
        assert_eq!(failed.body.message, "Could not show the file");
    }
}
