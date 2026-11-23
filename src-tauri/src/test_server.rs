//! A stand-in for the backend, for tests: a small HTTP server on a named pipe that answers with
//! what the test says and remembers what it was asked.
//!
//! Tests that need the real backend program use `live_backend_tests.rs`. This one is for the
//! cases the real backend cannot be made to produce on demand (a busy backend, an answer of 16 MiB,
//! a media route that has no provider yet).

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rain_pipe::PipeClient;
use tokio::net::windows::named_pipe::ServerOptions;
use tokio_util::sync::CancellationToken;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// One request as the server saw it.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path_and_query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn path(&self) -> &str {
        self.path_and_query
            .split_once('?')
            .map_or(self.path_and_query.as_str(), |(path, _)| path)
    }
}

/// What the server answers.
#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Reply {
    pub fn new(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self::new(status, body.to_string().as_bytes()).header("content-type", "application/json")
    }

    pub fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_owned()));
        self
    }
}

type Handler = dyn Fn(&Seen) -> Reply + Send + Sync;

/// Serves until it is dropped. Every server has a pipe of its own.
pub struct FakeServer {
    pipe_name: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: CancellationToken,
}

impl FakeServer {
    /// The pipe exists when this returns, so a client may connect at once.
    pub fn start(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Self {
        let pipe_name = format!(
            "rain-fakeserver-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = CancellationToken::new();
        let path = format!(r"\\.\pipe\{pipe_name}");
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .expect("the test pipe can be created");
        tokio::spawn(accept_loop(
            path,
            first,
            Arc::new(handler),
            Arc::clone(&seen),
            stop.clone(),
        ));
        Self {
            pipe_name,
            seen,
            stop,
        }
    }

    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    pub fn client(&self) -> PipeClient {
        PipeClient::new(&self.pipe_name)
    }

    /// The requests so far, oldest first.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .expect("the list is never poisoned")
            .clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// Takes one connection after the other. The next pipe instance is created before the current
/// connection is handed over, so a client never finds the pipe missing.
async fn accept_loop(
    path: String,
    mut next: tokio::net::windows::named_pipe::NamedPipeServer,
    handler: Arc<Handler>,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: CancellationToken,
) {
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            connected = next.connect() => if connected.is_err() { return },
        }
        let Ok(following) = ServerOptions::new().create(&path) else {
            return;
        };
        let connection = std::mem::replace(&mut next, following);
        let handler = Arc::clone(&handler);
        let seen = Arc::clone(&seen);
        let stop = stop.clone();
        tokio::spawn(async move {
            let service =
                service_fn(move |request| answer(request, Arc::clone(&handler), Arc::clone(&seen)));
            let serving = http1::Builder::new().serve_connection(TokioIo::new(connection), service);
            tokio::select! {
                () = stop.cancelled() => {}
                _ = serving => {}
            }
        });
    }
}

async fn answer(
    request: Request<Incoming>,
    handler: Arc<Handler>,
    seen: Arc<Mutex<Vec<Seen>>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let request = Seen {
        method: parts.method.to_string(),
        path_and_query: parts
            .uri
            .path_and_query()
            .map(ToString::to_string)
            .unwrap_or_default(),
        headers: parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect(),
        body: body.to_vec(),
    };
    let reply = handler(&request);
    seen.lock()
        .expect("the list is never poisoned")
        .push(request);
    let mut response = Response::builder().status(reply.status);
    for (name, value) in &reply.headers {
        response = response.header(*name, value);
    }
    Ok(response
        .body(Full::new(Bytes::from(reply.body)))
        .expect("a test reply is a valid response"))
}
