//! One TOML file of the user's data folder: loaded into a typed value, edited without losing the
//! user's comments, written so that a crash cannot leave half a file. Foundation section 10.4.
//!
//! The rules in short:
//! - The file is the truth, the typed value is a copy. `get()` returns the last good copy and
//!   never touches the disk.
//! - A file that cannot be read keeps the last good copy, and a `ConfigIssue` goes to `/health`.
//!   Rain never overwrites a file that has an issue.
//! - `edit()` reads the file again, changes it with `toml_edit` (so comments and key order
//!   survive), checks the result, and replaces the file through a temporary file and a rename.
//! - The hub's file watcher calls `refresh` after a debounced change event, so a hand edit
//!   reaches `subscribe` within about a second. `refresh()` also stays as a cheap safety net: the
//!   settings read calls it, and it works when the watcher could not start.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use rain_types::ConfigIssue;
use rain_types::limits::{TOML_TRUNCATION_SETTLE, TOML_WRITE_RETRIES, TOML_WRITE_RETRY_DELAY};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::watch;
use toml_edit::{DocumentMut, Item, Table, Value};
use tracing::{error, warn};

use super::hub::ConfigHub;
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::health::Health;

/// The newest `schema_version` this Rain understands. A file with a higher number was written by
/// a newer Rain: it is read but never written (foundation 10.3, rule 2).
const KNOWN_SCHEMA_VERSION: u32 = 1;

// ---- what can be wrong with a file ----

/// Why the last good value is kept. It is what `/health` shows and what an edit refuses with.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Problem {
    /// The file cannot be read, is not valid TOML, or breaks the rules of its schema.
    Broken { line: Option<u32>, message: String },
    /// `schema_version` is higher than `KNOWN_SCHEMA_VERSION`.
    Newer,
}

impl Problem {
    fn broken(message: impl Into<String>) -> Self {
        Self::Broken {
            line: None,
            message: message.into(),
        }
    }

    fn issue(&self, file: &str) -> ConfigIssue {
        match self {
            Self::Broken { line, message } => ConfigIssue {
                file: file.to_owned(),
                line: *line,
                message: message.clone(),
            },
            Self::Newer => ConfigIssue {
                file: file.to_owned(),
                line: None,
                message: "Written by a newer Rain".to_owned(),
            },
        }
    }

    /// The answer of an edit that is refused because of this problem (reasons of section 06 8.1).
    fn refusal(&self, file: &str) -> ApiError {
        match self {
            Self::Broken {
                line: Some(line), ..
            } => ApiError::conflict(
                "configBroken",
                format!("{file} has an error on line {line}. Fix the file, then try again."),
            ),
            Self::Broken { line: None, .. } => ApiError::conflict(
                "configBroken",
                format!("{file} has an error. Fix the file, then try again."),
            ),
            Self::Newer => ApiError::conflict(
                "configNewer",
                format!("{file} was written by a newer Rain."),
            ),
        }
    }
}

/// Reads `text` as a `T` and applies the rules of the schema. Pure: no disk, no state.
fn parse<T: DeserializeOwned>(
    text: &str,
    validate: fn(&T) -> Result<(), String>,
) -> Result<T, Problem> {
    /// Only the version, so that a file from the future is recognised before its other keys,
    /// which this Rain may not understand, are read.
    #[derive(Deserialize)]
    struct Header {
        #[serde(default)]
        schema_version: Option<u32>,
    }

    let header: Header = toml::from_str(text).map_err(|e| from_toml_error(text, &e))?;
    if header
        .schema_version
        .is_some_and(|version| version > KNOWN_SCHEMA_VERSION)
    {
        return Err(Problem::Newer);
    }
    let value: T = toml::from_str(text).map_err(|e| from_toml_error(text, &e))?;
    validate(&value).map_err(Problem::broken)?;
    Ok(value)
}

fn from_toml_error(text: &str, error: &toml::de::Error) -> Problem {
    let line = error.span().map(|span| {
        let before = &text.as_bytes()[..span.start.min(text.len())];
        before.iter().filter(|byte| **byte == b'\n').count() as u32 + 1
    });
    Problem::Broken {
        line,
        message: error.message().to_owned(),
    }
}

// ---- reading the disk ----

/// Length and modification time: what `refresh()` compares to see that a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

enum Disk {
    Missing,
    Text { text: String, stamp: Stamp },
}

fn read_disk(path: &Path) -> io::Result<Disk> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Disk::Missing),
        Err(e) => return Err(e),
    };
    let stamp = Stamp::of(&file.metadata()?);
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(Disk::Text { text, stamp })
}

/// Tells two texts apart cheaply. Only compared inside this process, so the hasher need not be
/// stable between Rust versions.
fn hash_of(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

// ---- the loaded state ----

struct Loaded<T> {
    /// The last good value: the defaults until a file has been read successfully.
    value: Arc<T>,
    /// What the disk looked like at the last look. `None`: the file did not exist (or could not
    /// be read, which makes the next `refresh()` try again).
    stamp: Option<Stamp>,
    hash: Option<u64>,
    /// Set while the file on disk cannot be used. `value` is then the last good one.
    problem: Option<Problem>,
}

/// What the blocking threads share with the `TomlFile`.
struct Shared<T> {
    path: PathBuf,
    file_name: &'static str,
    validate: fn(&T) -> Result<(), String>,
    health: Arc<Health>,
    loaded: Mutex<Loaded<T>>,
    changes: watch::Sender<Arc<T>>,
    /// True once this process has made its `.bak` copy of the file (foundation 10.4, step 6).
    backed_up: AtomicBool,
}

impl<T: DeserializeOwned + Default + Send + Sync + 'static> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, Loaded<T>> {
        // Every change below is a plain assignment of whole values, so a lock that a panic
        // poisoned still holds a consistent state.
        self.loaded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes what the disk holds into the loaded state, and reports a changed problem to
    /// `/health`. A value that changed is announced to the subscribers.
    fn absorb(&self, disk: io::Result<Disk>) {
        let (before, after) = {
            let mut loaded = self.lock();
            let before = loaded.problem.clone();
            match disk {
                Ok(Disk::Missing) => self.absorb_missing(&mut loaded),
                Ok(Disk::Text { text, stamp }) => self.absorb_text(&mut loaded, &text, stamp),
                Err(e) => {
                    loaded.problem = Some(Problem::broken(format!(
                        "Rain could not read this file ({e})"
                    )));
                    loaded.stamp = None;
                    loaded.hash = None;
                }
            }
            (before, loaded.problem.clone())
        };
        if before != after {
            let issues = after
                .iter()
                .map(|problem| problem.issue(self.file_name))
                .collect();
            self.health.set_file_issues(self.file_name, issues);
        }
    }

    /// A file that is gone means all defaults ("a deleted file", foundation 10.4).
    fn absorb_missing(&self, loaded: &mut Loaded<T>) {
        if loaded.hash.is_some() {
            self.replace_value(loaded, T::default());
        }
        loaded.problem = None;
        loaded.stamp = None;
        loaded.hash = None;
    }

    fn absorb_text(&self, loaded: &mut Loaded<T>, text: &str, stamp: Stamp) {
        let hash = hash_of(text);
        if loaded.hash != Some(hash) {
            match parse(text, self.validate) {
                Ok(value) => {
                    self.replace_value(loaded, value);
                    loaded.problem = None;
                }
                Err(problem) => loaded.problem = Some(problem),
            }
            loaded.hash = Some(hash);
        }
        loaded.stamp = Some(stamp);
    }

    fn replace_value(&self, loaded: &mut Loaded<T>, value: T) {
        loaded.value = Arc::new(value);
        self.changes.send_replace(Arc::clone(&loaded.value));
    }

    /// The compare-and-reload that stands in for the watcher.
    fn refresh(&self) {
        let known = self.lock().stamp;
        let mut now = self.stat(known);
        if now.is_some_and(|stamp| self.may_be_mid_save(stamp.len)) {
            self.pause_for_save();
            now = self.stat(known);
        }
        if now != known {
            self.absorb(read_disk(&self.path));
        }
    }

    /// The file's stamp now. `fallback` stands in when the disk cannot be asked; the next call or
    /// the next edit looks again.
    fn stat(&self, fallback: Option<Stamp>) -> Option<Stamp> {
        match fs::metadata(&self.path) {
            Ok(metadata) => Some(Stamp::of(&metadata)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(_) => fallback,
        }
    }

    /// An editor may save by emptying the file and then writing it. An empty file that held text
    /// a moment ago is probably caught half-way, and an empty file is valid TOML, so reading it
    /// now would show all defaults (and an edit would drop the user's other keys).
    fn may_be_mid_save(&self, len_now: u64) -> bool {
        len_now == 0 && self.lock().stamp.is_some_and(|known| known.len > 0)
    }

    fn pause_for_save(&self) {
        // Allowed timer T4 of foundation 11.5: a short wait for one operation. This runs on a
        // blocking thread, so no async worker waits.
        #[allow(clippy::disallowed_methods)]
        std::thread::sleep(TOML_TRUNCATION_SETTLE);
    }

    /// Reads the file for an edit; looks once more when it seems to be caught mid-save.
    fn read_for_edit(&self) -> io::Result<Disk> {
        let disk = read_disk(&self.path)?;
        match &disk {
            Disk::Text { stamp, .. } if self.may_be_mid_save(stamp.len) => {
                self.pause_for_save();
                read_disk(&self.path)
            }
            _ => Ok(disk),
        }
    }

    /// Steps 2 to 7 of the write of foundation section 10.4 (step 1, the lock, is the caller's).
    fn edit(&self, change: impl FnOnce(&mut DocumentMut) -> ApiResult<()>) -> ApiResult<Arc<T>> {
        let file = self.file_name;
        let disk = self.read_for_edit().map_err(|e| {
            error!(file, error = %e, "could not read the file before an edit");
            ApiError::failed(format!("Could not read {file}.")).with_reason("readFailed")
        })?;
        let (text, read_stamp) = match &disk {
            Disk::Missing => (String::new(), None),
            Disk::Text { text, stamp } => (text.clone(), Some(*stamp)),
        };
        // Looking at the fresh text also takes in a hand edit, so the closure sees the user's text.
        self.absorb(Ok(disk));
        if let Some(problem) = self.lock().problem.clone() {
            return Err(problem.refusal(file));
        }

        let mut document: DocumentMut = text
            .parse()
            .map_err(|_| Problem::broken("not valid TOML").refusal(file))?;
        let before = document.to_string();
        change(&mut document)?;
        if document.to_string() == before {
            return Ok(Arc::clone(&self.lock().value)); // nothing changed: write nothing
        }

        let new_text = render(&document, text.contains("\r\n"));
        let value = parse(&new_text, self.validate).map_err(|problem| {
            error!(
                file,
                ?problem,
                "an edit produced a file that cannot be read back"
            );
            save_failed(file)
        })?;
        self.write(&new_text, read_stamp)?;
        Ok(self.remember_written(&new_text, value))
    }

    /// `read_stamp` is how the file looked when the edit read it. If it looks different now, the
    /// user saved it by hand meanwhile, and the rename would silently replace their change.
    fn write(&self, new_text: &str, read_stamp: Option<Stamp>) -> ApiResult<()> {
        let file = self.file_name;
        if !self.backed_up.swap(true, Ordering::Relaxed) && self.path.exists() {
            let backup = self.path.with_file_name(format!("{file}.bak"));
            if let Err(e) = fs::copy(&self.path, &backup) {
                warn!(file, error = %e, "could not make the .bak copy; saving anyway");
            }
        }
        if self.stat(read_stamp) != read_stamp {
            warn!(
                file,
                "the file changed during the edit; nothing was written"
            );
            return Err(ApiError::conflict(
                "configChanged",
                format!("{file} changed while Rain was saving. Try again."),
            ));
        }
        write_atomic(&self.path, new_text).map_err(|e| {
            error!(file, error = %e, "could not write the file");
            save_failed(file)
        })
    }

    /// After our own write the loaded state is what we wrote; no second read is needed.
    fn remember_written(&self, text: &str, value: T) -> Arc<T> {
        let stamp = fs::metadata(&self.path).ok().map(|m| Stamp::of(&m));
        let mut loaded = self.lock();
        self.replace_value(&mut loaded, value);
        loaded.stamp = stamp;
        loaded.hash = Some(hash_of(text));
        Arc::clone(&loaded.value)
    }
}

fn save_failed(file: &str) -> ApiError {
    ApiError::failed(format!("Could not save {file}.")).with_reason("writeFailed")
}

// ---- the public type ----

pub struct TomlFile<T> {
    shared: Arc<Shared<T>>,
    /// One writer at a time: an `edit` or a `refresh` holds it from the read to the end. The
    /// watcher's reloader shares it, so a reload never runs in the middle of an edit.
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl<T: DeserializeOwned + Default + Send + Sync + 'static> TomlFile<T> {
    /// Opens `file_name` in the data folder and reads it once. A missing file means all defaults;
    /// a broken one means defaults plus an issue in `/health`. `validate` states the rules that
    /// the types cannot (lengths, allowed characters); it runs for every load and every write.
    ///
    /// Departure from the foundation signature: `open` is `async`, because the first
    /// read runs on a blocking thread.
    pub async fn open(
        hub: &ConfigHub,
        file_name: &'static str,
        validate: fn(&T) -> Result<(), String>,
    ) -> ApiResult<Self> {
        let (changes, _) = watch::channel(Arc::new(T::default()));
        let shared = Arc::new(Shared {
            path: hub.file_path(file_name),
            file_name,
            validate,
            health: Arc::clone(hub.health()),
            loaded: Mutex::new(Loaded {
                value: Arc::new(T::default()),
                stamp: None,
                hash: None,
                problem: None,
            }),
            changes,
            backed_up: AtomicBool::new(false),
        });
        let first_read = Arc::clone(&shared);
        blocking::run(move || first_read.absorb(read_disk(&first_read.path))).await?;
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        hub.register(&shared.path, reloader(&shared, &lock));
        Ok(Self { shared, lock })
    }

    /// The last good value. Never touches the disk.
    pub fn get(&self) -> Arc<T> {
        Arc::clone(&self.shared.lock().value)
    }

    /// Looks at the file's length and modification time, and reloads it when it changed (a hand
    /// edit). Cheap enough to call before every read of the settings. The watcher does the same
    /// after a change event; this stays as a safety net (foundation 10.4).
    pub async fn refresh(&self) {
        let _writer = self.lock.lock().await;
        let shared = Arc::clone(&self.shared);
        // A failed blocking task is already logged; the last good value simply stays.
        let _ = blocking::run(move || shared.refresh()).await;
    }

    /// Changes the file. `change` gets the file's current text as a document and edits it. It
    /// returns an error when it cannot find what it must change (the answer is then a
    /// `conflict`, reason `configChanged`). When it changes nothing, nothing is written.
    pub async fn edit(
        &self,
        change: impl FnOnce(&mut DocumentMut) -> ApiResult<()> + Send + 'static,
    ) -> ApiResult<Arc<T>> {
        let _writer = self.lock.lock().await;
        let shared = Arc::clone(&self.shared);
        blocking::run(move || shared.edit(change)).await?
    }

    /// Whether an edit would be accepted now: `Ok`, or the refusal an edit would give
    /// (`configBroken` for a file that cannot be read, `configNewer` for a file from a newer
    /// Rain). A change with an effect outside the file asks first, so that it does nothing when
    /// the file would refuse its own part (section 06 6.2, rule R4).
    pub fn writable(&self) -> ApiResult<()> {
        match self.shared.lock().problem.clone() {
            Some(problem) => Err(problem.refusal(self.shared.file_name)),
            None => Ok(()),
        }
    }

    /// Fires when the value changes: on a load that found a hand edit, and on every write.
    pub fn subscribe(&self) -> watch::Receiver<Arc<T>> {
        self.shared.changes.subscribe()
    }
}

/// What the hub calls after a change event for this file: the same `refresh`, started on the
/// runtime and behind the same lock as an edit. A failed reload keeps the last good value.
fn reloader<T: DeserializeOwned + Default + Send + Sync + 'static>(
    shared: &Arc<Shared<T>>,
    lock: &Arc<tokio::sync::Mutex<()>>,
) -> super::hub::Reloader {
    let shared = Arc::clone(shared);
    let lock = Arc::clone(lock);
    let runtime = tokio::runtime::Handle::current();
    Arc::new(move || {
        let (shared, lock) = (Arc::clone(&shared), Arc::clone(&lock));
        runtime.spawn(async move {
            let _writer = lock.lock().await;
            // A failed blocking task is already logged; the last good value simply stays.
            let _ = blocking::run(move || shared.refresh()).await;
        });
    })
}

// ---- writing text ----

/// The text of the document, with the line ends of the original. `toml_edit` turns every CRLF
/// into LF when it parses, so a Windows file would otherwise change all its line ends.
pub fn render(document: &DocumentMut, crlf: bool) -> String {
    let text = document.to_string();
    if crlf {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text
    }
}

/// Replaces `path` with `text`: a temporary file in the same folder (so the last step is a
/// rename on one volume), flushed to disk, then renamed over the target. A program that holds
/// the target open without sharing (an editor, antivirus, a sync client) makes the rename fail
/// for a moment, so it is retried briefly. The temporary file never stays behind.
pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(folder)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // Unique per call, not only per process: two writers of one file must never share a temp file.
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let call = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = folder.join(format!(".{name}.rain-tmp-{}-{call}", std::process::id()));
    let written = fs::File::create(&temp).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all() // the data is on disk before the name flips
    });
    let result = written.and_then(|()| rename_with_retries(&temp, path));
    if result.is_err() {
        // The temporary file may not exist (the failure came before it was made); that is fine.
        let _ = fs::remove_file(&temp);
    }
    result
}

fn rename_with_retries(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match fs::rename(from, to) {
            Err(e) if is_held_by_another_program(&e) && attempt < TOML_WRITE_RETRIES => {
                attempt += 1;
                // Allowed timer T4 of foundation 11.5: a short wait for one operation. This runs
                // on a blocking thread (`TomlFile::edit`), so no async worker waits.
                #[allow(clippy::disallowed_methods)]
                std::thread::sleep(TOML_WRITE_RETRY_DELAY);
            }
            other => return other,
        }
    }
}

/// Windows says "access denied" (5), "sharing violation" (32) or "lock violation" (33) while
/// another program holds the file.
fn is_held_by_another_program(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

// ---- helpers for the code that edits a document ----

/// Sets `key` to `new`. An existing value keeps the text around it (the space before it and a
/// trailing `# comment`); `table[key] = value(..)` would drop that comment (spike finding).
pub fn set_value(table: &mut Table, key: &str, mut new: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            *new.decor_mut() = old.decor().clone();
            *old = new;
        }
        None => {
            new.decor_mut().clear(); // let toml_edit write the usual ` = value`
            table.insert(key, Item::Value(new));
        }
    }
}

/// A string as the user would write it: a literal string (single quotes) so that a Windows path
/// reads naturally, with no doubled backslashes. A string that holds a `'` cannot be a literal,
/// so it becomes an ordinary quoted string.
pub fn string_value(text: &str) -> Value {
    if text.contains('\'') || text.chars().any(char::is_control) {
        return Value::from(text);
    }
    // `toml_edit` cannot set the representation of a string directly, but it keeps the one it
    // parsed. So the literal is written as text and parsed.
    format!("'{text}'")
        .parse()
        .unwrap_or_else(|_| Value::from(text))
}

/// A list of strings, written on one line: `['D:\Code', 'D:\Work']`; an empty list is `[]`.
pub fn string_list_value(items: &[String]) -> Value {
    let written: Vec<String> = items
        .iter()
        .map(|item| string_value(item).to_string().trim().to_owned())
        .collect();
    format!("[{}]", written.join(", "))
        .parse()
        .unwrap_or_else(|_| Value::Array(items.iter().map(String::as_str).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::Duration;

    #[derive(Debug, Default, Deserialize, PartialEq)]
    #[serde(default)]
    struct Sample {
        schema_version: Option<u32>,
        enabled: Option<bool>,
        name: String,
        roots: Vec<String>,
    }

    fn no_rules(_: &Sample) -> Result<(), String> {
        Ok(())
    }

    fn name_must_be_short(sample: &Sample) -> Result<(), String> {
        if sample.name.len() > 8 {
            return Err("name is too long".into());
        }
        Ok(())
    }

    /// A data folder in a temporary place and the hub that goes with it.
    struct Fixture {
        _temp: tempfile::TempDir,
        hub: ConfigHub,
        health: Arc<Health>,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let health = Arc::new(Health::new(
                "start".into(),
                "0.1.0",
                crate::core::bus::EventBus::new(),
            ));
            let hub = ConfigHub::new(temp.path().join("data"), Arc::clone(&health));
            Self {
                _temp: temp,
                hub,
                health,
            }
        }

        fn path(&self) -> PathBuf {
            self.hub.file_path("sample.toml")
        }

        fn write(&self, text: &str) {
            fs::create_dir_all(self.hub.data_dir()).unwrap();
            fs::write(self.path(), text).unwrap();
        }

        fn read(&self) -> String {
            fs::read_to_string(self.path()).unwrap()
        }

        async fn open(&self) -> TomlFile<Sample> {
            TomlFile::open(&self.hub, "sample.toml", no_rules)
                .await
                .unwrap()
        }

        fn issues(&self) -> Vec<ConfigIssue> {
            self.health.snapshot().config_issues
        }
    }

    /// An edit that turns `enabled` on.
    fn enable(document: &mut DocumentMut) -> ApiResult<()> {
        set_value(document.as_table_mut(), "enabled", Value::from(true));
        Ok(())
    }

    // ---- loading ----

    #[tokio::test]
    async fn a_missing_file_means_defaults_and_no_issue() {
        let fixture = Fixture::new();
        let file = fixture.open().await;
        assert_eq!(*file.get(), Sample::default());
        assert!(fixture.issues().is_empty());
        assert!(!fixture.path().exists(), "opening writes nothing");
    }

    #[tokio::test]
    async fn the_values_of_an_existing_file_are_read() {
        let fixture = Fixture::new();
        fixture.write("enabled = true\nname = 'rain'\nroots = ['D:\\Code']\n");
        let file = fixture.open().await;
        assert_eq!(file.get().enabled, Some(true));
        assert_eq!(file.get().name, "rain");
        assert_eq!(file.get().roots, ["D:\\Code"]);
    }

    #[tokio::test]
    async fn a_broken_file_gives_defaults_and_an_issue_with_its_line() {
        let fixture = Fixture::new();
        fixture.write("# header\nenabled = true\nname = [\n");
        let file = fixture.open().await;

        assert_eq!(*file.get(), Sample::default());
        let issues = fixture.issues();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, "sample.toml");
        assert_eq!(issues[0].line, Some(3));
    }

    #[tokio::test]
    async fn a_value_of_the_wrong_type_is_an_issue_not_a_crash() {
        let fixture = Fixture::new();
        fixture.write("enabled = \"yes\"\n");
        let file = fixture.open().await;
        assert_eq!(*file.get(), Sample::default());
        assert_eq!(fixture.issues()[0].line, Some(1));
    }

    #[tokio::test]
    async fn the_rules_of_the_schema_make_a_file_an_issue_too() {
        let fixture = Fixture::new();
        fixture.write("name = 'much too long a name'\n");
        let file = TomlFile::<Sample>::open(&fixture.hub, "sample.toml", name_must_be_short)
            .await
            .unwrap();
        assert_eq!(file.get().name, "");
        assert_eq!(fixture.issues()[0].message, "name is too long");
    }

    #[tokio::test]
    async fn a_newer_schema_is_read_only_and_says_so() {
        let fixture = Fixture::new();
        fixture.write("schema_version = 2\nname = 'future'\n");
        let file = fixture.open().await;

        assert_eq!(fixture.issues()[0].message, "Written by a newer Rain");
        let refusal = file.edit(enable).await.unwrap_err();
        assert_eq!(refusal.body.reason.as_deref(), Some("configNewer"));
        assert_eq!(fixture.read(), "schema_version = 2\nname = 'future'\n");
    }

    // ---- refresh: a hand edit ----

    #[tokio::test]
    async fn refresh_takes_in_a_hand_edit_and_notifies_the_subscribers() {
        let fixture = Fixture::new();
        fixture.write("name = 'one'\n");
        let file = fixture.open().await;
        let mut changes = file.subscribe();

        fixture.write("name = 'two, edited by hand'\n");
        file.refresh().await;

        assert_eq!(file.get().name, "two, edited by hand");
        assert!(changes.has_changed().unwrap());
        assert_eq!(changes.borrow_and_update().name, "two, edited by hand");
    }

    #[tokio::test]
    async fn refresh_of_an_unchanged_file_does_nothing() {
        let fixture = Fixture::new();
        fixture.write("name = 'one'\n");
        let file = fixture.open().await;
        let changes = file.subscribe();

        file.refresh().await;
        assert!(!changes.has_changed().unwrap());
    }

    #[tokio::test]
    async fn a_deleted_file_means_all_defaults() {
        let fixture = Fixture::new();
        fixture.write("name = 'one'\n");
        let file = fixture.open().await;

        fs::remove_file(fixture.path()).unwrap();
        file.refresh().await;
        assert_eq!(*file.get(), Sample::default());
    }

    #[tokio::test]
    async fn a_hand_edit_that_breaks_the_file_keeps_the_last_good_value() {
        let fixture = Fixture::new();
        fixture.write("name = 'good'\n");
        let file = fixture.open().await;

        fixture.write("name = 'good'\nroots = [\n");
        file.refresh().await;
        assert_eq!(file.get().name, "good", "the last good value stays");
        assert_eq!(fixture.issues().len(), 1);

        fixture.write("name = 'fixed again'\n");
        file.refresh().await;
        assert_eq!(file.get().name, "fixed again");
        assert!(
            fixture.issues().is_empty(),
            "the issue goes when the file is fine"
        );
    }

    // ---- edit: comments, order, line ends ----

    const COMMENTED: &str = "# My Rain file\n\
        schema_version = 1\n\
        \n\
        # keep this comment\n\
        enabled = false   # a trailing comment\n\
        name = 'rain'\n\
        unknown_key = 42\n\
        \n\
        [section]\n\
        x = 1  # tail\n";

    #[tokio::test]
    async fn an_edit_changes_one_value_and_keeps_comments_order_and_unknown_keys() {
        let fixture = Fixture::new();
        fixture.write(COMMENTED);
        let file = fixture.open().await;

        let value = file.edit(enable).await.unwrap();

        assert_eq!(value.enabled, Some(true));
        assert_eq!(
            fixture.read(),
            COMMENTED.replace("enabled = false   # a", "enabled = true   # a")
        );
    }

    #[tokio::test]
    async fn an_edit_adds_a_missing_key_at_the_end_of_the_top_level_keys() {
        let fixture = Fixture::new();
        fixture.write("# top\nname = 'rain'\n\n[section]\nx = 1\n");
        let file = fixture.open().await;

        file.edit(enable).await.unwrap();

        let text = fixture.read();
        assert!(
            text.starts_with("# top\nname = 'rain'\nenabled = true\n"),
            "{text}"
        );
        assert!(text.contains("[section]\nx = 1"));
    }

    #[tokio::test]
    async fn a_file_with_crlf_line_ends_keeps_them() {
        let fixture = Fixture::new();
        let crlf = COMMENTED.replace('\n', "\r\n");
        fixture.write(&crlf);
        let file = fixture.open().await;

        file.edit(enable).await.unwrap();

        let text = fixture.read();
        assert_eq!(
            text,
            crlf.replace("enabled = false   # a", "enabled = true   # a")
        );
        assert_eq!(text.matches('\n').count(), text.matches("\r\n").count());
    }

    #[tokio::test]
    async fn an_edit_that_changes_nothing_writes_nothing() {
        let fixture = Fixture::new();
        fixture.write(COMMENTED);
        let file = fixture.open().await;
        let before = fs::metadata(fixture.path()).unwrap().modified().unwrap();
        let mut changes = file.subscribe();
        changes.mark_unchanged();

        file.edit(|_| Ok(())).await.unwrap();

        assert_eq!(
            fs::metadata(fixture.path()).unwrap().modified().unwrap(),
            before
        );
        assert!(
            !changes.has_changed().unwrap(),
            "no change is announced either"
        );
        assert!(!fixture.hub.file_path("sample.toml.bak").exists());
    }

    #[tokio::test]
    async fn an_error_of_the_closure_is_returned_and_writes_nothing() {
        let fixture = Fixture::new();
        fixture.write(COMMENTED);
        let file = fixture.open().await;

        let error = file
            .edit(|_| Err(ApiError::conflict("configChanged", "The file changed.")))
            .await
            .unwrap_err();

        assert_eq!(error.body.reason.as_deref(), Some("configChanged"));
        assert_eq!(fixture.read(), COMMENTED);
    }

    #[tokio::test]
    async fn a_hand_edit_saved_while_an_edit_is_under_way_is_not_overwritten() {
        let fixture = Fixture::new();
        fixture.write("name = 'one'\n");
        let file = fixture.open().await;
        let path = fixture.path();

        // The closure runs between the read and the write of the edit: the user saves then.
        let error = file
            .edit(move |document| {
                fs::write(&path, "name = 'typed by the user meanwhile'\n").unwrap();
                enable(document)
            })
            .await
            .unwrap_err();

        assert_eq!(error.body.reason.as_deref(), Some("configChanged"));
        assert_eq!(fixture.read(), "name = 'typed by the user meanwhile'\n");
    }

    /// Empties the file now and writes `text` into it a moment later, as an editor that saves by
    /// truncating does.
    fn save_slowly(path: PathBuf, text: &'static str) -> std::thread::JoinHandle<()> {
        fs::File::create(&path).unwrap();
        std::thread::spawn(move || {
            #[allow(clippy::disallowed_methods)] // a test thread of its own, nothing else waits
            std::thread::sleep(Duration::from_millis(20));
            fs::write(path, text).unwrap();
        })
    }

    #[tokio::test]
    async fn refresh_does_not_show_defaults_for_a_file_caught_mid_save() {
        let fixture = Fixture::new();
        fixture.write("name = 'one'\n");
        let file = fixture.open().await;
        let mut changes = file.subscribe();

        let saver = save_slowly(fixture.path(), "name = 'two, saved slowly'\n");
        file.refresh().await;
        saver.join().unwrap();

        assert_eq!(file.get().name, "two, saved slowly");
        // The empty moment never reached the subscribers: the only change is to the final value.
        assert_eq!(changes.borrow_and_update().name, "two, saved slowly");
    }

    #[tokio::test]
    async fn an_edit_does_not_drop_the_keys_of_a_file_caught_mid_save() {
        let fixture = Fixture::new();
        fixture.write("name = 'one'\n");
        let file = fixture.open().await;

        let saver = save_slowly(fixture.path(), "name = 'two'\nroots = ['D:/Code']\n");
        file.edit(enable).await.unwrap();
        saver.join().unwrap();

        assert_eq!(file.get().name, "two");
        assert_eq!(file.get().roots, ["D:/Code"]);
        assert_eq!(file.get().enabled, Some(true));
    }

    #[tokio::test]
    async fn a_file_the_user_really_emptied_means_defaults() {
        let fixture = Fixture::new();
        fixture.write("name = 'one'\n");
        let file = fixture.open().await;

        fixture.write("");
        file.refresh().await;
        assert_eq!(*file.get(), Sample::default());
    }

    #[tokio::test]
    async fn the_first_edit_creates_the_file_and_the_data_folder() {
        let fixture = Fixture::new();
        let file = fixture.open().await;
        assert!(!fixture.hub.data_dir().exists());

        file.edit(enable).await.unwrap();

        assert_eq!(fixture.read(), "enabled = true\n");
        assert!(
            !fixture.hub.file_path("sample.toml.bak").exists(),
            "nothing to back up yet"
        );
    }

    #[tokio::test]
    async fn the_first_write_of_a_run_keeps_a_bak_copy_and_later_writes_do_not_replace_it() {
        let fixture = Fixture::new();
        fixture.write(COMMENTED);
        let file = fixture.open().await;

        file.edit(enable).await.unwrap();
        file.edit(|document| {
            set_value(document.as_table_mut(), "name", string_value("second"));
            Ok(())
        })
        .await
        .unwrap();

        let backup = fs::read_to_string(fixture.hub.file_path("sample.toml.bak")).unwrap();
        assert_eq!(
            backup, COMMENTED,
            "the copy holds the file as it was before Rain wrote"
        );
    }

    // ---- edit: a hand edit between two writes ----

    #[tokio::test]
    async fn a_file_edited_by_hand_between_two_writes_is_not_overwritten() {
        let fixture = Fixture::new();
        fixture.write("# mine\nname = 'one'\n");
        let file = fixture.open().await;
        file.edit(enable).await.unwrap();

        // The user edits the file in an editor. Rain does not know yet.
        fixture.write("# mine\nname = 'one'\nenabled = true\nroots = ['added by hand']\n");
        let value = file
            .edit(|document| {
                set_value(document.as_table_mut(), "name", string_value("two"));
                Ok(())
            })
            .await
            .unwrap();

        assert_eq!(
            value.roots,
            ["added by hand"],
            "the closure worked on the user's text"
        );
        let text = fixture.read();
        assert!(text.contains("roots = ['added by hand']"), "{text}");
        assert!(text.contains("name = 'two'"), "{text}");
    }

    #[tokio::test]
    async fn an_edit_refuses_a_file_that_has_an_issue_and_leaves_it_alone() {
        let fixture = Fixture::new();
        fixture.write("name = [\n");
        let file = fixture.open().await;

        let error = file.edit(enable).await.unwrap_err();

        assert_eq!(error.body.reason.as_deref(), Some("configBroken"));
        assert!(
            error.body.message.contains("line 1"),
            "{}",
            error.body.message
        );
        assert_eq!(fixture.read(), "name = [\n");
    }

    #[tokio::test]
    async fn an_edit_that_would_break_the_schema_is_not_written() {
        let fixture = Fixture::new();
        fixture.write("name = 'rain'\n");
        let file = TomlFile::<Sample>::open(&fixture.hub, "sample.toml", name_must_be_short)
            .await
            .unwrap();

        let error = file
            .edit(|document| {
                set_value(
                    document.as_table_mut(),
                    "name",
                    string_value("far too long"),
                );
                Ok(())
            })
            .await
            .unwrap_err();

        assert_eq!(error.body.reason.as_deref(), Some("writeFailed"));
        assert_eq!(fixture.read(), "name = 'rain'\n");
    }

    // ---- the helpers for editing ----

    #[test]
    fn a_replaced_value_keeps_its_trailing_comment() {
        let mut document: DocumentMut = "a = 1   # keep me\n".parse().unwrap();
        set_value(document.as_table_mut(), "a", Value::from(2));
        assert_eq!(document.to_string(), "a = 2   # keep me\n");
    }

    #[test]
    fn a_new_key_is_written_in_the_usual_way() {
        let mut document: DocumentMut = "a = 1\n".parse().unwrap();
        set_value(document.as_table_mut(), "b", string_value("text"));
        assert_eq!(document.to_string(), "a = 1\nb = 'text'\n");
    }

    #[test]
    fn strings_are_written_as_literals_and_read_back_exactly() {
        let samples = [
            r"D:\Code",
            r"\\server\share\folder",
            "with space",
            r#"say "hi""#,
            "it's quoted",
            r"mixed 'single' and \ backslash",
            "tab\there",
            "",
        ];
        for sample in samples {
            let mut document = DocumentMut::new();
            set_value(document.as_table_mut(), "k", string_value(sample));
            let text = document.to_string();
            let back: toml::Table = toml::from_str(&text).unwrap();
            assert_eq!(back["k"].as_str(), Some(sample), "written as {text}");
        }
        assert_eq!(string_value(r"D:\Code").to_string(), r"'D:\Code'");
    }

    #[test]
    fn a_list_of_strings_is_written_on_one_line_and_read_back() {
        let items: Vec<String> = [r"D:\Code", "it's", "plain"].map(String::from).to_vec();
        let mut document = DocumentMut::new();
        set_value(document.as_table_mut(), "roots", string_list_value(&items));
        let text = document.to_string();
        assert!(text.starts_with("roots = ['D:\\Code', "), "{text}");
        let back: toml::Table = toml::from_str(&text).unwrap();
        let read: Vec<&str> = back["roots"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(toml::Value::as_str)
            .collect();
        assert_eq!(read, ["D:\\Code", "it's", "plain"]);

        let mut empty = DocumentMut::new();
        set_value(empty.as_table_mut(), "roots", string_list_value(&[]));
        assert_eq!(empty.to_string(), "roots = []\n");
    }

    #[test]
    fn a_replaced_list_keeps_its_comment() {
        let mut document: DocumentMut = "roots = ['a']   # my folders\n".parse().unwrap();
        set_value(
            document.as_table_mut(),
            "roots",
            string_list_value(&["b".to_owned(), "c".to_owned()]),
        );
        assert_eq!(document.to_string(), "roots = ['b', 'c']   # my folders\n");
    }

    // ---- the atomic replace ----

    fn only_entry_names(folder: &Path) -> Vec<String> {
        fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a.toml");
        fs::write(&path, "v = 1\n").unwrap();

        write_atomic(&path, "v = 2\n").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "v = 2\n");
        assert_eq!(only_entry_names(temp.path()), ["a.toml"]);
    }

    #[test]
    fn a_program_that_shares_the_file_with_delete_does_not_block_the_write() {
        // Rust's own `File::open` shares read, write and delete, like most editors.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a.toml");
        fs::write(&path, "v = 1\n").unwrap();
        let _holder = fs::File::open(&path).unwrap();

        write_atomic(&path, "v = 2\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "v = 2\n");
    }

    #[test]
    fn a_file_that_stays_locked_fails_cleanly_and_is_written_when_released() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a.toml");
        fs::write(&path, "v = 1\n").unwrap();
        // Share mode 1 is "read only": nobody may delete or replace the file while this is open.
        let holder = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();

        assert!(write_atomic(&path, "v = 2\n").is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "v = 1\n",
            "the target is untouched"
        );
        assert_eq!(
            only_entry_names(temp.path()),
            ["a.toml"],
            "the temporary file is removed"
        );

        drop(holder);
        write_atomic(&path, "v = 3\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "v = 3\n");
    }

    #[test]
    fn a_lock_that_ends_inside_the_retry_window_does_not_fail_the_write() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a.toml");
        fs::write(&path, "v = 1\n").unwrap();
        let holder = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let release = std::thread::spawn(move || {
            // A test thread, not the backend: it only has to outlast one retry.
            #[allow(clippy::disallowed_methods)]
            std::thread::sleep(Duration::from_millis(80));
            drop(holder);
        });

        write_atomic(&path, "v = 2\n").unwrap();

        release.join().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "v = 2\n");
    }
}
