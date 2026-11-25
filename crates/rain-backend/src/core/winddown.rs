//! The port through which game mode ends the work that Rain itself started.
//! Feature section 06 6.7.1; foundation 11.1.
//!
//! It lives in `core` so that `jobs` and `idle` (which implement it) do not import `gamemode`
//! (which uses it). Game mode never ends anything else: not a user's Claude session, not a
//! program the user launched.

use std::time::Duration;

use futures_util::future::BoxFuture;
use tokio::sync::watch;

/// How much work a module has. `running` is the number of runs in progress that a stop would
/// interrupt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub enabled: u32,
    pub running: u32,
}

/// What happened to one run that was ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    Stopped,
    /// It did not end in time and was ended by force.
    Forced,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub name: String,
    pub outcome: StopOutcome,
}

pub trait WindDownPort: Send + Sync {
    fn counts(&self) -> Counts;

    /// The number of running runs. Changes wake the wait phase of game mode, so it needs no timer.
    fn running_watch(&self) -> watch::Receiver<u32>;

    /// `true`: start no new run. `false`: resume; a job that came due meanwhile runs once.
    fn pause(&self, paused: bool);

    /// Ends every run, with `budget` for each. The module records the outcome `stopped` with the
    /// reason `gameMode` in its own history.
    fn stop_all(&self, budget: Duration) -> BoxFuture<'_, Vec<Stopped>>;
}
