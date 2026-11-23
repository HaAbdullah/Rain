//! `GET /machine` on a real backend, with a fake idle source (feature section 05).

mod common;

use std::sync::Arc;

use common::TestBackend;
use hyper::StatusCode;
use rain_backend::modules::machine::IdleSource;
use rain_types::MachineState;
use serde_json::{Value, json};

struct FakeIdle(Option<u64>);

impl IdleSource for FakeIdle {
    fn idle_ms(&self) -> Option<u64> {
        self.0
    }
}

async fn start(idle: Option<u64>) -> TestBackend {
    TestBackend::builder()
        .with::<dyn IdleSource>(Arc::new(FakeIdle(idle)))
        .start()
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_who_just_typed_is_active_with_no_idle_minutes() {
    let backend = start(Some(0)).await;
    let state: MachineState = backend.get("/machine").await;
    assert_eq!(
        serde_json::to_value(state).unwrap(),
        json!({ "userActive": true, "idleMinutes": 0 })
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_a_minute_without_input_the_user_is_idle() {
    let backend = start(Some(61_000)).await;
    let state: MachineState = backend.get("/machine").await;
    assert_eq!(
        serde_json::to_value(state).unwrap(),
        json!({ "userActive": false, "idleMinutes": 1 })
    );
    // The idle jobs read the same number through the service, not through the route.
    assert_eq!(backend.state.machine().idle_ms(), Some(61_000));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn when_windows_refuses_the_route_is_unavailable_and_the_service_says_not_idle() {
    let backend = start(None).await;
    let (status, body): (StatusCode, Value) = backend.send("GET", "/machine", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["reason"], "idleUnavailable");
    assert_eq!(body["code"], "unavailable");
    assert_eq!(backend.state.machine().idle_ms(), None);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_source_answers_with_a_reading() {
    let backend = TestBackend::start().await;
    let (status, body) = backend.send("GET", "/machine", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["idleMinutes"].is_u64());
    backend.stop().await;
}
