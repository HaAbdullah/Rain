//! The note index: for a window of days ending today, which daily notes exist and which
//! top-level true/false properties they have. Section 04 6.4.
//!
//! The index is rebuilt by looking at one path per day (date driven, no folder scan). A note
//! whose file time and size did not change since the last look is not read again; its
//! properties come from the cache. Only the NAMES and VALUES of the true/false properties are
//! kept: no note text is ever stored, logged or sent.
//!
//! All file access is read only and goes through the small `VaultFs` trait, so that the unit
//! tests can use an in-memory fake with settable file times, sizes, content and errors.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;
use std::time::{Instant, UNIX_EPOCH};

use super::daily::DailyConfig;
use super::dates::CivilDate;
use super::frontmatter;
use super::{HEAD_READ_MAX, LIFE_INDEX_DAYS};

/// What the index needs to know about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMeta {
    pub is_file: bool,
    /// Last write time, nanoseconds since 1970.
    pub mtime_ns: i64,
    pub size: u64,
}

/// The read-only part of the file system that the index uses.
pub trait VaultFs: Send + Sync {
    /// `Ok(None)` when nothing is there.
    fn metadata(&self, path: &Path) -> io::Result<Option<FileMeta>>;
    /// The first `max` bytes of the file.
    fn read_head(&self, path: &Path, max: usize) -> io::Result<Vec<u8>>;
}

/// The real file system. Opens files for reading only.
pub struct RealFs;

impl VaultFs for RealFs {
    fn metadata(&self, path: &Path) -> io::Result<Option<FileMeta>> {
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            // A path part that is a file, not a folder, is also "nothing here".
            Err(e) if e.kind() == io::ErrorKind::NotADirectory => return Ok(None),
            Err(e) => return Err(e),
        };
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos().min(i64::MAX as u128) as i64);
        Ok(Some(FileMeta {
            is_file: meta.is_file(),
            mtime_ns,
            size: meta.len(),
        }))
    }

    fn read_head(&self, path: &Path, max: usize) -> io::Result<Vec<u8>> {
        let mut head = Vec::new();
        std::fs::File::open(path)?
            .take(max as u64)
            .read_to_end(&mut head)?;
        Ok(head)
    }
}

/// What the index keeps of one note. No text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub mtime_ns: i64,
    pub size: u64,
    /// The top-level true/false properties, as written in the note.
    pub bools: Vec<(String, bool)>,
}

impl Note {
    /// The value of a property, when the note has it as true or false.
    pub fn value(&self, property: &str) -> Option<bool> {
        self.bools
            .iter()
            .find(|(name, _)| name == property)
            .map(|(_, value)| *value)
    }
}

/// The state of all notes at the last check.
#[derive(Debug, Clone)]
pub struct Index {
    /// Which vault, folder and format this was built for (`DailyConfig::scope`).
    pub scope: String,
    /// The day the window was built for.
    pub today: CivilDate,
    /// Only dates that have a file.
    pub notes: BTreeMap<CivilDate, Note>,
    /// When the last full check ended.
    pub checked_at: Instant,
}

impl Index {
    /// An index with no notes: no vault, a bad format, or game mode before the first check.
    pub fn empty(today: CivilDate) -> Self {
        Self {
            scope: String::new(),
            today,
            notes: BTreeMap::new(),
            checked_at: Instant::now(),
        }
    }
}

/// The oldest day of the window ending at `today`: `LIFE_INDEX_DAYS` days, today included.
pub fn window_from(today: CivilDate) -> CivilDate {
    today.add_days(-(LIFE_INDEX_DAYS - 1))
}

/// The result of one check.
#[derive(Debug, PartialEq, Eq)]
pub struct Checked {
    pub notes: BTreeMap<CivilDate, Note>,
    /// Notes that exist but could not be read and had no cached row (counted, never named).
    pub unreadable: u32,
}

/// Looks at the one path of every day of the window. `previous` holds what the last check or
/// the cache knew; a note whose `(mtime, size)` is unchanged keeps its cached properties and is
/// not read. Blocking: call it from `blocking::run`.
pub fn check(
    fs: &dyn VaultFs,
    config: &DailyConfig,
    today: CivilDate,
    previous: &BTreeMap<CivilDate, Note>,
) -> Checked {
    let mut notes = BTreeMap::new();
    let mut unreadable = 0;
    let first = window_from(today);
    let mut date = first;
    while date <= today {
        let found = config
            .file_for(date)
            .ok()
            .map(|path| (path.clone(), fs.metadata(&path)));
        if let Some((path, result)) = found {
            match result {
                Ok(Some(meta)) if meta.is_file => {
                    match previous.get(&date) {
                        Some(known)
                            if known.mtime_ns == meta.mtime_ns && known.size == meta.size =>
                        {
                            notes.insert(date, known.clone());
                        }
                        _ => match fs.read_head(&path, HEAD_READ_MAX) {
                            Ok(head) => {
                                let text = String::from_utf8_lossy(&head);
                                notes.insert(
                                    date,
                                    Note {
                                        mtime_ns: meta.mtime_ns,
                                        size: meta.size,
                                        bools: frontmatter::read_bools(&text),
                                    },
                                );
                            }
                            // A sharing violation or access denied: keep the old row if there
                            // is one, else the day counts as "no note".
                            Err(_) => match previous.get(&date) {
                                Some(known) => {
                                    notes.insert(date, known.clone());
                                }
                                None => unreadable += 1,
                            },
                        },
                    }
                }
                Ok(_) => {} // nothing there, or a folder
                // The metadata itself failed: keep the old row if there is one.
                Err(_) => match previous.get(&date) {
                    Some(known) => {
                        notes.insert(date, known.clone());
                    }
                    None => unreadable += 1,
                },
            }
        }
        date = date.add_days(1);
    }
    Checked { notes, unreadable }
}

/// What changed between two sets of rows: the rows to write and the dates to delete.
pub fn diff(
    old: &BTreeMap<CivilDate, Note>,
    new: &BTreeMap<CivilDate, Note>,
) -> (Vec<(CivilDate, Note)>, Vec<CivilDate>) {
    let changed = new
        .iter()
        .filter(|(date, note)| old.get(date) != Some(note))
        .map(|(date, note)| (*date, note.clone()))
        .collect();
    let removed = old
        .keys()
        .filter(|date| !new.contains_key(date))
        .copied()
        .collect();
    (changed, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// An in-memory vault: files with settable time, size and content, and errors.
    #[derive(Default)]
    struct FakeFs {
        files: Mutex<HashMap<PathBuf, (i64, String)>>,
        broken: Mutex<Vec<PathBuf>>,
        reads: Mutex<Vec<PathBuf>>,
    }

    impl FakeFs {
        fn put(&self, path: &str, mtime: i64, text: &str) {
            self.files
                .lock()
                .unwrap()
                .insert(PathBuf::from(path), (mtime, text.to_owned()));
        }
        fn remove(&self, path: &str) {
            self.files.lock().unwrap().remove(Path::new(path));
        }
        fn break_file(&self, path: &str) {
            self.broken.lock().unwrap().push(PathBuf::from(path));
        }
        fn reads(&self) -> usize {
            self.reads.lock().unwrap().len()
        }
    }

    impl VaultFs for FakeFs {
        fn metadata(&self, path: &Path) -> io::Result<Option<FileMeta>> {
            Ok(self
                .files
                .lock()
                .unwrap()
                .get(path)
                .map(|(mtime, text)| FileMeta {
                    is_file: true,
                    mtime_ns: *mtime,
                    size: text.len() as u64,
                }))
        }

        fn read_head(&self, path: &Path, max: usize) -> io::Result<Vec<u8>> {
            self.reads.lock().unwrap().push(path.to_path_buf());
            if self.broken.lock().unwrap().iter().any(|p| p == path) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"));
            }
            let files = self.files.lock().unwrap();
            let (_, text) = files
                .get(path)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            Ok(text.as_bytes()[..text.len().min(max)].to_vec())
        }
    }

    fn date(text: &str) -> CivilDate {
        CivilDate::parse(text).expect("a real date")
    }

    fn config() -> DailyConfig {
        DailyConfig::new(Some(Path::new(r"V:\vault")), "Daily", "YYYY-MM-DD").unwrap()
    }

    fn path_of(day: &str) -> String {
        format!(r"V:\vault\Daily\{day}.md")
    }

    const TODAY: &str = "2026-11-22";

    #[test]
    fn only_days_with_a_file_are_notes_and_their_booleans_are_read() {
        let fs = FakeFs::default();
        fs.put(
            &path_of("2026-11-20"),
            1,
            "---\ngym: true\nread: false\n---\nbody",
        );
        fs.put(&path_of("2026-11-22"), 1, "no frontmatter");
        fs.put(r"V:\vault\Daily\notes.txt", 1, "stray");
        let checked = check(&fs, &config(), date(TODAY), &BTreeMap::new());
        assert_eq!(checked.notes.len(), 2);
        let note = &checked.notes[&date("2026-11-20")];
        assert_eq!(
            note.bools,
            vec![("gym".to_owned(), true), ("read".to_owned(), false)]
        );
        assert_eq!(note.value("gym"), Some(true));
        assert_eq!(note.value("nope"), None);
        assert!(checked.notes[&date(TODAY)].bools.is_empty());
        assert_eq!(checked.unreadable, 0);
    }

    #[test]
    fn the_window_is_800_days_and_older_notes_are_not_looked_at() {
        let fs = FakeFs::default();
        let today = date(TODAY);
        fs.put(
            &path_of(&window_from(today).to_string()),
            1,
            "---\na: true\n---\n",
        );
        fs.put(
            &path_of(&window_from(today).add_days(-1).to_string()),
            1,
            "---\na: true\n---\n",
        );
        let checked = check(&fs, &config(), today, &BTreeMap::new());
        assert_eq!(
            checked.notes.len(),
            1,
            "the day before the window is outside"
        );
        assert_eq!(window_from(today), date("2024-09-14"));
    }

    #[test]
    fn an_unchanged_time_and_size_is_not_read_again_but_a_change_is_read_once() {
        let fs = FakeFs::default();
        fs.put(&path_of("2026-11-20"), 5, "---\ngym: false\n---\n");
        let first = check(&fs, &config(), date(TODAY), &BTreeMap::new());
        assert_eq!(fs.reads(), 1);

        let second = check(&fs, &config(), date(TODAY), &first.notes);
        assert_eq!(fs.reads(), 1, "an unchanged note comes from the cache");
        assert_eq!(second.notes, first.notes);

        fs.put(&path_of("2026-11-20"), 6, "---\ngym: true\n---\n"); // same size, newer time
        let third = check(&fs, &config(), date(TODAY), &second.notes);
        assert_eq!(fs.reads(), 2);
        assert_eq!(third.notes[&date("2026-11-20")].value("gym"), Some(true));

        fs.put(&path_of("2026-11-20"), 6, "---\ngym: true\nx: 1\n---\n"); // other size
        check(&fs, &config(), date(TODAY), &third.notes);
        assert_eq!(fs.reads(), 3);
    }

    #[test]
    fn a_deleted_file_disappears() {
        let fs = FakeFs::default();
        fs.put(&path_of("2026-11-20"), 1, "---\ngym: true\n---\n");
        let first = check(&fs, &config(), date(TODAY), &BTreeMap::new());
        fs.remove(&path_of("2026-11-20"));
        let second = check(&fs, &config(), date(TODAY), &first.notes);
        assert!(second.notes.is_empty());
        let (changed, removed) = diff(&first.notes, &second.notes);
        assert!(changed.is_empty());
        assert_eq!(removed, vec![date("2026-11-20")]);
    }

    #[test]
    fn an_unreadable_file_keeps_its_old_row_or_counts_as_no_note() {
        let fs = FakeFs::default();
        fs.put(&path_of("2026-11-20"), 1, "---\ngym: true\n---\n");
        let first = check(&fs, &config(), date(TODAY), &BTreeMap::new());

        // The file changed and is now locked: the old row stays.
        fs.put(&path_of("2026-11-20"), 2, "---\ngym: false\n---\n");
        fs.break_file(&path_of("2026-11-20"));
        let second = check(&fs, &config(), date(TODAY), &first.notes);
        assert_eq!(second.notes[&date("2026-11-20")].value("gym"), Some(true));
        assert_eq!(second.unreadable, 0);

        // A new file that cannot be read has no old row: no note, one unreadable.
        fs.put(&path_of("2026-11-21"), 1, "---\ngym: true\n---\n");
        fs.break_file(&path_of("2026-11-21"));
        let third = check(&fs, &config(), date(TODAY), &second.notes);
        assert!(!third.notes.contains_key(&date("2026-11-21")));
        assert_eq!(third.unreadable, 1);
    }

    #[test]
    fn only_the_head_of_a_big_note_is_read() {
        let fs = FakeFs::default();
        let big = format!("---\ngym: true\n---\n{}", "x".repeat(200_000));
        fs.put(&path_of("2026-11-20"), 1, &big);
        let checked = check(&fs, &config(), date(TODAY), &BTreeMap::new());
        assert_eq!(checked.notes[&date("2026-11-20")].value("gym"), Some(true));
        assert_eq!(checked.notes[&date("2026-11-20")].size, big.len() as u64);
    }

    #[test]
    fn a_nested_format_is_found_without_any_folder_scan() {
        let fs = FakeFs::default();
        fs.put(
            r"V:\vault\Daily\2026\11\2026-11-20.md",
            1,
            "---\ngym: true\n---\n",
        );
        let nested =
            DailyConfig::new(Some(Path::new(r"V:\vault")), "Daily", "YYYY/MM/YYYY-MM-DD").unwrap();
        let checked = check(&fs, &nested, date(TODAY), &BTreeMap::new());
        assert_eq!(checked.notes.len(), 1);
    }

    #[test]
    fn diff_lists_new_and_changed_rows() {
        let note = |mtime, flag| Note {
            mtime_ns: mtime,
            size: 3,
            bools: vec![("gym".to_owned(), flag)],
        };
        let old = BTreeMap::from([
            (date("2026-11-01"), note(1, true)),
            (date("2026-11-02"), note(1, true)),
        ]);
        let new = BTreeMap::from([
            (date("2026-11-02"), note(2, true)),
            (date("2026-11-03"), note(1, false)),
        ]);
        let (changed, removed) = diff(&old, &new);
        let dates: Vec<_> = changed.iter().map(|(d, _)| d.to_string()).collect();
        assert_eq!(dates, ["2026-11-02", "2026-11-03"]);
        assert_eq!(removed, vec![date("2026-11-01")]);
    }
}
