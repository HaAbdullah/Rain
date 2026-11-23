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
use rain_backend::modules::gamemode::{Neighbours, Timings};
use rain_types::{Frame, GameModePhase, GameModeState, WindDownEffect, WindDownStatus};
use serde_json::json;

fn fast() -> Timings {
    Timings {
        wait_limit: Duration::from_millis(300),
        review_abandon: Duration::from_secs(300),
        item_min_visible: Duration::from_millis(10),
        item_budget: Duration::from_millis(200),
        launch_budget: Duration::from_millis(300),
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
    let jobs = &started.items[1];
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
        done.items[1].status,
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
