//! Serving the router on a Windows named pipe. Foundation section 3.3.
//! Copied in shape from the spike `types-stream` (`server.rs`).
//!
//! axum cannot serve a named pipe by itself, so this uses hyper directly.
//!
//! A pipe has no listen backlog: every client takes one pipe INSTANCE, and a client that finds no
//! free instance gets `ERROR_PIPE_BUSY`. So the server keeps several instances waiting, one for
//! each acceptor task, and each acceptor makes its replacement the moment a client has connected.

use std::io;
use std::sync::Arc;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::Limits;
use crate::platform::pipe_acl::PipeSecurity;

/// The pipe, created but not yet served.
pub struct PipeListener {
    name: String,
    security: Arc<PipeSecurity>,
    pending: Vec<NamedPipeServer>,
}

impl PipeListener {
    /// Creates `acceptors` pipe instances, the first one with `first_pipe_instance(true)`.
    /// Fails with "access denied" (os error 5) when the name exists: another backend runs.
    pub fn bind(name: &str, acceptors: usize) -> io::Result<Self> {
        let security = Arc::new(PipeSecurity::current_user_only()?);
        let mut pending = vec![security.create(name, true)?];
        for _ in 1..acceptors {
            pending.push(security.create(name, false)?);
        }
        Ok(Self {
            name: name.to_owned(),
            security,
            pending,
        })
    }
}

/// One acceptor: waits for a client on its instance, creates the replacement instance at once,
/// and hands the connected instance to the serve loop.
async fn acceptor(
    mut next: NamedPipeServer,
    name: String,
    security: Arc<PipeSecurity>,
    connected: mpsc::Sender<NamedPipeServer>,
    stop: CancellationToken,
) {
    loop {
        tokio::select! {
            result = next.connect() => {
                if let Err(e) = result {
                    error!(error = %e, "waiting for a client on the pipe failed; this acceptor stops");
                    return;
                }
            }
            () = stop.cancelled() => return,
        }
        let fresh = match security.create(&name, false) {
            Ok(fresh) => fresh,
            Err(e) => {
                error!(error = %e, "could not create the next pipe instance; this acceptor stops");
                return;
            }
        };
        let client = std::mem::replace(&mut next, fresh);
        if connected.send(client).await.is_err() {
            return;
        }
    }
}

/// Serves until `stop` is cancelled, or until no acceptor is left. Then: stops accepting (the pipe name disappears, so new
/// clients get "file not found"), lets open requests finish for at most `limits.shutdown_grace`,
/// and ends whatever is still open.
pub async fn serve(listener: PipeListener, app: Router, stop: CancellationToken, limits: Limits) {
    let (connected_tx, mut connected_rx) = mpsc::channel::<NamedPipeServer>(listener.pending.len());
    let mut acceptors = JoinSet::new();
    for pending in listener.pending {
        acceptors.spawn(acceptor(
            pending,
            listener.name.clone(),
            listener.security.clone(),
            connected_tx.clone(),
            stop.clone(),
        ));
    }
    drop(connected_tx);

    let mut connections: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            Some(pipe) = connected_rx.recv() => {
                // Count only the connections that are still open: collect the finished ones first.
                while connections.try_join_next().is_some() {}
                if connections.len() >= limits.max_connections {
                    // Dropping the pipe closes it: the client sees the connection end at once.
                    warn!(open = connections.len(), "too many connections; one was closed");
                    continue;
                }
                connections.spawn(serve_connection(pipe, app.clone(), limits.header_read_timeout, stop.clone()));
            }
            () = stop.cancelled() => break,
            // Collect the connections that ended, so the set does not grow without end.
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            // An acceptor that ended without a stop has logged why. Without any acceptor nobody
            // can connect any more, so the loop ends and the caller stops the process (a zombie
            // that holds no pipe name would let a second backend start).
            Some(_) = acceptors.join_next(), if !stop.is_cancelled() && !acceptors.is_empty() => {
                if acceptors.is_empty() {
                    error!("no pipe acceptor is left; the server ends");
                    break;
                }
            }
        }
    }

    // Closing the receiver wakes an acceptor that waits to hand over a client, so it can end.
    drop(connected_rx);
    while acceptors.join_next().await.is_some() {}
    let drained = tokio::time::timeout(limits.shutdown_grace, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        warn!(
            open = connections.len(),
            "grace period over; the remaining connections are ended"
        );
        connections.abort_all();
    }
    info!("pipe closed");
}

/// Serves one client until it leaves or `stop` is cancelled.
async fn serve_connection(
    pipe: NamedPipeServer,
    app: Router,
    header_read_timeout: std::time::Duration,
    stop: CancellationToken,
) {
    let connection = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(header_read_timeout)
        .serve_connection(TokioIo::new(pipe), TowerToHyperService::new(app));
    tokio::pin!(connection);
    let result = tokio::select! {
        result = connection.as_mut() => result,
        () = stop.cancelled() => {
            // Finish the request that is in flight, and refuse new ones on this connection.
            connection.as_mut().graceful_shutdown();
            connection.await
        }
    };
    if let Err(e) = result {
        debug!(error = %e, "connection ended with an error");
    }
}
