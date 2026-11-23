//! The times of the jobs and idle jobs modules, as one value that a test can shorten.
//!
//! The foundation fixes these numbers (60 s catch-up delay, 30 s idle sampling ...). A test that
//! waited for them would take minutes, so the module asks `state.seam::<Tuning>()` and a test
//! puts a `Tuning` with short times there. The product never does: it uses `Tuning::default()`.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

#[derive(Debug, Clone)]
pub struct Tuning {
    /// The wait after a start or a resume from sleep before missed runs are made up (so that the
    /// network is up). Section 03 6.11.5.
    pub catch_up_delay: Duration,
    /// What "a minute" is for the time limit of a run: `timeout_minutes` times this.
    pub minute: Duration,
    /// How long a burst of log lines waits to become one database write (timer T5).
    pub log_flush: Duration,
    /// The idle sampler's period while no idle run is going (timer T3).
    pub idle_slow: Duration,
    /// The idle sampler's period while an idle run is going or frozen.
    pub idle_fast: Duration,
    /// The longest wait of the scheduler when the resume-from-sleep notice is not available.
    pub degraded_cap: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            catch_up_delay: Duration::from_secs(60),
            minute: Duration::from_secs(60),
            log_flush: Duration::from_millis(200),
            idle_slow: Duration::from_secs(30),
            idle_fast: Duration::from_secs(5),
            degraded_cap: Duration::from_secs(5 * 60),
        }
    }
}

/// A test's stand-in for the notice that the PC woke from sleep. When it is in place the
/// scheduler listens to it and registers nothing with Windows.
#[derive(Debug, Default)]
pub struct ResumeSignal(pub Arc<Notify>);
