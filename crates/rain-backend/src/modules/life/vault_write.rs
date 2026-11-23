//! THE ONLY FILE IN RAIN THAT WRITES INSIDE THE OBSIDIAN VAULT. Section 04 6.8.
//!
//! One function, `set_habit_property`, sets one true/false property of one daily note when the
//! user ticks or unticks a habit, and changes no other byte of the note. If the note does not
//! exist, a ticked habit creates a bare note that holds only that property.
//!
//! `tests/source_rules.rs` keeps this the only file under `modules/life/` that may name a file
//! write call, and the only file of the backend that may do so near a vault path. For the same
//! reason this file does not use `crate::store` or `core::blocking`: it has its own temporary
//! file code, and the route calls it from inside `blocking::run`.
//!
//! The text edit itself is in `frontmatter.rs` (pure text). This file only deals with the disk:
//! read, edit in memory, check that the note did not change meanwhile, replace it atomically.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::frontmatter::{self, NoteError};
use super::{NOTE_RENAME_DELAY, NOTE_RENAME_RETRIES, STALE_ROUNDS};

/// What `set_habit_property` did. It is stored in the audit table (`life_write.outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// The note did not exist; a new one holds the property.
    Created,
    /// The note changed.
    Updated,
    /// Nothing was written: the note already had that value, or there was nothing to untick.
    Unchanged,
}

impl Tick {
    pub fn as_str(self) -> &'static str {
        match self {
            Tick::Created => "created",
            Tick::Updated => "updated",
            Tick::Unchanged => "unchanged",
        }
    }
}

/// Why nothing was written. In every case the note was left exactly as it was.
#[derive(Debug)]
pub enum WriteError {
    /// The property is not a checkbox in that note, appears twice, or the top of the note is
    /// not YAML-like.
    Note(NoteError),
    /// The file is not valid UTF-8.
    NotUtf8,
    /// The note's folder does not exist. Rain never creates a folder in the vault.
    NoFolder,
    /// The note kept changing, or stayed locked by another program.
    Busy,
    Io(io::Error),
}

/// Sets one boolean property of one daily note. `path` is the full path of the note. Called from
/// inside `blocking::run`, because it waits for the disk.
///
/// `value = true` is a tick, `false` an untick. An untick never creates a note and never adds
/// a property. The steps are in section 6.8 of the design.
pub fn set_habit_property(path: &Path, key: &str, value: bool) -> Result<Tick, WriteError> {
    write_note(path, key, value, &mut || {})
}

/// `set_habit_property` with a hook that runs after the note was read and before the check
/// that it did not change. The tests use it to change the note "at the wrong moment".
fn write_note(
    path: &Path,
    key: &str,
    value: bool,
    between_read_and_check: &mut dyn FnMut(),
) -> Result<Tick, WriteError> {
    // A bad key is refused before anything is touched (no note is created for it).
    frontmatter::check_key(key).map_err(WriteError::Note)?;
    for _ in 0..STALE_ROUNDS {
        let before = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !value {
                    return Ok(Tick::Unchanged); // nothing exists to untick
                }
                match create_note(path, key)? {
                    Some(tick) => return Ok(tick),
                    None => continue, // someone made the note meanwhile: edit theirs instead
                }
            }
            Err(e) => return Err(WriteError::Io(e)),
        };
        let bytes = fs::read(path).map_err(WriteError::Io)?;
        let text = String::from_utf8(bytes).map_err(|_| WriteError::NotUtf8)?;
        let edited = if value {
            frontmatter::set_bool(&text, key, true)
        } else {
            frontmatter::set_bool_existing(&text, key, false)
        }
        .map_err(WriteError::Note)?;
        if edited == text {
            return Ok(Tick::Unchanged); // no write, so the file time stays
        }
        between_read_and_check();
        // Did the note change while we worked? Then the edit is based on stale text.
        let now = fs::metadata(path).map_err(WriteError::Io)?;
        let same = now.len() == before.len()
            && matches!((now.modified(), before.modified()), (Ok(a), Ok(b)) if a == b);
        if !same {
            continue;
        }
        replace_note(path, &edited, NOTE_RENAME_RETRIES, NOTE_RENAME_DELAY)?;
        return Ok(Tick::Updated);
    }
    Err(WriteError::Busy)
}

/// Creates the bare note: LF line ends, no byte order mark, only this property, no template.
/// `Ok(None)` when the note appeared in the meantime.
fn create_note(path: &Path, key: &str) -> Result<Option<Tick>, WriteError> {
    use io::Write;
    let text = frontmatter::new_note(&[(key, true)], "\n").map_err(WriteError::Note)?;
    // `create_new` fails when the note exists, so a note made a moment ago is never replaced.
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(text.as_bytes()).map_err(WriteError::Io)?;
            file.sync_all().map_err(WriteError::Io)?;
            Ok(Some(Tick::Created))
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(WriteError::NoFolder),
        Err(e) => Err(WriteError::Io(e)),
    }
}

/// Replaces the note with `text`: a dot-prefixed temporary file in the SAME folder (hidden from
/// Obsidian's file list, and a rename inside one folder is atomic), flushed to disk, then
/// renamed over the note. The temporary file never stays behind.
fn replace_note(path: &Path, text: &str, retries: u32, delay: Duration) -> Result<(), WriteError> {
    use io::Write;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp: PathBuf = path.with_file_name(format!(".{name}.rain-tmp-{}", std::process::id()));
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all() // the data is on disk before the name flips
        });
    if let Err(e) = written {
        let _ = fs::remove_file(&temp);
        return Err(WriteError::Io(e));
    }
    let renamed = rename_with_retries(&temp, path, retries, delay);
    if renamed.is_err() {
        let _ = fs::remove_file(&temp); // may not exist; that is fine
    }
    renamed
}

/// Obsidian, a sync tool or an antivirus may hold the note for a moment without sharing
/// (Windows errors 5, 32 and 33). Wait a little and try again; give up with `Busy`.
fn rename_with_retries(
    from: &Path,
    to: &Path,
    attempts: u32,
    delay: Duration,
) -> Result<(), WriteError> {
    for attempt in 1..=attempts {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                if attempt < attempts {
                    // Allowed timer T4 of foundation 11.5: a short wait inside one operation.
                    // This runs on a blocking thread (the route uses `blocking::run`), so no
                    // async worker waits.
                    #[allow(clippy::disallowed_methods)]
                    std::thread::sleep(delay);
                }
            }
            Err(e) => return Err(WriteError::Io(e)),
        }
    }
    Err(WriteError::Busy)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // the tests wait for a lock on a thread of their own
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::SystemTime;

    fn folder() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary folder")
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut found: Vec<String> = fs::read_dir(dir)
            .expect("a readable folder")
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        found
    }

    fn modified(path: &Path) -> SystemTime {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .expect("a file time")
    }

    #[test]
    fn a_missing_note_is_created_with_exactly_the_property() {
        let dir = folder();
        let note = dir.path().join("2026-11-22.md");
        assert!(matches!(
            set_habit_property(&note, "gym", true),
            Ok(Tick::Created)
        ));
        assert_eq!(fs::read(&note).unwrap(), b"---\ngym: true\n---\n");
        assert_eq!(names(dir.path()), ["2026-11-22.md"], "no debris");
    }

    #[test]
    fn an_existing_note_changes_one_line_and_a_repeat_writes_nothing() {
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(
            &note,
            "---\r\ngym: false # x\r\nread: true\r\n---\r\nbody\r\n",
        )
        .unwrap();
        assert!(matches!(
            set_habit_property(&note, "gym", true),
            Ok(Tick::Updated)
        ));
        assert_eq!(
            fs::read(&note).unwrap(),
            b"---\r\ngym: true # x\r\nread: true\r\n---\r\nbody\r\n"
        );
        let time = modified(&note);
        assert!(matches!(
            set_habit_property(&note, "gym", true),
            Ok(Tick::Unchanged)
        ));
        assert_eq!(modified(&note), time, "the file time did not move");
        // a second habit is added as one line
        assert!(matches!(
            set_habit_property(&note, "walk", true),
            Ok(Tick::Updated)
        ));
        assert_eq!(
            fs::read(&note).unwrap(),
            b"---\r\ngym: true # x\r\nread: true\r\nwalk: true\r\n---\r\nbody\r\n"
        );
        assert_eq!(names(dir.path()), ["a.md"], "no debris");
    }

    #[test]
    fn unticking_flips_to_false_and_never_creates_or_adds() {
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(&note, "---\ngym: true\n---\n").unwrap();
        assert!(matches!(
            set_habit_property(&note, "gym", false),
            Ok(Tick::Updated)
        ));
        assert_eq!(fs::read(&note).unwrap(), b"---\ngym: false\n---\n");

        // a missing note stays missing
        let missing = dir.path().join("none.md");
        assert!(matches!(
            set_habit_property(&missing, "gym", false),
            Ok(Tick::Unchanged)
        ));
        assert!(!missing.exists());

        // a missing property is not added, and the file time stays
        let other = dir.path().join("b.md");
        fs::write(&other, "---\nread: true\n---\nbody\n").unwrap();
        let time = modified(&other);
        assert!(matches!(
            set_habit_property(&other, "gym", false),
            Ok(Tick::Unchanged)
        ));
        assert_eq!(fs::read(&other).unwrap(), b"---\nread: true\n---\nbody\n");
        assert_eq!(modified(&other), time);

        // a note without any frontmatter is not given one
        let plain = dir.path().join("c.md");
        fs::write(&plain, "# Title\n").unwrap();
        assert!(matches!(
            set_habit_property(&plain, "gym", false),
            Ok(Tick::Unchanged)
        ));
        assert_eq!(fs::read(&plain).unwrap(), b"# Title\n");
    }

    #[test]
    fn a_note_without_frontmatter_gets_a_block_on_top_and_the_rest_stays() {
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(&note, "\u{feff}# Title\ntext").unwrap();
        assert!(matches!(
            set_habit_property(&note, "gym", true),
            Ok(Tick::Updated)
        ));
        assert_eq!(
            fs::read(&note).unwrap(),
            "\u{feff}---\ngym: true\n---\n# Title\ntext".as_bytes()
        );
        let empty = dir.path().join("e.md");
        fs::write(&empty, "").unwrap();
        assert!(matches!(
            set_habit_property(&empty, "gym", true),
            Ok(Tick::Updated)
        ));
        assert_eq!(fs::read(&empty).unwrap(), b"---\ngym: true\n---\n");
    }

    #[test]
    fn a_missing_folder_is_never_created() {
        let dir = folder();
        let note = dir.path().join("NoSuchFolder").join("x.md");
        assert!(matches!(
            set_habit_property(&note, "gym", true),
            Err(WriteError::NoFolder)
        ));
        assert!(!dir.path().join("NoSuchFolder").exists());
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn every_refusal_leaves_the_note_byte_for_byte_and_no_debris() {
        let dir = folder();
        let cases: &[(&str, &[u8])] = &[
            ("yes", b"---\ngym: yes\n---\nbody\n"),
            ("empty", b"---\ngym:\n---\n"),
            ("list", b"---\ngym: [true]\n---\n"),
            ("dup", b"---\ngym: true\ngym: false\n---\n"),
            ("prose", b"---\nThis is a sentence.\n---\nbody\n"),
            ("utf8", b"---\ngym: false\n---\n\xff\xfe"),
        ];
        for (name, bytes) in cases {
            let note = dir.path().join(format!("{name}.md"));
            fs::write(&note, bytes).unwrap();
            let before = modified(&note);
            for value in [true, false] {
                let result = set_habit_property(&note, "gym", value);
                assert!(
                    matches!(result, Err(WriteError::Note(_) | WriteError::NotUtf8)),
                    "{name}: {result:?}"
                );
                assert_eq!(&fs::read(&note).unwrap(), bytes, "{name}");
                assert_eq!(modified(&note), before, "{name}");
            }
        }
        let utf8 = dir.path().join("utf8.md");
        assert!(matches!(
            set_habit_property(&utf8, "gym", true),
            Err(WriteError::NotUtf8)
        ));
        let leftovers = names(dir.path())
            .into_iter()
            .filter(|n| n.starts_with('.'))
            .count();
        assert_eq!(leftovers, 0, "no temporary file stays");
    }

    #[test]
    fn a_bad_property_name_writes_nothing_and_creates_nothing() {
        let dir = folder();
        let note = dir.path().join("a.md");
        let result = set_habit_property(&note, "a:b", true);
        assert!(matches!(
            result,
            Err(WriteError::Note(NoteError::InvalidKey))
        ));
        assert!(!note.exists());
    }

    #[test]
    fn a_big_note_is_read_whole_and_changes_one_line() {
        let dir = folder();
        let note = dir.path().join("big.md");
        let body = "a line of text that makes the note long\n".repeat(30_000); // about 1.2 MiB
        let original = format!("---\ngym: false\n---\n{body}");
        fs::write(&note, &original).unwrap();
        assert!(matches!(
            set_habit_property(&note, "gym", true),
            Ok(Tick::Updated)
        ));
        assert_eq!(
            fs::read_to_string(&note).unwrap(),
            original.replacen("gym: false", "gym: true", 1)
        );
    }

    #[test]
    fn a_note_that_changed_between_read_and_check_is_edited_again_from_the_new_text() {
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(&note, "---\ngym: false\n---\nold\n").unwrap();
        let mut changes = 0;
        let mut hook = || {
            if changes == 0 {
                // Obsidian saved a longer text between our read and our check.
                fs::write(&note, "---\ngym: false\nread: true\n---\nnew text\n").unwrap();
            }
            changes += 1;
        };
        let result = write_note(&note, "gym", true, &mut hook);
        assert!(matches!(result, Ok(Tick::Updated)), "{result:?}");
        assert_eq!(
            fs::read(&note).unwrap(),
            b"---\ngym: true\nread: true\n---\nnew text\n",
            "the second round edited the new text"
        );
    }

    #[test]
    fn a_note_that_keeps_changing_gives_busy_after_three_rounds() {
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(&note, "---\ngym: false\n---\n").unwrap();
        let mut rounds = 0;
        let mut hook = || {
            rounds += 1;
            let padding = "x".repeat(rounds);
            fs::write(&note, format!("---\ngym: false\n---\n{padding}\n")).unwrap();
        };
        let result = write_note(&note, "gym", true, &mut hook);
        assert!(matches!(result, Err(WriteError::Busy)), "{result:?}");
        assert_eq!(rounds, STALE_ROUNDS as usize);
        assert!(
            fs::read_to_string(&note).unwrap().contains("gym: false"),
            "nothing was written"
        );
        assert_eq!(names(dir.path()), ["a.md"]);
    }

    /// Holds the note open for reading without allowing a replace (no delete sharing), as
    /// a program that locks it does.
    fn lock(note: &Path) -> fs::File {
        fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE, not FILE_SHARE_DELETE
            .open(note)
            .expect("the note could be locked")
    }

    #[test]
    fn a_locked_note_gives_busy_and_stays_unchanged_and_the_temp_file_goes() {
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(&note, "---\ngym: false\n---\n").unwrap();
        let held = lock(&note);
        let started = std::time::Instant::now();
        let result = set_habit_property(&note, "gym", true);
        let waited = started.elapsed();
        drop(held);
        assert!(matches!(result, Err(WriteError::Busy)), "{result:?}");
        assert!(
            waited >= Duration::from_millis(100),
            "it retried for a while: {waited:?}"
        );
        assert_eq!(fs::read(&note).unwrap(), b"---\ngym: false\n---\n");
        assert_eq!(
            names(dir.path()),
            ["a.md"],
            "the temporary file was removed"
        );
    }

    #[test]
    fn a_lock_that_ends_inside_the_retry_window_lets_the_tick_through() {
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(&note, "---\ngym: false\n---\n").unwrap();
        let held = lock(&note);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(70));
            drop(held);
        });
        let result = set_habit_property(&note, "gym", true);
        release.join().expect("the helper thread ended");
        assert!(matches!(result, Ok(Tick::Updated)), "{result:?}");
        assert_eq!(fs::read(&note).unwrap(), b"---\ngym: true\n---\n");
        assert_eq!(names(dir.path()), ["a.md"]);
    }

    #[test]
    fn a_note_made_in_the_meantime_is_edited_not_replaced() {
        // `create_note` reports `None` for an existing note; the loop then edits that note.
        let dir = folder();
        let note = dir.path().join("a.md");
        fs::write(&note, "---\nread: true\n---\n").unwrap();
        assert!(matches!(create_note(&note, "gym"), Ok(None)));
        assert_eq!(fs::read(&note).unwrap(), b"---\nread: true\n---\n");
    }
}
