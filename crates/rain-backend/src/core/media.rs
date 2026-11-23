//! Binary resources (icons, covers): who provides them. Foundation section 4.7.
//!
//! JSON never carries image bytes. A module that has images registers one `MediaProvider` for a
//! kind (`app-icon`, `game-cover`, ...), and the route `GET /media/{kind}/{id}` (see
//! `server/media.rs`) asks it. The route does the caching headers and the `304` answers, so a
//! provider only says: here are the bytes of this id, or there are none.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use axum::body::Bytes;
use futures_util::future::BoxFuture;

use super::error::ApiResult;

/// One image, as a provider returns it.
pub struct Media {
    pub bytes: Bytes,
    /// For example `image/png`.
    pub content_type: &'static str,
    /// Changes when the image changes (a short hash, or the file's modification time in hex). It
    /// becomes part of the `ETag`.
    pub version: String,
}

pub trait MediaProvider: Send + Sync {
    /// The image of `id`, or `None` for an unknown id. A provider that reads files does it on a
    /// blocking thread (`core::blocking::run`).
    fn get(&self, id: &str) -> BoxFuture<'_, ApiResult<Option<Media>>>;
}

/// The providers by kind. `AppState` holds one registry.
#[derive(Default)]
pub struct MediaRegistry {
    providers: RwLock<HashMap<&'static str, Arc<dyn MediaProvider>>>,
}

impl MediaRegistry {
    /// One line per kind, in the module's `init`: `state.media.register("app-icon", provider)`.
    /// A kind that is registered twice is a bug in the code that registers it.
    pub fn register(&self, kind: &'static str, provider: Arc<dyn MediaProvider>) {
        let replaced = self
            .providers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(kind, provider);
        assert!(
            replaced.is_none(),
            "the media kind {kind} was registered twice"
        );
    }

    pub fn provider(&self, kind: &str) -> Option<Arc<dyn MediaProvider>> {
        self.providers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(kind)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;

    impl MediaProvider for Fixed {
        fn get(&self, _id: &str) -> BoxFuture<'_, ApiResult<Option<Media>>> {
            Box::pin(async { Ok(None) })
        }
    }

    #[test]
    fn a_registered_kind_is_found_and_another_is_not() {
        let registry = MediaRegistry::default();
        registry.register("app-icon", Arc::new(Fixed));
        assert!(registry.provider("app-icon").is_some());
        assert!(registry.provider("game-cover").is_none());
    }

    #[test]
    #[should_panic(expected = "registered twice")]
    fn a_kind_cannot_be_registered_twice() {
        let registry = MediaRegistry::default();
        registry.register("app-icon", Arc::new(Fixed));
        registry.register("app-icon", Arc::new(Fixed));
    }
}
