//! Game mode through the real API, with fakes in place of every other module.
//! Section 06 10.3 (`tests/gamemode.rs`).
//!
//! The other modules are replaced by `FakeNeighbours` (two `FakeWindDown` ports, a session
//! counter, a list of games). Nothing here starts, stops or kills a real process. The timings
//! are short (tens of milliseconds) instead of the 180 s and 10 minutes of the product, so the
//! tests use the real clock and never pause it: the pipe needs real time.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::fakes::{FakeNeighbours, FakeWindDown, LaunchBehaviour};
use common::{PATIENCE, TestBackend};
use hyper::StatusCode;
use rain_backend::modules::gamemode::{CloseTimings, Neighbours, Timings};
use rain_types::{Frame, GameModePhase, GameModeState, WindDownEffect, WindDownStatus};
use serde_json::json;

fn fast() -> Timings {
    Timings {
        wait_limit: Duration::from_millis(300),
        review_abandon: Duration::from_secs(300),
        item_min_visible: Duration::from_millis(10),
        item_budget: Duration::from_millis(200),
        launch_budget: Duration::from_millis(300),
        closing: CloseTimings {
            polite_wait: Duration::from_millis(100),
            hidden_wait: Duration::from_millis(50),
            poll: Duration::from_millis(5),
        },
    }
}

async fn backend(fake: &Arc<FakeNeighbours>, timings: Timings) -> TestBackend {
    TestBackend::builder()
        // The countdown is 5 seconds, the shortest the setting allows.
        .file(
            "settings.toml",
            "schema_version = 1\ngame_mode_countdown_seconds = 5\n",
        )
        .with::<dyn Neighbours>(Arc::clone(fake) as Arc<dyn Neighbours>)
        .with::<Timings>(Arc::new(timings))
        .start()
        .await
}

fn fake() -> Arc<FakeNeighbours> {
    Arc::new(FakeNeighbours::default())
}

async fn state(backend: &TestBackend) -> GameModeState {
    backend.get("/game-mode").await
}

async fn start(backend: &TestBackend) -> GameModeState {
    let (status, body) = backend.send("POST", "/game-mode", Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).expect("a game mode state")
}

async fn choose(backend: &TestBackend, choice: &str) -> (StatusCode, serde_json::Value) {
    backend
        .send(
            "POST",
            "/game-mode/choice",
            Some(json!({ "choice": choice })),
        )
        .await
}

/// Polls the state until `done` holds, for at most `PATIENCE`.
async fn wait_for(
    backend: &TestBackend,
    what: &str,
    done: impl Fn(&GameModeState) -> bool,
) -> GameModeState {
    let waiting = async {
        loop {
            let now = state(backend).await;
            if done(&now) {
                return now;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(PATIENCE, waiting)
        .await
        .unwrap_or_else(|_| panic!("game mode never reached: {what}"))
}

fn journal(fake: &FakeNeighbours) -> Vec<String> {
    fake.journal.lock().expect("journal").clone()
}

/// One job runs, the user stops it anyway, closes now, and the run ends with the process.
/// Returns every event of the stream, which ends when the backend closes it.
async fn full_run(
    backend: &mut TestBackend,
    fake: &FakeNeighbours,
    game: Option<&str>,
) -> Vec<(String, Frame)> {
    fake.jobs.set_running(1);
    *fake.jobs.stops.lock().expect("stops") = vec![FakeWindDown::stopped("Demo wait")];
    let mut stream = backend.stream().await;
    let body = game.map_or(json!({}), |id| json!({ "thenLaunchGameId": id }));
    let (status, _) = backend.send("POST", "/game-mode", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state(backend).await.phase, GameModePhase::Review);
    assert_eq!(choose(backend, "stopAnyway").await.0, StatusCode::OK);
    assert_eq!(choose(backend, "closeNow").await.0, StatusCode::OK);
    stream.rest().await
}

fn game_mode_states(events: &[(String, Frame)]) -> Vec<GameModeState> {
    events
        .iter()
        .filter(|(_, frame)| frame.topic == "gameMode")
        .map(|(_, frame)| serde_json::from_value(frame.payload.clone()).expect("a state"))
        .collect()
}

/// The countdown seconds published so far, read from the bus without any I/O (so the paused
/// clock cannot jump while a test waits for a pipe).
fn seconds_published(
    frames: &mut tokio::sync::broadcast::Receiver<Arc<rain_backend::core::bus::FrameBytes>>,
) -> Vec<u32> {
    let mut seen = Vec::new();
    while let Ok(frame) = frames.try_recv() {
        if frame.topic == "gameMode" {
            let frame: Frame = serde_json::from_str(&frame.json).expect("a frame");
            let state: GameModeState = serde_json::from_value(frame.payload).expect("a state");
            seen.push(state.seconds_left);
        }
    }
    seen
}

/// Lets every task that is ready run, without moving the clock.
async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

/// The driver sleeps between the seconds: it does not measure in a loop (a regression test for
/// a driver that woke itself with every input and never slept), and each second is published
/// when it passes. The test calls the service directly and pauses the clock, so no pipe waits on
/// real time; this needs a single-threaded runtime.
#[tokio::test]
async fn the_driver_sleeps_and_publishes_one_frame_each_second() {
    let fake = fake();
    let b = backend(&fake, fast()).await;
    let mut frames = b.state.bus.subscribe();
    tokio::time::pause();
    let service = b.state.gamemode();
    let started = service.start(Default::default()).await.expect("start");
    assert_eq!(started.seconds_left, 5);
    settle().await;
    let after_start = fake.measures.load(Ordering::SeqCst);
    assert_eq!(seconds_published(&mut frames), [5], "the start frame");

    // Half a second in: nothing is due, so nothing is published.
    tokio::time::advance(Duration::from_millis(500)).await;
    settle().await;
    assert!(seconds_published(&mut frames).is_empty());

    // Each second that passes gives exactly one frame, when it passes, and none between. The
    // timer wheel of tokio rounds a deadline up to the next millisecond, hence the small slack.
    let slack = Duration::from_millis(5);
    let mut passed = Duration::from_millis(500);
    for expected in [4u32, 3, 2] {
        let reached = Duration::from_secs(u64::from(5 - expected)) + slack;
        tokio::time::advance(reached - passed).await;
        settle().await;
        assert_eq!(seconds_published(&mut frames), [expected]);
        tokio::time::advance(Duration::from_millis(900)).await;
        settle().await;
        assert!(
            seconds_published(&mut frames).is_empty(),
            "no frame before the next second"
        );
        passed = reached + Duration::from_millis(900);
    }

    // One measure for each wake of the driver (a few), never a loop.
    let measured = fake.measures.load(Ordering::SeqCst) - after_start;
    assert!(
        measured <= 10,
        "the driver measured {measured} times in 3.5 s of countdown"
    );
    service
        .choose(rain_types::GameModeChoice::Cancel)
        .expect("cancel");
    // The pipe and the stop need real time.
    tokio::time::resume();
    b.stop().await;
}

// ---- start ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_with_nothing_running_goes_to_countdown() {
    let fake = fake();
    let b = backend(&fake, fast()).await;
    let started = start(&b).await;
    assert_eq!(started.phase, GameModePhase::Countdown);
    assert_eq!(
        started.countdown_seconds, 5,
        "the length comes from Settings"
    );
    assert_eq!(started.seconds_left, 5);
    assert!(b.state.game_mode_active());
    assert_eq!(*fake.jobs.calls.lock().unwrap(), ["pause(true)"]);
    assert_eq!(*fake.idle.calls.lock().unwrap(), ["pause(true)"]);
    assert_eq!(
        fake.scans_cancelled.load(Ordering::SeqCst),
        1,
        "a running scan is cancelled"
    );
    assert_eq!(choose(&b, "cancel").await.0, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_countdown_counts_down_by_itself() {
    let fake = fake();
    let b = backend(&fake, fast()).await;
    start(&b).await;
    let later = wait_for(&b, "a second has passed", |s| s.seconds_left < 5).await;
    assert!(later.seconds_left <= 4);
    choose(&b, "cancel").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_with_a_running_job_goes_to_review() {
    let fake = fake();
    fake.jobs.set_running(1);
    fake.sessions.store(2, Ordering::SeqCst);
    let b = backend(&fake, fast()).await;
    let started = start(&b).await;
    assert_eq!(started.phase, GameModePhase::Review);
    let jobs = &started.items[2];
    assert_eq!(jobs.effect, WindDownEffect::Interrupts);
    let sessions = &started.items[0];
    assert_eq!(sessions.count, 2);
    assert_eq!(sessions.effect, WindDownEffect::LeftRunning);
    assert_eq!(sessions.status, WindDownStatus::Pending);
    choose(&b, "cancel").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_start_returns_the_run_that_is_under_way() {
    let fake = fake();
    let b = backend(&fake, fast()).await;
    let first = start(&b).await;
    let (status, body) = b
        .send(
            "POST",
            "/game-mode",
            Some(json!({ "thenLaunchGameId": "ignored" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the body is ignored, so an unknown game is no error"
    );
    let second: GameModeState = serde_json::from_value(body).unwrap();
    assert_eq!(second.rev, first.rev);
    assert_eq!(second.then_launch_game_id, None);
    choose(&b, "cancel").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_game_404_and_not_installed_409() {
    let fake = Arc::new(FakeNeighbours::default().with_game("game-b", "Game B", false));
    let b = backend(&fake, fast()).await;

    let (status, body) = b
        .send(
            "POST",
            "/game-mode",
            Some(json!({ "thenLaunchGameId": "nope" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["reason"], "unknownGame");
    assert_eq!(body["source"], "games");

    let (status, body) = b
        .send(
            "POST",
            "/game-mode",
            Some(json!({ "thenLaunchGameId": "game-b" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "notInstalled");

    // Neither changed anything: no flag, no pause, no state.
    assert_eq!(state(&b).await.phase, GameModePhase::Off);
    assert!(!b.state.game_mode_active());
    assert!(journal(&fake).is_empty());
}

// ---- review, wait, abandon ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_ends_when_the_run_ends() {
    let fake = fake();
    fake.jobs.set_running(1);
    let b = backend(
        &fake,
        Timings {
            wait_limit: Duration::from_secs(60),
            ..fast()
        },
    )
    .await;
    start(&b).await;
    let (_, body) = choose(&b, "wait").await;
    assert_eq!(body["waiting"], true);

    fake.jobs.set_running(0);
    let now = wait_for(&b, "the countdown", |s| s.phase == GameModePhase::Countdown).await;
    assert!(!now.waiting);
    choose(&b, "cancel").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_times_out_and_review_says_so() {
    let fake = fake();
    fake.jobs.set_running(1);
    let b = backend(&fake, fast()).await;
    start(&b).await;
    choose(&b, "wait").await;
    let now = wait_for(&b, "the wait limit", |s| !s.waiting).await;
    assert_eq!(now.phase, GameModePhase::Review);
    assert_eq!(now.wait_timed_out, Some(true));
    choose(&b, "cancel").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_abandon_cancels_and_resumes_the_work() {
    let fake = fake();
    fake.jobs.set_running(1);
    let b = backend(
        &fake,
        Timings {
            review_abandon: Duration::from_millis(300),
            ..fast()
        },
    )
    .await;
    start(&b).await;
    wait_for(&b, "the abandon limit", |s| s.phase == GameModePhase::Off).await;
    assert!(!b.state.game_mode_active());
    assert_eq!(
        *fake.jobs.calls.lock().unwrap(),
        ["pause(true)", "pause(false)"]
    );
}

// ---- cancel ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_clears_the_flag_before_it_resumes_the_work() {
    let fake = Arc::new(FakeNeighbours::default().with_game("game-a", "Game A", true));
    let b = backend(&fake, fast()).await;
    for port in [&fake.jobs, &fake.idle] {
        let state = b.state.clone();
        *port.probe.lock().unwrap() = Some(Box::new(move || state.game_mode_active()));
    }
    let mut stream = b.stream().await;
    let body = json!({ "thenLaunchGameId": "game-a" });
    let (status, _) = b.send("POST", "/game-mode", Some(body)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = choose(&b, "cancel").await;
    assert_eq!(status, StatusCode::OK);
    let cancelled: GameModeState = serde_json::from_value(body).unwrap();
    assert_eq!(cancelled.phase, GameModePhase::Off);
    assert_eq!(cancelled.then_launch_game_id, None, "the game is forgotten");
    assert!(!b.state.game_mode_active());

    // Order of section 06 6.7.2: the flag is cleared first, then the work resumes.
    assert_eq!(
        journal(&fake),
        [
            "jobs:pause(true) active=true",
            "idle:pause(true) active=true",
            "scan:cancel",
            "jobs:pause(false) active=false",
            "idle:pause(false) active=false",
        ]
        .map(String::from)
    );

    // The last frame of a cancelled run is its final state.
    let mut last = None;
    while let Some(event) = tokio::time::timeout(Duration::from_millis(300), stream.next_event())
        .await
        .ok()
        .flatten()
    {
        if event.1.topic == "gameMode" {
            last = Some(event.1);
        }
    }
    let last: GameModeState = serde_json::from_value(last.expect("a frame").payload).unwrap();
    assert_eq!(last.phase, GameModePhase::Off);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_and_idle_jobs_are_paused_from_start_to_cancel() {
    let fake = fake();
    let b = backend(&fake, fast()).await;
    start(&b).await;
    assert!(b.state.game_mode_active());
    assert_eq!(
        journal(&fake),
        ["jobs:pause(true)", "idle:pause(true)", "scan:cancel"].map(String::from)
    );
    choose(&b, "cancel").await;
    assert_eq!(
        &journal(&fake)[3..],
        ["jobs:pause(false)", "idle:pause(false)"].map(String::from)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_after_the_wind_down_began_is_409() {
    let fake = fake();
    *fake.jobs.stop_delay.lock().unwrap() = Duration::from_millis(150);
    fake.jobs.set_running(1);
    let mut b = backend(
        &fake,
        Timings {
            item_budget: Duration::from_secs(2),
            ..fast()
        },
    )
    .await;
    start(&b).await;
    choose(&b, "stopAnyway").await;
    choose(&b, "closeNow").await;

    let (status, body) = choose(&b, "cancel").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "windingDown");
    assert_eq!(body["message"], "Rain is already closing.");
    b.wait_exited().await;
}

// ---- choices ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn choices_are_idempotent_by_the_table() {
    let fake = fake();
    let b = backend(&fake, fast()).await;

    // off: every choice but cancel is "not running"; cancel is a no-op.
    for choice in ["wait", "stopAnyway", "closeNow"] {
        let (status, body) = choose(&b, choice).await;
        assert_eq!(status, StatusCode::CONFLICT, "{choice}");
        assert_eq!(body["reason"], "notRunning");
    }
    assert_eq!(choose(&b, "cancel").await.0, StatusCode::OK);

    // countdown: wait and stopAnyway change nothing.
    let started = start(&b).await;
    for choice in ["wait", "stopAnyway"] {
        let (status, body) = choose(&b, choice).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["phase"], "countdown");
        assert_eq!(body["rev"], started.rev, "{choice} changed nothing");
    }
    choose(&b, "cancel").await;

    // review: closeNow changes nothing; wait twice is one wait.
    fake.jobs.set_running(1);
    let started = start(&b).await;
    let (_, body) = choose(&b, "closeNow").await;
    assert_eq!(body["rev"], started.rev);
    let (_, first) = choose(&b, "wait").await;
    let (_, second) = choose(&b, "wait").await;
    assert_eq!(first["rev"], second["rev"]);
    choose(&b, "cancel").await;

    // a body that is not a choice
    let (status, body) = b
        .send(
            "POST",
            "/game-mode/choice",
            Some(json!({ "choice": "later" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid");
}

// ---- the wind-down ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_anyway_then_close_now_runs_the_stops_in_order() {
    let fake = Arc::new(FakeNeighbours::default().with_game("game-a", "Game A", true));
    fake.sessions.store(3, Ordering::SeqCst);
    let mut b = backend(&fake, fast()).await;
    full_run(&mut b, &fake, Some("game-a")).await;
    b.wait_exited().await;

    // The ports are paused, the scan cancelled, jobs and idle jobs stopped, then the game started.
    // Nothing else happened: no `pause(false)`, and no call that touches a session (there is
    // no such method).
    assert_eq!(
        journal(&fake),
        [
            "jobs:pause(true)",
            "idle:pause(true)",
            "scan:cancel",
            "jobs:stop_all",
            "idle:stop_all",
            "launch:game-a",
        ]
        .map(String::from)
    );
    assert_eq!(
        fake.sessions.load(Ordering::SeqCst),
        3,
        "the sessions are untouched"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn done_is_the_last_topic_frame_and_the_stream_then_ends_with_the_lifecycle_tail() {
    let fake = fake();
    let mut b = backend(&fake, fast()).await;
    let events = full_run(&mut b, &fake, None).await;
    b.wait_exited().await;

    let last_topic = events
        .iter()
        .rposition(|(_, frame)| frame.topic == "gameMode")
        .expect("a game mode frame");
    let done: GameModeState = serde_json::from_value(events[last_topic].1.payload.clone()).unwrap();
    assert_eq!(done.phase, GameModePhase::Done);
    assert!(done.items.iter().all(|item| {
        item.effect == WindDownEffect::LeftRunning || item.status == WindDownStatus::Stopped
    }));

    // After `done` only the lifecycle tail comes, and then the stream ends.
    let after: Vec<_> = events[last_topic + 1..].iter().map(|(_, f)| f).collect();
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(after[0].topic, "lifecycle");
    assert_eq!(
        after[0].payload,
        json!({ "kind": "stopping", "reason": "gameMode" })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rev_only_grows_across_a_full_run() {
    let fake = fake();
    // Slow steps, so that the throttle (4 frames a second) lets several frames through.
    let slow = Timings {
        item_min_visible: Duration::from_millis(300),
        ..fast()
    };
    let mut b = backend(&fake, slow).await;
    let events = full_run(&mut b, &fake, None).await;
    b.wait_exited().await;
    let revs: Vec<u64> = game_mode_states(&events).iter().map(|s| s.rev).collect();
    assert!(revs.len() >= 3, "{revs:?}");
    assert!(revs.windows(2).all(|pair| pair[0] < pair[1]), "{revs:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rows_go_through_stopping_to_stopped() {
    let fake = fake();
    // Slow steps, so that the throttle (4 frames a second) lets several frames through.
    let slow = Timings {
        item_min_visible: Duration::from_millis(300),
        ..fast()
    };
    let mut b = backend(&fake, slow).await;
    let events = full_run(&mut b, &fake, None).await;
    b.wait_exited().await;
    let states = game_mode_states(&events);
    let saw_stopping = states
        .iter()
        .any(|s| s.items.iter().any(|i| i.status == WindDownStatus::Stopping));
    assert!(
        saw_stopping,
        "a row was seen stopping (the frames may be throttled, but not all)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_failure_sets_launch_error_and_still_exits() {
    let fake = Arc::new(FakeNeighbours::default().with_game("game-a", "Game A", true));
    *fake.launch.lock().unwrap() = LaunchBehaviour::Fails("The game could not be started.".into());
    let mut b = backend(&fake, fast()).await;
    let events = full_run(&mut b, &fake, Some("game-a")).await;

    let done = game_mode_states(&events).pop().expect("a final state");
    assert_eq!(done.phase, GameModePhase::Done);
    assert_eq!(
        done.launch_error.as_deref(),
        Some("The game could not be started.")
    );

    // The record keeps it, and the next start shows it.
    b.restart_following_machine_file().await;
    let off = state(&b).await;
    let last = off.last.expect("a recap");
    assert_eq!(
        last.launch_error.as_deref(),
        Some("The game could not be started.")
    );
    assert_eq!(last.launched_title.as_deref(), Some("Game A"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_launch_that_never_answers_ends_at_its_budget() {
    let fake = Arc::new(FakeNeighbours::default().with_game("game-a", "Game A", true));
    *fake.launch.lock().unwrap() = LaunchBehaviour::Hangs;
    let mut b = backend(&fake, fast()).await;
    let events = full_run(&mut b, &fake, Some("game-a")).await;
    b.wait_exited().await;
    let done = game_mode_states(&events).pop().expect("a final state");
    assert_eq!(
        done.launch_error.as_deref(),
        Some("The game did not start in time.")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_over_budget_is_recorded_failed_and_the_wind_down_continues() {
    let fake = fake();
    // The stop would take 3 s; its budget is 200 ms, so the step is cut at 400 ms.
    *fake.jobs.stop_delay.lock().unwrap() = Duration::from_secs(3);
    let mut b = backend(&fake, fast()).await;
    let events = full_run(&mut b, &fake, None).await;

    let done = game_mode_states(&events).pop().expect("a final state");
    assert_eq!(done.phase, GameModePhase::Done, "the wind-down went on");
    assert_eq!(
        done.items[2].status,
        WindDownStatus::Stopped,
        "the row still reached stopped"
    );

    b.restart_following_machine_file().await;
    let last = state(&b).await.last.expect("a recap");
    let jobs = last
        .items
        .iter()
        .find(|item| item.label == "Scheduled jobs")
        .expect("a jobs line");
    assert_eq!(jobs.outcome, rain_types::RecapOutcome::Failed);
}

// ---- the record and the recap ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn record_and_recap_after_restart() {
    let fake = fake();
    fake.sessions.store(3, Ordering::SeqCst);
    let mut b = backend(&fake, fast()).await;
    full_run(&mut b, &fake, None).await;

    b.restart_following_machine_file().await;
    let off = state(&b).await;
    assert_eq!(off.phase, GameModePhase::Off);
    let last = off.last.expect("the last game mode is reported");
    assert!(last.complete);
    assert!(!last.seen);
    let sessions = last
        .items
        .iter()
        .find(|item| item.kind == rain_types::WindDownKind::Session)
        .expect("a sessions line");
    assert_eq!(sessions.count, 3);
    assert_eq!(sessions.outcome, rain_types::RecapOutcome::LeftRunning);
    assert!(sessions.names.is_empty(), "a session never carries a name");
    let jobs = last
        .items
        .iter()
        .find(|item| item.kind == rain_types::WindDownKind::Job)
        .expect("a jobs line");
    assert_eq!(jobs.names, ["Demo wait"]);

    let (status, _) = b.send("POST", "/game-mode/recap/seen", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state(&b).await.last.expect("still reported").seen);
    // And it stays seen for the next start.
    b.restart().await;
    assert!(state(&b).await.last.expect("still reported").seen);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_record_becomes_incomplete() {
    let fake = fake();
    let mut b = backend(&fake, fast()).await;
    // A backend that died during the wind-down leaves a record that is still `running`.
    b.state
        .db
        .call(|conn| {
            conn.execute(
                "INSERT INTO gamemode_record (started_at, outcome, countdown_seconds)
                 VALUES ('2026-11-22T10:00:00.000Z', 'running', 5)",
                [],
            )
        })
        .await
        .expect("a running record");
    b.restart().await;
    let last = state(&b).await.last.expect("a recap");
    assert!(!last.complete, "\"Game mode did not finish.\"");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marking_a_recap_seen_when_there_is_none_is_harmless() {
    let fake = fake();
    let b = backend(&fake, fast()).await;
    let (status, _) = b.send("POST", "/game-mode/recap/seen", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state(&b).await.last.is_none());
}

// ---- the other programs of the PC (section 06 6.11) ----

use common::fakes::FakePc;
use rain_backend::modules::gamemode::Pc;
use rain_types::{OtherApps, RecapOutcome, WindDownKind};

/// A PC with a few programs on it: two that close when asked, one that hides in the tray, one
/// that asks the user something, and a game launcher that is never touched.
fn pc_with_programs() -> Arc<FakePc> {
    let pc = FakePc::default();
    pc.add_process(500, 1, "Discord.exe", r"C:\Apps\Discord.exe", 600_000_000);
    pc.add_window(1, 500, true, true);
    pc.add_process(510, 500, "Discord.exe", r"C:\Apps\Discord.exe", 200_000_000);
    pc.add_process(501, 1, "Spotify.exe", r"C:\Apps\Spotify.exe", 300_000_000);
    pc.add_window(2, 501, true, true);
    pc.add_process(502, 1, "Updater.exe", r"C:\Apps\Updater.exe", 50_000_000);
    pc.add_window(3, 502, false, false);
    pc.add_process(503, 1, "Writer.exe", r"C:\Apps\Writer.exe", 100_000_000);
    pc.add_window(4, 503, true, false);
    pc.add_process(600, 1, "steam.exe", r"C:\Steam\steam.exe", 90_000_000);
    pc.add_window(5, 600, true, true);
    Arc::new(pc)
}

async fn backend_with_pc(fake: &Arc<FakeNeighbours>, pc: &Arc<FakePc>) -> TestBackend {
    TestBackend::builder()
        .file(
            "settings.toml",
            "schema_version = 1\ngame_mode_countdown_seconds = 5\n",
        )
        .with::<dyn Neighbours>(Arc::clone(fake) as Arc<dyn Neighbours>)
        .with::<dyn Pc>(Arc::clone(pc) as Arc<dyn Pc>)
        .with::<Timings>(Arc::new(fast()))
        .start()
        .await
}

async fn apps_of(backend: &TestBackend) -> OtherApps {
    let state: GameModeState = backend.get("/game-mode?otherApps=true").await;
    state.other_apps.expect("the list was asked for")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_list_is_only_made_when_the_screen_asks_and_closes_nothing() {
    let (fake, pc) = (fake(), pc_with_programs());
    let b = backend_with_pc(&fake, &pc).await;

    // A plain read and a start-up never look at the processes.
    let plain = state(&b).await;
    assert!(plain.other_apps.is_none());
    assert!(pc.journal().is_empty(), "no scan yet: {:?}", pc.journal());

    let list = apps_of(&b).await;
    assert!(list.enabled);
    let ids: Vec<&str> = list.apps.iter().map(|app| app.id.as_str()).collect();
    assert_eq!(
        ids,
        ["discord.exe", "spotify.exe", "writer.exe", "updater.exe"],
        "biggest memory first, and the launcher is not listed"
    );
    assert_eq!(list.apps[0].processes, 2);
    assert_eq!(list.apps[0].memory_bytes, 800_000_000);
    assert!(list.apps[0].has_window);
    assert!(!list.apps[3].has_window, "a hidden window is no window");
    assert!(list.apps.iter().all(|app| app.close));
    assert_eq!(
        list.freed_bytes,
        800_000_000 + 300_000_000 + 100_000_000 + 50_000_000
    );
    assert_eq!(list.always_kept, 1, "the launcher");
    assert_eq!(pc.journal(), ["scan"], "one scan, nothing asked or ended");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unticking_a_program_keeps_it_in_the_file_and_the_switch_turns_the_closing_off() {
    let (fake, pc) = (fake(), pc_with_programs());
    let b = backend_with_pc(&fake, &pc).await;

    let (status, body) = b
        .send(
            "PATCH",
            "/game-mode/other-apps",
            Some(json!({ "app": { "id": "discord.exe", "close": false } })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: OtherApps = serde_json::from_value(body).expect("a list");
    let discord = list.apps.iter().find(|a| a.id == "discord.exe").unwrap();
    assert!(!discord.close, "listed, but unticked");
    assert_eq!(list.freed_bytes, 300_000_000 + 100_000_000 + 50_000_000);

    let text = std::fs::read_to_string(b.data_dir.join("gamemode.toml")).expect("the file");
    assert!(text.contains("keep = ['discord.exe']"), "{text}");
    assert!(text.contains("# Game mode:"), "the comments stay: {text}");

    // Ticking it again takes it out.
    let (status, _) = b
        .send(
            "PATCH",
            "/game-mode/other-apps",
            Some(json!({ "app": { "id": "discord.exe", "close": true } })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let text = std::fs::read_to_string(b.data_dir.join("gamemode.toml")).unwrap();
    assert!(text.contains("keep = []"), "{text}");

    // The master switch.
    let (_, body) = b
        .send(
            "PATCH",
            "/game-mode/other-apps",
            Some(json!({ "enabled": false })),
        )
        .await;
    let list: OtherApps = serde_json::from_value(body).unwrap();
    assert!(!list.enabled);
    assert_eq!(list.freed_bytes, 0);
    assert!(
        std::fs::read_to_string(b.data_dir.join("gamemode.toml"))
            .unwrap()
            .contains("close_other_apps = false")
    );
    let (status, _) = b
        .send("PATCH", "/game-mode/other-apps", Some(json!({ "nope": 1 })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_of_the_keep_list_reaches_the_list() {
    let (fake, pc) = (fake(), pc_with_programs());
    let b = backend_with_pc(&fake, &pc).await;
    let _ = apps_of(&b).await;
    let path = b.data_dir.join("gamemode.toml");
    std::fs::write(&path, "schema_version = 1\nkeep = ['SPOTIFY']\n").unwrap();
    let waiting = async {
        loop {
            let list = apps_of(&b).await;
            if list.apps.iter().any(|a| a.id == "spotify.exe" && !a.close) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(PATIENCE, waiting)
        .await
        .expect("the edited file was read");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_run_closes_the_other_programs_in_order_and_records_each() {
    let (fake, pc) = (fake(), pc_with_programs());
    let mut b = backend_with_pc(&fake, &pc).await;
    let (status, body) = b
        .send(
            "PATCH",
            "/game-mode/other-apps",
            Some(json!({ "app": { "id": "spotify.exe", "close": false } })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut stream = b.stream().await;
    let started = start(&b).await;
    assert_eq!(
        started.phase,
        GameModePhase::Countdown,
        "nothing interrupts"
    );
    let row = &started.items[1];
    assert_eq!(row.kind, WindDownKind::OtherApp);
    assert_eq!(row.count, 3, "discord, writer and updater");
    assert_eq!(row.effect, WindDownEffect::Safe);
    // Nothing is closed before the countdown ends: the PC was only looked at.
    assert!(
        pc.journal().iter().all(|e| e == "scan"),
        "{:?}",
        pc.journal()
    );

    assert_eq!(choose(&b, "closeNow").await.0, StatusCode::OK);
    let events = stream.rest().await;

    let states = game_mode_states(&events);
    let done = states.last().expect("a final state");
    assert_eq!(done.phase, GameModePhase::Done);
    assert_eq!(done.items[1].status, WindDownStatus::Stopped);
    assert_eq!(
        done.items[1].progress, None,
        "the count goes with the stopping"
    );

    // Politely first: every visible window asked, then the end of what has none. The launcher
    // and the program that was unticked are never asked or ended.
    let journal = pc.journal();
    let first_end = journal
        .iter()
        .position(|e| e.starts_with("terminate"))
        .unwrap();
    assert!(
        journal[..first_end]
            .iter()
            .all(|e| e == "scan" || e.starts_with("close")),
        "{journal:?}"
    );
    assert!(journal.contains(&"close:500".to_owned()));
    assert!(
        journal.contains(&"close:503".to_owned()),
        "asked, though it will not close"
    );
    assert!(
        journal.contains(&"close:502".to_owned()),
        "its hidden window was asked"
    );
    assert!(journal.contains(&"terminate:502".to_owned()));
    for untouched in ["500", "503", "501", "600"] {
        assert!(
            !journal.contains(&format!("terminate:{untouched}")),
            "{untouched} must not be ended: {journal:?}"
        );
    }
    assert!(!journal.contains(&"close:501".to_owned()));
    assert!(!journal.contains(&"close:600".to_owned()));
    assert!(pc.is_dead(500) && pc.is_dead(502));
    assert!(!pc.is_dead(503) && !pc.is_dead(501) && !pc.is_dead(600));

    b.restart_following_machine_file().await;
    let last = state(&b).await.last.expect("a recap");
    let line = |outcome: RecapOutcome| {
        last.items
            .iter()
            .find(|i| i.kind == WindDownKind::OtherApp && i.outcome == outcome)
            .map(|i| i.names.clone())
    };
    assert_eq!(
        line(RecapOutcome::Stopped),
        Some(vec!["Discord".to_owned()])
    );
    assert_eq!(line(RecapOutcome::Forced), Some(vec!["Updater".to_owned()]));
    assert_eq!(
        line(RecapOutcome::LeftRunning),
        Some(vec!["Writer".to_owned()])
    );
    assert_eq!(line(RecapOutcome::Failed), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_program_is_a_failed_line_and_is_not_tried_again() {
    let (fake, pc) = (fake(), pc_with_programs());
    pc.world.lock().unwrap().refuse.insert(502);
    let mut b = backend_with_pc(&fake, &pc).await;
    let mut stream = b.stream().await;
    start(&b).await;
    assert_eq!(choose(&b, "closeNow").await.0, StatusCode::OK);
    stream.rest().await;
    let ends = pc
        .journal()
        .iter()
        .filter(|e| *e == "terminate:502")
        .count();
    assert_eq!(ends, 1, "one attempt");
    b.restart_following_machine_file().await;
    let last = state(&b).await.last.expect("a recap");
    let failed = last
        .items
        .iter()
        .find(|i| i.kind == WindDownKind::OtherApp && i.outcome == RecapOutcome::Failed)
        .expect("a failed line");
    assert_eq!(failed.names, ["Updater"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_during_the_countdown_closes_nothing() {
    let (fake, pc) = (fake(), pc_with_programs());
    let b = backend_with_pc(&fake, &pc).await;
    start(&b).await;
    assert_eq!(choose(&b, "cancel").await.0, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(pc.journal(), ["scan"], "only the look at the start");
    assert_eq!(state(&b).await.phase, GameModePhase::Off);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_the_switch_off_game_mode_behaves_as_before() {
    let (fake, pc) = (fake(), pc_with_programs());
    let mut b = backend_with_pc(&fake, &pc).await;
    let (status, _) = b
        .send(
            "PATCH",
            "/game-mode/other-apps",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let before = pc.journal().len();
    let mut stream = b.stream().await;
    let started = start(&b).await;
    assert_eq!(started.items[1].count, 0);
    assert_eq!(choose(&b, "closeNow").await.0, StatusCode::OK);
    stream.rest().await;
    b.wait_exited().await;
    let after = pc.journal();
    assert!(
        after[before..].iter().all(|e| e == "scan")
            && !after[before..].contains(&"close:500".into()),
        "nothing was asked or ended: {after:?}"
    );
    assert!(!pc.is_dead(500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_that_starts_keeps_its_own_program() {
    let fake = Arc::new(FakeNeighbours::default().with_game("game-a", "Game A", true));
    {
        let mut games = fake.games.lock().unwrap();
        games[0].1.exe_path = Some(r"D:\Games\A\A.exe".to_owned());
        games[0].1.install_path = Some(r"D:\Games\A".to_owned());
    }
    let pc = pc_with_programs();
    pc.add_process(700, 1, "A.exe", r"D:\Games\A\A.exe", 4_000_000_000);
    pc.add_process(
        701,
        1,
        "crashreporter.exe",
        r"D:\Games\A\bin\crashreporter.exe",
        10_000_000,
    );
    let mut b = backend_with_pc(&fake, &pc).await;
    let mut stream = b.stream().await;
    let (status, _) = b
        .send(
            "POST",
            "/game-mode",
            Some(json!({ "thenLaunchGameId": "game-a" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(choose(&b, "closeNow").await.0, StatusCode::OK);
    stream.rest().await;
    b.wait_exited().await;
    assert!(
        !pc.is_dead(700) && !pc.is_dead(701),
        "the game and its folder stay"
    );
    assert!(pc.is_dead(500), "the others were closed first");
    let journal = journal(&fake);
    assert!(journal.contains(&"launch:game-a".to_owned()), "{journal:?}");
}
