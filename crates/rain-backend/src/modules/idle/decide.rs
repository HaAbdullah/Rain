//! The rules of idle jobs as pure functions. Section 03 6.12.2 and 6.12.4.
//!
//! `decide` looks at the jobs and at the machine (how long it has been idle, whether a person or a
//! game is using it) and says what to do: start, freeze, resume, or put a finished job back in
//! the queue. It does nothing itself, so every row of the table in the design is a plain test.

use std::time::{Duration, SystemTime};

use rain_types::{IdleResource, IdleState};

use super::model::Rerun;

const DAY: Duration = Duration::from_secs(24 * 3600);

/// What `decide` knows about one job.
#[derive(Debug, Clone)]
pub struct JobSim {
    pub id: String,
    pub enabled: bool,
    pub resource: IdleResource,
    pub idle_after: Duration,
    pub state: IdleState,
    /// A process exists: the job is `running`, or `paused` (frozen).
    pub has_process: bool,
    /// The current run was started by "Run now", which ignores idleness.
    pub manual: bool,
    pub rerun: Rerun,
    pub min_gap: Duration,
    pub last_started: Option<SystemTime>,
    pub last_ended: Option<SystemTime>,
}

/// What the machine says right now.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub now: SystemTime,
    pub idle: Duration,
    /// A person is using the PC (or a game runs, or Windows could not say).
    pub user_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Start a new run.
    Start(String),
    /// Freeze the run of a job: the PC is in use.
    Suspend(String),
    /// Let a frozen run go on.
    Resume(String),
    /// A finished job is due again.
    Requeue(String),
}

/// Whether a finished job is due again (section 03 6.12.4). `local_date` gives the calendar date
/// of the PC, so that "daily" means "since midnight" and not "24 hours".
pub fn rerun_due(job: &JobSim, now: SystemTime, local_date: &dyn Fn(SystemTime) -> String) -> bool {
    match job.rerun {
        Rerun::Manual => false,
        Rerun::Daily => job
            .last_started
            .is_none_or(|started| local_date(now) > local_date(started)),
        Rerun::Weekly => job
            .last_started
            .is_none_or(|started| now >= started + 7 * DAY),
        Rerun::EveryIdle => job
            .last_ended
            .or(job.last_started)
            .is_none_or(|ended| now >= ended + job.min_gap),
    }
}

fn ready(job: &JobSim, inputs: &Inputs) -> bool {
    job.enabled && !inputs.user_present && inputs.idle >= job.idle_after
}

fn holds_resource(job: &JobSim, resource: IdleResource) -> bool {
    job.resource == resource
        && job.has_process
        && matches!(job.state, IdleState::Running | IdleState::Paused)
}

/// Changes the states in `book` as the actions will, and returns the actions. In this order:
/// finished jobs that are due become queued; frozen jobs resume (before a new job may start);
/// running jobs freeze when the PC is in use; queued jobs start when their resource is free.
pub fn decide(
    book: &mut [JobSim],
    inputs: &Inputs,
    local_date: &dyn Fn(SystemTime) -> String,
) -> Vec<Action> {
    let mut actions = Vec::new();

    for job in book.iter_mut() {
        if job.enabled
            && matches!(job.state, IdleState::Success | IdleState::Failed)
            && rerun_due(job, inputs.now, local_date)
        {
            job.state = IdleState::Queued;
            actions.push(Action::Requeue(job.id.clone()));
        }
    }
    for job in book.iter_mut() {
        if job.state == IdleState::Paused && job.has_process && ready(job, inputs) {
            job.state = IdleState::Running;
            actions.push(Action::Resume(job.id.clone()));
        }
    }
    for job in book.iter_mut() {
        // A run that was started by "Run now" ignores idleness: it is never frozen.
        if job.state == IdleState::Running && job.has_process && !job.manual && inputs.user_present
        {
            job.state = IdleState::Paused;
            actions.push(Action::Suspend(job.id.clone()));
        }
    }
    for index in 0..book.len() {
        let job = &book[index];
        if job.state != IdleState::Queued || !ready(job, inputs) {
            continue;
        }
        let resource = job.resource;
        if book.iter().any(|other| holds_resource(other, resource)) {
            continue;
        }
        let job = &mut book[index];
        job.state = IdleState::Running;
        job.has_process = true;
        job.manual = false;
        actions.push(Action::Start(job.id.clone()));
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> SystemTime {
        // 2027-01-15 08:00 UTC plus `minutes`.
        let seconds = 1_799_999_200 + minutes * 60;
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64)
    }

    /// A date that changes every 24 hours at 00:00 UTC, standing in for the PC's calendar.
    fn date(time: SystemTime) -> String {
        let days = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            / 86_400;
        format!("day-{days:06}")
    }

    fn job(id: &str, state: IdleState) -> JobSim {
        JobSim {
            id: id.to_owned(),
            enabled: true,
            resource: IdleResource::Cpu,
            idle_after: Duration::from_secs(10 * 60),
            state,
            has_process: matches!(state, IdleState::Running | IdleState::Paused),
            manual: false,
            rerun: Rerun::Daily,
            min_gap: Duration::from_secs(3600),
            last_started: None,
            last_ended: None,
        }
    }

    fn quiet(minutes: u64) -> Inputs {
        Inputs {
            now: at(100),
            idle: Duration::from_secs(minutes * 60),
            user_present: false,
        }
    }

    fn busy() -> Inputs {
        Inputs {
            now: at(100),
            idle: Duration::from_secs(5),
            user_present: true,
        }
    }

    fn run(book: &mut [JobSim], inputs: &Inputs) -> Vec<Action> {
        decide(book, inputs, &date)
    }

    #[test]
    fn decide_table() {
        // queued, idle long enough: start
        let mut book = [job("a", IdleState::Queued)];
        assert_eq!(run(&mut book, &quiet(10)), [Action::Start("a".into())]);
        assert_eq!(book[0].state, IdleState::Running);

        // queued, not idle long enough: nothing
        let mut book = [job("a", IdleState::Queued)];
        assert!(run(&mut book, &quiet(9)).is_empty());

        // queued, long idle but a person (or a game) is present: nothing
        let mut book = [job("a", IdleState::Queued)];
        let mut inputs = quiet(60);
        inputs.user_present = true;
        assert!(run(&mut book, &inputs).is_empty());

        // queued but disabled: never starts
        let mut book = [job("a", IdleState::Queued)];
        book[0].enabled = false;
        assert!(run(&mut book, &quiet(60)).is_empty());

        // running (started by idle), the PC is in use: freeze, progress is kept elsewhere
        let mut book = [job("a", IdleState::Running)];
        assert_eq!(run(&mut book, &busy()), [Action::Suspend("a".into())]);
        assert_eq!(book[0].state, IdleState::Paused);

        // running (started by Run now), the PC is in use: nothing
        let mut book = [job("a", IdleState::Running)];
        book[0].manual = true;
        assert!(run(&mut book, &busy()).is_empty());
        assert_eq!(book[0].state, IdleState::Running);

        // paused with a process, the PC is quiet long enough: resume
        let mut book = [job("a", IdleState::Paused)];
        assert_eq!(run(&mut book, &quiet(10)), [Action::Resume("a".into())]);
        assert_eq!(book[0].state, IdleState::Running);

        // paused with a process, not quiet long enough: stays frozen
        let mut book = [job("a", IdleState::Paused)];
        assert!(run(&mut book, &quiet(3)).is_empty());

        // paused without a process (it was switched off): not resumed, no resource held
        let mut book = [job("a", IdleState::Paused)];
        book[0].has_process = false;
        assert!(run(&mut book, &quiet(30)).is_empty());

        // a running job on a quiet PC: nothing to do
        let mut book = [job("a", IdleState::Running)];
        assert!(run(&mut book, &quiet(30)).is_empty());
    }

    #[test]
    fn a_frozen_job_resumes_before_a_new_one_may_start() {
        let mut book = [
            job("queued", IdleState::Queued),
            job("frozen", IdleState::Paused),
        ];
        let actions = run(&mut book, &quiet(30));
        assert_eq!(actions, [Action::Resume("frozen".into())]);
        assert_eq!(book[0].state, IdleState::Queued, "the resource is taken");
    }

    #[test]
    fn only_one_job_runs_per_resource_and_two_resources_run_together() {
        let mut gpu = job("gpu", IdleState::Queued);
        gpu.resource = IdleResource::Gpu;
        let mut book = [
            job("cpu-1", IdleState::Queued),
            job("cpu-2", IdleState::Queued),
            gpu,
        ];
        let actions = run(&mut book, &quiet(30));
        assert_eq!(
            actions,
            [Action::Start("cpu-1".into()), Action::Start("gpu".into())]
        );
        assert_eq!(book[1].state, IdleState::Queued);
    }

    #[test]
    fn a_finished_job_is_queued_again_when_the_rule_says_so_and_may_start_in_the_same_pass() {
        let mut done = job("a", IdleState::Success);
        done.last_started = Some(at(-24 * 60));
        let mut book = [done];
        let actions = run(&mut book, &quiet(30));
        assert_eq!(
            actions,
            [Action::Requeue("a".into()), Action::Start("a".into())]
        );
    }

    #[test]
    fn rerun_rule() {
        let local = &date;
        let mut j = job("a", IdleState::Success);
        let now = at(0); // 08:00 UTC
        // never ran: due (daily, weekly, every_idle); manual: never
        for rerun in [Rerun::Daily, Rerun::Weekly, Rerun::EveryIdle] {
            j.rerun = rerun;
            assert!(rerun_due(&j, now, local), "{rerun:?} never ran");
        }
        j.rerun = Rerun::Manual;
        assert!(!rerun_due(&j, now, local));

        // daily: across midnight yes, the same day no, even 23 hours later
        j.rerun = Rerun::Daily;
        j.last_started = Some(at(-9 * 60)); // 23:00 the day before
        assert!(rerun_due(&j, now, local), "a new calendar day");
        j.last_started = Some(at(-7 * 60)); // 01:00 the same day
        assert!(!rerun_due(&j, now, local), "the same day, however long ago");

        // weekly: 7 days, not 6 days 23 hours
        j.rerun = Rerun::Weekly;
        j.last_started = Some(at(-7 * 24 * 60));
        assert!(rerun_due(&j, now, local));
        j.last_started = Some(at(-7 * 24 * 60 + 60));
        assert!(!rerun_due(&j, now, local));

        // every_idle: the gap after the END of the last run
        j.rerun = Rerun::EveryIdle;
        j.last_started = Some(at(-300));
        j.last_ended = Some(at(-30));
        assert!(!rerun_due(&j, now, local), "30 minutes of a 60 minute gap");
        j.last_ended = Some(at(-61));
        assert!(rerun_due(&j, now, local));
        j.min_gap = Duration::ZERO;
        j.last_ended = Some(now);
        assert!(rerun_due(&j, now, local), "gap 0: at once");
    }

    #[test]
    fn a_manual_job_that_never_ran_starts_once_when_idle() {
        // Its state is queued from the start (section 03 6.12.6); the rerun rule only applies
        // after a run.
        let mut j = job("a", IdleState::Queued);
        j.rerun = Rerun::Manual;
        let mut book = [j];
        assert_eq!(run(&mut book, &quiet(30)), [Action::Start("a".into())]);
    }

    #[test]
    fn decide_machine_none_counts_as_present() {
        // The sampler passes `user_present = true` and idle 0 when Windows refused the call.
        let inputs = Inputs {
            now: at(0),
            idle: Duration::ZERO,
            user_present: true,
        };
        let mut book = [
            job("queued", IdleState::Queued),
            job("running", IdleState::Running),
        ];
        let actions = run(&mut book, &inputs);
        assert_eq!(actions, [Action::Suspend("running".into())]);
    }

    /// For every combination of states and inputs: never two jobs on one resource that run or
    /// hold a process at the same time, and a frozen job has a process.
    #[test]
    fn idle_one_per_resource() {
        let states = [
            IdleState::Queued,
            IdleState::Running,
            IdleState::Paused,
            IdleState::Success,
            IdleState::Failed,
        ];
        let mut cases = 0;
        for a in states {
            for b in states {
                for c in states {
                    for present in [false, true] {
                        for minutes in [0, 10, 60] {
                            // Start from a legal situation: at most one process per resource.
                            let mut gpu = job("c", c);
                            gpu.resource = IdleResource::Gpu;
                            let mut book = [job("a", a), job("b", b), gpu];
                            if book[0].has_process && book[1].has_process {
                                book[1].has_process = false;
                                book[1].state = IdleState::Queued;
                            }
                            let mut inputs = quiet(minutes);
                            inputs.user_present = present;
                            decide(&mut book, &inputs, &date);
                            let holding = |resource| {
                                book.iter().filter(|j| holds_resource(j, resource)).count()
                            };
                            assert!(
                                holding(IdleResource::Cpu) <= 1,
                                "{a:?} {b:?} {present} {minutes}"
                            );
                            assert!(holding(IdleResource::Gpu) <= 1);
                            assert!(
                                book.iter()
                                    .all(|j| j.state != IdleState::Paused || j.has_process),
                                "a frozen job has a process"
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 5 * 5 * 5 * 2 * 3);
    }
}
