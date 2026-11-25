//! Cover pictures: checking and importing the ones the user picks, finding the ones next to a
//! game, and serving them as media (`GET /media/game-cover/{id}`). Section 02, 4.3 and 6.12.
//!
//! Nothing is downloaded and nothing is resized: a cover is a file Rain already has or one the
//! user points at. Picked files are copied into `<data>\media\` under a name that holds a hash of
//! the content, so a new picture gets a new name and the cached URL changes with it.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::body::Bytes;
use futures_util::future::BoxFuture;

use super::model::image_content_type;
use super::names::{fnv8, fnv8_bytes, path_key, slug};
use super::program_icon::is_program;
use crate::core::blocking;
use crate::core::error::ApiResult;
use crate::core::media::{Media, MediaProvider};
use crate::core::state::WeakAppState;

/// The largest cover that is accepted or served.
pub const COVER_MAX: u64 = 8 * 1024 * 1024;

/// The folder below the data folder, and the prefix of a cover value in `games.toml`.
pub const MEDIA_DIR: &str = "media";

/// Names that mark a picture next to a game as its cover (without the extension).
const COVER_NAMES: &[&str] = &[
    "cover",
    "folder",
    "poster",
    "boxart",
    "box",
    "library_600x900",
];
const COVER_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverProblem {
    NotFound,
    NotImage,
    TooLarge,
}

/// Reads a picture the user named and checks it: it must exist, be at most `COVER_MAX` and have
/// the first bytes of a PNG, JPEG, WEBP or GIF. Returns the bytes and the extension to use.
/// Blocking.
pub fn read_and_check(path: &Path) -> Result<(Vec<u8>, &'static str), CoverProblem> {
    let file = fs::File::open(path).map_err(|_| CoverProblem::NotFound)?;
    let meta = file.metadata().map_err(|_| CoverProblem::NotFound)?;
    if !meta.is_file() {
        return Err(CoverProblem::NotFound);
    }
    if meta.len() > COVER_MAX {
        return Err(CoverProblem::TooLarge);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(COVER_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CoverProblem::NotFound)?;
    if bytes.len() as u64 > COVER_MAX {
        return Err(CoverProblem::TooLarge);
    }
    let extension = sniff(&bytes).ok_or(CoverProblem::NotImage)?;
    Ok((bytes, extension))
}

/// The image type by its first bytes, not by the name.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

/// Copies a checked picture into `<data>\media\<id>-<hash>.<ext>` through a temporary file and a
/// rename. Returns the value for `games.toml`: `media/<file>`. Blocking.
pub fn import(data_dir: &Path, game_id: &str, bytes: &[u8], extension: &str) -> io::Result<String> {
    let folder = data_dir.join(MEDIA_DIR);
    fs::create_dir_all(&folder)?;
    let name = format!("{}-{}.{extension}", slug(game_id), fnv8_bytes(bytes));
    let target = folder.join(&name);
    if !target.exists() {
        let temp = folder.join(format!(".{name}.rain-tmp-{}", std::process::id()));
        let written = (|| {
            let mut file = fs::File::create(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()
        })();
        if let Err(error) = written.and_then(|()| fs::rename(&temp, &target)) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    }
    Ok(format!("{MEDIA_DIR}/{name}"))
}

/// The file a `cover` value of `games.toml` names: `media/<file>` below the data folder, or an
/// absolute path.
pub fn cover_path(data_dir: &Path, value: &str) -> PathBuf {
    match value.strip_prefix("media/") {
        Some(file) if !file.contains(['/', '\\']) => data_dir.join(MEDIA_DIR).join(file),
        _ => PathBuf::from(value),
    }
}

/// Removes an imported cover file. Only a file Rain wrote itself (`media/<file>` of the data
/// folder) is removed: a cover the user pointed at by an absolute path is never touched.
pub fn remove_imported(data_dir: &Path, value: &str) {
    if let Some(file) = value.strip_prefix("media/")
        && !file.contains(['/', '\\'])
    {
        let _ = fs::remove_file(data_dir.join(MEDIA_DIR).join(file));
    }
}

/// Whether a `media/<file>` value is still named by any of `values` (several games could name one
/// file, because the name holds a hash of the content).
pub fn still_used(value: &str, values: impl IntoIterator<Item = String>) -> bool {
    values.into_iter().any(|other| other == value)
}

/// The first picture next to the game that is named like a cover, at the top level of the folder.
pub fn find_in_folder(folder: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_lowercase);
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_lowercase);
            stem.is_some_and(|stem| COVER_NAMES.contains(&stem.as_str()))
                && extension.is_some_and(|ext| COVER_EXTENSIONS.contains(&ext.as_str()))
                && path.is_file()
        })
        .collect();
    // The order of COVER_NAMES decides, then the name, so the answer is stable.
    found.sort_by_key(|path| {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        (
            COVER_NAMES.iter().position(|name| *name == stem),
            path.clone(),
        )
    });
    found.into_iter().next()
}

/// A short text that changes when the file changes: its modification time in seconds and its
/// length, in hex. Part of the cover URL.
pub fn version_of(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let seconds = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    Some(format!("{seconds:x}{:x}", meta.len()))
}

/// The version of a game's picture: `version_of`, and for a program file (whose icon is the
/// picture) also a hash of its path, so a corrected program file gives a new address even when the
/// two files happen to share a size and a time.
pub fn cover_version(path: &Path) -> Option<String> {
    let version = version_of(path)?;
    Some(if is_program(path) {
        format!("{version}{}", fnv8(&path_key(&path.to_string_lossy())))
    } else {
        version
    })
}

// ---- the media provider ----

/// Serves `GET /media/game-cover/<game id>`. It maps the id to a file through the library, never
/// from the URL, so no path can be asked for.
pub struct GameCovers {
    pub state: WeakAppState,
}

impl MediaProvider for GameCovers {
    fn get(&self, id: &str) -> BoxFuture<'_, ApiResult<Option<Media>>> {
        let id = id.to_owned();
        Box::pin(async move {
            let Some(state) = self.state.upgrade() else {
                return Ok(None);
            };
            let Some(path) = state.games().cover_file(&id).await else {
                return Ok(None);
            };
            // A program file stands in for a missing cover: its icon is the picture.
            if is_program(&path) {
                let Some((bytes, version)) = state.games().program_icon(&path).await else {
                    return Ok(None);
                };
                return Ok(Some(Media {
                    bytes: Bytes::from(bytes),
                    content_type: "image/png",
                    version,
                }));
            }
            let is_icon = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"));
            let content_type = if is_icon {
                "image/png"
            } else {
                let Some(content_type) = image_content_type(&path) else {
                    return Ok(None);
                };
                content_type
            };
            // A file that has vanished, is too large or cannot be read is "no picture": the
            // tile then falls back to the title.
            let read = blocking::run(move || {
                let version = version_of(&path)?;
                let meta = fs::metadata(&path).ok()?;
                if meta.len() > COVER_MAX {
                    return None;
                }
                let bytes = fs::read(&path).ok()?;
                // An icon file holds several sizes (Riot's product icons): serve the largest.
                let bytes = if is_icon {
                    super::ico::largest_png(&bytes)?
                } else {
                    bytes
                };
                Some((bytes, version))
            })
            .await?;
            Ok(read.map(|(bytes, version)| Media {
                bytes: Bytes::from(bytes),
                content_type,
                version,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
    const JPG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2];

    #[test]
    fn the_four_image_types_are_known_by_their_first_bytes() {
        assert_eq!(sniff(PNG), Some("png"));
        assert_eq!(sniff(JPG), Some("jpg"));
        assert_eq!(sniff(b"RIFF\x01\x02\x03\x04WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"GIF89a.."), Some("gif"));
        assert_eq!(sniff(b"not an image"), None);
        assert_eq!(sniff(b"RIFF\x01\x02\x03\x04WAVEfmt "), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn a_picture_is_checked_for_existence_type_and_size() {
        let temp = tempfile::tempdir().unwrap();
        let png = temp.path().join("a.png");
        fs::write(&png, PNG).unwrap();
        assert_eq!(read_and_check(&png).unwrap().1, "png");

        let text = temp.path().join("fake.png");
        fs::write(&text, "this is text").unwrap();
        assert_eq!(read_and_check(&text), Err(CoverProblem::NotImage));
        assert_eq!(
            read_and_check(&temp.path().join("none.png")),
            Err(CoverProblem::NotFound)
        );
        assert_eq!(read_and_check(temp.path()), Err(CoverProblem::NotFound));

        let big = temp.path().join("big.png");
        let file = fs::File::create(&big).unwrap();
        file.set_len(COVER_MAX + 1).unwrap();
        assert_eq!(read_and_check(&big), Err(CoverProblem::TooLarge));
    }

    #[test]
    fn an_import_is_named_by_its_content_and_repeats_are_harmless() {
        let temp = tempfile::tempdir().unwrap();
        let first = import(temp.path(), "game-celeste", PNG, "png").unwrap();
        assert!(
            first.starts_with("media/game-celeste-") && first.ends_with(".png"),
            "{first}"
        );
        assert_eq!(first.len(), "media/game-celeste-".len() + 8 + ".png".len());
        assert_eq!(
            import(temp.path(), "game-celeste", PNG, "png").unwrap(),
            first
        );
        let other = import(temp.path(), "game-celeste", JPG, "jpg").unwrap();
        assert_ne!(other, first);
        assert_eq!(fs::read(cover_path(temp.path(), &first)).unwrap(), PNG);
        // No temporary file is left behind.
        let leftovers = fs::read_dir(temp.path().join("media"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("rain-tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn only_files_rain_wrote_are_removed() {
        let temp = tempfile::tempdir().unwrap();
        let value = import(temp.path(), "game-x", PNG, "png").unwrap();
        let outside = temp.path().join("mine.png");
        fs::write(&outside, PNG).unwrap();
        remove_imported(temp.path(), &outside.to_string_lossy());
        remove_imported(temp.path(), "media/../mine.png");
        assert!(outside.exists());
        remove_imported(temp.path(), &value);
        assert!(!cover_path(temp.path(), &value).exists());
        assert_eq!(
            cover_path(temp.path(), "media/../x.png"),
            PathBuf::from("media/../x.png")
        );
    }

    #[test]
    fn a_cover_next_to_the_game_is_found_by_name() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("screenshot.png"), PNG).unwrap();
        fs::write(temp.path().join("notes.txt"), "x").unwrap();
        assert_eq!(find_in_folder(temp.path()), None);
        fs::write(temp.path().join("Folder.JPG"), JPG).unwrap();
        fs::write(temp.path().join("cover.png"), PNG).unwrap();
        assert!(find_in_folder(temp.path()).unwrap().ends_with("cover.png"));
    }

    #[test]
    fn the_version_changes_with_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("c.png");
        fs::write(&file, PNG).unwrap();
        let before = version_of(&file).unwrap();
        fs::write(&file, [PNG, b"more"].concat()).unwrap();
        assert_ne!(before, version_of(&file).unwrap());
        assert_eq!(version_of(&temp.path().join("none")), None);
    }
}
