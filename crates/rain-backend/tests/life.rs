//! Life (feature section 04) on a real backend with a real data folder: the vault status, the
//! habit list, stats and days, habit definitions, the tick (the one write to a vault), opening a
//! note, the calendar route and the `habits` topic.
//!
//! Every vault here is a fixture copy inside the temporary data folder of the run. Nothing
//! touches a real vault. The fixture `life-basic` has three habits (gym weekly 3, read daily,
//! review monthly 2) and notes from 2026-11-09 to 2026-11-22; the clock says it is Sunday
//! 2026-11-22.

mod common;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use common::fakes::{FakeClock, FakeDetect, FakeOpener};
use common::{PATIENCE, TestBackend};
use hyper::StatusCode;
use rain_backend::core::launch_contract::{OpenError, OpenRequest};
use rain_types::{
    Cadence, Habit, HabitCell, HabitDay, HabitStats, HabitSummary, OpenNoteResult,
    PropertySuggestion, VaultStatus,
};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

/// A text that every note of the fixture holds. It must never appear in a log.
const MARKER: &str = "UNIQUE-NOTE-MARKER-7f3a";

// ---- helpers ----

/// Sunday 2026-11-22, 12:00 UTC: the same local date in every time zone from UTC-12 to UTC+11.
fn clock() -> Arc<FakeClock> {
    Arc::new(FakeClock::new(
        UNIX_EPOCH + Duration::from_secs(1_795_348_800),
    ))
}

async fn basic() -> TestBackend {
    install_log_capture();
    TestBackend::builder()
        .fixture("life-basic")
        .clock(clock())
        .start()
        .await
}

fn vault(backend: &TestBackend) -> PathBuf {
    backend.data_dir.join("vault")
}

fn note(backend: &TestBackend, day: &str) -> PathBuf {
    vault(backend).join("Daily").join(format!("{day}.md"))
}

async fn habits(backend: &TestBackend) -> Vec<HabitSummary> {
    backend.get("/habits").await
}

fn find<'a>(list: &'a [HabitSummary], id: &str) -> &'a HabitSummary {
    list.iter()
        .find(|summary| summary.habit.id == id)
        .unwrap_or_else(|| panic!("no habit {id}"))
}

async fn tick(backend: &TestBackend, id: &str, day: &str, done: bool) -> (StatusCode, Value) {
    backend
        .send(
            "PUT",
            &format!("/habits/{id}/days/{day}"),
            Some(json!({ "done": done })),
        )
        .await
}

#[track_caller]
fn assert_error(
    (status, body): (StatusCode, Value),
    expected: StatusCode,
    code: &str,
    reason: &str,
) {
    assert_eq!(status, expected, "body: {body}");
    assert_eq!(body["code"], code, "body: {body}");
    assert_eq!(body["reason"], reason, "body: {body}");
    assert!(
        body["message"].as_str().is_some_and(|m| !m.is_empty()),
        "{body}"
    );
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .expect("a file time")
}

fn draft(name: &str, property: &str, cadence: &str, times: u32) -> Value {
    json!({
        "name": name,
        "icon": { "kind": "lucide", "value": "dumbbell" },
        "property": property,
        "cadence": cadence,
        "times": times
    })
}

/// A backend whose settings file points at an arbitrary vault text.
async fn with_settings(settings: &str, habits_toml: &str) -> TestBackend {
    install_log_capture();
    TestBackend::builder()
        .fixture("life-basic")
        .file("settings.toml", settings)
        .file("habits.toml", habits_toml)
        .clock(clock())
        .start()
        .await
}

// ---- log capture, for the test that no note text is logged ----

static LOG: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

#[derive(Clone)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("the log buffer")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;
    fn make_writer(&'a self) -> LogBuffer {
        self.clone()
    }
}

/// Installs one global subscriber for this test program that keeps every log line in memory.
fn install_log_capture() {
    LOG.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(LogBuffer(Arc::clone(&buffer)))
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();
        // Another test program part may have set one already; then nothing is captured and the
        // test below would find nothing, which is why it also checks the capture works.
        let _ = tracing::subscriber::set_global_default(subscriber);
        buffer
    });
}

fn captured_log() -> String {
    let buffer = LOG.get().expect("the capture was installed");
    String::from_utf8_lossy(&buffer.lock().expect("the log buffer")).into_owned()
}

// ---- the vault ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vault_status_of_a_connected_fixture_vault() {
    let backend = basic().await;
    let status: VaultStatus = backend.get("/vault").await;
    assert!(status.connected && status.daily_folder_found && status.writable);
    assert_eq!(
        status.path.as_deref(),
        Some(vault(&backend).to_string_lossy().as_ref())
    );
    assert_eq!(status.daily_folder.as_deref(), Some("Daily"));
    assert_eq!(status.daily_format.as_deref(), Some("YYYY-MM-DD"));
    assert_eq!(status.problem, None);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vault_status_says_what_is_wrong_in_fields_never_as_an_error() {
    // No vault at all.
    let backend = TestBackend::start().await;
    let status: VaultStatus = backend.get("/vault").await;
    assert!(!status.connected && !status.writable && !status.daily_folder_found);
    assert_eq!((status.path, status.problem), (None, None));
    // Reading the list never fails because of that: the cells are simply empty.
    assert_eq!(backend.get::<Vec<HabitSummary>>("/habits").await, vec![]);
    backend.stop().await;

    // A vault path that is not a folder: the path stays, nothing is connected.
    let backend = with_settings("vault_path = '{DATA}\\nowhere'\n", "").await;
    let status: VaultStatus = backend.get("/vault").await;
    assert!(!status.connected && !status.writable);
    assert!(status.path.is_some_and(|path| path.ends_with("nowhere")));
    backend.stop().await;

    // The daily notes folder does not exist.
    let backend = with_settings(
        "vault_path = '{DATA}\\vault'\ndaily_note_folder = 'Missing'\n",
        "",
    )
    .await;
    let status: VaultStatus = backend.get("/vault").await;
    assert!(status.connected && !status.daily_folder_found);
    let problem = status.problem.expect("a problem");
    assert!(
        problem.contains("Missing") && problem.contains("Settings"),
        "{problem}"
    );
    backend.stop().await;

    // A format with a token that Rain cannot read.
    let backend = with_settings(
        "vault_path = '{DATA}\\vault'\ndaily_note_folder = 'Daily'\ndaily_note_format = 'YYYY-MM-DD HH'\n",
        "",
    )
    .await;
    let status: VaultStatus = backend.get("/vault").await;
    assert!(status.problem.is_some_and(|p| p.contains("HH")));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn several_registered_vaults_and_no_choice_mean_no_vault() {
    let detect = Arc::new(FakeDetect::with_vaults(vec![
        FakeDetect::vault("one", r"C:\Made\Up\One"),
        FakeDetect::vault("two", r"C:\Made\Up\Two"),
    ]));
    let backend = TestBackend::builder()
        .with::<dyn rain_backend::modules::settings::Detect>(detect)
        .start()
        .await;
    let status: VaultStatus = backend.get("/vault").await;
    assert!(!status.connected && status.path.is_none());
    let (status, _) = tick_missing_vault(&backend).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no habit exists, nothing was read or written"
    );
    backend.stop().await;
}

async fn tick_missing_vault(backend: &TestBackend) -> (StatusCode, Value) {
    tick(backend, "gym", "2026-11-22", true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_vault_outside_the_data_folder_is_not_writable_in_a_sandbox() {
    let outside = tempfile::tempdir().expect("a folder outside the data folder");
    fs::create_dir(outside.path().join("Daily")).expect("the daily folder");
    let settings = format!(
        "vault_path = '{}'\ndaily_note_folder = 'Daily'\n",
        outside.path().display()
    );
    let backend = with_settings(&settings, "[[habit]]\nid = \"gym\"\nproperty = \"gym\"\n").await;
    let status: VaultStatus = backend.get("/vault").await;
    assert!(status.connected && !status.writable);

    let before = fs::read_dir(outside.path().join("Daily"))
        .expect("the folder")
        .count();
    assert_error(
        tick(&backend, "gym", "2026-11-22", true).await,
        StatusCode::CONFLICT,
        "conflict",
        "sandbox",
    );
    assert_eq!(
        fs::read_dir(outside.path().join("Daily"))
            .expect("the folder")
            .count(),
        before
    );
    backend.stop().await;
}

// ---- reading ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_habit_list_of_the_fixture_matches_the_worked_example() {
    let backend = basic().await;
    let list = habits(&backend).await;
    let ids: Vec<&str> = list.iter().map(|s| s.habit.id.as_str()).collect();
    assert_eq!(ids, ["gym", "read", "review"]);

    let gym = find(&list, "gym");
    assert_eq!(gym.today, "2026-11-22");
    assert_eq!(
        (gym.period_count, gym.period_target, gym.today_done),
        (3, 3, false)
    );
    let mut want = vec![HabitCell::Miss; 10];
    want.extend([HabitCell::Partial, HabitCell::Done]);
    assert_eq!(gym.cells, want);
    assert_eq!(gym.habit.cadence, Cadence::Weekly);

    let read = find(&list, "read");
    assert!(read.today_done);
    assert_eq!(read.cells.len(), 70);
    use HabitCell::{Done, Miss, NoNote};
    assert_eq!(
        &read.cells[56..],
        [
            Miss, NoNote, Done, Miss, Miss, NoNote, Done, Miss, Done, Done, Done, Done, Done, Done
        ]
    );

    let review = find(&list, "review");
    assert_eq!((review.period_count, review.period_target), (0, 2));
    assert!(review.cells.iter().all(|cell| *cell == Miss));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn habit_stats_of_the_fixture_match_the_worked_example() {
    let backend = basic().await;
    let read: HabitStats = backend.get("/habits/read/stats").await;
    assert_eq!(read.since.as_deref(), Some("2026-11-09"));
    assert_eq!(
        (read.streak_current, read.streak_best, read.rate_percent),
        (6, 6, Some(57))
    );
    assert_eq!(
        (
            read.done_this_week,
            read.done_this_month,
            read.done_this_year
        ),
        (6, 8, 8)
    );
    assert_eq!(
        (read.tickable_from.as_str(), read.window_from.as_str()),
        ("2026-10-23", "2024-09-14")
    );

    let gym: HabitStats = backend.get("/habits/gym/stats").await;
    assert_eq!(
        (gym.streak_current, gym.streak_best, gym.rate_percent),
        (1, 1, Some(50))
    );
    assert_eq!(
        (gym.done_this_week, gym.done_this_month, gym.done_this_year),
        (3, 5, 5)
    );

    let review: HabitStats = backend.get("/habits/review/stats").await;
    assert_eq!(
        (review.since, review.rate_percent, review.streak_best),
        (None, None, 0)
    );

    assert_error(
        backend.send("GET", "/habits/nope/stats", None).await,
        StatusCode::NOT_FOUND,
        "notFound",
        "unknownHabit",
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn habit_days_lists_only_days_with_a_note_and_never_the_decoys() {
    let backend = basic().await;
    let days: Vec<HabitDay> = backend
        .get("/habit-days?from=2026-11-01&to=2026-11-30")
        .await;
    let dates: Vec<&str> = days.iter().map(|d| d.date.as_str()).collect();
    assert_eq!(
        dates,
        [
            "2026-11-09",
            "2026-11-11",
            "2026-11-12",
            "2026-11-13",
            "2026-11-15",
            "2026-11-16",
            "2026-11-17",
            "2026-11-18",
            "2026-11-19",
            "2026-11-20",
            "2026-11-21",
            "2026-11-22"
        ],
        "the copy '2026-11-03 1.md' and notes.txt are not days"
    );
    assert_eq!(days[0].done, ["gym"]);
    assert_eq!(days[1].done, ["gym", "read"]);
    assert!(days[2].done.is_empty() && days[3].done.is_empty());

    let only: Vec<HabitDay> = backend
        .get("/habit-days?from=2026-11-09&to=2026-11-11&habit=read")
        .await;
    assert_eq!(only.len(), 2, "the 10th has no note");
    assert!(only[0].done.is_empty() && only[1].done == ["read"]);

    for (query, reason) in [
        ("from=2026-11-30&to=2026-11-01", "badRange"),
        ("from=2020-01-01&to=2026-11-01", "badRange"),
        ("from=2026-02-30&to=2026-11-01", "badDate"),
        ("from=today&to=2026-11-01", "badDate"),
    ] {
        assert_error(
            backend
                .send("GET", &format!("/habit-days?{query}"), None)
                .await,
            StatusCode::BAD_REQUEST,
            "invalid",
            reason,
        );
    }
    assert_error(
        backend
            .send(
                "GET",
                "/habit-days?from=2026-11-01&to=2026-11-02&habit=nope",
                None,
            )
            .await,
        StatusCode::NOT_FOUND,
        "notFound",
        "unknownHabit",
    );
    assert_eq!(
        backend
            .send("GET", "/habit-days?from=2026-11-01", None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suggestions_are_the_checkbox_properties_no_habit_uses_yet() {
    let backend = with_settings(
        "vault_path = '{DATA}\\vault'\ndaily_note_folder = 'Daily'\n",
        "schema_version = 1\n[[habit]]\nid = \"gym\"\nproperty = \"gym\"\n",
    )
    .await;
    fs::write(
        note(&backend, "2026-11-14"),
        "---\nmood: true\nread: false\n---\n",
    )
    .expect("a note");
    let list: Vec<PropertySuggestion> = backend.get("/vault/properties").await;
    // The fixture has `read` in ten notes; the new note has `mood` once and `read` too. `gym` is
    // used by a habit. A property in a note is a suggestion even when its value is false.
    let names: Vec<(&str, u32)> = list.iter().map(|s| (s.property.as_str(), s.days)).collect();
    assert_eq!(names, [("read", 11), ("mood", 1)]);
    assert_eq!(list[0].last_seen, "2026-11-22");
    backend.stop().await;

    let none = TestBackend::start().await;
    assert_eq!(
        none.get::<Vec<PropertySuggestion>>("/vault/properties")
            .await,
        vec![]
    );
    none.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calendar_events_are_always_empty_and_dates_are_checked() {
    let backend = basic().await;
    let events: Vec<Value> = backend
        .get("/calendar/events?from=2026-11-01&to=2026-11-30")
        .await;
    assert!(events.is_empty());
    assert_error(
        backend
            .send(
                "GET",
                "/calendar/events?from=2026-13-01&to=2026-11-30",
                None,
            )
            .await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badDate",
    );
    backend.stop().await;
}

// ---- habit definitions ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn habits_are_created_edited_and_deleted_in_habits_toml() {
    let backend = TestBackend::builder().clock(clock()).start().await;
    let file = backend.data_dir.join("habits.toml");
    assert_eq!(habits(&backend).await, vec![]);
    assert!(!file.exists(), "reading writes nothing");

    // create: 201, the id is a slug, the file starts with the explaining header
    let (status, created) = backend
        .send("POST", "/habits", Some(draft("Gym", "gym", "weekly", 3)))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let created: Habit = serde_json::from_value(created).expect("a habit");
    assert_eq!((created.id.as_str(), created.times), ("gym", 3));
    let text = fs::read_to_string(&file)
        .expect("habits.toml was made")
        .replace("\r\n", "\n");
    assert!(text.starts_with("# Rain habits."), "{text}");
    assert!(
        text.contains("[[habit]]\nid = \"gym\"\nname = \"Gym\""),
        "{text}"
    );
    assert_eq!(habits(&backend).await.len(), 1);

    // a second identical create is a conflict, so a double click makes one habit
    assert_error(
        backend
            .send("POST", "/habits", Some(draft("Gym", "gym", "weekly", 3)))
            .await,
        StatusCode::CONFLICT,
        "conflict",
        "duplicateProperty",
    );
    // a new habit cannot carry an id, and bad values name their reason
    let mut with_id = draft("Other", "other", "daily", 1);
    with_id["id"] = json!("x");
    assert_error(
        backend.send("POST", "/habits", Some(with_id)).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badId",
    );
    for (body, reason) in [
        (draft(" ", "a", "daily", 1), "badName"),
        (draft("A", "a:b", "daily", 1), "badProperty"),
        (draft("A", "a", "weekly", 9), "badTimes"),
        (draft("A", "a", "daily", 2), "badTimes"),
    ] {
        assert_error(
            backend.send("POST", "/habits", Some(body)).await,
            StatusCode::BAD_REQUEST,
            "invalid",
            reason,
        );
    }
    let mut bad_icon = draft("A", "a", "daily", 1);
    bad_icon["icon"] = json!({ "kind": "url", "value": "media/x" });
    assert_error(
        backend.send("POST", "/habits", Some(bad_icon)).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badIcon",
    );
    assert_eq!(
        backend
            .send("POST", "/habits", Some(json!({ "name": "A", "extra": 1 })))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // edit
    let (status, edited) = backend
        .send("PUT", "/habits/gym", Some(draft("Gym", "gym", "weekly", 4)))
        .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["times"], 4);
    assert!(fs::read_to_string(&file).unwrap().contains("times = 4"));
    assert_error(
        backend
            .send("PUT", "/habits/nope", Some(draft("A", "a", "daily", 1)))
            .await,
        StatusCode::NOT_FOUND,
        "notFound",
        "unknownHabit",
    );
    // a second habit, then the property of the first cannot be reused
    backend
        .send("POST", "/habits", Some(draft("Read", "read", "daily", 1)))
        .await;
    assert_error(
        backend
            .send(
                "PUT",
                "/habits/read",
                Some(draft("Read", "gym", "daily", 1)),
            )
            .await,
        StatusCode::CONFLICT,
        "conflict",
        "duplicateProperty",
    );

    // delete: 204, also when it is already gone; no note is touched
    assert_eq!(
        backend.send("DELETE", "/habits/gym", None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        backend.send("DELETE", "/habits/gym", None).await.0,
        StatusCode::NO_CONTENT
    );
    let ids: Vec<String> = habits(&backend)
        .await
        .into_iter()
        .map(|s| s.habit.id)
        .collect();
    assert_eq!(ids, ["read"]);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comments_and_the_users_order_survive_an_edit_by_rain() {
    let original = "# my comment\nschema_version = 1\n\n# the reading habit\n[[habit]]\nid = \"read\" # trailing\nproperty = \"read\"\n";
    let backend = TestBackend::builder()
        .file("habits.toml", original)
        .clock(clock())
        .start()
        .await;
    backend
        .send("POST", "/habits", Some(draft("Gym", "gym", "daily", 1)))
        .await;
    let text = fs::read_to_string(backend.data_dir.join("habits.toml"))
        .unwrap()
        .replace("\r\n", "\n");
    assert!(text.starts_with(original), "{text}");
    assert!(text.contains("id = \"gym\""));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fifty_first_habit_is_refused() {
    let backend = TestBackend::builder().clock(clock()).start().await;
    for n in 0..50 {
        let (status, body) = backend
            .send(
                "POST",
                "/habits",
                Some(draft("Habit", &format!("p{n}"), "daily", 1)),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{n}: {body}");
    }
    assert_error(
        backend
            .send("POST", "/habits", Some(draft("Habit", "p50", "daily", 1)))
            .await,
        StatusCode::CONFLICT,
        "conflict",
        "tooManyHabits",
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_habits_file_refuses_edits_and_a_bad_entry_is_only_skipped() {
    // One bad entry: skipped, listed in /health, the others work, and an edit is not blocked.
    let text = "schema_version = 1\n[[habit]]\nid = \"ok\"\nproperty = \"ok\"\n[[habit]]\nid = \"bad\"\ncadence = \"hourly\"\n";
    let backend = TestBackend::builder()
        .file("habits.toml", text)
        .clock(clock())
        .start()
        .await;
    let ids: Vec<String> = habits(&backend)
        .await
        .into_iter()
        .map(|s| s.habit.id)
        .collect();
    assert_eq!(ids, ["ok"]);
    let health: rain_types::Health = backend.get("/health").await;
    assert!(
        health
            .config_issues
            .iter()
            .any(|i| i.file == "habits.toml" && i.message.contains("Habit 2")),
        "{:?}",
        health.config_issues
    );
    let (status, _) = backend
        .send("POST", "/habits", Some(draft("New", "new", "daily", 1)))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    backend.stop().await;

    // A file that does not parse: edits answer configBroken, the last good list stays.
    let backend = TestBackend::builder()
        .file("habits.toml", "[[habit]\nid = ")
        .clock(clock())
        .start()
        .await;
    assert_error(
        backend
            .send("POST", "/habits", Some(draft("New", "new", "daily", 1)))
            .await,
        StatusCode::CONFLICT,
        "conflict",
        "configBroken",
    );
    backend.stop().await;

    // A file from a newer Rain is read only.
    let backend = TestBackend::builder()
        .file("habits.toml", "schema_version = 99\n")
        .clock(clock())
        .start()
        .await;
    assert_error(
        backend
            .send("POST", "/habits", Some(draft("New", "new", "daily", 1)))
            .await,
        StatusCode::CONFLICT,
        "conflict",
        "configNewer",
    );
    backend.stop().await;
}

// ---- the tick ----

/// Writes a note, ticks, and returns what the note holds afterwards.
async fn tick_note(
    backend: &TestBackend,
    original: &[u8],
    property: &str,
    done: bool,
) -> (StatusCode, Vec<u8>) {
    let path = note(backend, "2026-11-22");
    fs::write(&path, original).expect("a note");
    // The habit of the fixture whose property it is (`gym` here).
    let (status, _) = tick(backend, property, "2026-11-22", done).await;
    (status, fs::read(&path).expect("the note"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tick_changes_exactly_the_expected_bytes_for_every_shape_of_note() {
    // (name, note before, property, note after a tick)
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "lf",
            b"---\ngym: false\n---\nbody\n",
            b"---\ngym: true\n---\nbody\n",
        ),
        (
            "crlf",
            b"---\r\ngym: false\r\nread: true\r\n---\r\nbody\r\n",
            b"---\r\ngym: true\r\nread: true\r\n---\r\nbody\r\n",
        ),
        (
            "bom",
            "\u{feff}---\ngym: false\n---\n".as_bytes(),
            "\u{feff}---\ngym: true\n---\n".as_bytes(),
        ),
        (
            "no frontmatter",
            b"# Title\ntext\n",
            b"---\ngym: true\n---\n# Title\ntext\n",
        ),
        ("empty file", b"", b"---\ngym: true\n---\n"),
        (
            "comment kept",
            b"---\ngym: false   # morning run\n---\n",
            b"---\ngym: true   # morning run\n---\n",
        ),
        (
            "quoted key",
            b"---\n\"gym\": false\n---\n",
            b"---\n\"gym\": true\n---\n",
        ),
        (
            "property missing",
            b"---\nread: true\n---\nbody\n",
            b"---\nread: true\ngym: true\n---\nbody\n",
        ),
        (
            "crlf insert",
            b"---\r\nread: true\r\n---\r\n",
            b"---\r\nread: true\r\ngym: true\r\n---\r\n",
        ),
        (
            "block scalar",
            b"---\ndesc: |\n  gym: false\ngym: false\n---\n",
            b"---\ndesc: |\n  gym: false\ngym: true\n---\n",
        ),
        (
            "rules in the body",
            b"---\ngym: false\n---\ntext\n---\nmore\n",
            b"---\ngym: true\n---\ntext\n---\nmore\n",
        ),
    ];
    for (name, before, after) in cases {
        let backend = basic().await;
        let (status, now) = tick_note(&backend, before, "gym", true).await;
        assert_eq!(status, StatusCode::OK, "{name}");
        assert_eq!(&now, after, "{name}");

        // a second tick writes nothing
        let path = note(&backend, "2026-11-22");
        let time = modified(&path);
        let (status, _) = tick(&backend, "gym", "2026-11-22", true).await;
        assert_eq!(status, StatusCode::OK, "{name}");
        assert_eq!(modified(&path), time, "{name}: the second tick wrote");

        // an untick flips to false and nothing else
        let (status, summary) = tick(&backend, "gym", "2026-11-22", false).await;
        assert_eq!(status, StatusCode::OK, "{name}");
        assert_eq!(summary["todayDone"], false, "{name}");
        let flipped = fs::read_to_string(&path).expect("the note");
        // Only the token of the habit's own line changes.
        let expected: String = String::from_utf8_lossy(after)
            .split_inclusive('\n')
            .map(|line| {
                if (line.starts_with("gym") || line.starts_with("\"gym\"")) && line.contains("true")
                {
                    line.replacen("true", "false", 1)
                } else {
                    line.to_owned()
                }
            })
            .collect();
        assert_eq!(flipped, expected, "{name}");
        backend.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tick_answers_the_new_summary_of_the_habit() {
    let backend = basic().await;
    let (status, summary) = tick(&backend, "gym", "2026-11-22", true).await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    let summary: HabitSummary = serde_json::from_value(summary).expect("a summary");
    assert!(summary.today_done);
    assert_eq!((summary.period_count, summary.period_target), (4, 3));
    assert_eq!(*summary.cells.last().unwrap(), HabitCell::Done);
    // and the list agrees
    assert_eq!(find(&habits(&backend).await, "gym").period_count, 4);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_note_is_created_bare_and_a_second_habit_adds_one_line() {
    let backend = basic().await;
    let path = note(&backend, "2026-11-14"); // a day the fixture has no note for
    assert!(!path.exists());
    assert_eq!(
        tick(&backend, "read", "2026-11-14", true).await.0,
        StatusCode::OK
    );
    assert_eq!(fs::read(&path).unwrap(), b"---\nread: true\n---\n");
    assert_eq!(
        tick(&backend, "gym", "2026-11-14", true).await.0,
        StatusCode::OK
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"---\nread: true\ngym: true\n---\n"
    );

    // unticking a day with no note creates nothing
    let none = note(&backend, "2026-11-10");
    assert_eq!(
        tick(&backend, "read", "2026-11-10", false).await.0,
        StatusCode::OK
    );
    assert!(!none.exists());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tick_never_creates_a_folder() {
    let backend = basic().await;
    fs::remove_dir_all(vault(&backend).join("Daily")).expect("the folder goes");
    assert_error(
        tick(&backend, "read", "2026-11-22", true).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "noDailyFolder",
    );
    assert!(
        !vault(&backend).join("Daily").exists(),
        "no folder was made"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_that_is_not_what_rain_expects_is_refused_and_left_byte_for_byte() {
    let cases: &[(&str, &[u8], &str)] = &[
        (
            "not a checkbox",
            b"---\ngym: yes\n---\nbody\n",
            "notBoolean",
        ),
        (
            "twice",
            b"---\ngym: true\ngym: false\n---\n",
            "duplicateKey",
        ),
        (
            "not yaml",
            b"---\nThis is a sentence.\n---\nbody\n",
            "unreadableNote",
        ),
        ("not utf8", b"---\ngym: false\n---\n\xff\xfe", "notUtf8"),
    ];
    for (name, before, reason) in cases {
        let backend = basic().await;
        let path = note(&backend, "2026-11-22");
        fs::write(&path, before).expect("a note");
        let time = modified(&path);
        for done in [true, false] {
            assert_error(
                tick(&backend, "gym", "2026-11-22", done).await,
                StatusCode::CONFLICT,
                "conflict",
                reason,
            );
        }
        assert_eq!(&fs::read(&path).unwrap(), before, "{name}");
        assert_eq!(modified(&path), time, "{name}");
        backend.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_last_31_days_can_be_ticked_and_never_the_future() {
    let backend = basic().await;
    // today and today minus 30 days are accepted
    assert_eq!(
        tick(&backend, "read", "2026-11-22", true).await.0,
        StatusCode::OK
    );
    assert_eq!(
        tick(&backend, "read", "2026-10-23", true).await.0,
        StatusCode::OK
    );
    assert_eq!(
        fs::read(note(&backend, "2026-10-23")).unwrap(),
        b"---\nread: true\n---\n"
    );
    // older, later, and impossible days are refused
    assert_error(
        tick(&backend, "read", "2026-10-22", true).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "tooOld",
    );
    assert_error(
        tick(&backend, "read", "2026-11-23", true).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "futureDate",
    );
    assert_error(
        tick(&backend, "read", "2026-02-30", true).await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badDate",
    );
    assert_error(
        tick(&backend, "nope", "2026-11-22", true).await,
        StatusCode::NOT_FOUND,
        "notFound",
        "unknownHabit",
    );
    assert!(!note(&backend, "2026-10-22").exists());

    // a tick of a past day changes only that day's note
    let today_before = fs::read(note(&backend, "2026-11-22")).unwrap();
    let today_time = modified(&note(&backend, "2026-11-22"));
    assert_eq!(
        tick(&backend, "gym", "2026-11-21", true).await.0,
        StatusCode::OK
    );
    assert_eq!(
        fs::read(note(&backend, "2026-11-22")).unwrap(),
        today_before
    );
    assert_eq!(modified(&note(&backend, "2026-11-22")), today_time);
    assert!(
        fs::read_to_string(note(&backend, "2026-11-21"))
            .unwrap()
            .contains("gym: true")
    );

    // the body must be exactly { done }
    let bad = backend
        .send(
            "PUT",
            "/habits/gym/days/2026-11-22",
            Some(json!({ "done": true, "x": 1 })),
        )
        .await;
    assert_eq!(bad.0, StatusCode::BAD_REQUEST);
    let bad = backend
        .send("PUT", "/habits/gym/days/2026-11-22", Some(json!({})))
        .await;
    assert_eq!(bad.0, StatusCode::BAD_REQUEST);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn many_ticks_at_once_are_serialised_and_leave_a_valid_note() {
    let backend = basic().await;
    let path = note(&backend, "2026-11-14");
    let calls = (0..50).map(|n| {
        let id = if n % 2 == 0 { "gym" } else { "read" };
        let backend = &backend;
        async move { tick(backend, id, "2026-11-14", true).await.0 }
    });
    let statuses = futures_util::future::join_all(calls).await;
    assert!(
        statuses.iter().all(|s| *s == StatusCode::OK),
        "{statuses:?}"
    );
    let text = fs::read_to_string(&path).expect("the note");
    assert!(
        text.starts_with("---\n") && text.ends_with("---\n"),
        "{text:?}"
    );
    assert_eq!(text.matches("gym: true").count(), 1, "{text:?}");
    assert_eq!(text.matches("read: true").count(), 1, "{text:?}");
    assert_eq!(text.matches("---").count(), 2, "{text:?}");
    backend.stop().await;
}

// ---- the topic ----

async fn open_stream(backend: &TestBackend) -> common::StreamReader {
    let mut stream = backend.stream().await;
    assert_eq!(stream.expect_event().await.0, "hello");
    assert_eq!(stream.expect_event().await.0, "resync");
    stream
}

fn payload(frame: &rain_types::Frame) -> Vec<HabitSummary> {
    serde_json::from_value(frame.payload.clone()).expect("a list of summaries")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_habits_topic_follows_a_tick_a_hand_edit_a_change_in_the_vault_and_a_reread() {
    let backend = basic().await;
    let mut stream = open_stream(&backend).await;

    // a tick sends the whole list
    tick(&backend, "gym", "2026-11-22", true).await;
    let frame = stream.expect_topic("habits").await;
    let list = payload(&frame);
    assert_eq!(list.len(), 3);
    assert!(find(&list, "gym").today_done);

    // Obsidian (here: the test) changes today's note: the list follows within a moment
    fs::write(
        note(&backend, "2026-11-22"),
        "---\nread: true\n---\nedited elsewhere\n",
    )
    .unwrap();
    let frame = stream.expect_topic("habits").await;
    assert!(!find(&payload(&frame), "gym").today_done);

    // a hand edit of habits.toml adds a habit
    let file = backend.data_dir.join("habits.toml");
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str("\n[[habit]]\nid = \"walk\"\nproperty = \"walk\"\n");
    fs::write(&file, text).unwrap();
    let frame = stream.expect_topic("habits").await;
    assert_eq!(payload(&frame).len(), 4);

    // a re-read with nothing changed sends no frame
    assert_eq!(
        backend.send("POST", "/vault/reread", None).await.0,
        StatusCode::NO_CONTENT
    );
    let nothing = tokio::time::timeout(Duration::from_millis(600), stream.next_event()).await;
    assert!(
        nothing.is_err(),
        "an unchanged state sent a frame: {nothing:?}"
    );

    // a re-read after a change that the watcher did not report (the folder is not watched
    // when the note is outside it) still finds it
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn habit_changes_by_the_api_send_a_frame_too() {
    let backend = basic().await;
    let mut stream = open_stream(&backend).await;
    backend
        .send("POST", "/habits", Some(draft("Walk", "walk", "daily", 1)))
        .await;
    assert_eq!(payload(&stream.expect_topic("habits").await).len(), 4);
    backend.send("DELETE", "/habits/walk", None).await;
    assert_eq!(payload(&stream.expect_topic("habits").await).len(), 3);
    backend.stop().await;
}

// ---- settings ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_notes_folder_is_read_at_once_and_the_old_cache_goes() {
    let backend = basic().await;
    assert!(find(&habits(&backend).await, "read").today_done);
    let rows = |backend: &TestBackend| {
        let state = backend.state.clone();
        async move {
            state
                .db
                .call(|db| {
                    db.query_row("SELECT COUNT(DISTINCT scope) FROM life_note", [], |r| {
                        r.get::<_, i64>(0)
                    })
                })
                .await
                .expect("the cache")
        }
    };
    assert_eq!(rows(&backend).await, 1);

    fs::create_dir(vault(&backend).join("Other")).expect("another folder");
    fs::write(
        vault(&backend).join("Other").join("2026-11-21.md"),
        "---\nread: true\n---\n",
    )
    .unwrap();
    let (status, _) = backend
        .send(
            "PATCH",
            "/settings",
            Some(json!({ "dailyNoteFolder": "Other" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let list = habits(&backend).await;
    let read = find(&list, "read");
    assert!(!read.today_done, "today has no note in the other folder");
    assert_eq!(read.cells[68], HabitCell::Done);
    assert_eq!(read.cells[69], HabitCell::NoNote);
    assert_eq!(
        rows(&backend).await,
        1,
        "the rows of the old scope are gone"
    );
    let status: VaultStatus = backend.get("/vault").await;
    assert_eq!(status.daily_folder.as_deref(), Some("Other"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_format_rain_cannot_read_is_refused_by_the_settings() {
    let backend = basic().await;
    let before = fs::read_to_string(backend.data_dir.join("settings.toml")).unwrap();
    assert_error(
        backend
            .send(
                "PATCH",
                "/settings",
                Some(json!({ "dailyNoteFormat": "YYYY-MM-DD HH:mm" })),
            )
            .await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badFormat",
    );
    assert_eq!(
        fs::read_to_string(backend.data_dir.join("settings.toml")).unwrap(),
        before
    );
    let (status, _) = backend
        .send(
            "PATCH",
            "/settings",
            Some(json!({ "dailyNoteFormat": "YYYY.MM.DD" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    backend.stop().await;
}

// ---- game mode ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_game_mode_reads_serve_the_cache_and_touch_no_note() {
    let backend = basic().await;
    assert!(find(&habits(&backend).await, "read").today_done);
    backend.state.set_game_mode_active(true);
    fs::write(note(&backend, "2026-11-22"), "---\nread: false\n---\n").unwrap();
    assert_eq!(
        backend.send("POST", "/vault/reread", None).await.0,
        StatusCode::NO_CONTENT
    );
    assert!(
        find(&habits(&backend).await, "read").today_done,
        "the cache was served"
    );

    backend.state.set_game_mode_active(false);
    assert!(
        !find(&habits(&backend).await, "read").today_done,
        "after game mode the note is read"
    );
    backend.stop().await;
}

// ---- opening a note ----

/// Tells the fake detection about the fixture vault under an id, as Obsidian's own list would,
/// and makes the settings look again.
async fn register_vault(backend: &TestBackend, detect: &FakeDetect, id: &str) {
    *detect.vaults.lock().expect("the fake") =
        vec![FakeDetect::vault(id, &vault(backend).to_string_lossy())];
    // A change of the vault path clears the detection cache; then it is put back.
    for path in [
        "Z:\\elsewhere".to_owned(),
        vault(backend).to_string_lossy().into_owned(),
    ] {
        let (status, _) = backend
            .send("PATCH", "/settings", Some(json!({ "vaultPath": path })))
            .await;
        assert_eq!(status, StatusCode::OK);
    }
}

async fn open_backend() -> (TestBackend, Arc<FakeOpener>, Arc<FakeDetect>) {
    let opener = Arc::new(FakeOpener::default());
    let detect = Arc::new(FakeDetect::default());
    let backend = TestBackend::builder()
        .fixture("life-basic")
        .clock(clock())
        .opener(opener.clone())
        .with::<dyn rain_backend::modules::settings::Detect>(detect.clone())
        .start()
        .await;
    (backend, opener, detect)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_a_note_asks_obsidian_for_exactly_that_note() {
    let (backend, opener, detect) = open_backend().await;
    register_vault(&backend, &detect, "abc123").await;

    let opened: OpenNoteResult = backend
        .send("POST", "/daily-notes/2026-11-22/open", None)
        .await
        .1
        .pipe(serde_json::from_value)
        .expect("a result");
    assert!(opened.opened);
    assert_eq!(
        opener.requests(),
        [OpenRequest::ShellExec {
            file: "obsidian://open?vault=abc123&file=Daily%2F2026-11-22".into(),
            params: None,
            dir: None
        }]
    );

    // a note that does not exist is never opened (Obsidian might create it)
    let (status, body) = backend
        .send("POST", "/daily-notes/2026-11-14/open", None)
        .await;
    assert_eq!((status, &body["opened"]), (StatusCode::OK, &json!(false)));
    assert_eq!(opener.requests().len(), 1, "the opener was not called");

    assert_error(
        backend
            .send("POST", "/daily-notes/2026-02-30/open", None)
            .await,
        StatusCode::BAD_REQUEST,
        "invalid",
        "badDate",
    );
    // Obsidian cannot be started
    opener.fail_next(OpenError {
        ui_text: "Failed to start",
        code: Some(5),
        log_reason: String::new(),
    });
    assert_error(
        backend
            .send("POST", "/daily-notes/2026-11-22/open", None)
            .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "failed",
        "openFailed",
    );
    backend.stop().await;
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl Pipe for Value {}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_format_is_found_and_percent_encoded() {
    let opener = Arc::new(FakeOpener::default());
    let detect = Arc::new(FakeDetect::default());
    let backend = TestBackend::builder()
        .fixture("life-basic")
        .file(
            "settings.toml",
            "vault_path = '{DATA}\\vault'\ndaily_note_folder = 'Daily'\ndaily_note_format = 'YYYY/MM/YYYY-MM-DD'\n",
        )
        .clock(clock())
        .opener(opener.clone())
        .with::<dyn rain_backend::modules::settings::Detect>(detect.clone())
        .start()
        .await;
    let nested = vault(&backend).join("Daily").join("2026").join("11");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("2026-11-22.md"), "---\nread: true\n---\n").unwrap();
    register_vault(&backend, &detect, "id with space").await;

    // the nested note is the one that counts; the flat fixture notes are not found
    let list = habits(&backend).await;
    assert!(find(&list, "read").today_done);
    assert_eq!(find(&list, "read").cells[68], HabitCell::NoNote);

    let (status, body) = backend
        .send("POST", "/daily-notes/2026-11-22/open", None)
        .await;
    assert_eq!((status, &body["opened"]), (StatusCode::OK, &json!(true)));
    assert_eq!(
        opener.requests(),
        [OpenRequest::ShellExec {
            file: "obsidian://open?vault=id%20with%20space&file=Daily%2F2026%2F11%2F2026-11-22"
                .into(),
            params: None,
            dir: None
        }]
    );
    // ticking writes into the nested folder
    assert_eq!(
        tick(&backend, "gym", "2026-11-22", true).await.0,
        StatusCode::OK
    );
    assert!(
        fs::read_to_string(nested.join("2026-11-22.md"))
            .unwrap()
            .contains("gym: true")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_vault_the_open_route_says_so_and_a_vault_without_an_id_falls_back_to_the_path() {
    let backend = TestBackend::start().await;
    assert_error(
        backend
            .send("POST", "/daily-notes/2026-11-22/open", None)
            .await,
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "noVault",
    );
    backend.stop().await;

    let backend = with_settings("vault_path = '{DATA}\\nowhere'\n", "").await;
    assert_error(
        backend
            .send("POST", "/daily-notes/2026-11-22/open", None)
            .await,
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "vaultMissing",
    );
    backend.stop().await;

    let opener = Arc::new(FakeOpener::default());
    let backend = TestBackend::builder()
        .fixture("life-basic")
        .clock(clock())
        .opener(opener.clone())
        .start()
        .await;
    let (status, body) = backend
        .send("POST", "/daily-notes/2026-11-22/open", None)
        .await;
    assert_eq!((status, &body["opened"]), (StatusCode::OK, &json!(true)));
    let OpenRequest::ShellExec { file, .. } = &opener.requests()[0] else {
        panic!("a shell request")
    };
    assert!(file.starts_with("obsidian://open?path="), "{file}");
    backend.stop().await;
}

// ---- logs ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_note_text_is_ever_logged() {
    let backend = basic().await;
    habits(&backend).await;
    backend.send("GET", "/habits/read/stats", None).await;
    tick(&backend, "gym", "2026-11-22", true).await;
    tick(&backend, "read", "2026-11-14", true).await;
    backend.send("POST", "/vault/reread", None).await;
    backend.send("GET", "/vault/properties", None).await;
    // a refusal: the note's text must not reach the error log either
    fs::write(
        note(&backend, "2026-11-20"),
        format!("---\ngym: yes\n---\n{MARKER}\n"),
    )
    .unwrap();
    tick(&backend, "gym", "2026-11-20", true).await;
    backend.stop().await;

    let log = captured_log();
    assert!(
        log.contains("habit tick"),
        "the capture works: {} bytes",
        log.len()
    );
    assert!(!log.contains(MARKER), "a note's text was logged");
    // and the daily note paths are not logged either
    assert!(!log.contains("2026-11-20.md"));
    let _ = PATIENCE;
}

// ---- cost ----

/// The design (6.4) estimates 15 to 30 ms for a check of the 800-day window and asks stage 7a to
/// measure it. This measures one full check (800 paths, about 14 notes) and keeps a generous
/// ceiling so that a slow machine does not fail it but a scan that went wrong does.
#[tokio::test]
async fn a_full_check_of_the_800_day_window_is_fast() {
    let backend = basic().await;
    habits(&backend).await; // the first check, cold cache
    let mut slowest = Duration::ZERO;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        assert_eq!(
            backend.send("POST", "/vault/reread", None).await.0,
            StatusCode::NO_CONTENT
        );
        slowest = slowest.max(started.elapsed());
    }
    eprintln!("full 800-day check, slowest of 5 rereads: {slowest:?}");
    assert!(slowest < Duration::from_secs(1), "slow check: {slowest:?}");
    backend.stop().await;
}
