//! The only test that starts the real `rain-backend.exe` (foundation section 12.2).
//!
//! It proves the program itself: the environment variables reach it, it listens on the pipe, a
//! second copy ends quietly, and `--stop` ends the first one. Everything else is tested with the
//! backend library inside the test process.

mod common;

use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use common::{PATIENCE, copy_dir, fixture_data_dir, unique_pipe_name};
use hyper::Method;
use rain_pipe::{PipeClient, SseFrame, SseReader, pipe_exists};
use rain_types::limits::API_PREFIX;
use rain_types::{AppEntry, Ping, Settings};
use tempfile::TempDir;

const BACKEND_EXE: &str = env!("CARGO_BIN_EXE_rain-backend");

/// A backend process that is ended when the test ends, also when the test fails.
struct Process {
    child: Child,
}

impl Drop for Process {
    fn drop(&mut self) {
        // Ending a process that has ended already is an error that does not matter here.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The folders and the pipe of one test run, passed to the program by environment variables.
struct Setup {
    pipe_name: String,
    data_dir: PathBuf,
    local_dir: PathBuf,
    _temp: TempDir,
}

impl Setup {
    /// The data folder starts as a copy of the fixture `typical`, so that the test can see that
    /// `RAIN_DATA_DIR` reaches the settings.
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let data_dir = temp.path().join("data");
        copy_dir(&fixture_data_dir("typical"), &data_dir);
        Self {
            pipe_name: unique_pipe_name(),
            data_dir,
            local_dir: temp.path().join("local"),
            _temp: temp,
        }
    }

    // The test starts the real program; `platform::process` is for code inside the backend.
    #[allow(clippy::disallowed_methods)]
    fn command(&self, flags: &[&str]) -> Command {
        let mut command = Command::new(BACKEND_EXE);
        command
            .args(flags)
            .env("RAIN_PIPE", &self.pipe_name)
            .env("RAIN_DATA_DIR", &self.data_dir)
            .env("RAIN_LOCAL_DIR", &self.local_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn start(&self) -> Process {
        Process {
            child: self
                .command(&[])
                .spawn()
                .expect("the backend program started"),
        }
    }
}

async fn wait_for_ping(client: &PipeClient) -> Ping {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Ok(ping) = client
            .json::<Ping>(Method::GET, &format!("{API_PREFIX}/ping"), None::<&()>)
            .await
        {
            return ping;
        }
        assert!(
            Instant::now() < deadline,
            "the backend did not answer in time"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_exit(process: &mut Process) -> ExitStatus {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(status) = process.child.try_wait().expect("the process can be asked") {
            return status;
        }
        assert!(Instant::now() < deadline, "the process did not end in time");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_backend_program_serves_the_pipe_ends_a_second_copy_and_stops_on_request() {
    let setup = Setup::new();
    let mut backend = setup.start();
    let client = PipeClient::new(&setup.pipe_name);

    // 1. It answers on the pipe named by the environment, as a sandbox, as this very process.
    let ping = wait_for_ping(&client).await;
    assert_eq!(ping.app, "rain-backend");
    assert!(ping.sandbox);
    assert_eq!(ping.pid, backend.child.id());

    // The data folder of the environment is the one the settings are read from.
    let settings: Settings = client
        .json(Method::GET, &format!("{API_PREFIX}/settings"), None::<&()>)
        .await
        .expect("the settings were read");
    assert_eq!(
        settings.reset_after_seconds,
        rain_types::ResetAfterSeconds::Seconds300
    );

    // The real shell answers for the app list (the count is not asserted; no program is started),
    // and at least one icon is a PNG when there are apps.
    let apps: Vec<AppEntry> = client
        .json(Method::GET, &format!("{API_PREFIX}/apps"), None::<&()>)
        .await
        .expect("the real app list was read");
    if !apps.is_empty() {
        let mut png_seen = false;
        for app in apps.iter().take(8) {
            let path = format!("{API_PREFIX}/{}", app.icon.value);
            let answer = client
                .request(Method::GET, &path, None, &[])
                .await
                .expect("the icon route answered");
            png_seen |= answer.status == hyper::StatusCode::OK
                && answer.body.starts_with(&[0x89, b'P', b'N', b'G']);
        }
        assert!(png_seen, "one of the first apps has a PNG icon");
    }

    // 2. The first two frames of the stream are hello and resync.
    let response = client
        .send(
            Method::GET,
            &format!("{API_PREFIX}/stream"),
            None,
            &[("accept", "text/event-stream")],
        )
        .await
        .expect("the stream opened");
    let mut reader = SseReader::new(response.into_body());
    let mut names = Vec::new();
    while names.len() < 2 {
        if let Some(SseFrame::Event { event, .. }) = reader.next().await.expect("a frame") {
            names.push(event);
        }
    }
    assert_eq!(names, ["hello", "resync"]);

    // 3. A second copy on the same pipe ends at once, without an error.
    let mut second = Process {
        child: setup.command(&[]).spawn().expect("a second copy started"),
    };
    let status = wait_for_exit(&mut second).await;
    assert!(status.success(), "the second copy ended with {status}");
    assert_eq!(
        client
            .json::<Ping>(Method::GET, &format!("{API_PREFIX}/ping"), None::<&()>)
            .await
            .expect("still answering")
            .pid,
        backend.child.id()
    );

    // 4. `--stop` ends the first copy and returns when the pipe is gone.
    let stop = setup.command(&["--stop"]).status().expect("--stop ran");
    assert!(stop.success(), "--stop ended with {stop}");
    assert!(
        !pipe_exists(&setup.pipe_name),
        "the pipe is gone after --stop"
    );
    assert!(wait_for_exit(&mut backend).await.success());

    // 5. With nothing running, `--stop` says so and succeeds.
    let stop = setup.command(&["--stop"]).status().expect("--stop ran");
    assert!(stop.success());

    // The program wrote its log to the local folder it was told about.
    assert!(setup.local_dir.join("logs").is_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_force_ends_a_backend_that_leaves_by_itself_without_ending_its_process_by_force() {
    let setup = Setup::new();
    let mut backend = setup.start();
    let client = PipeClient::new(&setup.pipe_name);
    wait_for_ping(&client).await;

    // The backend leaves within the wait, so `--force` has nothing to end by pid: the exit code
    // is 0 and the process ended through its own shutdown (the path of `--stop` without force).
    let stop = setup
        .command(&["--stop", "--force"])
        .status()
        .expect("--stop --force ran");
    assert!(stop.success(), "--stop --force ended with {stop}");
    assert!(!pipe_exists(&setup.pipe_name));
    assert!(wait_for_exit(&mut backend).await.success());

    // The program made its database in the local folder and left no write-ahead log behind.
    let wal = setup.local_dir.join("rain.db-wal");
    assert!(setup.local_dir.join("rain.db").is_file());
    assert!(!wal.exists() || std::fs::metadata(&wal).expect("the log can be read").len() == 0);

    // `--force` alone is a mistake in the command line.
    let alone = setup.command(&["--force"]).status().expect("--force ran");
    assert_eq!(alone.code(), Some(2));
}
