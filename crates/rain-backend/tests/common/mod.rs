//! The integration harness (foundation section 12.2).
//!
//! `TestBackend` runs the backend library inside the test process, on a pipe of its own with
//! temporary folders. The client is `rain_pipe::PipeClient`, the same pooled client that the shell
//! uses, so every test also exercises the real transport.
//!
//! Each test file includes this module with `mod common;`, and none of them uses every helper.
#![allow(dead_code)]

pub mod fakes;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use hyper::{HeaderMap, Method, StatusCode};
use rain_backend::config::{Config, Limits};
use rain_backend::core::clock::Clock;
use rain_backend::core::launch_contract::Opener;
use rain_backend::core::secrets::Secrets;
use rain_backend::core::state::{AppState, AppStateBuilder};
use rain_backend::modules::gamemode::Pc;
use rain_backend::modules::launcher::{AppSource, IconRenderer, PathProbe};
use rain_backend::modules::settings::Detect;
use rain_backend::platform::process::ProcessLister;
use rain_backend::platform::registry::RunKey;
use rain_backend::{Running, start};
use rain_pipe::{PipeClient, SseFrame, SseReader, pipe_exists};
use rain_types::env::pipe_path;
use rain_types::limits::API_PREFIX;
use rain_types::{Frame, StopReason};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tempfile::TempDir;

/// How long a test waits for something that should happen at once.
pub const PATIENCE: Duration = Duration::from_secs(5);

static PIPE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A pipe name that no other test and no real backend uses: `rain-test-<process id>-<number>`.
pub fn unique_pipe_name() -> String {
    let number = PIPE_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("rain-test-{}-{number}", std::process::id())
}

/// A change to the state that a test wants, kept so that a restart applies it again.
type Tweak = Arc<dyn Fn(AppStateBuilder) -> AppStateBuilder + Send + Sync>;
/// A change to the `Config`, kept for the same reason.
type ConfigTweak = Arc<dyn Fn(&mut Config) + Send + Sync>;

pub struct TestBackend {
    pub client: PipeClient,
    pub pipe_name: String,
    pub data_dir: PathBuf,
    pub local_dir: PathBuf,
    pub state: AppState,
    running: Option<Running>,
    tweaks: Vec<Tweak>,
    config_tweaks: Vec<ConfigTweak>,
    _temp: TempDir,
}

pub struct TestBackendBuilder {
    limits: Limits,
    extra: Router<AppState>,
    fixture: Option<String>,
    files: Vec<(String, String)>,
    local_files: Vec<(String, Vec<u8>)>,
    tweaks: Vec<Tweak>,
    config_tweaks: Vec<ConfigTweak>,
}

impl TestBackend {
    /// A backend with default limits and no extra routes.
    pub async fn start() -> Self {
        Self::builder().start().await
    }

    /// A backend whose data folder starts as a copy of `tests/fixtures/<name>/data`.
    pub async fn with_fixture(name: &str) -> Self {
        Self::builder().fixture(name).start().await
    }

    pub fn builder() -> TestBackendBuilder {
        TestBackendBuilder {
            limits: Limits::default(),
            extra: Router::new(),
            fixture: None,
            files: Vec::new(),
            local_files: Vec::new(),
            // A backend never reads the real PC: nothing is detected, and the autostart entry is
            // in memory. A test that wants to see them replaces these with `with`.
            tweaks: vec![
                Arc::new(|b| b.with::<dyn Detect>(Arc::new(fakes::FakeDetect::default()))),
                Arc::new(|b| b.with::<dyn RunKey>(Arc::new(fakes::FakeRunKey::default()))),
                // launcher and apps (section 01): no real app list, icon, process list, file check or
                // program start in any test. A test that wants to see them replaces these.
                Arc::new(|b| b.with::<dyn AppSource>(Arc::new(fakes::FakeAppSource::default()))),
                Arc::new(|b| {
                    b.with::<dyn IconRenderer>(Arc::new(fakes::FakeIconRenderer::default()))
                }),
                Arc::new(|b| b.with::<dyn PathProbe>(Arc::new(fakes::FakePathProbe::default()))),
                Arc::new(|b| {
                    b.with::<dyn ProcessLister>(Arc::new(fakes::FakeProcesses::default()))
                }),
                Arc::new(|b| b.opener(Arc::new(fakes::FakeOpener::default()))),
                // hardware (section 05): no real device is read.
                Arc::new(|b| {
                    b.with::<dyn rain_backend::modules::hardware::Source>(Arc::new(
                        fakes::FakeHardware::default(),
                    ))
                }),
                // game mode (section 06 6.11): no test ever sees, asks or ends a real program.
                Arc::new(|b| b.with::<dyn Pc>(Arc::new(fakes::FakePc::default()))),
            ],
            config_tweaks: Vec::new(),
        }
    }

    /// `path` is the path after `/api/v1`.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> T {
        let (status, body) = self.send("GET", path, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path} answered {body}");
        serde_json::from_value(body).expect("the answer has the expected shape")
    }

    /// Sends a request with a JSON body. Returns the status and the JSON of the answer
    /// (`null` when the body is empty).
    pub async fn send(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let bytes = body.map(|value| value.to_string().into());
        self.send_bytes(method, path, bytes).await
    }

    /// Like `send`, but the body is sent exactly as given, so a test can send broken JSON.
    pub async fn send_bytes(
        &self,
        method: &str,
        path: &str,
        body: Option<hyper::body::Bytes>,
    ) -> (StatusCode, Value) {
        let method = Method::from_bytes(method.as_bytes()).expect("a valid method");
        let raw = self
            .client
            .request(method, &format!("{API_PREFIX}{path}"), body, &[])
            .await
            .expect("the backend answered");
        let json = if raw.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&raw.body).expect("the answer is JSON")
        };
        (raw.status, json)
    }

    /// Opens the live stream.
    pub async fn stream(&self) -> StreamReader {
        let response = self
            .client
            .send(
                Method::GET,
                &format!("{API_PREFIX}/stream"),
                None,
                &[("accept", "text/event-stream")],
            )
            .await
            .expect("the stream opened");
        assert_eq!(response.status(), StatusCode::OK);
        StreamReader {
            headers: response.headers().clone(),
            reader: SseReader::new(response.into_body()),
        }
    }

    /// Stops the backend and starts a new one on the same data and local folders, as the shell
    /// does after a crash. The new backend has a new pipe, a new `startId` and no extra routes,
    /// and default limits.
    pub async fn restart(&mut self) {
        self.request_stop("restart").await;
        self.wait_exited().await;
        self.wait_pipe_gone().await;

        self.pipe_name = unique_pipe_name();
        let mut config = Config::for_test(
            &self.pipe_name,
            self.data_dir.clone(),
            self.local_dir.clone(),
        );
        self.config_tweaks
            .iter()
            .for_each(|tweak| tweak(&mut config));
        let (state, running) = boot(config, Router::new(), &self.tweaks).await;
        self.client = PipeClient::new(&self.pipe_name);
        self.state = state;
        self.running = Some(running);
    }

    /// Like `restart`, but the new backend takes its data folder from `machine.toml`, as a real
    /// start does (`Config::apply_machine_file`). After a change of the data folder in Settings
    /// this is what the shell's restart gives. `data_dir` follows the new folder.
    pub async fn restart_following_machine_file(&mut self) {
        self.wait_exited().await;
        self.wait_pipe_gone().await;

        self.pipe_name = unique_pipe_name();
        let mut config = Config::for_test(
            &self.pipe_name,
            self.data_dir.clone(),
            self.local_dir.clone(),
        );
        self.config_tweaks
            .iter()
            .for_each(|tweak| tweak(&mut config));
        config.apply_machine_file();
        self.data_dir = config.data_dir.clone();
        let (state, running) = boot(config, Router::new(), &self.tweaks).await;
        self.client = PipeClient::new(&self.pipe_name);
        self.state = state;
        self.running = Some(running);
    }

    /// Asks for a stop with the given reason, the way the shell does. Does not wait.
    pub async fn request_stop(&self, reason: &str) {
        let (status, _) = self
            .send(
                "POST",
                "/internal/shutdown",
                Some(json!({ "reason": reason })),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// Waits until the backend has finished its stop. Its pipe may still exist: the name stays
    /// while any client, also one that does not read, holds its end open.
    pub async fn wait_exited(&mut self) {
        let running = self
            .running
            .take()
            .expect("the backend was not stopped yet");
        tokio::time::timeout(PATIENCE, running.wait())
            .await
            .expect("the backend stopped in time")
            .expect("the shutdown task ended normally");
    }

    /// Waits until the backend has stopped, and checks that its pipe is gone.
    pub async fn wait_stopped(mut self) {
        if self.running.is_some() {
            self.wait_exited().await;
        }
        self.wait_pipe_gone().await;
    }

    /// Checks that the pipe name vanishes. Every client must have closed its end by then.
    pub async fn wait_pipe_gone(&self) {
        // The name stays until the last client has closed its end, which the pooled client does
        // a moment after it sees the connection end.
        let gone = async {
            while pipe_exists(&self.pipe_name) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        assert!(
            tokio::time::timeout(PATIENCE, gone).await.is_ok(),
            "the pipe is gone after a stop"
        );
    }

    /// `POST /internal/shutdown`, then waits until the backend has stopped.
    pub async fn stop(self) {
        self.request_stop("stop").await;
        self.wait_stopped().await;
    }
}

impl Drop for TestBackend {
    /// A test that fails half way must not leave a backend behind.
    fn drop(&mut self) {
        if self.running.is_some() {
            self.state.shutdown.request(StopReason::Stop);
        }
    }
}

impl TestBackendBuilder {
    /// Changes the limits of the server, for example to make a timeout short.
    pub fn limits(mut self, change: impl FnOnce(&mut Limits)) -> Self {
        change(&mut self.limits);
        self
    }

    /// Starts with a copy of `tests/fixtures/<name>/data` as the data folder.
    pub fn fixture(mut self, name: &str) -> Self {
        self.fixture = Some(name.to_owned());
        self
    }

    /// Adds routes to the API (they get the same layers as every real route).
    pub fn extra_routes(mut self, routes: Router<AppState>) -> Self {
        self.extra = routes;
        self
    }

    /// Puts a file into the data folder before the backend starts. `{DATA}` and `{TEMP}` in the
    /// text become the data folder and the temporary folder of this run.
    pub fn file(mut self, name: &str, text: &str) -> Self {
        self.files.push((name.to_owned(), text.to_owned()));
        self
    }

    /// Puts a file into the local folder (database, cache, logs) before the backend starts.
    pub fn local_file(mut self, name: &str, bytes: &[u8]) -> Self {
        self.local_files.push((name.to_owned(), bytes.to_vec()));
        self
    }

    /// The clock of the backend: a `FakeClock` that the test moves.
    pub fn clock(self, clock: Arc<dyn Clock>) -> Self {
        self.tweak(move |b| b.clock(Arc::clone(&clock)))
    }

    pub fn secrets(self, secrets: Arc<dyn Secrets>) -> Self {
        self.tweak(move |b| b.secrets(Arc::clone(&secrets)))
    }

    /// Replaces the opener, so that no program is started.
    pub fn opener(self, opener: Arc<dyn Opener>) -> Self {
        self.tweak(move |b| b.opener(Arc::clone(&opener)))
    }

    /// Replaces a part that a module reaches through a trait:
    /// `.with::<dyn AppSource>(Arc::new(FakeApps::new(..)))`.
    pub fn with<T: ?Sized + Send + Sync + 'static>(self, part: Arc<T>) -> Self {
        self.tweak(move |b| b.with::<T>(Arc::clone(&part)))
    }

    /// A real (not sandbox) run, for the tests of the few things that a sandbox refuses (the
    /// autostart entry). The fakes still stand in for the PC.
    pub fn real_mode(self) -> Self {
        self.config(|config| config.sandbox = false)
    }

    /// Changes the `Config` before the backend starts.
    pub fn config(mut self, change: impl Fn(&mut Config) + Send + Sync + 'static) -> Self {
        self.config_tweaks.push(Arc::new(change));
        self
    }

    fn tweak(
        mut self,
        change: impl Fn(AppStateBuilder) -> AppStateBuilder + Send + Sync + 'static,
    ) -> Self {
        self.tweaks.push(Arc::new(change));
        self
    }

    pub async fn start(self) -> TestBackend {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let data_dir = temp.path().join("data");
        let local_dir = temp.path().join("local");
        let pipe_name = unique_pipe_name();

        let places = Places {
            data: data_dir.to_string_lossy().into_owned(),
            temp: temp.path().to_string_lossy().into_owned(),
        };
        if let Some(name) = &self.fixture {
            copy_fixture(&fixture_data_dir(name), &data_dir, &places);
        }
        for (name, text) in &self.files {
            std::fs::create_dir_all(&data_dir).expect("the data folder was created");
            std::fs::write(data_dir.join(name), places.resolve(text)).expect("a seeded file");
        }
        for (name, bytes) in &self.local_files {
            std::fs::create_dir_all(&local_dir).expect("the local folder was created");
            std::fs::write(local_dir.join(name), bytes).expect("a seeded local file");
        }
        let mut config = Config::for_test(&pipe_name, data_dir.clone(), local_dir.clone());
        config.limits = self.limits;
        self.config_tweaks
            .iter()
            .for_each(|tweak| tweak(&mut config));
        let (state, running) = boot(config, self.extra, &self.tweaks).await;

        TestBackend {
            client: PipeClient::new(&pipe_name),
            pipe_name,
            data_dir,
            local_dir,
            state,
            running: Some(running),
            tweaks: self.tweaks,
            config_tweaks: self.config_tweaks,
            _temp: temp,
        }
    }
}

/// Reads the frames of an open stream.
pub struct StreamReader {
    /// The headers of the response that opened the stream.
    pub headers: HeaderMap,
    reader: SseReader,
}

impl StreamReader {
    /// The next event and its frame, without the keep-alive comments. `None` when the server
    /// closed the stream. Panics when nothing arrives within `PATIENCE`.
    pub async fn next_event(&mut self) -> Option<(String, Frame)> {
        loop {
            let frame = tokio::time::timeout(PATIENCE, self.reader.next())
                .await
                .expect("a frame arrived in time")
                .expect("the stream was readable");
            match frame {
                None => return None,
                Some(SseFrame::Comment(_)) => continue,
                Some(SseFrame::Event { event, data }) => {
                    let frame: Frame = serde_json::from_str(&data).expect("a frame envelope");
                    return Some((event, frame));
                }
            }
        }
    }

    /// The next raw frame, keep-alive comments included.
    pub async fn next_raw(&mut self) -> SseFrame {
        tokio::time::timeout(PATIENCE, self.reader.next())
            .await
            .expect("a frame arrived in time")
            .expect("the stream was readable")
            .expect("the stream is still open")
    }

    /// The next frame of the topic `topic`; the frames of other topics before it are skipped.
    pub async fn expect_topic(&mut self, topic: &str) -> Frame {
        loop {
            let (_, frame) = self.expect_event().await;
            if frame.topic == topic {
                return frame;
            }
        }
    }

    /// Like `next_event`, but the stream must not have ended.
    pub async fn expect_event(&mut self) -> (String, Frame) {
        self.next_event().await.expect("the stream is still open")
    }

    /// Everything until the stream ends.
    pub async fn rest(&mut self) -> Vec<(String, Frame)> {
        let mut events = Vec::new();
        while let Some(event) = self.next_event().await {
            events.push(event);
        }
        events
    }
}

/// The path of a pipe, for a test that connects without the client.
pub fn path_of(pipe_name: &str) -> String {
    pipe_path(pipe_name)
}

/// Starts the backend library in this process.
async fn boot(config: Config, extra: Router<AppState>, tweaks: &[Tweak]) -> (AppState, Running) {
    let state = tweaks
        .iter()
        .fold(AppState::builder(config), |builder, tweak| tweak(builder))
        .build();
    let running = start(state.clone(), extra)
        .await
        .expect("the test backend started");
    (state, running)
}

/// `tests/fixtures/<name>/data`: a made-up data folder with the files of a situation.
pub fn fixture_data_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
        .join("data")
}

/// The places that a fixture names with placeholders.
struct Places {
    data: String,
    temp: String,
}

impl Places {
    fn resolve(&self, text: &str) -> String {
        text.replace("{DATA}", &self.data)
            .replace("{TEMP}", &self.temp)
    }
}

/// Copies a fixture folder. In the text files (`.toml`, `.json`, `.md`) `{DATA}` and `{TEMP}`
/// become the folders of this run, so a fixture never holds the path of a real PC.
fn copy_fixture(from: &Path, to: &Path, places: &Places) {
    std::fs::create_dir_all(to).expect("the target folder was created");
    for entry in std::fs::read_dir(from).expect("the fixture folder exists") {
        let entry = entry.expect("a fixture entry");
        let target = to.join(entry.file_name());
        let path = entry.path();
        let is_text = path.extension().is_some_and(|ext| {
            ["toml", "json", "md"]
                .iter()
                .any(|text| ext.eq_ignore_ascii_case(text))
        });
        if path.is_dir() {
            copy_fixture(&path, &target, places);
        } else if is_text {
            let text = std::fs::read_to_string(&path).expect("a UTF-8 fixture file");
            std::fs::write(target, places.resolve(&text)).expect("a fixture file was written");
        } else {
            std::fs::copy(&path, target).expect("a fixture file was copied");
        }
    }
}

/// Copies a folder and everything in it. The target is created.
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("the target folder was created");
    for entry in std::fs::read_dir(from).expect("the fixture folder exists") {
        let entry = entry.expect("a fixture entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("a fixture file was copied");
        }
    }
}
