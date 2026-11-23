//! The pipe itself: one backend for each name (foundation section 2.4) and the access list of
//! section 3.2, read back from a live pipe.

mod common;

use axum::Router;
use common::{TestBackend, path_of};
use rain_backend::config::Config;
use rain_backend::core::state::AppState;
use rain_backend::platform::pipe_acl::{current_user_sid_string, read_back_acl};
use rain_backend::{StartError, start};
use rain_types::Ping;
use tokio::net::windows::named_pipe::ClientOptions;

/// The access mask that Windows writes for "all rights" (`GA` in the SDDL text).
const ALL_RIGHTS: u32 = 0x001F_01FF;
/// The Windows error "access denied".
const ERROR_ACCESS_DENIED: i32 = 5;
/// The well-known SID of the local system account.
const SYSTEM_SID: &str = "S-1-5-18";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_backend_on_the_same_pipe_does_not_start_and_the_first_is_not_disturbed() {
    let backend = TestBackend::start().await;

    let config = Config::for_test(
        &backend.pipe_name,
        backend.data_dir.clone(),
        backend.local_dir.clone(),
    );
    let second = start(AppState::builder(config).build(), Router::new()).await;
    assert!(
        matches!(second, Err(StartError::AlreadyRunning)),
        "the second backend must report that another one runs"
    );

    let ping: Ping = backend.get("/ping").await;
    assert_eq!(
        ping.start_id, backend.state.start_id,
        "the first backend still answers"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_the_first_backend_stopped_the_name_can_be_used_again() {
    let backend = TestBackend::start().await;
    let pipe_name = backend.pipe_name.clone();
    let (data_dir, local_dir) = (backend.data_dir.clone(), backend.local_dir.clone());
    backend.stop().await;

    let config = Config::for_test(&pipe_name, data_dir, local_dir);
    let state = AppState::builder(config).build();
    let again = start(state.clone(), Router::new())
        .await
        .expect("a new backend starts on the free name");
    state.shutdown.request(rain_types::StopReason::Stop);
    again.wait().await.expect("it stops again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_live_pipe_lets_in_only_the_current_user_and_the_system() {
    let backend = TestBackend::start().await;
    let client = ClientOptions::new()
        .open(path_of(&backend.pipe_name))
        .expect("the current user can connect");

    let acl = read_back_acl(&client).expect("the access list can be read from the live pipe");

    // Not `assert_eq!` on the SIDs: a failure message would print a machine identifier.
    assert!(
        acl.sddl.starts_with("D:P"),
        "the list is protected: it inherits nothing"
    );
    assert_eq!(acl.entries.len(), 2, "exactly two entries");
    assert!(
        acl.entries
            .iter()
            .all(|ace| ace.allows && ace.mask == ALL_RIGHTS)
    );
    let me = current_user_sid_string().expect("the SID of this process");
    assert!(
        acl.entries.iter().any(|ace| ace.sid == me),
        "the current user has an entry"
    );
    assert!(
        acl.entries.iter().any(|ace| ace.sid == SYSTEM_SID),
        "the system has an entry"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_from_another_machine_is_refused() {
    let backend = TestBackend::start().await;
    // The same pipe, reached through the network path (two leading backslashes: with one, Windows
    // looks for a file on the current drive and the test would pass for the wrong reason).
    // `reject_remote_clients` refuses it even though the name is `localhost`.
    let through_network = format!(r"\\localhost\pipe\{}", backend.pipe_name);
    let error = ClientOptions::new()
        .open(through_network)
        .expect_err("the network path must be refused");
    assert_eq!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED),
        "refused as access denied, not as not found"
    );
    backend.stop().await;
}
