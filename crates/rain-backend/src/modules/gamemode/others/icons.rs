//! The icons of the programs in the Game mode list, served as the media kind `proc-icon`.
//! Section 06 6.11.
//!
//! The address holds a hash of the program file's path. Every scan tells this provider which
//! paths exist now; an icon is drawn the first time it is asked for and kept in memory until
//! the program leaves the list (at most as many small PNGs as there are programs).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use axum::body::Bytes;
use futures_util::future::BoxFuture;

use super::select::norm;
use crate::core::error::ApiResult;
use crate::core::media::{Media, MediaProvider};
use crate::platform::icons::{self, IconSource};

/// The size that is drawn, like the launcher's.
const ICON_PX: u32 = 64;

#[derive(Default)]
pub struct ProcIcons {
    paths: Mutex<HashMap<String, PathBuf>>,
    drawn: Mutex<HashMap<String, Bytes>>,
}

/// The id of an icon: 16 hex digits of an FNV-1a hash of the normalised path.
pub fn icon_id(image: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in norm(image).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

impl ProcIcons {
    /// The program files of the list that was just built. Icons of programs that left are dropped.
    pub fn remember(&self, images: impl IntoIterator<Item = String>) {
        let paths: HashMap<String, PathBuf> = images
            .into_iter()
            .map(|image| (icon_id(&image), PathBuf::from(image)))
            .collect();
        self.drawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|id, _| paths.contains_key(id));
        *self.paths.lock().unwrap_or_else(PoisonError::into_inner) = paths;
    }
}

impl MediaProvider for ProcIcons {
    fn get(&self, id: &str) -> BoxFuture<'_, ApiResult<Option<Media>>> {
        let id = id.to_owned();
        Box::pin(async move {
            let known = self
                .drawn
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id)
                .cloned();
            let bytes = match known {
                Some(bytes) => bytes,
                None => {
                    let path = self
                        .paths
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&id)
                        .cloned();
                    let Some(path) = path else {
                        return Ok(None);
                    };
                    let Ok(png) = icons::icon_png(IconSource::File(&path), ICON_PX).await else {
                        return Ok(None);
                    };
                    let bytes = Bytes::from(png);
                    self.drawn
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(id.clone(), bytes.clone());
                    bytes
                }
            };
            Ok(Some(Media {
                bytes,
                content_type: "image/png",
                version: id,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_ignores_case_slashes_and_a_long_path_prefix() {
        let id = icon_id(r"C:\Apps\Discord.exe");
        assert_eq!(id.len(), 16);
        assert_eq!(id, icon_id("c:/apps/discord.EXE"));
        assert_eq!(id, icon_id(r"\\?\C:\Apps\Discord.exe"));
        assert_ne!(id, icon_id(r"C:\Apps\Slack.exe"));
    }

    #[tokio::test]
    async fn an_unknown_icon_is_no_image_and_a_forgotten_one_goes() {
        let icons = ProcIcons::default();
        assert!(icons.get("0000000000000000").await.unwrap().is_none());
        icons.remember([r"C:\Does\Not\Exist.exe".to_owned()]);
        // The path is known, the file is not: still no image, and no error.
        let id = icon_id(r"C:\Does\Not\Exist.exe");
        assert!(icons.get(&id).await.unwrap().is_none());
        icons.remember(Vec::new());
        assert!(icons.paths.lock().unwrap().is_empty());
    }
}
