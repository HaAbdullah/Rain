//! The icon of a local game's program file, shown when the game has no cover. Section 02, 4.3.
//!
//! A local game without a cover borrows the program file as its picture (`library.rs` sets the
//! file as the cover, `covers.rs` serves it). This file makes the picture, and makes it rarely:
//!
//! - The icon is drawn the first time the picture is asked for, which is when a screen that shows
//!   the game is open. A library scan only looks at the program file (its size and its modified
//!   time) to build the address; it draws nothing, and nothing is drawn while the window is hidden.
//! - The drawn PNG is kept in `<local>\cache\games\icons\<hash of path>-<hash of version>.png`.
//!   The version is the size and modified time of the program plus `RENDER_VERSION`, so a program
//!   that is replaced, a corrected program file or a new version of Rain gives a new file.
//! - Windows is asked for the largest size (`ICON_PX`) the file has; the PNG is not held in memory.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::future::BoxFuture;

use super::covers::cover_version;
use super::names::{fnv8, path_key};
use crate::core::blocking;
use crate::platform::icons::{self, IconSource};

/// The edge of the drawn icon in pixels: Windows gives the largest picture the file has up to this.
pub const ICON_PX: u32 = 256;

/// Changes when the way an icon is drawn changes, so that old files are not served again.
const RENDER_VERSION: &str = "1";

/// At most this many icon files are kept; the oldest go when a new one is written.
const KEEP_FILES: usize = 300;

/// Whether a picture path is a program file (so the picture is its icon and not an image to send).
pub fn is_program(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

/// Draws the icon of a program file. A trait, so that a test says what is drawn and counts the calls.
pub trait ProgramIconRenderer: Send + Sync {
    /// The PNG, or `None` when Windows cannot draw one.
    fn render<'a>(&'a self, exe: &'a Path) -> BoxFuture<'a, Option<Vec<u8>>>;
}

/// The real renderer: the icon that Windows shows for the file.
pub struct ShellProgramIcons;

impl ProgramIconRenderer for ShellProgramIcons {
    fn render<'a>(&'a self, exe: &'a Path) -> BoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { icons::icon_png(IconSource::File(exe), ICON_PX).await.ok() })
    }
}

/// The icons that were drawn, on disk, and the programs that could not be drawn.
pub struct ProgramIcons {
    folder: PathBuf,
    renderer: Arc<dyn ProgramIconRenderer>,
    /// File names that could not be drawn. Cleared by a refresh, so a later try may work.
    no_icon: Mutex<HashSet<String>>,
    /// One drawing at a time: a second request for the same icon finds the file the first wrote.
    drawing: tokio::sync::Mutex<()>,
}

impl ProgramIcons {
    pub fn new(local_dir: &Path, renderer: Arc<dyn ProgramIconRenderer>) -> Self {
        Self {
            folder: local_dir.join("cache").join("games").join("icons"),
            renderer,
            no_icon: Mutex::default(),
            drawing: tokio::sync::Mutex::new(()),
        }
    }

    fn failed(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.no_icon.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The programs that could not be drawn may be tried again.
    pub fn forget_failures(&self) {
        self.failed().clear();
    }

    /// The icon of a program as a PNG with the version of its address: from the file when it is
    /// there, else drawn and written. `None` when the program is gone or has no icon.
    pub async fn get(&self, exe: &Path) -> Option<(Vec<u8>, String)> {
        let path = exe.to_path_buf();
        let version = blocking::run(move || cover_version(&path)).await.ok()??;
        let name = file_name(exe, &version);
        if self.failed().contains(&name) {
            return None;
        }
        let file = self.folder.join(&name);
        if let Some(bytes) = read(&file).await {
            return Some((bytes, version));
        }
        let _one_at_a_time = self.drawing.lock().await;
        if let Some(bytes) = read(&file).await {
            return Some((bytes, version));
        }
        let Some(png) = self.renderer.render(exe).await else {
            self.failed().insert(name);
            return None;
        };
        let (folder, to_write) = (self.folder.clone(), png.clone());
        // A file that cannot be written only costs a new drawing next time.
        let _ = blocking::run(move || write(&folder, &name, &to_write)).await;
        Some((png, version))
    }

    #[cfg(test)]
    pub fn folder(&self) -> &Path {
        &self.folder
    }
}

async fn read(file: &Path) -> Option<Vec<u8>> {
    let file = file.to_path_buf();
    blocking::run(move || fs::read(&file).ok()).await.ok()?
}

/// `<hash of the path>-<hash of the version>.png`: the first part is the program, the second says
/// which state of it was drawn.
fn file_name(exe: &Path, version: &str) -> String {
    format!(
        "{}-{}.png",
        fnv8(&path_key(&exe.to_string_lossy())),
        fnv8(&format!("{version}|{RENDER_VERSION}"))
    )
}

/// Writes the icon, removes the older drawings of the same program, and keeps the folder small.
fn write(folder: &Path, name: &str, png: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(folder)?;
    fs::write(folder.join(name), png)?;
    let program = name.split('-').next().unwrap_or_default();
    let mut others: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in fs::read_dir(folder)?.flatten() {
        let other = entry.file_name().to_string_lossy().into_owned();
        if other == name {
            continue;
        }
        if other.starts_with(&format!("{program}-")) {
            let _ = fs::remove_file(entry.path());
        } else if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
            others.push((modified, entry.path()));
        }
    }
    // The newest ones stay (one place is taken by the file just written).
    others.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in others.into_iter().skip(KEEP_FILES - 1) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        calls: AtomicUsize,
        fail: Mutex<bool>,
    }

    impl ProgramIconRenderer for Counting {
        fn render<'a>(&'a self, _exe: &'a Path) -> BoxFuture<'a, Option<Vec<u8>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let fail = *self.fail.lock().unwrap_or_else(PoisonError::into_inner);
            Box::pin(async move { (!fail).then(|| b"png".to_vec()) })
        }
    }

    fn setup() -> (tempfile::TempDir, ProgramIcons, Arc<Counting>) {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let renderer = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            fail: Mutex::new(false),
        });
        let icons = ProgramIcons::new(&temp.path().join("local"), renderer.clone());
        (temp, icons, renderer)
    }

    fn program(temp: &tempfile::TempDir, name: &str, size: u64) -> PathBuf {
        let path = temp.path().join("games").join(name);
        fs::create_dir_all(path.parent().expect("a folder")).expect("a folder");
        fs::File::create(&path)
            .and_then(|file| file.set_len(size))
            .expect("a file");
        path
    }

    #[test]
    fn only_exe_files_are_programs() {
        assert!(is_program(Path::new("D:\\G\\Game.EXE")));
        assert!(!is_program(Path::new("D:\\G\\cover.png")));
        assert!(!is_program(Path::new("D:\\G\\exe")));
    }

    #[tokio::test]
    async fn an_icon_is_drawn_once_and_then_read_from_its_file() {
        let (temp, icons, renderer) = setup();
        let exe = program(&temp, "a.exe", 10);
        let (bytes, version) = icons.get(&exe).await.expect("drawn");
        assert_eq!(bytes, b"png");
        assert_eq!(Some(version.clone()), cover_version(&exe));
        assert_eq!(fs::read_dir(icons.folder()).unwrap().count(), 1);
        let (again, same) = icons.get(&exe).await.expect("read from the file");
        assert_eq!((again, same), (b"png".to_vec(), version));
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_changed_program_is_drawn_again_and_its_old_file_goes() {
        let (temp, icons, renderer) = setup();
        let exe = program(&temp, "a.exe", 10);
        icons.get(&exe).await.unwrap();
        // A new size gives a new version, even when the time of the change is the same second.
        fs::File::options()
            .write(true)
            .open(&exe)
            .unwrap()
            .set_len(20)
            .unwrap();
        icons.get(&exe).await.unwrap();
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            fs::read_dir(icons.folder()).unwrap().count(),
            1,
            "one file for one program"
        );
    }

    #[tokio::test]
    async fn two_programs_have_two_files_and_a_gone_program_has_no_icon() {
        let (temp, icons, renderer) = setup();
        let (a, b) = (program(&temp, "a.exe", 10), program(&temp, "b.exe", 10));
        icons.get(&a).await.unwrap();
        icons.get(&b).await.unwrap();
        assert_eq!(fs::read_dir(icons.folder()).unwrap().count(), 2);
        fs::remove_file(&a).unwrap();
        assert!(icons.get(&a).await.is_none());
        assert_eq!(
            renderer.calls.load(Ordering::SeqCst),
            2,
            "nothing is drawn for a gone file"
        );
    }

    #[tokio::test]
    async fn a_program_that_cannot_be_drawn_is_remembered_until_the_next_refresh() {
        let (temp, icons, renderer) = setup();
        *renderer.fail.lock().unwrap() = true;
        let exe = program(&temp, "a.exe", 10);
        assert!(icons.get(&exe).await.is_none());
        assert!(icons.get(&exe).await.is_none());
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 1, "not asked again");
        *renderer.fail.lock().unwrap() = false;
        icons.forget_failures();
        assert!(icons.get(&exe).await.is_some());
    }

    #[test]
    fn the_folder_keeps_only_the_newest_files() {
        let temp = tempfile::tempdir().unwrap();
        for n in 0..(KEEP_FILES + 20) {
            fs::write(temp.path().join(format!("{n:08x}-00000000.png")), b"x").unwrap();
        }
        write(temp.path(), "ffffffff-11111111.png", b"y").unwrap();
        let count = fs::read_dir(temp.path()).unwrap().count();
        assert_eq!(count, KEEP_FILES);
        assert!(temp.path().join("ffffffff-11111111.png").exists());
    }
}
