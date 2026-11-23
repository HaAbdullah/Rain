//! `GET /media/{kind}/{id}`: images with an `ETag`, and `304` before any bytes are produced.
//! Foundation section 4.7. The provider is a test provider; no production kind exists yet.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Bytes;
use common::TestBackend;
use futures_util::future::BoxFuture;
use hyper::{Method, StatusCode};
use rain_backend::core::error::ApiResult;
use rain_backend::core::media::{Media, MediaProvider};
use rain_types::limits::API_PREFIX;

/// Serves the text `image of <id>` as a PNG and counts how often it was asked.
#[derive(Default)]
struct TestImages {
    calls: AtomicUsize,
}

impl MediaProvider for TestImages {
    fn get(&self, id: &str) -> BoxFuture<'_, ApiResult<Option<Media>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let id = id.to_owned();
        Box::pin(async move {
            Ok((id != "missing").then(|| Media {
                bytes: Bytes::from(format!("image of {id}")),
                content_type: "image/png",
                version: "v7".to_owned(),
            }))
        })
    }
}

struct Answer {
    status: StatusCode,
    headers: hyper::HeaderMap,
    body: Bytes,
}

async fn get(backend: &TestBackend, path: &str, if_none_match: Option<&str>) -> Answer {
    let mut headers = Vec::new();
    if let Some(tag) = if_none_match {
        headers.push(("if-none-match", tag));
    }
    let raw = backend
        .client
        .request(Method::GET, &format!("{API_PREFIX}{path}"), None, &headers)
        .await
        .expect("the backend answered");
    Answer {
        status: raw.status,
        headers: raw.headers,
        body: raw.body,
    }
}

fn header<'a>(answer: &'a Answer, name: &str) -> &'a str {
    answer
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

async fn backend_with_images() -> (TestBackend, Arc<TestImages>) {
    let backend = TestBackend::start().await;
    let images = Arc::new(TestImages::default());
    backend.state.media.register("test-image", images.clone());
    (backend, images)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_image_is_served_with_its_type_length_etag_and_a_day_of_private_caching() {
    let (backend, _) = backend_with_images().await;
    let answer = get(&backend, "/media/test-image/logo", None).await;

    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(&answer.body[..], b"image of logo");
    assert_eq!(header(&answer, "content-type"), "image/png");
    assert_eq!(header(&answer, "content-length"), "13");
    assert_eq!(header(&answer, "x-content-type-options"), "nosniff");
    assert_eq!(header(&answer, "etag"), "\"test-image-logo-v7\"");
    assert_eq!(header(&answer, "cache-control"), "private, max-age=86400");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_with_a_version_may_be_cached_for_ever() {
    let (backend, _) = backend_with_images().await;
    let answer = get(&backend, "/media/test-image/logo?v=3fa9", None).await;

    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(header(&answer, "etag"), "\"test-image-logo-3fa9\"");
    assert_eq!(
        header(&answer, "cache-control"),
        "public, max-age=31536000, immutable"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_a_version_a_matching_etag_is_a_304_without_asking_the_provider() {
    let (backend, images) = backend_with_images().await;
    let answer = get(
        &backend,
        "/media/test-image/logo?v=3fa9",
        Some("\"test-image-logo-3fa9\""),
    )
    .await;

    assert_eq!(answer.status, StatusCode::NOT_MODIFIED);
    assert!(answer.body.is_empty());
    assert_eq!(header(&answer, "etag"), "\"test-image-logo-3fa9\"");
    assert_eq!(
        images.calls.load(Ordering::SeqCst),
        0,
        "the 304 was decided before any image was produced"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_version_the_provider_is_asked_and_its_version_makes_the_etag() {
    let (backend, images) = backend_with_images().await;
    let tag = "\"test-image-logo-v7\"";
    let answer = get(&backend, "/media/test-image/logo", Some(tag)).await;

    assert_eq!(answer.status, StatusCode::NOT_MODIFIED);
    assert_eq!(header(&answer, "cache-control"), "private, max-age=86400");
    assert_eq!(images.calls.load(Ordering::SeqCst), 1);

    let changed = get(
        &backend,
        "/media/test-image/logo",
        Some("\"test-image-logo-v6\""),
    )
    .await;
    assert_eq!(
        changed.status,
        StatusCode::OK,
        "another version: the bytes again"
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_image_and_an_unknown_kind_are_404_with_the_json_error_body() {
    let (backend, _) = backend_with_images().await;
    for path in [
        "/media/test-image/missing",
        "/media/no-such-kind/logo",
        "/media/test-image/bad%20id",
    ] {
        let answer = get(&backend, path, None).await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            header(&answer, "content-type").starts_with("application/json"),
            "{path}"
        );
        let body: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(body["code"], "notFound");
        assert_eq!(body["source"], "media");
    }
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_version_that_cannot_go_into_a_header_is_ignored_and_the_image_still_loads() {
    let (backend, _) = backend_with_images().await;
    let answer = get(&backend, "/media/test-image/logo?v=a%20b", None).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(header(&answer, "etag"), "\"test-image-logo-v7\"");
    backend.stop().await;
}
