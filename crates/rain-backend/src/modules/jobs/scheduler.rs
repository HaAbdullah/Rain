//! The scheduler: when each job is due, and the one task that waits for it. Section 03 6.11.3
//! to 6.11.5.
//!
//! One task, one `sleep` at a time, no tick (foundation timer T2). The sleep is computed from the
//! wall clock and recomputed after every wake, so a long sleep cannot drift, and a clock that
//! was changed is healed by the next nudge (every `GET /jobs`, every stream connect).
//!
//! `handled_until` is the scheduler's memory per job: every occurrence up to that instant has
//! been dealt with (run, skipped because the job still ran, or deliberately not run). Missed
//! runs are found from it: an occurrence after `handled_until` and not after now.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use croner::Cron;
use tokio::time::Instant;
use tracing::info;

use super::schedule::next_after;
use super::service::{Inner, Trigger};

/// An occurrence this close ahead counts as due: the timer may fire a little early.
pub const FIRE_SLACK: Duration = Duration::from_millis(500);

/// The instant after which the scheduler looks for the next occurrence of a job. While the
/// start-up catch-up is pending, occurrences before `floor` (the start, or the last resume) are
/// not fired as scheduled runs: they belong to the catch-up, which runs them once.
pub fn origin(handled_until: SystemTime, floor: Option<SystemTime>) -> SystemTime {
    floor.map_or(handled_until, |floor| floor.max(handled_until))
}

/// The occurrence of a job that is due at `now` (or within `FIRE_SLACK` of it), if any.
pub fn due_now(
    cron: &Cron,
    handled_until: SystemTime,
    floor: Option<SystemTime>,
    now: SystemTime,
) -> Option<SystemTime> {
    next_after(cron, origin(handled_until, floor))
        .filter(|occurrence| *occurrence <= now + FIRE_SLACK)
}

/// The earliest of the next occurrences. `None` when there is nothing to wait for.
pub fn earliest(
    next_occurrences: impl IntoIterator<Item = Option<SystemTime>>,
) -> Option<SystemTime> {
    next_occurrences.into_iter().flatten().min()
}

/// What the catch-up does for one enabled job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchUp {
    /// Run once. The time is the first occurrence that was missed.
    Run(SystemTime),
    /// Run nothing; the job is only marked as handled up to now.
    Advance,
}

/// Section 03 6.11.5. A job with no memory yet (first seen enabled) has no past: nothing is made
/// up. Otherwise one run, however many occurrences were missed.
pub fn catch_up_decision(
    catch_up: bool,
    cron: &Cron,
    handled_until: Option<SystemTime>,
    now: SystemTime,
) -> CatchUp {
    let Some(handled_until) = handled_until else {
        return CatchUp::Advance;
    };
    match next_after(cron, handled_until) {
        Some(missed) if catch_up && missed <= now => CatchUp::Run(missed),
        _ => CatchUp::Advance,
    }
}

/// `handled_until` for a run that was stopped on purpose: just before its occurrence, so that the
/// next start runs it once more.
pub fn set_back(occurrence: SystemTime) -> SystemTime {
    occurrence - Duration::from_millis(1)
}

// ---- the task ----

async fn sleep_for(wait: Option<Duration>) {
    match wait {
        Some(wait) => tokio::time::sleep(wait).await,
        None => std::future::pending().await,
    }
}

async fn sleep_until_opt(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// The scheduler task. It ends with the shutdown token.
pub(super) async fn run(inner: Arc<Inner>) {
    // Armed at the start: the missed runs are made up `catch_up_delay` after the start.
    let mut catch_up_at = Some(Instant::now() + inner.tuning.catch_up_delay);
    loop {
        inner.wakeups.fetch_add(1, Ordering::SeqCst);
        if inner.shutdown.is_cancelled() {
            break;
        }
        if inner.refusing() {
            // Game mode: nothing starts and nothing advances. There is nothing to compute and
            // nothing to sleep for: a `due` in the past would end a sleep at once and make the
            // loop spin for the whole review. Only a change can end this wait.
            tokio::select! {
                () = inner.wake.notified() => {}
                () = inner.resumed.notified() => {
                    catch_up_at = Some(inner.arm_catch_up());
                }
                () = inner.shutdown.cancelled() => break,
            }
            continue;
        }

        let now = inner.clock.now();
        inner.ensure_handled(now).await;
        let due = inner.next_due();
        let mut wait = due.map(|due| due.duration_since(now).unwrap_or(Duration::ZERO));
        if inner.degraded.load(Ordering::SeqCst) {
            // Without the wake-up notice the sleep is capped, so a sleeping PC is noticed late.
            wait = wait.map(|wait| wait.min(inner.tuning.degraded_cap));
        }

        tokio::select! {
            () = sleep_for(wait) => inner.fire_due().await,
            () = inner.wake.notified() => {}
            () = inner.resumed.notified() => {
                catch_up_at = Some(inner.arm_catch_up());
            }
            () = sleep_until_opt(catch_up_at) => {
                catch_up_at = None;
                inner.plan_catch_up().await;
            }
            () = inner.shutdown.cancelled() => break,
        }
    }
    info!("the job scheduler ended");
}

impl Inner {
    /// Fires every job whose occurrence is due: marks it handled, then starts a run, or skips
    /// the occurrence when the job still runs (section 03 6.11.4).
    async fn fire_due(self: &Arc<Self>) {
        let now = self.clock.now();
        let mut fired = Vec::new();
        {
            let mut state = self.lock();
            let floor = state.floor();
            let defs = state.defs.clone();
            for def in defs.iter().filter(|def| def.enabled) {
                let Some(runtime) = state.runtime.get_mut(&def.id) else {
                    continue;
                };
                let Some(handled) = runtime.handled_until else {
                    continue;
                };
                if let Some(occurrence) = due_now(&def.cron, handled, floor, now) {
                    runtime.handled_until = Some(occurrence);
                    fired.push((Arc::clone(def), occurrence));
                }
            }
        }
        for (def, occurrence) in fired {
            self.save_handled(&def.id, occurrence).await;
            if self
                .begin_run(&def, Trigger::Schedule, Some(occurrence))
                .is_none()
            {
                // The contract has no way to show a skipped occurrence, so it is only logged.
                info!(job = %def.id, "the job is still running; this occurrence was skipped");
            }
        }
        // `nextRunAt` moved.
        self.publish();
    }

    /// Makes up the runs that were missed while Rain or the PC was off: one run per job, however
    /// many occurrences fell in the gap. Runs after the start-up delay, after a resume from
    /// sleep, and at once when game mode ends.
    pub(super) async fn plan_catch_up(self: &Arc<Self>) {
        if self.refusing() {
            return; // game mode: nothing starts and nothing advances
        }
        let now = self.clock.now();
        let mut runs = Vec::new();
        let mut handled = Vec::new();
        {
            let mut state = self.lock();
            state.catch_up_pending = false;
            let defs = state.defs.clone();
            for def in defs.iter().filter(|def| def.enabled) {
                let Some(runtime) = state.runtime.get_mut(&def.id) else {
                    continue;
                };
                match catch_up_decision(def.catch_up, &def.cron, runtime.handled_until, now) {
                    CatchUp::Run(occurrence) => runs.push((Arc::clone(def), occurrence)),
                    CatchUp::Advance => {}
                }
                runtime.handled_until = Some(now);
                handled.push(def.id.clone());
            }
        }
        for id in handled {
            self.save_handled(&id, now).await;
        }
        for (def, occurrence) in runs {
            if self
                .begin_run(&def, Trigger::CatchUp, Some(occurrence))
                .is_none()
            {
                info!(job = %def.id, "the job is running already; the missed run was not made up");
            }
        }
        self.publish();
    }

    /// The first occurrence of any enabled job, counted after what was handled.
    fn next_due(&self) -> Option<SystemTime> {
        let state = self.lock();
        let floor = state.floor();
        earliest(state.defs.iter().filter(|def| def.enabled).map(|def| {
            let handled = state.runtime.get(&def.id)?.handled_until?;
            next_after(&def.cron, origin(handled, floor))
        }))
    }

    /// A job that was just seen enabled has no memory yet: its past starts now.
    async fn ensure_handled(&self, now: SystemTime) {
        let fresh: Vec<String> = {
            let mut state = self.lock();
            let defs = state.defs.clone();
            let mut fresh = Vec::new();
            for def in defs.iter().filter(|def| def.enabled) {
                if let Some(runtime) = state.runtime.get_mut(&def.id)
                    && runtime.handled_until.is_none()
                {
                    runtime.handled_until = Some(now);
                    fresh.push(def.id.clone());
                }
            }
            fresh
        };
        for id in fresh {
            self.save_handled(&id, now).await;
        }
    }

    /// Starts the wait that ends in `plan_catch_up`, after a resume from sleep.
    pub(super) fn arm_catch_up(&self) -> Instant {
        let mut state = self.lock();
        state.catch_up_pending = true;
        state.floor = Some(self.clock.now());
        Instant::now() + self.tuning.catch_up_delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::jobs::schedule;

    fn at(minutes: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + minutes * 60)
    }

    fn every_minute() -> Cron {
        schedule::parse("* * * * *").unwrap()
    }

    #[test]
    fn due_computation() {
        let cron = every_minute();
        // Handled up to minute 10: the next occurrence is minute 11.
        let next = next_after(&cron, at(10)).unwrap();
        assert!(next > at(10) && next <= at(11));
        assert_eq!(due_now(&cron, at(10), None, next), Some(next));
        // The timer may fire 400 ms early; 2 s early is too early.
        assert!(due_now(&cron, at(10), None, next - Duration::from_millis(400)).is_some());
        assert!(due_now(&cron, at(10), None, next - Duration::from_secs(2)).is_none());
    }

    #[test]
    fn the_floor_keeps_missed_occurrences_for_the_catch_up() {
        let cron = every_minute();
        // Handled long ago, but the backend started at minute 50: the next occurrence is after it.
        let next = next_after(&cron, origin(at(0), Some(at(50)))).unwrap();
        assert!(next > at(50));
        assert_eq!(origin(at(60), Some(at(50))), at(60));
    }

    #[test]
    fn the_earliest_occurrence_wins_and_nothing_means_no_sleep() {
        assert_eq!(earliest([Some(at(5)), None, Some(at(3))]), Some(at(3)));
        assert_eq!(earliest([None, None]), None);
        assert_eq!(earliest(Vec::new()), None);
    }

    #[test]
    fn catchup_runs_once_after_long_off() {
        let daily = schedule::parse("0 6 * * *").unwrap();
        let ten_days = Duration::from_secs(10 * 24 * 3600);
        let decision = catch_up_decision(true, &daily, Some(at(0)), at(0) + ten_days);
        let CatchUp::Run(first_missed) = decision else {
            panic!("a run was missed: {decision:?}");
        };
        // One run, for the first missed occurrence: not ten.
        assert!(first_missed > at(0) && first_missed < at(0) + Duration::from_secs(25 * 3600));
    }

    #[test]
    fn catchup_skips_job_enabled_after_the_miss() {
        let daily = schedule::parse("0 6 * * *").unwrap();
        // Enabling sets handled_until to now, so nothing lies between it and now.
        let now = at(0);
        assert_eq!(
            catch_up_decision(true, &daily, Some(now), now),
            CatchUp::Advance
        );
        // A job that opts out of catch-up only advances.
        let long_after = at(0) + Duration::from_secs(3 * 24 * 3600);
        assert_eq!(
            catch_up_decision(false, &daily, Some(at(0)), long_after),
            CatchUp::Advance
        );
    }

    #[test]
    fn first_seen_sets_handled_until() {
        // No memory yet: no past to make up.
        assert_eq!(
            catch_up_decision(true, &every_minute(), None, at(100)),
            CatchUp::Advance
        );
    }

    #[test]
    fn catchup_after_game_mode_stop() {
        // A run stopped on purpose sets handled_until back to just before its occurrence ...
        let cron = schedule::parse("0 6 * * *").unwrap();
        let occurrence = next_after(&cron, at(0)).unwrap();
        let handled = set_back(occurrence);
        // ... so that the next catch-up finds exactly that occurrence again.
        assert_eq!(
            catch_up_decision(
                true,
                &cron,
                Some(handled),
                occurrence + Duration::from_secs(600)
            ),
            CatchUp::Run(occurrence)
        );
    }
}
