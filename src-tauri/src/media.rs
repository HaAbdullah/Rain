//! The custom scheme `rain-media`: how the page loads icons and covers (foundation sections 4.7
//! and 6.3).
//!
//! JSON never carries image bytes. The backend sends a reference such as
//! `media/app-icon/app-vscode?v=3fa9`; the frontend turns it into
//! `http://rain-media.localhost/media/app-icon/app-vscode?v=3fa9`; Windows hands that request to
//! the handler here; the handler asks the backend for `GET /api/v1/media/...` over the pipe and
//! gives the answer back to the web view. The shell never reads a file itself: the backend decides
//! what may be read, and the shell never learns the layout of its cache.
//!
//! The scheme must not change. Tauri derives the web view's origin from it, and with it where the
//! page's localStorage lives (the screens keep their selections there). `useHttpsScheme` in
//! `tauri.conf.json` stays at its default, `false`.
//!
//! `MediaFetcher::fetch` is the whole logic and uses no Tauri type, so tests run it against a
//! stand-in backend on a test pipe. `handle_scheme` at the bottom only converts types.

use std::sync::Arc;

use hyper::Method;
use hyper::body::Bytes;
use rain_pipe::PipeClient;
use rain_types::limits::{API_PREFIX, BRIDGE_TIMEOUT, RESPONSE_MAX, URL_MAX};
use tauri::http;
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};
use tokio::sync::Semaphore;
use tracing::debug;

/// The scheme name. The page builds its URLs with it (`mediaUrl` in `src/shell/tauri-bridge.ts`).
pub const SCHEME: &str = "rain-media";

/// Every request path starts with this, and it is also the start of the route in the backend.
const MEDIA_PREFIX: &str = "/media/";

/// Icons and covers that are fetched from the backend at the same moment. A screen with 100 icons
/// asks for all of them at once; without a limit they would fill the pipe and make the calls of
/// the page wait. The calls of the page have their own limit (`CLIENT_MAX_INFLIGHT`).
const MEDIA_MAX_INFLIGHT: usize = 8;

/// The headers of the backend's answer that the web view needs. The others (date, length) are
/// the HTTP library's business.
const COPIED_HEADERS: [&str; 3] = ["content-type", "cache-control", "etag"];

/// What the web view asked for, reduced to what the handler looks at.
#[derive(Debug, Clone, Copy)]
pub struct MediaRequest<'a> {
    pub method: &'a str,
    /// The path of the URL, such as `/media/app-icon/app-vscode`.
    pub path: &'a str,
    /// The query without the `?`, such as `v=3fa9`.
    pub query: Option<&'a str>,
    /// The `ETag` the web view already holds, from its `If-None-Match` header.
    pub if_none_match: Option<&'a str>,
}

/// What goes back to the web view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAnswer {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Bytes,
}

impl MediaAnswer {
    /// The answer to everything that goes wrong: nothing to show. An `<img>` then draws its
    /// fallback (the screens show a letter tile).
    fn not_found() -> Self {
        Self {
            status: 404,
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }
}

/// The path and query to ask the backend for, or `None` when the request is not a plain read of
/// `/media/{kind}/{id}`. Nothing the page can write into a URL may lead anywhere else.
fn backend_path(request: &MediaRequest) -> Option<String> {
    if request.method != "GET" {
        return None;
    }
    let tricks = ["..", "\\", "//", "://", "#"];
    let clean = |text: &str| !text.chars().any(char::is_control) && !text.contains('#');
    let path_ok = request.path.len() <= URL_MAX
        && clean(request.path)
        && !tricks.iter().any(|trick| request.path.contains(trick));
    let query_ok = request
        .query
        .is_none_or(|query| query.len() <= URL_MAX && clean(query));
    if !path_ok || !query_ok {
        return None;
    }
    // Exactly `kind/id`: the id is one path segment (foundation section 4.7).
    let mut segments = request.path.strip_prefix(MEDIA_PREFIX)?.split('/');
    let (kind, id, rest) = (segments.next()?, segments.next()?, segments.next());
    if kind.is_empty() || id.is_empty() || rest.is_some() {
        return None;
    }
    let mut path = format!("{API_PREFIX}{MEDIA_PREFIX}{kind}/{id}");
    if let Some(query) = request.query.filter(|query| !query.is_empty()) {
        path.push('?');
        path.push_str(query);
    }
    Some(path)
}

/// Fetches media from the backend. Cheap to clone: all clones share one set of places.
#[derive(Clone)]
pub struct MediaFetcher {
    client: PipeClient,
    places: Arc<Semaphore>,
}

impl MediaFetcher {
    pub fn new(client: PipeClient) -> Self {
        Self {
            client,
            places: Arc::new(Semaphore::new(MEDIA_MAX_INFLIGHT)),
        }
    }

    /// Answers one request. Never fails: whatever goes wrong is a 404 with an empty body, because
    /// an image that cannot be shown is not an error for the user. It never starts the backend:
    /// the page's own calls do that, and an icon is not worth a start.
    pub async fn fetch(&self, request: MediaRequest<'_>) -> MediaAnswer {
        let Some(path) = backend_path(&request) else {
            debug!(path = request.path, "refused a media request");
            return MediaAnswer::not_found();
        };
        let Ok(_place) = self.places.acquire().await else {
            return MediaAnswer::not_found();
        };
        let mut headers = vec![("accept", "image/*")];
        if let Some(tag) = request.if_none_match {
            headers.push(("if-none-match", tag));
        }
        let call = self.client.request(Method::GET, &path, None, &headers);
        match tokio::time::timeout(BRIDGE_TIMEOUT, call).await {
            Ok(Ok(raw)) => answer_from(raw.status.as_u16(), &raw.headers, raw.body),
            Ok(Err(error)) => {
                debug!(message = %error.message, "the backend could not give media");
                MediaAnswer::not_found()
            }
            Err(_) => {
                debug!("the backend did not give media in time");
                MediaAnswer::not_found()
            }
        }
    }
}

/// Looks at the status BEFORE it treats the body as an image: a missing image is a 404 with a JSON
/// error body, which must never reach an `<img>` as if it were a picture.
fn answer_from(status: u16, headers: &hyper::HeaderMap, body: Bytes) -> MediaAnswer {
    let body = match status {
        200 if body.len() <= RESPONSE_MAX => body,
        304 => Bytes::new(),
        _ => return MediaAnswer::not_found(),
    };
    let mut answer = MediaAnswer {
        status,
        headers: Vec::new(),
        body,
    };
    for name in COPIED_HEADERS {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            answer.headers.push((name, value.to_owned()));
        }
    }
    // The bytes are shown as the image type the backend named, never guessed from their content.
    answer
        .headers
        .push(("x-content-type-options", "nosniff".to_owned()));
    // The page (`http://tauri.localhost`, or the dev server) is another origin than this scheme.
    // An `<img>` does not need this; a `fetch` or a canvas read would.
    answer
        .headers
        .push(("access-control-allow-origin", "*".to_owned()));
    answer
}

/// The handler Tauri calls for every request to `rain-media://`. Only converts types; the work
/// is `MediaFetcher::fetch`.
pub fn handle_scheme<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let Some(fetcher) = context.app_handle().try_state::<MediaFetcher>() else {
        responder.respond(to_response(MediaAnswer::not_found()));
        return;
    };
    let fetcher = fetcher.inner().clone();
    tauri::async_runtime::spawn(async move {
        let uri = request.uri();
        let if_none_match = request
            .headers()
            .get(http::header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok());
        let answer = fetcher
            .fetch(MediaRequest {
                method: request.method().as_str(),
                path: uri.path(),
                query: uri.query(),
                if_none_match,
            })
            .await;
        responder.respond(to_response(answer));
    });
}

fn to_response(answer: MediaAnswer) -> http::Response<Vec<u8>> {
    let mut response = http::Response::builder().status(answer.status);
    for (name, value) in &answer.headers {
        response = response.header(*name, value);
    }
    response
        .body(answer.body.to_vec())
        .unwrap_or_else(|_| http::Response::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{FakeServer, Reply};

    const PNG: &[u8] = b"\x89PNG fake image bytes";

    fn get<'a>(path: &'a str, query: Option<&'a str>) -> MediaRequest<'a> {
        MediaRequest {
            method: "GET",
            path,
            query,
            if_none_match: None,
        }
    }

    fn header<'a>(answer: &'a MediaAnswer, name: &str) -> Option<&'a str> {
        answer
            .headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    fn image_reply() -> Reply {
        Reply::new(200, PNG)
            .header("content-type", "image/png")
            .header("etag", "\"app-icon-app-vscode-3fa9\"")
            .header("cache-control", "public, max-age=31536000, immutable")
    }

    // ---- which requests go to the backend ----

    #[test]
    fn a_plain_media_path_goes_to_the_media_route() {
        assert_eq!(
            backend_path(&get("/media/app-icon/app-vscode", Some("v=3fa9"))).as_deref(),
            Some("/api/v1/media/app-icon/app-vscode?v=3fa9")
        );
        assert_eq!(
            backend_path(&get("/media/game-cover/steam-220", None)).as_deref(),
            Some("/api/v1/media/game-cover/steam-220")
        );
        assert_eq!(
            backend_path(&get("/media/game-cover/steam-220", Some(""))).as_deref(),
            Some("/api/v1/media/game-cover/steam-220")
        );
    }

    #[test]
    fn an_encoded_id_is_passed_on_as_it_is() {
        assert_eq!(
            backend_path(&get("/media/app-icon/a%20b%2Fc", None)).as_deref(),
            Some("/api/v1/media/app-icon/a%20b%2Fc")
        );
    }

    #[test]
    fn only_get_is_served() {
        for method in ["POST", "PUT", "DELETE", "HEAD", "OPTIONS", "get"] {
            let request = MediaRequest {
                method,
                ..get("/media/app-icon/x", None)
            };
            assert_eq!(backend_path(&request), None, "{method}");
        }
    }

    #[test]
    fn anything_but_a_media_path_is_refused() {
        for path in [
            "",
            "/",
            "/media",
            "/media/",
            "/settings",
            "/api/v1/settings",
            "/internal/shutdown",
            "/medias/app-icon/x",
            "media/app-icon/x",
        ] {
            assert_eq!(backend_path(&get(path, None)), None, "{path}");
        }
    }

    #[test]
    fn a_media_path_must_be_exactly_kind_and_id() {
        for path in [
            "/media/app-icon",
            "/media/app-icon/",
            "/media//x",
            "/media/app-icon/a/b",
            "/media/app-icon/x/",
        ] {
            assert_eq!(backend_path(&get(path, None)), None, "{path}");
        }
    }

    #[test]
    fn a_path_that_could_change_the_destination_is_refused() {
        for path in [
            "/media/../settings",
            "/media/app-icon/..",
            "/media/app-icon/a\\b",
            "/media/app-icon/http://evil.example",
            "/media/app-icon/x\n",
            "/media/app-icon/x#top",
            "/media/app-icon/x\0",
        ] {
            assert_eq!(backend_path(&get(path, None)), None, "{path:?}");
        }
        assert_eq!(backend_path(&get("/media/a/b", Some("v=1\r\nx: y"))), None);
        assert_eq!(backend_path(&get("/media/a/b", Some("v=1#x"))), None);
    }

    #[test]
    fn a_path_over_2048_bytes_is_refused() {
        let long = format!("/media/app-icon/{}", "a".repeat(URL_MAX));
        assert_eq!(backend_path(&get(&long, None)), None);
        let query = "v".repeat(URL_MAX + 1);
        assert_eq!(backend_path(&get("/media/a/b", Some(&query))), None);
    }

    // ---- what the backend's answers become ----

    #[tokio::test]
    async fn an_image_is_passed_on_with_its_headers() {
        let server = FakeServer::start(|_| image_reply());
        let fetcher = MediaFetcher::new(server.client());

        let answer = fetcher
            .fetch(get("/media/app-icon/app-vscode", Some("v=3fa9")))
            .await;

        assert_eq!(answer.status, 200);
        assert_eq!(answer.body, PNG);
        assert_eq!(header(&answer, "content-type"), Some("image/png"));
        assert_eq!(
            header(&answer, "cache-control"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(
            header(&answer, "etag"),
            Some("\"app-icon-app-vscode-3fa9\"")
        );
        assert_eq!(header(&answer, "x-content-type-options"), Some("nosniff"));
        assert_eq!(header(&answer, "access-control-allow-origin"), Some("*"));
        let seen = server.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "GET");
        assert_eq!(
            seen[0].path_and_query,
            "/api/v1/media/app-icon/app-vscode?v=3fa9"
        );
    }

    #[tokio::test]
    async fn a_matching_etag_is_sent_on_and_the_304_comes_back() {
        let server = FakeServer::start(|seen| {
            if seen.header("if-none-match") == Some("\"abc\"") {
                Reply::new(304, b"")
                    .header("etag", "\"abc\"")
                    .header("cache-control", "private, max-age=86400")
            } else {
                image_reply()
            }
        });
        let fetcher = MediaFetcher::new(server.client());

        let answer = fetcher
            .fetch(MediaRequest {
                if_none_match: Some("\"abc\""),
                ..get("/media/app-icon/x", None)
            })
            .await;

        assert_eq!(answer.status, 304);
        assert!(answer.body.is_empty());
        assert_eq!(header(&answer, "etag"), Some("\"abc\""));
        assert_eq!(
            header(&answer, "cache-control"),
            Some("private, max-age=86400")
        );
    }

    #[tokio::test]
    async fn a_missing_image_is_a_404_without_a_body() {
        let server = FakeServer::start(|_| {
            Reply::new(
                404,
                br#"{"code":"notFound","message":"No such image","reason":"noMedia"}"#,
            )
            .header("content-type", "application/json")
        });
        let fetcher = MediaFetcher::new(server.client());

        let answer = fetcher.fetch(get("/media/app-icon/nobody", None)).await;

        assert_eq!(answer, MediaAnswer::not_found());
    }

    #[tokio::test]
    async fn any_answer_but_200_and_304_is_a_404() {
        for status in [201, 204, 301, 400, 409, 500, 503] {
            let server = FakeServer::start(move |_| Reply::new(status, b"x"));
            let fetcher = MediaFetcher::new(server.client());
            let answer = fetcher.fetch(get("/media/app-icon/x", None)).await;
            assert_eq!(answer.status, 404, "backend answered {status}");
            assert!(answer.body.is_empty() && answer.headers.is_empty());
        }
    }

    #[tokio::test]
    async fn without_a_backend_the_answer_is_a_404_and_nothing_is_started() {
        let server = FakeServer::start(|_| image_reply());
        let fetcher = MediaFetcher::new(server.client());
        drop(server); // the pipe is gone

        let answer = fetcher.fetch(get("/media/app-icon/x", None)).await;

        assert_eq!(answer, MediaAnswer::not_found());
    }

    #[tokio::test]
    async fn a_refused_request_never_reaches_the_backend() {
        let server = FakeServer::start(|_| image_reply());
        let fetcher = MediaFetcher::new(server.client());

        for path in ["/settings", "/media/../settings", "/media/app-icon"] {
            assert_eq!(fetcher.fetch(get(path, None)).await.status, 404);
        }
        let post = MediaRequest {
            method: "POST",
            ..get("/media/app-icon/x", None)
        };
        assert_eq!(fetcher.fetch(post).await.status, 404);

        assert!(server.seen().is_empty());
    }

    #[tokio::test]
    async fn an_answer_over_16_mib_is_not_passed_on() {
        let big = vec![0_u8; RESPONSE_MAX + 1];
        let server = FakeServer::start(move |_| Reply::new(200, &big));
        let fetcher = MediaFetcher::new(server.client());

        let answer = fetcher.fetch(get("/media/game-cover/huge", None)).await;

        assert_eq!(answer.status, 404);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_burst_of_requests_is_all_answered() {
        let server = FakeServer::start(|_| image_reply());
        let fetcher = MediaFetcher::new(server.client());

        let answers = futures_util::future::join_all((0..40).map(|number| {
            let fetcher = fetcher.clone();
            async move {
                let path = format!("/media/app-icon/app-{number}");
                fetcher.fetch(get(&path, None)).await.status
            }
        }))
        .await;

        assert!(answers.iter().all(|status| *status == 200));
        assert_eq!(server.seen().len(), 40);
    }

    // ---- the settings that must never change ----

    #[test]
    fn the_scheme_is_the_one_the_page_builds_its_urls_with() {
        assert_eq!(SCHEME, "rain-media");
        let page = include_str!("../../src/shell/tauri-bridge.ts");
        assert!(
            page.contains("http://rain-media.localhost"),
            "mediaUrl in src/shell/tauri-bridge.ts must build the http form of the scheme"
        );
    }

    #[test]
    fn the_window_keeps_the_http_scheme_forever() {
        // `useHttpsScheme` moves the web view's localStorage and IndexedDB (foundation 6.3).
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        for window in config["app"]["windows"].as_array().unwrap() {
            assert_ne!(window["useHttpsScheme"], serde_json::json!(true));
        }
    }
}
