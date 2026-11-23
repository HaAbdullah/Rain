//! App icons: drawn by Windows on demand, kept as PNG files, served as the media kind
//! `app-icon`. Section 01 5 (cache folder), 6.2 (the address) and 8 (errors).
//!
//! An icon is drawn the first time somebody asks for it (1 to 65 ms on the COM thread), written
//! to `<local>\cache\launcher\icons\<hex16>.png` and read from there afterwards. The name of the
//! file is a hash of the app key, the modified time of its program and `ICON_RENDER_VERSION`, so
//! a changed program or a new version gives a new file and a new address. The bytes are never
//! held in memory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use futures_util::future::BoxFuture;

use super::apps::AppRecord;
use super::limits::ICON_PX;
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::media::{Media, MediaProvider};
use crate::core::state::WeakAppState;
use crate::platform::icons::{self, IconSource};
use rain_types::AppKind;

/// Draws the icon of an app. A trait, so that a test says what is drawn and counts the calls.
pub trait IconRenderer: Send + Sync {
    /// The PNG, or `None` when Windows cannot draw one.
    fn render<'a>(&'a self, app: &'a AppRecord) -> BoxFuture<'a, Option<Vec<u8>>>;
}

/// The real renderer: the icon that Windows shows for the item in `shell:AppsFolder`, else the
/// icon of the program file.
pub struct ShellIconRenderer;

impl IconRenderer for ShellIconRenderer {
    fn render<'a>(&'a self, app: &'a AppRecord) -> BoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            if !app.aumid.is_empty()
                && let Ok(png) = icons::icon_png(IconSource::AppId(&app.aumid), ICON_PX).await
            {
                return Some(png);
            }
            if app.kind == AppKind::Program && !app.exe_path.is_empty() {
                return icons::icon_png(IconSource::File(Path::new(&app.exe_path)), ICON_PX)
                    .await
                    .ok();
            }
            None
        })
    }
}

/// The cache of drawn icons and the memory of the icons that cannot be drawn.
pub struct Icons {
    folder: PathBuf,
    renderer: Arc<dyn IconRenderer>,
    /// Ids that could not be drawn. Cleared at every scan, so a later try may work.
    no_icon: Mutex<HashSet<String>>,
}

impl Icons {
    pub fn new(local_dir: &Path, renderer: Arc<dyn IconRenderer>) -> Self {
        Self {
            folder: local_dir.join("cache").join("launcher").join("icons"),
            renderer,
            no_icon: Mutex::default(),
        }
    }

    fn no_icon(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.no_icon.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The icon of `app`: from the file when it is there, else drawn and written.
    pub async fn get(&self, app: &AppRecord) -> ApiResult<Media> {
        if self.no_icon().contains(&app.id) {
            return Err(no_icon());
        }
        let file = self.folder.join(&app.icon_file);
        let wanted = file.clone();
        let cached = blocking::run(move || std::fs::read(&wanted).ok()).await?;
        let bytes = match cached {
            Some(bytes) => bytes,
            None => {
                let Some(png) = self.renderer.render(app).await else {
                    self.no_icon().insert(app.id.clone());
                    return Err(no_icon());
                };
                let (folder, to_write) = (self.folder.clone(), png.clone());
                // A file that cannot be written only costs a new drawing next time.
                let _ = blocking::run(move || {
                    std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&file, to_write))
                })
                .await?;
                png
            }
        };
        Ok(Media {
            bytes: Bytes::from(bytes),
            content_type: "image/png",
            version: app.icon_version.clone(),
        })
    }

    /// After a scan: the files that no app refers to any more go, and the apps that could not be
    /// drawn may be tried again. At most as many files stay as there are apps.
    pub async fn clean(&self, apps: &[AppRecord]) {
        self.no_icon().clear();
        let keep: HashSet<String> = apps.iter().map(|app| app.icon_file.clone()).collect();
        let folder = self.folder.clone();
        // A folder that cannot be read has nothing to clean.
        let _ = blocking::run(move || {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                return;
            };
            for entry in entries.flatten() {
                if !keep.contains(entry.file_name().to_string_lossy().as_ref()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        })
        .await;
    }

    #[cfg(test)]
    pub fn folder(&self) -> &Path {
        &self.folder
    }
}

fn no_icon() -> ApiError {
    ApiError::not_found("No icon")
        .with_source("launcher")
        .with_reason("noIcon")
}

/// `GET /media/app-icon/{id}`.
pub struct AppIcons {
    pub state: WeakAppState,
}

impl MediaProvider for AppIcons {
    fn get(&self, id: &str) -> BoxFuture<'_, ApiResult<Option<Media>>> {
        let id = id.to_owned();
        Box::pin(async move {
            let state = self
                .state
                .upgrade()
                .ok_or_else(crate::core::error::shutting_down_error)?;
            state.launcher().icon(&id).await.map(Some)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        calls: AtomicUsize,
        fail: Mutex<bool>,
    }

    impl IconRenderer for Counting {
        fn render<'a>(&'a self, _app: &'a AppRecord) -> BoxFuture<'a, Option<Vec<u8>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let fail = *self.fail.lock().unwrap_or_else(PoisonError::into_inner);
            Box::pin(async move { (!fail).then(|| b"png".to_vec()) })
        }
    }

    fn app(id: &str, file: &str) -> AppRecord {
        AppRecord {
            id: id.into(),
            key: "k".into(),
            aumid: "a".into(),
            name: "N".into(),
            kind: AppKind::Other,
            exe_path: String::new(),
            link_args: String::new(),
            keywords: Vec::new(),
            icon_version: "0a0b0c0d".into(),
            icon_file: file.into(),
        }
    }

    fn setup() -> (tempfile::TempDir, Icons, Arc<Counting>) {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let renderer = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            fail: Mutex::new(false),
        });
        let icons = Icons::new(temp.path(), renderer.clone());
        (temp, icons, renderer)
    }

    #[tokio::test]
    async fn an_icon_is_drawn_once_and_then_read_from_its_file() {
        let (_temp, icons, renderer) = setup();
        let app = app("app-a", "0000000000000001.png");
        let first = icons.get(&app).await.expect("drawn");
        assert_eq!(&first.bytes[..], b"png");
        assert_eq!(first.version, "0a0b0c0d");
        assert!(icons.folder().join("0000000000000001.png").exists());
        icons.get(&app).await.expect("read from the file");
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_icon_that_cannot_be_drawn_is_remembered_until_the_next_clean() {
        let (_temp, icons, renderer) = setup();
        *renderer.fail.lock().unwrap() = true;
        let app = app("app-a", "0000000000000002.png");
        let error = icons.get(&app).await.err().expect("no icon");
        assert_eq!(error.body.reason.as_deref(), Some("noIcon"));
        icons.get(&app).await.err().expect("still no icon");
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 1, "not asked again");
        *renderer.fail.lock().unwrap() = false;
        icons.clean(std::slice::from_ref(&app)).await;
        icons.get(&app).await.expect("a scan gives it another try");
    }

    #[tokio::test]
    async fn cleaning_removes_only_the_files_that_no_app_uses() {
        let (_temp, icons, _renderer) = setup();
        let (kept, gone) = (
            app("app-a", "000000000000000a.png"),
            app("app-b", "000000000000000b.png"),
        );
        icons.get(&kept).await.unwrap();
        icons.get(&gone).await.unwrap();
        icons.clean(std::slice::from_ref(&kept)).await;
        assert!(icons.folder().join("000000000000000a.png").exists());
        assert!(!icons.folder().join("000000000000000b.png").exists());
    }
}
