//! The pooled client. Copied in shape from the spike `types-stream` (`client.rs`); see foundation
//! section 6.10.
//!
//! hyper-util's pooled `Client` keeps idle connections and opens more when needed, so ten requests
//! at once simply use ten pipe connections. Cloning a `PipeClient` is cheap: all clones share one pool.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper::{HeaderMap, Method, Request, StatusCode, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rain_types::env::pipe_path;
use rain_types::limits::{
    CLIENT_POOL_IDLE_PER_HOST, CLIENT_POOL_IDLE_TIMEOUT, PIPE_BUSY_DEADLINE, PIPE_BUSY_RETRY,
};
use rain_types::{ApiError, ErrorCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

use crate::error::{ERROR_PIPE_BUSY, client_error, error_from_response, transport_error};

/// A pipe connection in the form hyper wants it. hyper's pool needs the `Connection` trait, which
/// tokio's pipe type does not have, so this thin wrapper hands every call to the pipe inside.
pub struct PipeIo(TokioIo<NamedPipeClient>);

impl Read for PipeIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl Write for PipeIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl Connection for PipeIo {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

/// Opens the pipe. `ERROR_PIPE_BUSY` means that every instance is taken at this moment (the server
/// is making the next one): wait a few milliseconds and try again. Any other error is final; a
/// missing pipe (`ERROR_FILE_NOT_FOUND`) means that the backend is not running.
async fn open_pipe(path: &str) -> io::Result<NamedPipeClient> {
    let deadline = Instant::now() + PIPE_BUSY_DEADLINE;
    loop {
        match ClientOptions::new().open(path) {
            Ok(pipe) => return Ok(pipe),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                tokio::time::sleep(PIPE_BUSY_RETRY).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Tells hyper's client how to reach "the server": by opening the pipe. The URI is ignored.
#[derive(Clone)]
struct PipeConnector {
    path: Arc<str>,
}

impl tower::Service<Uri> for PipeConnector {
    type Response = PipeIo;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<PipeIo>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        let path = self.path.clone();
        Box::pin(async move { Ok(PipeIo(TokioIo::new(open_pipe(&path).await?))) })
    }
}

/// A whole response, read into memory. A non-2xx status is not an error at this level;
/// see [`RawResponse::into_success`].
#[derive(Debug)]
pub struct RawResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl RawResponse {
    /// 2xx and 304 pass. Anything else becomes the error the backend described.
    pub fn into_success(self) -> Result<Self, ApiError> {
        if self.status.is_success() || self.status == StatusCode::NOT_MODIFIED {
            Ok(self)
        } else {
            Err(error_from_response(self.status, &self.body))
        }
    }
}

#[derive(Clone)]
pub struct PipeClient {
    http: Client<PipeConnector, Full<Bytes>>,
}

impl PipeClient {
    /// `pipe_name` is the name without the `\\.\pipe\` prefix.
    pub fn new(pipe_name: &str) -> Self {
        let connector = PipeConnector {
            path: pipe_path(pipe_name).into(),
        };
        let http = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new()) // without a timer the pool never drops idle connections
            .pool_max_idle_per_host(CLIENT_POOL_IDLE_PER_HOST)
            .pool_idle_timeout(CLIENT_POOL_IDLE_TIMEOUT)
            .build(connector);
        Self { http }
    }

    /// Sends a request and returns the response with its body not yet read.
    /// The stream (Server-Sent Events) needs this: its body never ends.
    ///
    /// `path` is the whole path including `/api/v1`.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Bytes>,
        headers: &[(&str, &str)],
    ) -> Result<hyper::Response<Incoming>, ApiError> {
        // The host name is a placeholder: the connector ignores it, but HTTP/1.1 needs one.
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("http://rain{path}"));
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(Full::new(body.unwrap_or_default()))
            .map_err(|e| {
                client_error(
                    ErrorCode::Failed,
                    "badRequest",
                    format!("Rain could not build the request: {e}"),
                )
            })?;
        self.http
            .request(request)
            .await
            .map_err(|e| transport_error(&e))
    }

    /// Sends a request and reads the whole response. A non-2xx status is returned, not raised.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Bytes>,
        headers: &[(&str, &str)],
    ) -> Result<RawResponse, ApiError> {
        let response = self.send(method, path, body, headers).await?;
        let (parts, incoming) = response.into_parts();
        let body = incoming
            .collect()
            .await
            .map_err(|e| transport_error(&e))?
            .to_bytes();
        Ok(RawResponse {
            status: parts.status,
            headers: parts.headers,
            body,
        })
    }

    /// JSON in, JSON out. A non-2xx answer becomes the error the backend sent.
    pub async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T, ApiError> {
        let bytes = body
            .map(|b| serde_json::to_vec(b).map(Bytes::from))
            .transpose()
            .map_err(|e| {
                client_error(
                    ErrorCode::Failed,
                    "badRequest",
                    format!("Rain could not write the request body: {e}"),
                )
            })?;
        let raw = self
            .request(method, path, bytes, &[("accept", "application/json")])
            .await?
            .into_success()?;
        parse_body(&raw.body)
    }

    /// The generic call behind the shell's `api_request` command. Returns the JSON of the answer
    /// (`null` for an empty body). Every failure is an `ApiError`.
    pub async fn api_request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, ApiError> {
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| {
            client_error(
                ErrorCode::Failed,
                "badRequest",
                format!("'{method}' is not an HTTP method"),
            )
        })?;
        let bytes = body.map(|value| Bytes::from(value.to_string()));
        let raw = self
            .request(method, path, bytes, &[("accept", "application/json")])
            .await?
            .into_success()?;
        if raw.body.is_empty() {
            return Ok(Value::Null);
        }
        parse_body(&raw.body)
    }
}

fn parse_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| {
        client_error(
            ErrorCode::Failed,
            "badResponse",
            "The Rain backend gave an answer that is not valid JSON",
        )
    })
}

/// True while a pipe of this name exists, so a backend (or something else) listens on it.
///
/// Looking at the pipe's attributes does not take one of its instances, unlike connecting to it.
/// A pipe that exists but refuses us still counts as existing.
pub fn pipe_exists(pipe_name: &str) -> bool {
    match std::fs::metadata(pipe_path(pipe_name)) {
        Ok(_) => true,
        Err(e) => e.kind() != io::ErrorKind::NotFound,
    }
}
