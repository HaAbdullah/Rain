//! Scheduled jobs on the harness (section 03 10.2). Every program that runs here is a tiny made-up
//! one (`cmd`, `ping`, the examples `fake_claude` and `test_tool`): a test never starts the real
//! `claude`. Time is a `FakeClock` that the test moves; the waits of the module (catch-up delay,
//! "a minute" of a time limit) are shortened through the `Tuning` seam.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::TestBackend;
use common::fakes::FakeClock;
use hyper::StatusCode;
use rain_backend::modules::jobs::{ResumeSignal, Tuning};
use rain_types::time::iso_from_system_time;
use rain_types::{Health, JobKind, LogLevel, LogLine, ScheduledJob, Status};
use serde_json::{Value, json};

// ---- helpers ----

/// A minute boundary, so that `* * * * *` is due exactly at `base() + n minutes`.
fn base() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(60 * (1_900_000_000 / 60))
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Short waits instead of the foundation's: the catch-up after `catch_up_ms`, a "minute" of a
/// time limit lasts `minute_ms`.
fn tuning(catch_up_ms: u64, minute_ms: u64) -> Arc<Tuning> {
    Arc::new(Tuning {
        catch_up_delay: Duration::from_millis(catch_up_ms),
        minute: Duration::from_millis(minute_ms),
        log_flush: Duration::from_millis(20),
        ..Tuning::default()
    })
}

/// One command job as TOML. Strings are literals, so Windows paths need no escaping.
fn job_toml(
    id: &str,
    schedule: &str,
    enabled: bool,
    command: &str,
    args: &[&str],
    extra: &str,
) -> String {
    let args: Vec<String> = args.iter().map(|a| format!("'{a}'")).collect();
    format!(
        "[[job]]\nid = '{id}'\nname = '{id}'\nkind = 'cron'\nschedule = '{schedule}'\nenabled = {enabled}\ncommand = '{command}'\nargs = [{}]\n{extra}\n",
        args.join(", ")
    )
}

async fn jobs(backend: &TestBackend) -> Vec<ScheduledJob> {
    backend.get("/jobs").await
}

/// A `GET /jobs` also makes the scheduler look at the (fake) clock again.
async fn nudge(backend: &TestBackend) {
    backend.send("GET", "/jobs", None).await;
}

async fn job(backend: &TestBackend, id: &str) -> ScheduledJob {
    jobs(backend)
        .await
        .into_iter()
        .find(|job| job.id == id)
        .unwrap_or_else(|| panic!("no job {id}"))
}

/// Polls the list until `done` is true for the job; panics after 15 seconds.
async fn wait_job(
    backend: &TestBackend,
    id: &str,
    done: impl Fn(&ScheduledJob) -> bool,
) -> ScheduledJob {
    let polling = async {
        loop {
            let current = job(backend, id).await;
            if done(&current) {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(15), polling)
        .await
        .unwrap_or_else(|_| panic!("job {id} never reached the state the test waits for"))
}

fn finished(job: &ScheduledJob) -> bool {
    job.recent_runs
        .first()
        .is_some_and(|run| run.status != Status::Running)
}

async fn log(backend: &TestBackend, run_id: &str) -> Vec<LogLine> {
    backend.get(&format!("/runs/{run_id}/log")).await
}

fn texts(lines: &[LogLine]) -> Vec<&str> {
    lines.iter().map(|line| line.text.as_str()).collect()
}

/// The path of a program that  builds from . The folder is found from the
/// test program (cargo may keep test programs apart from the finished files) and from the
/// manifest folder.
fn example(name: &str) -> PathBuf {
    let file = format!("{name}.exe");
    let exe = std::env::current_exe().expect("the test program has a path");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = exe
        .ancestors()
        .map(|folder| folder.join("examples").join(&file))
        .chain(["debug", "release"].map(|profile| {
            manifest
                .join("../../target")
                .join(profile)
                .join("examples")
                .join(&file)
        }));
    candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| panic!("{file} is missing: run  (cargo test does it)"))
}

fn with_clock(
    jobs_toml: &str,
    clock: &Arc<FakeClock>,
    tuning: Arc<Tuning>,
) -> common::TestBackendBuilder {
    TestBackend::builder()
        .file("jobs.toml", jobs_toml)
        .clock(clock.clone())
        .with::<Tuning>(tuning)
}

// ---- the files ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_list_empty_then_commented_file_written() {
    let mut backend = TestBackend::start().await;
    assert!(jobs(&backend).await.is_empty());

    let path = backend.data_dir.join("jobs.toml");
    let written = std::fs::read_to_string(&path).expect("the first start wrote the file");
    assert!(written.starts_with("schema_version = 1"));
    assert!(written.contains("# [[job]]"));
    assert!(!written.contains("\n[[job]]"), "the file has no job");
    assert!(backend.data_dir.join("idle-jobs.toml").exists());

    // A second start does not write it again.
    std::fs::write(&path, format!("{written}\n# my own note\n")).unwrap();
    backend.restart().await;
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .ends_with("# my own note\n")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_typical_folder_lists_its_five_jobs_in_file_order() {
    let backend = TestBackend::with_fixture("typical").await;
    let list = jobs(&backend).await;
    let ids: Vec<_> = list.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "job-echo",
            "job-fail",
            "job-claude",
            "job-weekly-tidy",
            "job-demo-wait"
        ]
    );
    assert_eq!(list[2].kind, JobKind::Claude);
    assert_eq!(
        list[3].schedule_label, "Mondays, nine thirty",
        "the label of the file wins"
    );
    assert_eq!(list[2].schedule_label, "Every day at 06:00");
    assert!(list[0].enabled && !list[1].enabled);
    assert!(
        list[1].next_run_at.is_none(),
        "a disabled job has no next run"
    );
    assert!(list[0].next_run_at.is_some());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_toggle_writes_enabled_keeps_comments() {
    let text = "schema_version = 1\n# my jobs\n\n# first\n[[job]]\nid = 'a'\nname = 'A'\nkind = 'cron'\nschedule = '0 0 1 1 *'\nenabled = false # keep me\ncommand = 'cmd'\n\n# second, no id\n[[job]]\nname = 'Second one'\nkind = 'cron'\nschedule = '0 0 1 1 *'\ncommand = 'cmd'\n";
    let backend = TestBackend::builder().file("jobs.toml", text).start().await;
    let path = backend.data_dir.join("jobs.toml");

    let (status, body) = backend
        .send("PATCH", "/jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], true);
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        text.replace("enabled = false # keep me", "enabled = true # keep me")
    );

    // Idempotent: the same value again changes nothing.
    let (status, _) = backend
        .send("PATCH", "/jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), after);

    // An entry without an id: the derived id is written first, `enabled` at the end of the entry.
    let (status, _) = backend
        .send(
            "PATCH",
            "/jobs/job-second-one",
            Some(json!({ "enabled": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("# second, no id\n[[job]]\nid = 'job-second-one'\nname = 'Second one'"));
    assert!(after.contains("command = 'cmd'\nenabled = true\n"));
    assert!(after.contains("# my jobs") && after.contains("# first"));

    // Switching it back leaves only the id behind.
    backend
        .send("PATCH", "/jobs/a", Some(json!({ "enabled": false })))
        .await;
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("enabled = false # keep me")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_routes_refuse_bad_requests() {
    let backend = TestBackend::builder()
        .file(
            "jobs.toml",
            &job_toml("a", "0 0 1 1 *", false, "cmd", &[], ""),
        )
        .start()
        .await;
    let (status, body) = backend
        .send("PATCH", "/jobs/nobody", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownJob"))
    );
    let (status, _) = backend
        .send("PATCH", "/jobs/a", Some(json!({ "enabled": "yes" })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = backend
        .send(
            "PATCH",
            "/jobs/a",
            Some(json!({ "enabled": true, "name": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = backend.send("POST", "/jobs/nobody/runs", None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownJob"))
    );
    let (status, body) = backend.send("GET", "/runs/run-nobody/log", None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknownRun"))
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_entry_that_moved_while_saving_gives_config_changed_and_a_broken_file_is_never_written()
{
    let backend = TestBackend::builder()
        .file(
            "jobs.toml",
            &job_toml("a", "0 0 1 1 *", false, "cmd", &[], ""),
        )
        .start()
        .await;
    let path = backend.data_dir.join("jobs.toml");

    // The user removes the entry by hand; the write finds the file as it is now.
    std::fs::write(&path, "schema_version = 1\n").unwrap();
    let (status, body) = backend
        .send("PATCH", "/jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("configChanged"))
    );

    // The entry is back (the watcher reloads it); then a file that does not parse is never
    // overwritten.
    let entry = job_toml("a", "0 0 1 1 *", false, "cmd", &[], "");
    std::fs::write(&path, &entry).unwrap();
    wait_for_ids(&backend, &["a"]).await;
    std::fs::write(&path, "[[job\nid = 'a'").unwrap();
    let (status, body) = backend
        .send("PATCH", "/jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "configBroken");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[[job\nid = 'a'");

    // A file from a newer Rain is read only.
    std::fs::write(&path, format!("schema_version = 9\n{entry}")).unwrap();
    let (status, body) = backend
        .send("PATCH", "/jobs/a", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("configNewer"))
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_entry_is_skipped_named_in_health_and_the_others_work() {
    let text = format!(
        "schema_version = 1\n\n{}\n[[job]]\nname = 'Bad schedule'\nkind = 'cron'\nschedule = 'every day'\ncommand = 'cmd'\n\n{}",
        job_toml("good-1", "0 0 1 1 *", false, "cmd", &[], ""),
        job_toml("good-2", "0 0 1 1 *", false, "cmd", &[], "")
    );
    let backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;
    let ids: Vec<_> = jobs(&backend).await.into_iter().map(|j| j.id).collect();
    assert_eq!(ids, ["good-1", "good-2"]);

    let health: Health = backend.get("/health").await;
    let issues: Vec<_> = health
        .config_issues
        .iter()
        .filter(|i| i.file == "jobs.toml")
        .collect();
    assert_eq!(issues.len(), 1, "{health:?}");
    assert!(issues[0].message.contains("Use five fields"));
    assert!(issues[0].message.contains("Bad schedule"));
    let header = text
        .lines()
        .enumerate()
        .filter(|(_, l)| *l == "[[job]]")
        .nth(1)
        .unwrap()
        .0
        + 1;
    assert_eq!(
        issues[0].line,
        Some(header as u32),
        "the line of the [[job]] header"
    );

    // A bad entry does not block a write for a good one, and its text is not touched.
    let (status, _) = backend
        .send("PATCH", "/jobs/good-2", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        std::fs::read_to_string(backend.data_dir.join("jobs.toml"))
            .unwrap()
            .contains("name = 'Bad schedule'")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_edit_reaches_the_list_and_the_stream_and_a_broken_file_keeps_the_last_list() {
    let backend = TestBackend::builder()
        .file(
            "jobs.toml",
            &job_toml("a", "0 0 1 1 *", false, "cmd", &[], ""),
        )
        .start()
        .await;
    let mut stream = backend.stream().await;
    let path = backend.data_dir.join("jobs.toml");

    let edited = format!(
        "{}{}",
        job_toml("a", "0 0 1 1 *", false, "cmd", &[], ""),
        job_toml("b", "0 0 1 1 *", false, "cmd", &[], "")
    );
    std::fs::write(&path, edited).unwrap();
    loop {
        let frame = stream.expect_topic("jobs").await;
        if frame.payload.as_array().is_some_and(|list| list.len() == 2) {
            break;
        }
    }
    assert_eq!(jobs(&backend).await.len(), 2);

    std::fs::write(&path, "[[job").unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(jobs(&backend).await.len(), 2, "the last good list is kept");
    let health: Health = backend.get("/health").await;
    assert!(health.config_issues.iter().any(|i| i.file == "jobs.toml"));
    backend.stop().await;
}

// ---- runs ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_runs_and_logs() {
    let text = format!(
        "{}{}{}",
        job_toml(
            "hello",
            "0 0 1 1 *",
            false,
            "cmd",
            &["/c", "echo hello there"],
            ""
        ),
        job_toml(
            "broken",
            "0 0 1 1 *",
            false,
            "cmd",
            &["/c", "echo oops 1>&2 & exit 3"],
            ""
        ),
        job_toml(
            "slow",
            "0 0 1 1 *",
            false,
            "ping",
            &["-n", "4", "127.0.0.1"],
            ""
        ),
    );
    let backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;
    let mut stream = backend.stream().await;

    // A disabled job may be run by hand. 201 and the run is `running`.
    let (status, run) = backend.send("POST", "/jobs/hello/runs", None).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert_eq!(run["status"], "running");
    assert!(run["id"].as_str().unwrap().starts_with("run-"));

    let done = wait_job(&backend, "hello", finished).await;
    let run = &done.recent_runs[0];
    assert_eq!((run.status, run.exit_code), (Status::Success, Some(0)));
    assert!(run.duration_ms.is_some());
    assert!(!done.running);
    let lines = log(&backend, &run.id).await;
    let all = texts(&lines);
    assert_eq!(all[0], "Started by Run now");
    assert!(all.contains(&"hello there"), "{all:?}");
    assert_eq!(*all.last().unwrap(), "Process exited with code 0");
    assert!(all.iter().any(|t| t.starts_with("Finished in ")));

    // A failing job: exit code 3, an error line, and standard error as a warning.
    backend.send("POST", "/jobs/broken/runs", None).await;
    let done = wait_job(&backend, "broken", finished).await;
    let run = &done.recent_runs[0];
    assert_eq!((run.status, run.exit_code), (Status::Failed, Some(3)));
    let lines = log(&backend, &run.id).await;
    assert!(
        lines
            .iter()
            .any(|l| l.level == LogLevel::Warn && l.text.trim() == "oops")
    );
    let last = lines.last().unwrap();
    assert_eq!(
        (last.level, last.text.as_str()),
        (LogLevel::Error, "Process exited with code 3")
    );

    // A second post while the run goes on returns that run and starts nothing.
    let (status, first) = backend.send("POST", "/jobs/slow/runs", None).await;
    assert_eq!(status, StatusCode::CREATED);
    // The topic shows it running (a run that ends within a quarter of a second would be
    // merged into one frame with its end: the newest state wins).
    loop {
        let frame = stream.expect_topic("jobs").await;
        let slow = &frame.payload[2];
        if slow["running"] == true {
            assert_eq!(slow["recentRuns"][0]["status"], "running");
            break;
        }
    }
    let (status, second) = backend.send("POST", "/jobs/slow/runs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["id"], second["id"]);
    let running = job(&backend, "slow").await;
    assert!(running.running && running.recent_runs.len() == 1);
    // While the run goes on its log can already be read.
    let lines = log(&backend, first["id"].as_str().unwrap()).await;
    assert_eq!(lines[0].text, "Started by Run now");
    wait_job(&backend, "slow", finished).await;
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_run_no_executable() {
    let text = job_toml(
        "ghost",
        "0 0 1 1 *",
        false,
        "rain-no-such-program-77.exe",
        &[],
        "",
    );
    let backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;
    backend.send("POST", "/jobs/ghost/runs", None).await;
    let done = wait_job(&backend, "ghost", finished).await;
    let run = &done.recent_runs[0];
    assert_eq!((run.status, run.exit_code), (Status::Failed, None));
    let lines = log(&backend, &run.id).await;
    assert!(
        lines
            .iter()
            .any(|l| l.level == LogLevel::Error && l.text.starts_with("Could not start: "))
    );

    // A folder that does not exist is a clear failure too.
    let text = job_toml(
        "lost",
        "0 0 1 1 *",
        false,
        "cmd",
        &["/c", "echo x"],
        "working_dir = 'Z:\\no\\such\\folder'",
    );
    std::fs::write(backend.data_dir.join("jobs.toml"), text).unwrap();
    wait_for_ids(&backend, &["lost"]).await;
    backend.send("POST", "/jobs/lost/runs", None).await;
    let done = wait_job(&backend, "lost", finished).await;
    let lines = log(&backend, &done.recent_runs[0].id).await;
    assert!(
        lines
            .iter()
            .any(|l| l.text.contains("working folder") && l.text.contains("does not exist"))
    );
    backend.stop().await;
}

async fn wait_for_ids(backend: &TestBackend, ids: &[&str]) {
    let polling = async {
        loop {
            let now: Vec<String> = jobs(backend).await.into_iter().map(|j| j.id).collect();
            if now == ids {
                return;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), polling)
        .await
        .expect("the list reached the wanted ids");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_timeout() {
    let text = job_toml(
        "long",
        "0 0 1 1 *",
        false,
        "ping",
        &["-n", "60", "127.0.0.1"],
        "timeout_minutes = 1",
    );
    let clock = Arc::new(FakeClock::new(base()));
    // "A minute" lasts 300 ms here.
    let backend = with_clock(&text, &clock, tuning(3_600_000, 300))
        .start()
        .await;
    backend.send("POST", "/jobs/long/runs", None).await;
    let done = wait_job(&backend, "long", finished).await;
    let run = &done.recent_runs[0];
    assert_eq!((run.status, run.exit_code), (Status::Failed, None));
    let lines = log(&backend, &run.id).await;
    assert!(lines.iter().any(
        |l| l.level == LogLevel::Error && l.text == "Stopped after the time limit of 1 minutes"
    ));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_log_caps_and_running_log() {
    // 6000 lines of output: more than the 4000 + 500 that a run keeps.
    let text = job_toml(
        "chatty",
        "0 0 1 1 *",
        false,
        "cmd",
        &["/c", "for /L %i in (1,1,6000) do @echo line %i"],
        "",
    );
    let backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;
    backend.send("POST", "/jobs/chatty/runs", None).await;
    let done = wait_job(&backend, "chatty", finished).await;
    let run = &done.recent_runs[0];
    assert_eq!(run.status, Status::Success);

    // The answer holds the last 2000 lines, led by a warning that says lines are missing.
    let lines = log(&backend, &run.id).await;
    assert_eq!(lines.len(), 2000);
    assert_eq!(lines[0].level, LogLevel::Warn);
    assert!(
        lines[0].text.ends_with("earlier lines are not shown"),
        "{}",
        lines[0].text
    );
    assert!(texts(&lines).contains(&"line 6000"));
    assert_eq!(lines.last().unwrap().text, "Process exited with code 0");

    // The stored rows: head, one marker, tail.
    let stored: (i64, i64) = backend
        .state
        .db
        .call(|c| {
            c.query_row(
                "SELECT log_lines, log_omitted FROM jobs_run ORDER BY started_at DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(stored.0, 4000 + 1 + 500);
    assert!(stored.1 > 1000, "{stored:?}");
    backend.stop().await;
}

// ---- Claude jobs ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_job_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let stream = dir.path().join("stream.jsonl");
    let dump = dir.path().join("dump.json");
    std::fs::write(
        &stream,
        concat!(
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-test-1\",\"model\":\"claude-test\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Working on it.\"},{\"type\":\"tool_use\",\"name\":\"Skill\",\"input\":{}}]}}\n",
            "{\"type\":\"system\",\"subtype\":\"permission_denied\",\"tool_name\":\"Write\"}\n",
            "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":2,\"total_cost_usd\":0.01}\n",
        ),
    )
    .unwrap();
    // A variable of the parent that a Claude job must not inherit.
    // SAFETY: Windows environment calls are thread safe; no other test reads this name.
    unsafe {
        std::env::set_var("CLAUDE_RAIN_TEST_REMOVED", "1");
        std::env::set_var("anthropic_rain_test_removed", "1");
    }
    let fake = example("fake_claude");
    let text = format!(
        "[[job]]\nid = 'ai'\nname = 'Morning notes'\nkind = 'claude'\nschedule = '0 0 1 1 *'\nenabled = false\n\
         skill = 'my-skill'\nprompt = 'Extra words.'\nmodel = 'test-model'\nmax_turns = 7\n\
         allowed_tools = ['Skill', 'Read']\nclaude_program = '{}'\n\
         env = {{ FAKE_CLAUDE_DUMP = '{}', FAKE_CLAUDE_STREAM = '{}' }}\n",
        fake.display(),
        dump.display(),
        stream.display()
    );
    let backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;

    backend.send("POST", "/jobs/ai/runs", None).await;
    let done = wait_job(&backend, "ai", finished).await;
    let run = &done.recent_runs[0];
    assert_eq!((run.status, run.exit_code), (Status::Success, Some(0)));

    let seen: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let args: Vec<&str> = seen["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(
        args,
        [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "test-model",
            "--permission-mode",
            "dontAsk",
            "--allowedTools",
            "Skill,Read",
            "--permission-prompts",
            "none",
            "--max-turns",
            "7",
            "--name",
            "Morning notes"
        ]
    );
    let prompt = seen["stdin"].as_str().unwrap();
    assert!(prompt.starts_with("Run the skill \"my-skill\" now"));
    assert!(prompt.ends_with("\n\nExtra words."));
    let names: Vec<String> = serde_json::from_value(seen["envNames"].clone()).unwrap();
    assert!(names.contains(&"RAIN_RUN_ID".to_owned()) && names.contains(&"RAIN_JOB_ID".to_owned()));
    assert!(
        !names
            .iter()
            .any(|n| n.to_uppercase().contains("RAIN_TEST_REMOVED")),
        "CLAUDE and ANTHROPIC variables must not be passed on: {names:?}"
    );
    assert!(
        seen["cwd"].as_str().unwrap().ends_with("cache\\jobs\\ai"),
        "{}",
        seen["cwd"]
    );

    let lines = log(&backend, &run.id).await;
    let all = texts(&lines);
    assert_eq!(all[0], "Started by Run now");
    assert_eq!(all[1], "Skill my-skill");
    assert!(all.contains(&"Claude session started (model claude-test)"));
    assert!(all.contains(&"Working on it."));
    assert!(all.contains(&"Skill"));
    assert!(
        lines
            .iter()
            .any(|l| l.level == LogLevel::Warn && l.text == "Denied Write")
    );
    assert!(
        all.contains(&"Finished in 0 ms, 2 turns, cost $0.01")
            || all
                .iter()
                .any(|t| t.starts_with("Finished in ") && t.ends_with("2 turns, cost $0.01"))
    );
    assert!(
        !all.iter().any(|t| t.contains("Extra words")),
        "the prompt is never logged"
    );

    let session: Option<String> = backend
        .state
        .db
        .call(|c| c.query_row("SELECT claude_session_id FROM jobs_run", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(session.as_deref(), Some("sess-test-1"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_claude_job_succeeds_only_when_the_result_line_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let fake = example("fake_claude");
    let make = |name: &str, lines: &str, exit: i32| {
        let stream = dir.path().join(format!("{name}.jsonl"));
        std::fs::write(&stream, lines).unwrap();
        format!(
            "[[job]]\nid = '{name}'\nname = '{name}'\nkind = 'claude'\nschedule = '0 0 1 1 *'\nprompt = 'x'\nclaude_program = '{}'\nenv = {{ FAKE_CLAUDE_STREAM = '{}', FAKE_CLAUDE_EXIT = '{exit}' }}\n",
            fake.display(),
            stream.display()
        )
    };
    let text = [
        make("no-result", "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\",\"model\":\"m\"}\n", 0),
        make("error-result", "{\"type\":\"result\",\"subtype\":\"error_max_turns\",\"errors\":[\"Ran out of turns\"]}\n", 0),
        make("bad-exit", "", 2),
    ]
    .join("\n");
    let backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;

    for (id, expected_code, expected_line) in [
        ("no-result", Some(0), "Claude ended without a result"),
        (
            "error-result",
            Some(0),
            "Claude reported an error: error_max_turns",
        ),
        ("bad-exit", Some(2), "Process exited with code 2"),
    ] {
        backend
            .send("POST", &format!("/jobs/{id}/runs"), None)
            .await;
        let done = wait_job(&backend, id, finished).await;
        let run = &done.recent_runs[0];
        assert_eq!(
            (run.status, run.exit_code),
            (Status::Failed, expected_code),
            "{id}"
        );
        let lines = log(&backend, &run.id).await;
        assert!(
            lines
                .iter()
                .any(|l| l.level == LogLevel::Error && l.text == expected_line),
            "{id}: {lines:?}"
        );
    }
    backend.stop().await;
}

// ---- the scheduler ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_fires_due_job() {
    let text = format!(
        "{}{}{}",
        job_toml("one", "* * * * *", true, "cmd", &["/c", "echo one"], ""),
        job_toml("two", "* * * * *", true, "cmd", &["/c", "echo two"], ""),
        job_toml("off", "* * * * *", false, "cmd", &["/c", "echo off"], ""),
    );
    let clock = Arc::new(FakeClock::new(base() + secs(10)));
    let backend = with_clock(&text, &clock, tuning(3_600_000, 60_000))
        .start()
        .await;

    // Nothing is due yet; the next run of an enabled job is the next minute, a disabled one has none.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let list = jobs(&backend).await;
    assert!(list.iter().all(|j| j.recent_runs.is_empty()));
    assert_eq!(
        list[0].next_run_at,
        Some(iso_from_system_time(base() + secs(60)))
    );
    assert_eq!(list[2].next_run_at, None);

    // The clock passes the minute: both due jobs run once, the disabled one does not.
    clock.set(base() + secs(61));
    let one = wait_job(&backend, "one", finished).await;
    let two = wait_job(&backend, "two", finished).await;
    assert_eq!((one.recent_runs.len(), two.recent_runs.len()), (1, 1));
    assert_eq!(one.recent_runs[0].status, Status::Success);
    assert_eq!(
        one.next_run_at,
        Some(iso_from_system_time(base() + secs(120))),
        "nextRunAt moved"
    );
    let lines = log(&backend, &one.recent_runs[0].id).await;
    assert_eq!(lines[0].text, "Started by schedule");

    // The same minute again does not run it twice.
    clock.set(base() + secs(62));
    nudge(&backend).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(job(&backend, "one").await.recent_runs.len(), 1);
    assert!(job(&backend, "off").await.recent_runs.is_empty());

    // The next minute runs it again.
    clock.set(base() + secs(121));
    nudge(&backend).await;
    wait_job(&backend, "one", |j| j.recent_runs.len() == 2 && finished(j)).await;
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_skips_overlap() {
    let text = job_toml(
        "slow",
        "* * * * *",
        true,
        "ping",
        &["-n", "30", "127.0.0.1"],
        "",
    );
    let clock = Arc::new(FakeClock::new(base() + secs(10)));
    let backend = with_clock(&text, &clock, tuning(3_600_000, 60_000))
        .start()
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    clock.set(base() + secs(61));
    wait_job(&backend, "slow", |j| j.running).await;

    // The next occurrence falls while the run goes on: no second run, no record of a skip.
    clock.set(base() + secs(121));
    nudge(&backend).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let now = job(&backend, "slow").await;
    assert_eq!(now.recent_runs.len(), 1);
    assert!(now.running);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_catch_up_on_start() {
    let text = format!(
        "{}{}",
        job_toml("missed", "* * * * *", true, "cmd", &["/c", "echo m"], ""),
        job_toml(
            "optout",
            "* * * * *",
            true,
            "cmd",
            &["/c", "echo o"],
            "catch_up = false"
        ),
    );
    let clock = Arc::new(FakeClock::new(base() + secs(10)));
    let mut backend = with_clock(&text, &clock, tuning(400, 60_000)).start().await;
    tokio::time::sleep(Duration::from_millis(700)).await; // the first catch-up found nothing
    assert!(
        jobs(&backend)
            .await
            .iter()
            .all(|j| j.recent_runs.is_empty())
    );

    // Rain is off for ten minutes.
    clock.set(base() + secs(10 * 60 + 10));
    backend.restart().await;
    let done = wait_job(&backend, "missed", |j| {
        !j.recent_runs.is_empty() && finished(j)
    })
    .await;
    assert_eq!(done.recent_runs.len(), 1, "ten missed minutes make one run");
    let lines = log(&backend, &done.recent_runs[0].id).await;
    assert_eq!(lines[0].text, "Started after a missed run");

    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        job(&backend, "missed").await.recent_runs.len(),
        1,
        "and not a second one"
    );
    assert!(
        job(&backend, "optout").await.recent_runs.is_empty(),
        "catch_up = false runs nothing"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_enabled_after_the_miss_is_not_made_up() {
    let text = job_toml("late", "* * * * *", false, "cmd", &["/c", "echo l"], "");
    let clock = Arc::new(FakeClock::new(base() + secs(10)));
    let mut backend = with_clock(&text, &clock, tuning(400, 60_000)).start().await;
    // Ten minutes pass while it is switched off, then it is switched on: no past to make up.
    clock.set(base() + secs(10 * 60 + 10));
    let (status, _) = backend
        .send("PATCH", "/jobs/late", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK);
    backend.restart().await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(job(&backend, "late").await.recent_runs.is_empty());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_resume_signal() {
    let text = job_toml("tick", "* * * * *", true, "cmd", &["/c", "echo t"], "");
    let clock = Arc::new(FakeClock::new(base() + secs(10)));
    let signal = Arc::new(ResumeSignal::default());
    let backend = with_clock(&text, &clock, tuning(300, 60_000))
        .with::<ResumeSignal>(signal.clone())
        .start()
        .await;
    tokio::time::sleep(Duration::from_millis(600)).await;

    // The PC sleeps through ten due times and wakes: one catch-up run after the delay.
    clock.set(base() + secs(10 * 60 + 10));
    signal.0.notify_one();
    let done = wait_job(&backend, "tick", |j| {
        !j.recent_runs.is_empty() && finished(j)
    })
    .await;
    assert_eq!(done.recent_runs.len(), 1);
    let lines = log(&backend, &done.recent_runs[0].id).await;
    assert_eq!(lines[0].text, "Started after a missed run");
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(job(&backend, "tick").await.recent_runs.len(), 1);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_game_mode() {
    let text = job_toml("tick", "* * * * *", true, "cmd", &["/c", "echo t"], "");
    let clock = Arc::new(FakeClock::new(base() + secs(10)));
    let backend = with_clock(&text, &clock, tuning(300, 60_000)).start().await;
    tokio::time::sleep(Duration::from_millis(600)).await;

    backend.state.set_game_mode_active(true);
    backend.state.jobs().wind_down().pause(true);
    let (status, body) = backend.send("POST", "/jobs/tick/runs", None).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some("gameMode"))
    );

    // Ten minutes pass: nothing starts, and the loop does not spin while an occurrence is overdue.
    clock.set(base() + secs(10 * 60 + 10));
    nudge(&backend).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = backend.state.jobs().scheduler_wakeups();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let after = backend.state.jobs().scheduler_wakeups();
    assert!(
        after - before <= 1,
        "the scheduler looped {} times in game mode",
        after - before
    );
    assert!(job(&backend, "tick").await.recent_runs.is_empty());

    // Game mode is cancelled: the missed run is made up once, at once.
    backend.state.set_game_mode_active(false);
    backend.state.jobs().wind_down().pause(false);
    let done = wait_job(&backend, "tick", |j| {
        !j.recent_runs.is_empty() && finished(j)
    })
    .await;
    assert_eq!(done.recent_runs.len(), 1);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_stop_all_marks_idle() {
    let text = job_toml(
        "long",
        "* * * * *",
        true,
        "ping",
        &["-n", "60", "127.0.0.1"],
        "",
    );
    let clock = Arc::new(FakeClock::new(base() + secs(10)));
    let mut backend = with_clock(&text, &clock, tuning(400, 60_000)).start().await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    clock.set(base() + secs(61));
    nudge(&backend).await;
    let running = wait_job(&backend, "long", |j| j.running).await;
    assert_eq!(backend.state.jobs().wind_down().counts().running, 1);

    // As game mode does it: nothing new starts, then the runs are stopped.
    backend.state.set_game_mode_active(true);
    backend.state.jobs().wind_down().pause(true);
    let stopped = backend.state.jobs().wind_down().stop_all(secs(5)).await;
    assert_eq!(stopped.len(), 1);
    assert_eq!(
        stopped[0].outcome,
        rain_backend::core::winddown::StopOutcome::Stopped
    );
    let done = wait_job(&backend, "long", finished).await;
    let run = &done.recent_runs[0];
    assert_eq!(run.id, running.recent_runs[0].id);
    assert_eq!(
        (run.status, run.exit_code),
        (Status::Idle, None),
        "stopped on purpose, not failed"
    );
    let lines = log(&backend, &run.id).await;
    assert!(
        lines
            .iter()
            .any(|l| l.level == LogLevel::Warn && l.text == "Stopped by game mode")
    );

    // The occurrence that was stopped is run once more at the next start.
    backend.state.set_game_mode_active(false);
    backend.restart().await;
    let again = wait_job(&backend, "long", |j| j.recent_runs.len() == 2).await;
    let lines = log(&backend, &again.recent_runs[0].id).await;
    assert_eq!(lines[0].text, "Started after a missed run");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_of_the_backend_ends_the_runs_as_stopped_on_purpose() {
    let text = job_toml(
        "long",
        "0 0 1 1 *",
        false,
        "ping",
        &["-n", "60", "127.0.0.1"],
        "",
    );
    let mut backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;
    backend.send("POST", "/jobs/long/runs", None).await;
    wait_job(&backend, "long", |j| j.running).await;
    assert!(
        backend.state.activity.busy(),
        "a run keeps the backend busy"
    );

    // `restart` is refused while a run goes on, so this is a plain stop and a start.
    backend.request_stop("stop").await;
    backend.restart_following_machine_file().await;
    let list = job(&backend, "long").await;
    assert_eq!(list.recent_runs[0].status, Status::Idle);
    assert!(!list.running);
    let lines = log(&backend, &list.recent_runs[0].id).await;
    assert!(
        lines
            .iter()
            .any(|l| l.text == "Stopped because Rain stopped")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_start_closes_running_rows() {
    let text = job_toml("a", "0 0 1 1 *", false, "cmd", &[], "");
    let mut backend = TestBackend::builder()
        .file("jobs.toml", &text)
        .start()
        .await;
    // A run that Rain never finished (a crash): a row that is still `running`.
    backend
        .state
        .db
        .call(|c| {
            c.execute(
                "INSERT INTO jobs_run (id, job_id, trigger_kind, status, started_at)
                 VALUES ('run-cut-off', 'a', 'schedule', 'running', '2026-11-21T09:00:00.000Z')",
                [],
            )
        })
        .await
        .unwrap();
    backend.restart().await;

    let list = job(&backend, "a").await;
    assert_eq!(list.recent_runs[0].id, "run-cut-off");
    assert_eq!(list.recent_runs[0].status, Status::Failed);
    assert!(!list.running);
    let lines = log(&backend, "run-cut-off").await;
    assert_eq!(lines[0].text, "Rain stopped while this run was in progress");
    assert_eq!(lines[0].level, LogLevel::Error);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_resync_on_connect_and_the_frames_carry_the_jobs() {
    let backend = TestBackend::with_fixture("typical").await;
    let mut stream = backend.stream().await;
    let (status, _) = backend
        .send("PATCH", "/jobs/job-fail", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK);
    let frame = stream.expect_topic("jobs").await;
    let list = frame.payload.as_array().unwrap();
    assert_eq!(list.len(), 5);
    assert!(
        list.iter()
            .any(|j| j["id"] == "job-fail" && j["enabled"] == true)
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_demo_wait_job_of_the_typical_folder_runs_for_a_while() {
    // Section 06 depends on this job: game mode finds a run in progress.
    let backend = TestBackend::with_fixture("typical").await;
    let (status, _) = backend.send("POST", "/jobs/job-demo-wait/runs", None).await;
    assert_eq!(status, StatusCode::CREATED);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(job(&backend, "job-demo-wait").await.running);
    let stopped = backend.state.jobs().wind_down().stop_all(secs(5)).await;
    assert_eq!(stopped[0].name, "Demo wait");
    backend.stop().await;
}
