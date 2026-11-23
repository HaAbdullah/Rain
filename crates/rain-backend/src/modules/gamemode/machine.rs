//! The state machine of game mode. Section 06 6.7.2.
//!
//! It is pure: no clock of its own, no I/O, no tasks. Time comes in as a `tokio::time::Instant`
//! that the caller passes, and what has to happen outside comes out as `Effect`s. So every row of
//! the transition table is a plain unit test, and the service only has to carry out the effects.
//!
//! Reading guide: `step` takes one `Input` and moves the phase. Every change that the screen can
//! see raises `rev` by one and asks for a publish.

use std::time::Duration;

use rain_types::{GameModeChoice, GameModePhase, GameModeState, WindDownItem, WindDownStatus};
use tokio::time::Instant;

use super::items;

/// The lengths of time in section 06 6.7.3. A test replaces them with short ones
/// (`state.seam::<Timings>()`).
#[derive(Debug, Clone)]
pub struct Timings {
    /// How long "Wait" waits for the running work to end (`GAME_MODE_WAIT_LIMIT`).
    pub wait_limit: Duration,
    /// A game mode that nobody answers in `review` cancels itself after this (`REVIEW_ABANDON`).
    pub review_abandon: Duration,
    /// Each row stays in `stopping` at least this long, so that the user can see it
    /// (`GAME_MODE_ITEM_MIN_VISIBLE`).
    pub item_min_visible: Duration,
    /// How long one group of runs may take to stop (`GAME_MODE_ITEM_BUDGET`).
    pub item_budget: Duration,
    /// How long the call that starts the game may take (`GAME_MODE_LAUNCH_BUDGET`).
    pub launch_budget: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            wait_limit: Duration::from_secs(180),
            review_abandon: Duration::from_secs(600),
            item_min_visible: Duration::from_millis(400),
            item_budget: Duration::from_secs(5),
            launch_budget: Duration::from_secs(10),
        }
    }
}

/// Why a request is refused. The service turns each into an `ApiError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A choice, but game mode is off.
    NotRunning,
    /// `cancel` after the wind-down began.
    WindingDown,
    /// `start` after `done`.
    Stopping,
}

/// What happened to the machine, from the outside.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// `items` were measured just before. `then_launch` is the game to start afterwards.
    Start {
        now: Instant,
        countdown_seconds: u32,
        items: Vec<WindDownItem>,
        then_launch: Option<String>,
    },
    Choice {
        choice: GameModeChoice,
        now: Instant,
    },
    /// A timer woke the driver: the countdown second, the wait limit or the abandon limit.
    Tick { now: Instant },
    /// The rows were measured again (running work changed, or a session opened or closed).
    Items {
        items: Vec<WindDownItem>,
        now: Instant,
    },
    /// The wind-down moves a row.
    Status {
        id: &'static str,
        status: WindDownStatus,
    },
    /// The game could not be started.
    LaunchError(String),
    /// The last step of the wind-down is done.
    Finished,
    /// Nothing changed in the machine, but the screen shows something that did (the recap).
    Refresh,
}

/// What the service must do. The machine has already changed its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Publish the new state.
    Publish,
    /// The wind-down begins: start its task.
    StartWindDown,
    /// Game mode ended without the wind-down: clear the flag, resume the work, publish the last
    /// state (in this order: section 06 6.7.2).
    Cancelled,
}

pub struct Machine {
    timings: Timings,
    phase: GameModePhase,
    waiting: bool,
    wait_timed_out: bool,
    /// The user chose "Wait" at least once. Goes into the record.
    waited: bool,
    countdown_seconds: u32,
    seconds_left: u32,
    items: Vec<WindDownItem>,
    then_launch: Option<String>,
    launch_error: Option<String>,
    rev: u64,
    ends_at: Option<Instant>,
    wait_deadline: Option<Instant>,
    abandon_deadline: Option<Instant>,
}

impl Machine {
    /// `countdown_seconds` is only what an `off` state shows until a run starts.
    pub fn new(timings: Timings, countdown_seconds: u32) -> Self {
        Self {
            timings,
            phase: GameModePhase::Off,
            waiting: false,
            wait_timed_out: false,
            waited: false,
            countdown_seconds,
            seconds_left: 0,
            items: items::idle_rows(),
            then_launch: None,
            launch_error: None,
            // Starts at 1 for each backend process (section 06 3.2).
            rev: 1,
            ends_at: None,
            wait_deadline: None,
            abandon_deadline: None,
        }
    }

    pub fn phase(&self) -> GameModePhase {
        self.phase
    }

    pub fn waited(&self) -> bool {
        self.waited
    }

    /// The state for the screen. The recap (`last`) is added by the service.
    pub fn state(&self) -> GameModeState {
        GameModeState {
            phase: self.phase,
            waiting: self.waiting,
            seconds_left: self.seconds_left,
            countdown_seconds: self.countdown_seconds,
            items: self.items.clone(),
            then_launch_game_id: self.then_launch.clone(),
            wait_timed_out: self.wait_timed_out.then_some(true),
            launch_error: self.launch_error.clone(),
            last: None,
            rev: self.rev,
        }
    }

    /// When the driver must wake next: the next whole countdown second, the wait limit or the
    /// abandon limit. `None` when no timer is needed.
    pub fn next_wake(&self) -> Option<Instant> {
        match self.phase {
            GameModePhase::Countdown => {
                let ends_at = self.ends_at?;
                // `seconds_left` drops when `seconds_left - 1` seconds remain.
                let before_end =
                    Duration::from_secs(u64::from(self.seconds_left.saturating_sub(1)));
                Some(ends_at.checked_sub(before_end).unwrap_or(ends_at))
            }
            GameModePhase::Review => {
                let wait = if self.waiting {
                    self.wait_deadline
                } else {
                    None
                };
                [wait, self.abandon_deadline].into_iter().flatten().min()
            }
            _ => None,
        }
    }

    pub fn step(&mut self, input: Input) -> Result<Vec<Effect>, Refusal> {
        match input {
            Input::Start {
                now,
                countdown_seconds,
                items,
                then_launch,
            } => self.start(now, countdown_seconds, items, then_launch),
            Input::Choice { choice, now } => self.choose(choice, now),
            Input::Tick { now } => Ok(self.tick(now)),
            Input::Items { items, now } => Ok(self.remeasure(items, now)),
            Input::Status { id, status } => Ok(self.set_status(id, status)),
            Input::LaunchError(message) => {
                self.launch_error = Some(message);
                Ok(self.touch())
            }
            Input::Finished => Ok(self.finish()),
            Input::Refresh => Ok(self.touch()),
        }
    }

    // ---- the transitions ----

    fn start(
        &mut self,
        now: Instant,
        countdown_seconds: u32,
        items: Vec<WindDownItem>,
        then_launch: Option<String>,
    ) -> Result<Vec<Effect>, Refusal> {
        match self.phase {
            // A run is under way: it is returned unchanged and the body is ignored.
            GameModePhase::Review | GameModePhase::Countdown | GameModePhase::WindingDown => {
                return Ok(Vec::new());
            }
            GameModePhase::Done => return Err(Refusal::Stopping),
            GameModePhase::Off => {}
        }
        self.countdown_seconds = countdown_seconds;
        self.waiting = false;
        self.wait_timed_out = false;
        self.waited = false;
        self.launch_error = None;
        self.then_launch = then_launch;
        self.items = items;
        if items::interrupting(&self.items) > 0 {
            self.phase = GameModePhase::Review;
            self.abandon_deadline = Some(now + self.timings.review_abandon);
        } else {
            self.begin_countdown(now);
        }
        Ok(self.touch())
    }

    fn choose(&mut self, choice: GameModeChoice, now: Instant) -> Result<Vec<Effect>, Refusal> {
        use GameModeChoice::{Cancel, CloseNow, StopAnyway, Wait};
        match (self.phase, choice) {
            (GameModePhase::Off, Cancel) => Ok(Vec::new()),
            (GameModePhase::Off, _) => Err(Refusal::NotRunning),

            (GameModePhase::Review, Wait) if !self.waiting => {
                self.waiting = true;
                self.waited = true;
                self.wait_timed_out = false;
                self.wait_deadline = Some(now + self.timings.wait_limit);
                self.abandon_deadline = Some(now + self.timings.review_abandon);
                Ok(self.touch())
            }
            (GameModePhase::Review, StopAnyway) => {
                self.begin_countdown(now);
                Ok(self.touch())
            }
            (GameModePhase::Review | GameModePhase::Countdown, Cancel) => Ok(self.cancel()),

            (GameModePhase::Countdown, CloseNow) => Ok(self.begin_wind_down()),

            (GameModePhase::WindingDown | GameModePhase::Done, Cancel) => Err(Refusal::WindingDown),
            // Everything else is a click that was already satisfied or overtaken: no change.
            _ => Ok(Vec::new()),
        }
    }

    fn tick(&mut self, now: Instant) -> Vec<Effect> {
        match self.phase {
            GameModePhase::Countdown => {
                let Some(ends_at) = self.ends_at else {
                    return Vec::new();
                };
                if now >= ends_at {
                    return self.begin_wind_down();
                }
                let left = seconds_until(ends_at, now);
                if left == self.seconds_left {
                    return Vec::new();
                }
                self.seconds_left = left;
                self.touch()
            }
            GameModePhase::Review => {
                if self.abandon_deadline.is_some_and(|at| now >= at) {
                    return self.cancel();
                }
                if self.waiting && self.wait_deadline.is_some_and(|at| now >= at) {
                    self.waiting = false;
                    self.wait_timed_out = true;
                    self.wait_deadline = None;
                    return self.touch();
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn remeasure(&mut self, items: Vec<WindDownItem>, now: Instant) -> Vec<Effect> {
        if !matches!(self.phase, GameModePhase::Review | GameModePhase::Countdown) {
            return Vec::new();
        }
        let mut changed = false;
        if items != self.items {
            self.items = items;
            changed = true;
        }
        // The work that interrupts has ended by itself: the gate is satisfied.
        if self.phase == GameModePhase::Review && items::interrupting(&self.items) == 0 {
            self.begin_countdown(now);
            changed = true;
        }
        if changed { self.touch() } else { Vec::new() }
    }

    fn set_status(&mut self, id: &str, status: WindDownStatus) -> Vec<Effect> {
        if self.phase != GameModePhase::WindingDown {
            return Vec::new();
        }
        match self.items.iter_mut().find(|item| item.id == id) {
            Some(item) if item.status != status => {
                item.status = status;
                self.touch()
            }
            _ => Vec::new(),
        }
    }

    fn finish(&mut self) -> Vec<Effect> {
        if self.phase != GameModePhase::WindingDown {
            return Vec::new();
        }
        self.phase = GameModePhase::Done;
        self.touch()
    }

    // ---- helpers ----

    fn begin_countdown(&mut self, now: Instant) {
        self.phase = GameModePhase::Countdown;
        self.waiting = false;
        self.wait_deadline = None;
        self.abandon_deadline = None;
        self.ends_at = Some(now + Duration::from_secs(u64::from(self.countdown_seconds)));
        self.seconds_left = self.countdown_seconds;
    }

    fn begin_wind_down(&mut self) -> Vec<Effect> {
        self.phase = GameModePhase::WindingDown;
        self.seconds_left = 0;
        self.ends_at = None;
        self.rev += 1;
        vec![Effect::Publish, Effect::StartWindDown]
    }

    /// Back to `off`: the run is forgotten, the rows show nothing.
    fn cancel(&mut self) -> Vec<Effect> {
        self.phase = GameModePhase::Off;
        self.waiting = false;
        self.wait_timed_out = false;
        self.seconds_left = 0;
        self.then_launch = None;
        self.launch_error = None;
        self.items = items::idle_rows();
        self.ends_at = None;
        self.wait_deadline = None;
        self.abandon_deadline = None;
        self.rev += 1;
        vec![Effect::Cancelled]
    }

    fn touch(&mut self) -> Vec<Effect> {
        self.rev += 1;
        vec![Effect::Publish]
    }
}

/// `ceil(ends_at - now)` in whole seconds.
fn seconds_until(ends_at: Instant, now: Instant) -> u32 {
    let remaining = ends_at.saturating_duration_since(now);
    u32::try_from(remaining.as_nanos().div_ceil(1_000_000_000)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::winddown::Counts;
    use GameModeChoice::{Cancel, CloseNow, StopAnyway, Wait};

    fn machine() -> Machine {
        Machine::new(Timings::default(), 10)
    }

    fn quiet() -> Vec<WindDownItem> {
        items::measure(0, Counts::default(), Counts::default())
    }

    fn busy() -> Vec<WindDownItem> {
        items::measure(
            0,
            Counts {
                enabled: 1,
                running: 1,
            },
            Counts::default(),
        )
    }

    fn start(
        m: &mut Machine,
        now: Instant,
        items: Vec<WindDownItem>,
    ) -> Result<Vec<Effect>, Refusal> {
        m.step(Input::Start {
            now,
            countdown_seconds: 10,
            items,
            then_launch: None,
        })
    }

    fn choose(
        m: &mut Machine,
        choice: GameModeChoice,
        now: Instant,
    ) -> Result<Vec<Effect>, Refusal> {
        m.step(Input::Choice { choice, now })
    }

    #[test]
    fn start_with_nothing_interrupting_goes_to_countdown() {
        let (mut m, now) = (machine(), Instant::now());
        assert_eq!(start(&mut m, now, quiet()), Ok(vec![Effect::Publish]));
        let state = m.state();
        assert_eq!(state.phase, GameModePhase::Countdown);
        assert_eq!(state.seconds_left, 10);
        assert_eq!(state.countdown_seconds, 10);
    }

    #[test]
    fn start_with_interrupting_work_goes_to_review() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, busy()).unwrap();
        assert_eq!(m.phase(), GameModePhase::Review);
        assert!(!m.state().waiting);
    }

    #[test]
    fn start_while_running_changes_nothing_and_after_done_is_refused() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, busy()).unwrap();
        let before = m.state();
        assert_eq!(start(&mut m, now, quiet()), Ok(Vec::new()));
        assert_eq!(m.state(), before);

        choose(&mut m, StopAnyway, now).unwrap();
        choose(&mut m, CloseNow, now).unwrap();
        assert_eq!(start(&mut m, now, quiet()), Ok(Vec::new()), "winding down");
        m.step(Input::Finished).unwrap();
        assert_eq!(start(&mut m, now, quiet()), Err(Refusal::Stopping));
    }

    /// Every cell of the choice table of section 06 6.7.2.
    #[test]
    fn the_choice_table() {
        let now = Instant::now();
        // (how to reach the phase, choice, expected phase after, expected result is an error)
        type Reach = fn(&mut Machine, Instant);
        let phases: [(&str, Reach); 6] = [
            ("off", |_, _| {}),
            ("review", |m, now| {
                start(m, now, busy()).unwrap();
            }),
            ("review waiting", |m, now| {
                start(m, now, busy()).unwrap();
                choose(m, Wait, now).unwrap();
            }),
            ("countdown", |m, now| {
                start(m, now, quiet()).unwrap();
            }),
            ("winding down", |m, now| {
                start(m, now, quiet()).unwrap();
                choose(m, CloseNow, now).unwrap();
            }),
            ("done", |m, now| {
                start(m, now, quiet()).unwrap();
                choose(m, CloseNow, now).unwrap();
                m.step(Input::Finished).unwrap();
            }),
        ];
        use GameModePhase::{Countdown, Done, Off, Review, WindingDown};
        // For each phase: the result of wait, stopAnyway, closeNow, cancel.
        let expected: [[Result<GameModePhase, Refusal>; 4]; 6] = [
            [
                Err(Refusal::NotRunning),
                Err(Refusal::NotRunning),
                Err(Refusal::NotRunning),
                Ok(Off),
            ],
            [Ok(Review), Ok(Countdown), Ok(Review), Ok(Off)],
            [Ok(Review), Ok(Countdown), Ok(Review), Ok(Off)],
            [Ok(Countdown), Ok(Countdown), Ok(WindingDown), Ok(Off)],
            [
                Ok(WindingDown),
                Ok(WindingDown),
                Ok(WindingDown),
                Err(Refusal::WindingDown),
            ],
            [Ok(Done), Ok(Done), Ok(Done), Err(Refusal::WindingDown)],
        ];
        for (row, (name, reach)) in phases.iter().enumerate() {
            for (column, choice) in [Wait, StopAnyway, CloseNow, Cancel].into_iter().enumerate() {
                let mut m = machine();
                reach(&mut m, now);
                let result = choose(&mut m, choice, now).map(|_| m.phase());
                assert_eq!(result, expected[row][column], "{name} + {choice:?}");
            }
        }
    }

    #[test]
    fn rev_rises_on_every_change_and_only_then() {
        let (mut m, now) = (machine(), Instant::now());
        assert_eq!(m.state().rev, 1);
        start(&mut m, now, busy()).unwrap();
        assert_eq!(m.state().rev, 2);
        choose(&mut m, Wait, now).unwrap();
        assert_eq!(m.state().rev, 3);
        choose(&mut m, Wait, now).unwrap();
        assert_eq!(m.state().rev, 3, "a repeated Wait changes nothing");
        m.step(Input::Tick { now }).unwrap();
        assert_eq!(m.state().rev, 3, "a tick with nothing due changes nothing");
        choose(&mut m, Cancel, now).unwrap();
        assert_eq!(m.state().rev, 4);
    }

    #[test]
    fn the_countdown_counts_whole_seconds_down_from_a_controlled_instant() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, quiet()).unwrap();
        let second = Duration::from_secs(1);
        assert_eq!(
            m.next_wake(),
            Some(now + second),
            "the first second ends after 1 s"
        );

        let mut left = Vec::new();
        for n in 1..=9u32 {
            let at = now + second * n;
            m.step(Input::Tick { now: at }).unwrap();
            left.push(m.state().seconds_left);
        }
        assert_eq!(left, [9, 8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(
            m.next_wake(),
            Some(now + second * 10),
            "the last second ends at the deadline"
        );
        assert_eq!(m.phase(), GameModePhase::Countdown);

        let effects = m
            .step(Input::Tick {
                now: now + second * 10,
            })
            .unwrap();
        assert_eq!(effects, [Effect::Publish, Effect::StartWindDown]);
        assert_eq!(m.phase(), GameModePhase::WindingDown);
        assert_eq!(m.state().seconds_left, 0);
    }

    #[test]
    fn a_tick_in_the_middle_of_a_second_rounds_up() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, quiet()).unwrap();
        m.step(Input::Tick {
            now: now + Duration::from_millis(1500),
        })
        .unwrap();
        assert_eq!(m.state().seconds_left, 9);
    }

    #[test]
    fn a_deadline_that_is_already_past_starts_the_wind_down() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, quiet()).unwrap();
        let effects = m
            .step(Input::Tick {
                now: now + Duration::from_secs(3600),
            })
            .unwrap();
        assert!(effects.contains(&Effect::StartWindDown));
    }

    #[test]
    fn wait_times_out_after_the_wait_limit() {
        let timings = Timings {
            wait_limit: Duration::from_secs(180),
            ..Timings::default()
        };
        let (mut m, now) = (Machine::new(timings, 10), Instant::now());
        start(&mut m, now, busy()).unwrap();
        choose(&mut m, Wait, now).unwrap();
        assert_eq!(m.next_wake(), Some(now + Duration::from_secs(180)));

        m.step(Input::Tick {
            now: now + Duration::from_secs(179),
        })
        .unwrap();
        assert!(m.state().waiting);
        m.step(Input::Tick {
            now: now + Duration::from_secs(180),
        })
        .unwrap();
        let state = m.state();
        assert!(!state.waiting);
        assert_eq!(state.wait_timed_out, Some(true));
        assert_eq!(state.phase, GameModePhase::Review);
        // The abandon limit is the next timer.
        assert_eq!(m.next_wake(), Some(now + Duration::from_secs(600)));
        // "Wait" works again and clears the note.
        choose(&mut m, Wait, now + Duration::from_secs(181)).unwrap();
        assert_eq!(m.state().wait_timed_out, None);
        assert!(m.state().waiting);
    }

    #[test]
    fn a_review_that_nobody_answers_cancels_itself_after_ten_minutes() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, busy()).unwrap();
        assert_eq!(m.next_wake(), Some(now + Duration::from_secs(600)));
        let effects = m
            .step(Input::Tick {
                now: now + Duration::from_secs(600),
            })
            .unwrap();
        assert_eq!(effects, [Effect::Cancelled]);
        assert_eq!(m.phase(), GameModePhase::Off);
    }

    #[test]
    fn when_the_interrupting_work_ends_by_itself_review_moves_on_to_the_countdown() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, busy()).unwrap();
        choose(&mut m, Wait, now).unwrap();
        m.step(Input::Items {
            items: quiet(),
            now,
        })
        .unwrap();
        let state = m.state();
        assert_eq!(state.phase, GameModePhase::Countdown);
        assert!(!state.waiting);
        assert_eq!(state.seconds_left, 10);
    }

    #[test]
    fn new_measurements_update_the_rows_without_moving_the_phase() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, busy()).unwrap();
        let more = items::measure(
            3,
            Counts {
                enabled: 1,
                running: 1,
            },
            Counts::default(),
        );
        m.step(Input::Items { items: more, now }).unwrap();
        assert_eq!(m.phase(), GameModePhase::Review);
        assert_eq!(m.state().items[0].count, 3);
        // The same numbers again are no change.
        let rev = m.state().rev;
        let same = m.state().items;
        assert!(
            m.step(Input::Items { items: same, now })
                .unwrap()
                .is_empty()
        );
        assert_eq!(m.state().rev, rev);
    }

    #[test]
    fn cancel_forgets_the_run_and_clears_the_game() {
        let (mut m, now) = (machine(), Instant::now());
        m.step(Input::Start {
            now,
            countdown_seconds: 5,
            items: quiet(),
            then_launch: Some("game-a".into()),
        })
        .unwrap();
        assert_eq!(m.state().then_launch_game_id.as_deref(), Some("game-a"));
        assert_eq!(choose(&mut m, Cancel, now), Ok(vec![Effect::Cancelled]));
        let state = m.state();
        assert_eq!(state.phase, GameModePhase::Off);
        assert_eq!(state.then_launch_game_id, None);
        assert!(state.items.iter().all(|item| item.count == 0));
        assert_eq!(m.next_wake(), None);
    }

    #[test]
    fn the_wind_down_moves_rows_and_ends_in_done() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, quiet()).unwrap();
        choose(&mut m, CloseNow, now).unwrap();
        m.step(Input::Status {
            id: items::JOBS,
            status: WindDownStatus::Stopping,
        })
        .unwrap();
        assert_eq!(m.state().items[1].status, WindDownStatus::Stopping);
        m.step(Input::LaunchError("no".into())).unwrap();
        assert_eq!(m.state().launch_error.as_deref(), Some("no"));
        m.step(Input::Finished).unwrap();
        assert_eq!(m.phase(), GameModePhase::Done);
    }

    #[test]
    fn a_row_status_is_ignored_outside_the_wind_down() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, quiet()).unwrap();
        let rev = m.state().rev;
        let effects = m
            .step(Input::Status {
                id: items::JOBS,
                status: WindDownStatus::Stopped,
            })
            .unwrap();
        assert!(effects.is_empty());
        assert_eq!(m.state().rev, rev);
    }

    #[test]
    fn a_second_start_after_a_cancel_begins_a_new_run() {
        let (mut m, now) = (machine(), Instant::now());
        start(&mut m, now, busy()).unwrap();
        choose(&mut m, Wait, now).unwrap();
        choose(&mut m, Cancel, now).unwrap();
        start(&mut m, now, busy()).unwrap();
        assert_eq!(m.phase(), GameModePhase::Review);
        assert!(!m.state().waiting, "the earlier Wait is forgotten");
        assert!(!m.waited());
    }
}
