//! `GET /media/{kind}/{id}`: the bytes of an icon or a cover. Foundation section 4.7.
//!
//! The `ETag` decides before any bytes are produced: a request that holds `v=<version>` and a
//! matching `If-None-Match` is answered `304` without calling the provider (measured in the
//! spike: 0.06 ms against 46 ms when the image was encoded first).

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;

use crate::core::error::{ApiError, ApiPath, ApiQuery, ApiResult};
use crate::core::state::AppState;

/// `v` is the content version that the module put into the reference of the image.
#[derive(Deserialize)]
struct MediaQuery {
    v: Option<String>,
}

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "private, max-age=86400";
/// Longest `v` and id that go into an `ETag` (the rule for ids is foundation 4.3).
const PART_MAX: usize = 128;

pub fn router() -> Router<AppState> {
    Router::new().route("/media/{kind}/{id}", get(media))
}

async fn media(
    State(state): State<AppState>,
    ApiPath((kind, id)): ApiPath<(String, String)>,
    ApiQuery(query): ApiQuery<MediaQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let provider = state
        .media
        .provider(&kind)
        .filter(|_| is_plain(&id))
        .ok_or_else(|| not_found("noImage"))?;

    // A version that cannot go into a header is treated as absent: an <img> must still load.
    let version = query.v.filter(|v| is_plain(v));
    if let Some(v) = &version {
        let etag = etag_of(&kind, &id, v);
        if has_etag(&headers, &etag) {
            return Ok(answer(StatusCode::NOT_MODIFIED, &etag, IMMUTABLE, None));
        }
    }
    let image = provider
        .get(&id)
        .await?
        .ok_or_else(|| not_found("noImage"))?;

    let etag = etag_of(&kind, &id, version.as_deref().unwrap_or(&image.version));
    let cache = if version.is_some() {
        IMMUTABLE
    } else {
        REVALIDATE
    };
    if has_etag(&headers, &etag) {
        return Ok(answer(StatusCode::NOT_MODIFIED, &etag, cache, None));
    }
    Ok(answer(
        StatusCode::OK,
        &etag,
        cache,
        Some((image.content_type, image.bytes)),
    ))
}

fn not_found(reason: &'static str) -> ApiError {
    ApiError::not_found("That image does not exist")
        .with_source("media")
        .with_reason(reason)
}

/// Ids and versions are 1 to 128 characters of `A-Za-z0-9._~-`, so they are safe in a header.
fn is_plain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= PART_MAX
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-'))
}

/// A strong `ETag`: the quoted text `kind-id-version`.
fn etag_of(kind: &str, id: &str, version: &str) -> String {
    format!("\"{kind}-{id}-{version}\"")
}

/// Whether `If-None-Match` names this `ETag` (a list, `W/` prefixes and `*` are understood).
fn has_etag(headers: &HeaderMap, etag: &str) -> bool {
    let Some(sent) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    sent.trim() == "*"
        || sent
            .split(',')
            .map(|candidate| candidate.trim().trim_start_matches("W/"))
            .any(|candidate| candidate == etag)
}

fn answer(
    status: StatusCode,
    etag: &str,
    cache: &'static str,
    image: Option<(&'static str, Bytes)>,
) -> Response {
    let mut headers = HeaderMap::new();
    // The kind comes from the registry and the other parts were checked, so this always holds.
    if let Ok(value) = HeaderValue::from_str(etag) {
        headers.insert(ETAG, value);
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    match image {
        Some((content_type, bytes)) => {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
            (status, headers, bytes).into_response()
        }
        None => (status, headers).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_text_goes_into_an_etag() {
        assert!(is_plain("app-vscode"));
        assert!(is_plain("3fa9.x_y~z"));
        assert!(!is_plain(""));
        assert!(!is_plain("a b"));
        assert!(!is_plain("a\"b"));
        assert!(!is_plain(&"x".repeat(129)));
    }

    #[test]
    fn if_none_match_understands_lists_weak_tags_and_star() {
        let etag = etag_of("app-icon", "a", "1");
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(IF_NONE_MATCH, HeaderValue::from_str(value).unwrap());
            headers
        };
        assert!(has_etag(&with(&etag), &etag));
        assert!(has_etag(&with(&format!("W/{etag}")), &etag));
        assert!(has_etag(&with(&format!("\"other\", {etag}")), &etag));
        assert!(has_etag(&with("*"), &etag));
        assert!(!has_etag(&with("\"other\""), &etag));
        assert!(!has_etag(&HeaderMap::new(), &etag));
    }
}
