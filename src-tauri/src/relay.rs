//! Carries the backend's live stream to the page (foundation section 6.2).
//!
//! One task lives as long as the shell. It keeps the stream open, passes every frame to the page
//! as the event `rain:stream`, tells the state machine what the stream says, and reacts when the
//! stream ends: a crash is answered with a new backend, a planned stop with whatever that stop
//! means (game mode ends Rain, a restart starts the new backend).
//!
//! The relay needs three things from the app and gets them through [`Host`], so that tests can
//! run it without a window.

use std::sync::Arc;
use std::time::Duration;

use rain_pipe::{Notice, SseFrame, run_stream};
use rain_types::limits::{API_PREFIX, SHUTDOWN_BUDGET, STREAM_DEAD_AFTER, UPGRADE_WAIT};
use rain_types::{Frame, Hello, Lifecycle, ShellFacts, StopReason};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::backend::{Backend, Event, wait_until_pipe_is_gone};
use crate::facts;

/// The page event that carries every frame of the stream, unchanged.
pub const EVENT_STREAM: &str = "rain:stream";
/// The page event that carries the state of the backend process.
pub const EVENT_BACKEND: &str = "rain:backend";

/// What the relay needs from the application around it.
pub trait Host: Send + Sync + 'static {
    /// Passes a frame on to the page.
    fn emit_frame(&self, frame: &Frame);
    /// The facts that only the shell knows, as they are now.
    fn shell_facts(&self) -> ShellFacts;
    /// Ends the application.
    fn exit_app(&self);
}

/// Runs until `stop` is cancelled. `exit_grace` is how long the shell waits after a game mode
/// stop before it exits (`GAME_MODE_EXIT_GRACE` in the app).
pub async fn run(
    backend: Backend,
    host: Arc<dyn Host>,
    exit_grace: Duration,
    stop: CancellationToken,
) {
    let (notices, mut incoming) = mpsc::unbounded_channel();
    let mut relay = Relay {
        backend,
        host,
        exit_grace,
        stop: stop.clone(),
        notices,
        stream: None,
        connected: false,
    };
    relay.open_stream();
    loop {
        tokio::select! {
            Some(notice) = incoming.recv() => relay.on_notice(notice).await,
            // The backend was just started or found: connect now instead of after the backoff.
            () = relay.backend.reconnect().notified() => relay.open_stream(),
            () = stop.cancelled() => break,
        }
    }
    relay.close_stream();
}

struct Relay {
    backend: Backend,
    host: Arc<dyn Host>,
    exit_grace: Duration,
    stop: CancellationToken,
    /// The stream task sends what it sees here, and `run` handles it one notice at a time.
    notices: mpsc::UnboundedSender<Notice>,
    stream: Option<JoinHandle<()>>,
    /// True from `Connected` until the stream ends.
    connected: bool,
}

impl Relay {
    /// (Re)starts the task that keeps the stream open and reconnects it with its own backoff.
    fn open_stream(&mut self) {
        self.close_stream();
        let client = self.backend.client();
        let notices = self.notices.clone();
        let stop = self.stop.clone();
        self.stream = Some(tokio::spawn(async move {
            let path = format!("{API_PREFIX}/stream");
            run_stream(client, &path, STREAM_DEAD_AFTER, stop, move |notice| {
                // The receiver only goes away when the relay itself ends.
                let _ = notices.send(notice);
            })
            .await;
        }));
    }

    fn close_stream(&mut self) {
        self.connected = false;
        if let Some(task) = self.stream.take() {
            task.abort();
        }
    }

    async fn on_notice(&mut self, notice: Notice) {
        match notice {
            Notice::Connecting { .. } => {}
            Notice::Connected => self.connected = true,
            Notice::Frame(SseFrame::Event { data, .. }) => self.on_frame(&data).await,
            // Keep-alive lines only prove that the connection lives.
            Notice::Frame(SseFrame::Comment(_)) => {}
            Notice::Disconnected { reason } => self.on_disconnected(&reason).await,
        }
    }

    async fn on_frame(&mut self, data: &str) {
        let frame: Frame = match serde_json::from_str(data) {
            Ok(frame) => frame,
            Err(error) => {
                warn!(%error, "the stream sent a frame that is not a frame");
                return;
            }
        };
        match frame.topic.as_str() {
            "hello" => self.on_hello(&frame).await,
            "lifecycle" => self.on_lifecycle(&frame),
            _ => {}
        }
        self.host.emit_frame(&frame);
    }

    /// A backend that says hello is up. The shell first tells it its facts, so that the page,
    /// which refetches everything after the `resync` that follows, already sees them.
    async fn on_hello(&mut self, frame: &Frame) {
        let Ok(hello) = serde_json::from_value::<Hello>(frame.payload.clone()) else {
            warn!("the hello frame has an unexpected shape");
            return;
        };
        let facts = self.host.shell_facts();
        if let Err(error) = facts::report(&self.backend.client(), &facts).await {
            warn!(message = %error.message, "could not tell the backend about the shell");
        }
        self.backend.apply(Event::Hello {
            start_id: hello.start_id,
        });
    }

    fn on_lifecycle(&mut self, frame: &Frame) {
        match serde_json::from_value::<Lifecycle>(frame.payload.clone()) {
            Ok(lifecycle) => self.backend.apply(Event::Stopping(lifecycle.reason)),
            Err(_) => warn!("the lifecycle frame has an unexpected shape"),
        }
    }

    async fn on_disconnected(&mut self, reason: &str) {
        let was_connected = std::mem::take(&mut self.connected);
        if let Some(stop) = self.backend.announced_stop() {
            self.on_planned_stop(stop).await;
            return;
        }
        self.backend.apply(Event::Lost);
        // A connection that never worked (the backend was not there to begin with) must not start
        // a backend again and again. Only the loss of a live stream does: that is a crash.
        if was_connected {
            warn!(reason, "the stream to the backend was lost");
            if let Err(error) = self.backend.ensure().await {
                warn!(message = %error.message, "the backend could not be started again");
            }
        }
    }

    /// The backend said why it stops. The stream is closed meanwhile: there is nothing to read.
    async fn on_planned_stop(&mut self, reason: StopReason) {
        self.close_stream();
        match reason {
            StopReason::GameMode => self.exit_after_game_mode(),
            StopReason::Stop => info!("the backend stopped; it starts again on the next call"),
            StopReason::Upgrade | StopReason::Restart => {
                wait_until_pipe_is_gone(self.backend.pipe_name(), UPGRADE_WAIT).await;
                if let Err(error) = self.backend.ensure().await {
                    warn!(message = %error.message, "the new backend could not be started");
                }
                self.open_stream();
            }
        }
    }

    /// Game mode ended Rain on purpose (foundation section 2.6): wait for the pipe to close, give
    /// the backend a moment more, and exit. Nothing starts Rain again.
    ///
    /// The wait runs as a task of its own that the backend handle remembers, so that a second
    /// launch of the shell during the wait can cancel it (`Backend::revive_after_game_mode`).
    fn exit_after_game_mode(&mut self) {
        info!("game mode stopped the backend; the shell exits");
        let pipe_name = self.backend.pipe_name().to_owned();
        let grace = self.exit_grace;
        let host = Arc::clone(&self.host);
        let exit = tokio::spawn(async move {
            wait_until_pipe_is_gone(&pipe_name, SHUTDOWN_BUDGET).await;
            tokio::time::sleep(grace).await;
            host.exit_app();
        });
        self.backend.set_exit_task(exit.abort_handle());
    }
}
