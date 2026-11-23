//! Game mode: the logic and the in-memory state. Section 06 6.7.
//!
//! How the pieces fit:
//! - `machine::Machine` is the pure state machine. Every change goes through its `step`.
//! - `Service::run_input` locks the machine, steps it, and carries out what comes back
//!   (publish a frame, resume the work after a cancel, start the wind-down).
//! - While game mode is not `off` there is ONE driver task (`drive`). It sleeps until the next
//!   thing that is due (the next countdown second, the wait limit) or until something changes,
//!   and feeds that into the machine. When game mode is `off` no task and no timer exists.
//! - The wind-down (`wind_down`) is a second task that runs the stops in order, writes the
//!   record, publishes `done` and then ends Rain.
//!
//! The one rule that matters most: game mode ends only what Rain itself started, and only
//! through `WindDownPort`. It never stops a Claude session; it only counts them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rain_types::time::iso_from_system_time;
use rain_types::{
    GameMode, GameModeChoice, GameModePhase, GameModeRecap, GameModeState, StartGameModeBody,
    StopReason, WindDownKind, WindDownStatus,
};
use tokio::sync::Notify;
use tokio::time::Instant;
use tracing::{info, warn};

use super::items;
use super::machine::{Effect, Input, Machine, Refusal, Timings};
use super::neighbours::Neighbours;
use super::recap;
use super::store::{self, Finish, ItemRow, NewRecord};
use crate::core::error::{ApiError, ApiResult};
use crate::core::shutdown::ActivityGuard;
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::core::throttle::Throttle;
use crate::core::winddown::{StopOutcome, WindDownPort};
use rain_types::RecapOutcome;

/// How often the topic may be published (section 06 2.3). In practice once a second in the
/// countdown plus one frame per step.
const TOPIC_RATE_PER_SECOND: u32 = 4;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Service {
    /// Weak: the state owns this service.
    state: WeakAppState,
    neighbours: Arc<dyn Neighbours>,
    timings: Timings,
    machine: Mutex<Machine>,
    throttle: Throttle<GameMode>,
    /// Wakes the driver when something OUTSIDE it changed the machine (a choice, a start), so it
    /// computes its next sleep again. The driver's own inputs must never notify this: it would
    /// store a permit for its own next `select!` and the loop would never sleep.
    wake: Notify,
    /// True while a driver task exists. Decided under the `machine` lock (see `drive`).
    driver_running: AtomicBool,
    /// One `start` at a time.
    start_lock: tokio::sync::Mutex<()>,
    /// The recap of the last game mode, shown while the phase is `off`.
    last: Mutex<Option<GameModeRecap>>,
    /// The title of the game to start afterwards, for the record.
    launch_title: Mutex<Option<String>>,
}

impl Service {
    pub fn new(state: &AppState, neighbours: Arc<dyn Neighbours>, timings: Timings) -> Self {
        let countdown = state.settings().resolved().countdown_seconds;
        Self {
            state: state.downgrade(),
            neighbours,
            machine: Mutex::new(Machine::new(timings.clone(), countdown)),
            timings,
            throttle: Throttle::new(state.bus.clone(), TOPIC_RATE_PER_SECOND),
            wake: Notify::new(),
            driver_running: AtomicBool::new(false),
            start_lock: tokio::sync::Mutex::new(()),
            last: Mutex::new(None),
            launch_title: Mutex::new(None),
        }
    }

    /// At backend start: a record that never finished becomes `incomplete`, then the newest
    /// finished record becomes the recap of the `off` state (section 06 6.7.7).
    pub async fn load_last(&self, state: &AppState) {
        if let Err(e) = store::mark_incomplete(&state.db).await {
            warn!(error = %e.body.message, "could not close an unfinished game mode record");
        }
        match store::latest(&state.db).await {
            Ok(found) => {
                *lock(&self.last) = found.map(|(record, rows)| recap::build(&record, &rows));
            }
            Err(e) => warn!(error = %e.body.message, "could not read the last game mode record"),
        }
    }

    // ---- what the routes call ----

    /// `GET /game-mode`.
    pub fn read(&self) -> GameModeState {
        self.snapshot(&lock(&self.machine))
    }

    /// `POST /game-mode`: starts game mode, or returns the run that is under way.
    pub async fn start(self: &Arc<Self>, body: StartGameModeBody) -> ApiResult<GameModeState> {
        let app = self.app()?;
        if app.shutdown.shutting_down() {
            return Err(closing());
        }
        let _one_at_a_time = self.start_lock.lock().await;

        match self.phase() {
            GameModePhase::Off => {}
            GameModePhase::Done => return Err(refusal_error(Refusal::Stopping)),
            // Under way: that run is returned and the body is ignored.
            _ => return Ok(self.read()),
        }

        // Validate before anything changes: the wind-down would otherwise end Rain and then fail
        // to start the game.
        let title = match body.then_launch_game_id.as_deref() {
            None => None,
            Some(id) => {
                let game = self.neighbours.game(id).ok_or_else(|| {
                    ApiError::not_found("Game not found")
                        .with_source("games")
                        .with_reason("unknownGame")
                })?;
                if !game.installed {
                    return Err(ApiError::conflict(
                        "notInstalled",
                        "That game is not installed.",
                    ));
                }
                Some(game.title)
            }
        };
        *lock(&self.launch_title) = title;

        // Order of section 06 6.7.2: the flag and the pause first, then measure (so a run that
        // starts in between cannot be missed), then choose review or countdown.
        app.set_game_mode_active(true);
        self.neighbours.jobs().pause(true);
        self.neighbours.idle().pause(true);
        // A scan with several threads must not run through the review and the countdown: that
        // is what game mode frees.
        self.neighbours.cancel_scan();

        let measured = self.measure();
        let countdown_seconds = app.settings().resolved().countdown_seconds;
        let input = Input::Start {
            now: Instant::now(),
            countdown_seconds,
            items: measured,
            then_launch: body.then_launch_game_id,
        };
        let state = match self.run_input(&app, input) {
            Ok(state) => state,
            Err(refusal) => {
                // Cannot happen while `start_lock` is held; undo what was done above anyway.
                self.resume(&app);
                return Err(refusal_error(refusal));
            }
        };
        info!(phase = ?state.phase, "game mode started");

        if self.driver_running.swap(true, Ordering::SeqCst) {
            // A driver from the run before has not seen its end yet: it measures again.
            self.wake.notify_one();
        } else {
            spawn_named("gamemode-driver", Arc::clone(self).drive(app));
        }
        Ok(state)
    }

    /// `POST /game-mode/choice`.
    pub fn choose(self: &Arc<Self>, choice: GameModeChoice) -> ApiResult<GameModeState> {
        let app = self.app()?;
        let input = Input::Choice {
            choice,
            now: Instant::now(),
        };
        let state = self.run_input(&app, input).map_err(refusal_error)?;
        // The driver computes its next sleep again (a choice can end the wait or the countdown).
        self.wake.notify_one();
        Ok(state)
    }

    /// `POST /game-mode/recap/seen`: the notice was shown, so it is not shown again.
    pub async fn mark_recap_seen(self: &Arc<Self>) -> ApiResult<()> {
        let app = self.app()?;
        let at = iso_from_system_time(app.clock.now());
        store::mark_seen(&app.db, at).await?;
        let changed = lock(&self.last).as_mut().is_some_and(|last| {
            let was_unseen = !last.seen;
            last.seen = true;
            was_unseen
        });
        // Only the `off` state carries the recap, so only then has the screen something to learn.
        if changed && self.phase() == GameModePhase::Off {
            // `Refresh` always succeeds.
            let _ = self.run_input(&app, Input::Refresh);
        }
        Ok(())
    }

    // ---- the machine and its effects ----

    fn phase(&self) -> GameModePhase {
        lock(&self.machine).phase()
    }

    fn app(&self) -> ApiResult<AppState> {
        self.state.upgrade().ok_or_else(closing)
    }

    /// The state for the screen: the machine's, plus the recap while `off`.
    fn snapshot(&self, machine: &Machine) -> GameModeState {
        let mut state = machine.state();
        if state.phase == GameModePhase::Off {
            state.last = lock(&self.last).clone();
        }
        state
    }

    /// The five rows with their numbers as they are now.
    fn measure(&self) -> Vec<rain_types::WindDownItem> {
        items::measure(
            self.neighbours.open_user_sessions(),
            self.neighbours.jobs().counts(),
            self.neighbours.idle().counts(),
        )
    }

    /// Gives the machine one input and does what it asks. The lock is never held across an
    /// `await`, so this is a plain function. It does NOT wake the driver: the driver calls it
    /// itself, and the callers that come from outside call `wake` after it.
    fn run_input(self: &Arc<Self>, app: &AppState, input: Input) -> Result<GameModeState, Refusal> {
        let mut machine = lock(&self.machine);
        let effects = machine.step(input)?;
        let snapshot = self.snapshot(&machine);
        let mut wind_down = false;
        for effect in effects {
            match effect {
                // `done` is the last frame the backend publishes: it replaces any waiting one.
                Effect::Publish if snapshot.phase == GameModePhase::Done => {
                    self.throttle.publish_final(&snapshot);
                }
                Effect::Publish => self.throttle.publish(&snapshot),
                Effect::Cancelled => {
                    // In this order (section 06 6.7.2): the flag, then the work, then the frame.
                    // The scheduler and the idle sampler check the flag when they resume.
                    self.resume(app);
                    self.throttle.publish_final(&snapshot);
                }
                Effect::StartWindDown => wind_down = true,
            }
        }
        drop(machine);

        if wind_down {
            let busy = app.activity.guard();
            spawn_named(
                "gamemode-wind-down",
                Arc::clone(self).wind_down(app.clone(), busy),
            );
        }
        Ok(snapshot)
    }

    /// Game mode is over without the wind-down: clear the flag, then let the jobs run again.
    fn resume(&self, app: &AppState) {
        app.set_game_mode_active(false);
        self.neighbours.jobs().pause(false);
        self.neighbours.idle().pause(false);
    }

    // ---- the driver ----

    /// Runs from `start` until game mode is cancelled or the wind-down begins.
    async fn drive(self: Arc<Self>, app: AppState) {
        let mut jobs_running = self.neighbours.jobs().running_watch();
        let mut idle_running = self.neighbours.idle().running_watch();
        let token = app.shutdown.token().clone();
        loop {
            let due = lock(&self.machine).next_wake();
            let mut timer = false;
            tokio::select! {
                () = self.wake.notified() => {}
                () = sleep_until_or_never(due) => timer = true,
                Ok(()) = jobs_running.changed() => {}
                Ok(()) = idle_running.changed() => {}
                () = token.cancelled() => {
                    self.driver_running.store(false, Ordering::SeqCst);
                    return;
                }
            }
            let now = Instant::now();
            // Running work and open sessions may have changed; this is an in-memory read.
            let _ = self.run_input(
                &app,
                Input::Items {
                    items: self.measure(),
                    now,
                },
            );
            if timer {
                let _ = self.run_input(&app, Input::Tick { now });
            }
            // The decision to leave is taken under the machine lock, so that a `start` that
            // comes right now either sees the flag still set (and this loop goes on) or finds it
            // cleared (and starts a new driver).
            let machine = lock(&self.machine);
            if !matches!(
                machine.phase(),
                GameModePhase::Review | GameModePhase::Countdown
            ) {
                self.driver_running.store(false, Ordering::SeqCst);
                return;
            }
        }
    }

    // ---- the wind-down ----

    /// Section 06 6.7.5. Each stop is independent: a failure is recorded and the next step runs.
    async fn wind_down(self: Arc<Self>, app: AppState, _busy: ActivityGuard) {
        let first = lock(&self.machine).state();
        // A std lock must not live across an `await`, so the title is copied out first.
        let then_launch_title = lock(&self.launch_title).clone();
        let record = NewRecord {
            started_at: iso_from_system_time(app.clock.now()),
            countdown_seconds: first.countdown_seconds,
            then_launch_game_id: first.then_launch_game_id.clone(),
            then_launch_title,
        };
        let mut keeper = Keeper::begin(&app, record).await;

        // 1. Sessions: nothing is stopped. They are only counted, for the recap.
        let sessions = self.neighbours.open_user_sessions();
        if sessions > 0 {
            keeper
                .add(
                    WindDownKind::Session,
                    items::label(WindDownKind::Session),
                    RecapOutcome::LeftRunning,
                    sessions,
                )
                .await;
        }

        // 2 and 3. The jobs Rain started.
        let jobs = self.neighbours.jobs();
        let idle = self.neighbours.idle();
        let mut interrupted = false;
        for (row, kind, port) in [
            (items::JOBS, WindDownKind::Job, jobs),
            (items::IDLE_JOBS, WindDownKind::IdleJob, idle),
        ] {
            if app.shutdown.shutting_down() {
                return self.abandon_wind_down();
            }
            interrupted |= self.stop_group(&app, &mut keeper, row, kind, port).await;
        }

        // 4. The game, outside Rain's process tree.
        if app.shutdown.shutting_down() {
            return self.abandon_wind_down();
        }
        let launched = match first.then_launch_game_id.as_deref() {
            Some(id) => Some(self.launch(&app, id).await),
            None => None,
        };

        // 5. The background service: the record is closed first, so it is whole when the row says
        // "stopped".
        if app.shutdown.shutting_down() {
            return self.abandon_wind_down();
        }
        self.set_status(&app, items::SERVICE, WindDownStatus::Stopping);
        let waited = lock(&self.machine).waited();
        let finish = Finish {
            finished_at: iso_from_system_time(app.clock.now()),
            waited,
            forced: interrupted,
            launched: launched.as_ref().map(Result::is_ok),
            launched_in_job: matches!(launched, Some(Ok(true))),
            launch_error: launched.and_then(Result::err),
        };
        keeper.finish(finish).await;
        tokio::time::sleep(self.timings.item_min_visible).await;
        self.set_status(&app, items::SERVICE, WindDownStatus::Stopped);

        // 6. The Rain window: the shell closes it a few seconds after the pipe is gone.
        self.set_status(&app, items::WINDOW, WindDownStatus::Stopping);
        tokio::time::sleep(self.timings.item_min_visible).await;
        self.set_status(&app, items::WINDOW, WindDownStatus::Stopped);

        // 7. Done. `done` is published with `publish_final` (inside `run_input`), so it is the last
        // topic frame; the bus is frozen so that nothing can follow it; then Rain ends.
        let _ = self.run_input(&app, Input::Finished);
        app.bus.freeze();
        // No pause is needed here. `done` is already in the buffer of every open stream (the bus
        // publishes synchronously), and a stream that is told to stop writes what is in its
        // buffer first and then the lifecycle frame (`server/stream.rs`, `live_events`).
        info!("game mode finished, Rain is closing");
        app.shutdown.request(StopReason::GameMode);
    }

    /// Something else is already stopping the backend: leave the record `running`, which the
    /// next start reports as "did not finish".
    fn abandon_wind_down(&self) {
        warn!("the backend is stopping for another reason; the game mode wind-down ends here");
    }

    /// Stops every run of one group through its port. Returns true when something was stopped.
    async fn stop_group(
        self: &Arc<Self>,
        app: &AppState,
        keeper: &mut Keeper,
        row: &'static str,
        kind: WindDownKind,
        port: &dyn WindDownPort,
    ) -> bool {
        self.set_status(app, row, WindDownStatus::Stopping);
        let budget = self.timings.item_budget;
        // The port gives each run `budget`. The whole step is cut at twice that, so a port that
        // hangs cannot hold up the wind-down: the process exit closes the job handles, and
        // KILL_ON_JOB_CLOSE ends what is left.
        let stop = tokio::time::timeout(budget * 2, port.stop_all(budget));
        let (result, ()) = tokio::join!(stop, tokio::time::sleep(self.timings.item_min_visible));
        let stopped = result.unwrap_or_else(|_| {
            warn!(row, "a stop took longer than its budget");
            vec![crate::core::winddown::Stopped {
                name: items::label(kind).to_owned(),
                outcome: StopOutcome::Failed,
            }]
        });
        info!(row, stopped = stopped.len(), "wind-down step done");
        for run in &stopped {
            let outcome = match run.outcome {
                StopOutcome::Stopped => RecapOutcome::Stopped,
                StopOutcome::Forced => RecapOutcome::Forced,
                StopOutcome::Failed => RecapOutcome::Failed,
            };
            keeper.add(kind, &run.name, outcome, 1).await;
        }
        self.set_status(app, row, WindDownStatus::Stopped);
        !stopped.is_empty()
    }

    /// Starts the game. `Ok(true)`: it started inside a job of Rain and may close with Rain.
    /// `Err` holds the message for the user (also put on the state).
    async fn launch(self: &Arc<Self>, app: &AppState, id: &str) -> Result<bool, String> {
        let call =
            tokio::time::timeout(self.timings.launch_budget, self.neighbours.launch_game(id));
        let result = match call.await {
            Ok(Ok(in_job)) => {
                if in_job {
                    warn!("the game was started inside a job and may close with Rain");
                }
                Ok(in_job)
            }
            Ok(Err(message)) => Err(message),
            Err(_) => Err("The game did not start in time.".to_owned()),
        };
        if let Err(message) = &result {
            warn!("the game did not start after game mode");
            let _ = self.run_input(app, Input::LaunchError(message.clone()));
        }
        result
    }

    fn set_status(self: &Arc<Self>, app: &AppState, id: &'static str, status: WindDownStatus) {
        let _ = self.run_input(app, Input::Status { id, status });
    }
}

/// Writes the record of one wind-down. A database problem is logged and never stops the
/// wind-down: the user asked for game mode, and the record is only for the recap.
struct Keeper {
    db: crate::store::db::Db,
    id: Option<i64>,
    seq: u32,
}

impl Keeper {
    async fn begin(app: &AppState, record: NewRecord) -> Self {
        let id = match store::begin_record(&app.db, record).await {
            Ok(id) => Some(id),
            Err(e) => {
                warn!(error = %e.body.message, "game mode could not start its record");
                None
            }
        };
        Self {
            db: app.db.clone(),
            id,
            seq: 0,
        }
    }

    async fn add(&mut self, kind: WindDownKind, name: &str, outcome: RecapOutcome, count: u32) {
        let Some(id) = self.id else { return };
        let row = ItemRow {
            kind,
            name: name.to_owned(),
            outcome,
            count,
        };
        if let Err(e) = store::add_item(&self.db, id, self.seq, row).await {
            warn!(error = %e.body.message, "game mode could not record a step");
        }
        self.seq += 1;
    }

    async fn finish(&self, finish: Finish) {
        let Some(id) = self.id else { return };
        if let Err(e) = store::finish_record(&self.db, id, finish).await {
            warn!(error = %e.body.message, "game mode could not close its record");
        }
    }
}

async fn sleep_until_or_never(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

fn closing() -> ApiError {
    ApiError::unavailable("gamemode", "Rain is closing.").with_reason("shuttingDown")
}

fn refusal_error(refusal: Refusal) -> ApiError {
    match refusal {
        Refusal::NotRunning => ApiError::conflict("notRunning", "Game mode is not running."),
        Refusal::WindingDown => ApiError::conflict("windingDown", "Rain is already closing."),
        Refusal::Stopping => ApiError::conflict("stopping", "Rain is closing."),
    }
}
