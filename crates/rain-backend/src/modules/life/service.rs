//! The logic and the in-memory state of Life. Other modules call it, and only it. Section 04.
//!
//! What lives here: the list of habits (from `habits.toml`), the note index (which daily notes
//! exist and what they tick), the summaries and stats the screen shows, the tick (the one vault
//! write, through `vault_write`), opening a note in Obsidian, and the `habits` topic.
//!
//! Reading is always safe: a problem with the vault never fails a read, it only makes the
//! history empty (the screen asks `GET /vault` why). Only the tick, the open and the habit
//! edits can answer errors.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use rain_types::limits::TOPIC_RATE_MAX;
use rain_types::{
    Habit, HabitDay, HabitDraft, HabitStats, HabitSummary, Habits, OpenNoteResult,
    PropertySuggestion, SourceState, VaultStatus,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::daily::{ConfigError, DailyConfig, normalise_folder};
use super::dates::CivilDate;
use super::index::{self, Index, RealFs, VaultFs};
use super::store::{self, HabitsFile, WriteRow};
use super::vault_write::{self, WriteError};
use super::{
    HABIT_DAYS_RANGE_MAX, RESTAT_MIN_INTERVAL, SOURCE, SUGGESTIONS_MAX, TICK_WINDOW_DAYS,
    VAULT_SOURCE, WATCH_QUIET, frontmatter, moment, stats,
};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::health::Health;
use crate::core::launch_contract::OpenRequest;
use crate::core::state::{AppState, WeakAppState};
use crate::core::throttle::Throttle;
use crate::modules::settings::Resolved;
use crate::platform::time::local_date;
use crate::store::hub::DirWatch;
use crate::store::toml_file::TomlFile;

pub struct Service {
    /// Weak: the state owns this service.
    state: WeakAppState,
    health: Arc<Health>,
    file: TomlFile<HabitsFile>,
    /// The habits of `habits.toml`, rebuilt whenever the file's value changes.
    habits: Mutex<Arc<Vec<Habit>>>,
    /// The note index. Held for the whole check, so that two reads never check twice.
    index: AsyncMutex<Option<Arc<Index>>>,
    /// Something may have changed in the notes (the watcher, a tick, a re-read, a settings
    /// change): the next read looks at the files again.
    dirty: AtomicBool,
    /// Two ticks never overlap.
    write_lock: Arc<AsyncMutex<()>>,
    throttle: Throttle<Habits>,
    /// What the `habits` topic last carried, so that an unchanged state sends no frame.
    published: Mutex<Option<Vec<HabitSummary>>>,
    fs: Arc<dyn VaultFs>,
    task: Mutex<Option<(CancellationToken, JoinHandle<()>)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // Every change of the guarded values is a whole assignment, so a poisoned lock is still
    // consistent.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- errors of this module ----

/// A request that is wrong, with the reason of the API (section 8).
fn invalid(reason: &'static str, message: &str) -> ApiError {
    ApiError::invalid(message)
        .with_source(SOURCE)
        .with_reason(reason)
}

fn unknown_habit() -> ApiError {
    ApiError::not_found("Habit not found")
        .with_source(SOURCE)
        .with_reason("unknownHabit")
}

fn bad_date() -> ApiError {
    invalid("badDate", "That is not a valid date")
}

fn parse_date(text: &str) -> ApiResult<CivilDate> {
    CivilDate::parse(text).ok_or_else(bad_date)
}

/// The vault is not usable: no vault, a wrong path, a format or folder that cannot work.
fn vault_unavailable(reason: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::unavailable(VAULT_SOURCE, message).with_reason(reason)
}

fn config_error(error: ConfigError) -> ApiError {
    match error {
        ConfigError::NoVault => vault_unavailable(
            "noVault",
            "No Obsidian vault is set. Choose one in Settings.",
        ),
        ConfigError::BadFormat(text) => vault_unavailable("badDailyFormat", text),
        ConfigError::BadFolder => vault_unavailable(
            "badDailyFolder",
            "The daily notes folder is not a folder inside the vault. Change it in Settings.",
        ),
    }
}

fn vault_conflict(reason: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::conflict(reason, message).with_source(VAULT_SOURCE)
}

/// The answer for a write that was refused or failed. The messages never hold note text.
fn write_error(error: WriteError, property: &str) -> ApiError {
    use frontmatter::NoteError;
    match error {
        WriteError::Note(NoteError::InvalidKey) => {
            ApiError::invalid("That property name cannot be used in a note")
                .with_source(SOURCE)
                .with_reason("badProperty")
        }
        WriteError::Note(NoteError::DuplicateKey) => vault_conflict(
            "duplicateKey",
            format!(
                "The property '{property}' appears twice in that note, so Rain left the note alone."
            ),
        ),
        WriteError::Note(NoteError::NotABoolean) => vault_conflict(
            "notBoolean",
            format!(
                "The property '{property}' in that note is not a checkbox, so Rain left the note alone."
            ),
        ),
        WriteError::Note(NoteError::UnreadableFrontmatter) => vault_conflict(
            "unreadableNote",
            "The top of that note is not in a format Rain understands, so Rain left the note alone.",
        ),
        WriteError::NotUtf8 => vault_conflict(
            "notUtf8",
            "That note is not valid UTF-8, so Rain left the note alone.",
        ),
        WriteError::NoFolder => vault_unavailable(
            "noDailyFolder",
            "The folder for that note does not exist. Rain never creates folders in your vault.",
        ),
        WriteError::Busy => vault_conflict(
            "noteBusy",
            "The note keeps changing. Try again in a moment.",
        ),
        WriteError::Io(e) => {
            // The operating system's text holds no path and no note text.
            error!(error = %e, "could not write a daily note");
            ApiError::failed("Could not save the note.")
                .with_source(VAULT_SOURCE)
                .with_reason("writeFailed")
        }
    }
}

/// The answer of `TomlFile::edit` for this module: the source is named, and a failed write is
/// `saveFailed` (section 8).
fn edit_error(error: ApiError) -> ApiError {
    let error = error.with_source(SOURCE);
    if error.body.reason.as_deref() == Some("writeFailed") {
        return ApiError::failed("Could not save habits.toml")
            .with_source(SOURCE)
            .with_reason("saveFailed");
    }
    error
}

// ---- the vault write rule W1 ----

/// A tick is accepted when this is not a sandbox run, or when the vault lies inside the data
/// folder (a fixture vault). In a debug build, in tests and with `RAIN_DATA_DIR` set, the real
/// vault is therefore never written.
fn write_allowed(vault: &Path, sandbox: bool, data_dir: &Path) -> bool {
    if !sandbox {
        return true;
    }
    match (
        std::fs::canonicalize(vault),
        std::fs::canonicalize(data_dir),
    ) {
        (Ok(vault), Ok(data)) => vault.starts_with(data),
        _ => false,
    }
}

// ---- the service ----

impl Service {
    pub async fn new(state: &AppState) -> ApiResult<Self> {
        let file = store::open(&state.hub).await?;
        let fs: Arc<dyn VaultFs> = state
            .seam::<dyn VaultFs>()
            .unwrap_or_else(|| Arc::new(RealFs));
        let service = Self {
            state: state.downgrade(),
            health: Arc::clone(&state.health),
            file,
            habits: Mutex::new(Arc::new(Vec::new())),
            index: AsyncMutex::new(None),
            dirty: AtomicBool::new(true),
            write_lock: Arc::new(AsyncMutex::new(())),
            throttle: Throttle::new(state.bus.clone(), TOPIC_RATE_MAX),
            published: Mutex::new(None),
            fs,
            task: Mutex::new(None),
        };
        service.apply_file(&service.file.get());
        Ok(service)
    }

    /// Called by the settings module before it saves a daily note format: a format with a
    /// token that Rain cannot render is refused with a sentence that names it.
    pub fn check_note_format(&self, format: &str) -> Result<(), String> {
        moment::validate(format)
    }

    fn state(&self) -> ApiResult<AppState> {
        self.state.upgrade().ok_or_else(|| {
            ApiError::unavailable("backend", "Rain is stopping").with_reason("shuttingDown")
        })
    }

    fn today(state: &AppState) -> CivilDate {
        CivilDate::parse(&local_date(state.clock.now()))
            .expect("the local date is always a real date")
    }

    fn habits(&self) -> Arc<Vec<Habit>> {
        Arc::clone(&lock(&self.habits))
    }

    fn habit(&self, id: &str) -> ApiResult<Habit> {
        self.habits()
            .iter()
            .find(|habit| habit.id == id)
            .cloned()
            .ok_or_else(unknown_habit)
    }

    /// Takes the value of `habits.toml` in: the list of habits, and the issues of the entries
    /// that were skipped. A file that does not parse keeps its own issue (the file object
    /// reports that one), so the entry issues are only set while the file is fine.
    fn apply_file(&self, file: &HabitsFile) {
        let (habits, issues) = store::load(file);
        *lock(&self.habits) = Arc::new(habits);
        if self.file.writable().is_ok() {
            self.health.set_file_issues(store::FILE_NAME, issues);
        }
    }

    // ---- the vault ----

    /// `GET /vault`: every problem is a field of the answer, never an error.
    pub async fn vault_status(&self) -> ApiResult<VaultStatus> {
        let state = self.state()?;
        let status = self.compute_status(&state).await?;
        report_health(&state, &status);
        Ok(status)
    }

    async fn compute_status(&self, state: &AppState) -> ApiResult<VaultStatus> {
        let resolved = state.settings().resolved();
        let (sandbox, data_dir) = (state.config.sandbox, state.config.data_dir.clone());
        blocking::run(move || status_of(&resolved, sandbox, &data_dir)).await
    }

    /// `POST /vault/reread`: looks at the notes again now, and sends the habits when they
    /// changed.
    pub async fn reread(&self) -> ApiResult<()> {
        self.dirty.store(true, Ordering::SeqCst);
        self.refresh().await?;
        self.publish_if_changed().await;
        Ok(())
    }

    /// The checkbox properties of the notes that no habit uses yet.
    pub async fn suggestions(&self) -> ApiResult<Vec<PropertySuggestion>> {
        let index = self.refresh().await?;
        let used: Vec<String> = self.habits().iter().map(|h| h.property.clone()).collect();
        // property -> (days, newest day)
        let mut seen: BTreeMap<&str, (u32, CivilDate)> = BTreeMap::new();
        for (date, note) in &index.notes {
            for (name, _) in &note.bools {
                let entry = seen.entry(name.as_str()).or_insert((0, *date));
                entry.0 += 1;
                entry.1 = entry.1.max(*date);
            }
        }
        let mut list: Vec<PropertySuggestion> = seen
            .into_iter()
            .filter(|(name, _)| !used.iter().any(|u| u == name))
            .filter(|(name, _)| frontmatter::check_key(name).is_ok())
            .map(|(name, (days, last))| PropertySuggestion {
                property: name.to_owned(),
                days,
                last_seen: last.to_string(),
            })
            .collect();
        list.sort_by(|a, b| {
            b.days
                .cmp(&a.days)
                .then_with(|| b.last_seen.cmp(&a.last_seen))
                .then_with(|| a.property.cmp(&b.property))
        });
        list.truncate(SUGGESTIONS_MAX);
        Ok(list)
    }

    // ---- reading the notes ----

    /// The index, checked again when it is out of date: another vault, folder or format, a new
    /// day, something that may have changed, or a last check older than two seconds. In game
    /// mode the cached index is served and no file is touched.
    async fn refresh(&self) -> ApiResult<Arc<Index>> {
        let state = self.state()?;
        let today = Self::today(&state);
        let resolved = state.settings().resolved();
        let config = DailyConfig::new(
            resolved.vault.as_deref(),
            &resolved.daily_note_folder,
            &resolved.daily_note_format,
        );
        let mut guard = self.index.lock().await;
        let Ok(config) = config else {
            // No vault, or a format or folder that cannot work: nothing to read.
            let empty = Arc::new(Index::empty(today));
            *guard = Some(Arc::clone(&empty));
            return Ok(empty);
        };
        if state.game_mode_active() {
            return Ok(guard
                .clone()
                .unwrap_or_else(|| Arc::new(Index::empty(today))));
        }
        let scope = config.scope();
        if let Some(index) = guard.as_ref() {
            let fresh = index.scope == scope
                && index.today == today
                && !self.dirty.load(Ordering::SeqCst)
                && index.checked_at.elapsed() < RESTAT_MIN_INTERVAL;
            if fresh {
                return Ok(Arc::clone(index));
            }
        }
        // A change during the check sets the flag again, so it is cleared first.
        self.dirty.store(false, Ordering::SeqCst);

        // What was known: the last index of this scope, else the cache in the database.
        let from_cache = !guard.as_ref().is_some_and(|index| index.scope == scope);
        let previous = match guard.as_ref().filter(|index| index.scope == scope) {
            Some(index) => index.notes.clone(),
            None => load_cached_notes(&state, &scope).await,
        };

        let (fs, check_config, known) = (Arc::clone(&self.fs), config.clone(), previous.clone());
        let checked = blocking::run(move || {
            if !matches!(fs.metadata(&check_config.vault), Ok(Some(meta)) if !meta.is_file) {
                return index::Checked {
                    notes: BTreeMap::new(),
                    unreadable: 0,
                };
            }
            index::check(fs.as_ref(), &check_config, today, &known)
        })
        .await?;
        if checked.unreadable > 0 {
            // A count only: never the name of a note.
            debug!(count = checked.unreadable, "daily notes could not be read");
        }

        // The cache follows the check: changed rows, deleted rows, and (after a change of
        // scope) the rows of the old scope and of days before the window.
        let (changed, removed) = index::diff(&previous, &checked.notes);
        if from_cache || !changed.is_empty() || !removed.is_empty() {
            let window = index::window_from(today);
            let rows_scope = scope.clone();
            let saved = state
                .db
                .call(move |db| store::save_notes(db, &rows_scope, &changed, &removed, window))
                .await;
            if let Err(e) = saved {
                warn!(error = %e, "the cache of the daily notes could not be saved");
            }
        }

        let fresh = Arc::new(Index {
            scope,
            today,
            notes: checked.notes,
            checked_at: Instant::now(),
        });
        *guard = Some(Arc::clone(&fresh));
        drop(guard);
        if let Ok(status) = self.compute_status(&state).await {
            report_health(&state, &status);
        }
        Ok(fresh)
    }

    fn summaries(&self, index: &Index) -> Vec<HabitSummary> {
        self.habits()
            .iter()
            .map(|habit| {
                let lookup = stats::Lookup {
                    notes: &index.notes,
                    property: &habit.property,
                };
                stats::summarise(habit, &lookup, index.today)
            })
            .collect()
    }

    /// `GET /habits`.
    pub async fn list_habits(&self) -> ApiResult<Vec<HabitSummary>> {
        let index = self.refresh().await?;
        let summaries = self.summaries(&index);
        // A page that fetched the list knows it: when nothing was sent yet, this is the state
        // the topic compares with, so the first `reread` of an unchanged vault sends no frame.
        lock(&self.published).get_or_insert_with(|| summaries.clone());
        Ok(summaries)
    }

    /// `GET /habits/{id}/stats`.
    pub async fn habit_stats(&self, id: &str) -> ApiResult<HabitStats> {
        let habit = self.habit(id)?;
        let index = self.refresh().await?;
        let lookup = stats::Lookup {
            notes: &index.notes,
            property: &habit.property,
        };
        Ok(stats::stats(&habit, &lookup, index.today))
    }

    /// `GET /habit-days`: the days that have a note, oldest first, and which habits each ticked.
    pub async fn habit_days(
        &self,
        from: &str,
        to: &str,
        habit: Option<&str>,
    ) -> ApiResult<Vec<HabitDay>> {
        let (from, to) = (parse_date(from)?, parse_date(to)?);
        if from > to || from.days_until(to) + 1 > HABIT_DAYS_RANGE_MAX {
            return Err(invalid("badRange", "The date range is not valid"));
        }
        let habits = match habit {
            Some(id) => vec![self.habit(id)?],
            None => self.habits().to_vec(),
        };
        let index = self.refresh().await?;
        Ok(index
            .notes
            .range(from..=to)
            .map(|(date, note)| HabitDay {
                date: date.to_string(),
                done: habits
                    .iter()
                    .filter(|habit| note.value(&habit.property) == Some(true))
                    .map(|habit| habit.id.clone())
                    .collect(),
            })
            .collect())
    }

    /// `GET /calendar/events`: no calendar is connected, so there are never any events. The
    /// route exists so that a calendar can be added later.
    pub fn calendar_events(
        &self,
        from: &str,
        to: &str,
    ) -> ApiResult<Vec<rain_types::CalendarEvent>> {
        parse_date(from)?;
        parse_date(to)?;
        Ok(Vec::new())
    }

    // ---- habit definitions ----

    /// `POST /habits` (`path_id` is `None`) and `PUT /habits/{id}`.
    pub async fn save_habit(&self, path_id: Option<&str>, draft: HabitDraft) -> ApiResult<Habit> {
        let clean = store::check_draft(&draft, path_id)?;
        if let Some(id) = path_id {
            self.habit(id)?; // 404 for a habit that does not exist
        }
        let edited = match path_id {
            None => {
                let (new, name) = (clean, draft.name.trim().to_owned());
                let property = new.property.clone();
                let file = self
                    .file
                    .edit(move |doc| store::append_habit(doc, &new, &name))
                    .await
                    .map_err(edit_error)?;
                (file, Found::Property(property))
            }
            Some(id) => {
                let id = id.to_owned();
                let id_for_edit = id.clone();
                let file = self
                    .file
                    .edit(move |doc| store::update_habit(doc, &id_for_edit, &clean))
                    .await
                    .map_err(edit_error)?;
                (file, Found::Id(id))
            }
        };
        let (file, found) = edited;
        self.apply_file(&file);
        self.publish_always().await;
        self.habits()
            .iter()
            .find(|habit| match &found {
                Found::Property(property) => &habit.property == property,
                Found::Id(id) => &habit.id == id,
            })
            .cloned()
            .ok_or_else(|| {
                // The file was changed by hand between the write and now.
                ApiError::conflict("configChanged", "habits.toml changed. Try again.")
                    .with_source(SOURCE)
            })
    }

    /// `DELETE /habits/{id}`: removes the definition only. No note is touched. A habit that is
    /// already gone is fine.
    pub async fn delete_habit(&self, id: &str) -> ApiResult<()> {
        let id = id.to_owned();
        let file = self
            .file
            .edit(move |doc| store::remove_habit(doc, &id))
            .await
            .map_err(edit_error)?;
        self.apply_file(&file);
        self.publish_always().await;
        Ok(())
    }

    // ---- the tick: the one write to the vault ----

    /// `PUT /habits/{id}/days/{date}`. Validates, writes the one property on a blocking thread,
    /// looks at the note again, and answers the new summary of the habit.
    ///
    /// This is the ONLY place that calls `vault_write::set_habit_property`, and only the route
    /// handler calls this (a rule of `tests/source_rules.rs`).
    pub async fn tick(&self, id: &str, date: &str, done: bool) -> ApiResult<HabitSummary> {
        let state = self.state()?;
        // 1. Validate.
        let day = parse_date(date)?;
        let today = Self::today(&state);
        if day > today {
            return Err(invalid("futureDate", "That day has not happened yet"));
        }
        if day < today.add_days(-(TICK_WINDOW_DAYS - 1)) {
            return Err(invalid(
                "tooOld",
                "Rain only changes the last 31 days. Change older days in Obsidian.",
            ));
        }
        let habit = self.habit(id)?;
        let resolved = state.settings().resolved();
        let config = DailyConfig::new(
            resolved.vault.as_deref(),
            &resolved.daily_note_folder,
            &resolved.daily_note_format,
        )
        .map_err(config_error)?;
        let note = config.file_for(day).map_err(config_error)?;
        let (sandbox, data_dir) = (state.config.sandbox, state.config.data_dir.clone());
        let (vault, folder) = (config.vault.clone(), note.parent().map(Path::to_path_buf));
        let checked = blocking::run(move || {
            if !vault.is_dir() {
                return Err(config_error_vault_missing(&vault));
            }
            if !write_allowed(&vault, sandbox, &data_dir) {
                return Err(vault_conflict(
                    "sandbox",
                    "Ticking is switched off in this development build.",
                ));
            }
            match folder {
                Some(folder) if folder.is_dir() => Ok(()),
                _ => Err(vault_unavailable(
                    "noDailyFolder",
                    "The folder for that note does not exist. Rain never creates folders in your vault.",
                )),
            }
        })
        .await?;
        checked?;

        // 2. The one write. Two ticks never overlap.
        let _writing = Arc::clone(&self.write_lock).lock_owned().await;
        let (path, key) = (note, habit.property.clone());
        let tick = blocking::run(move || vault_write::set_habit_property(&path, &key, done))
            .await?
            .map_err(|e| write_error(e, &habit.property))?;

        // 3. Look at the notes again, keep the audit row, tell the pages.
        self.dirty.store(true, Ordering::SeqCst);
        let index = self.refresh().await?;
        let row = WriteRow {
            at: rain_types::time::iso_from_system_time(state.clock.now()),
            date: day.to_string(),
            habit_id: habit.id.clone(),
            property: habit.property.clone(),
            done,
            outcome: tick.as_str(),
        };
        if let Err(e) = state.db.call(move |db| store::insert_write(db, &row)).await {
            warn!(error = %e, "the audit row of a tick could not be saved");
        }
        info!(habit = %habit.id, date = %day, outcome = tick.as_str(), "habit tick");
        let summaries = self.summaries(&index);
        self.publish_summaries(summaries.clone(), true);
        summaries
            .into_iter()
            .find(|summary| summary.habit.id == habit.id)
            .ok_or_else(unknown_habit)
    }

    // ---- opening a note ----

    /// `POST /daily-notes/{date}/open`: asks Obsidian to open the note. A note that does not
    /// exist is NOT opened (`opened: false`): a link to a missing note might make Obsidian
    /// create it, so the file is checked first.
    pub async fn open_note(&self, date: &str) -> ApiResult<OpenNoteResult> {
        let state = self.state()?;
        let day = parse_date(date)?;
        let resolved = state.settings().resolved();
        let config = DailyConfig::new(
            resolved.vault.as_deref(),
            &resolved.daily_note_folder,
            &resolved.daily_note_format,
        )
        .map_err(config_error)?;
        let file = config.file_for(day).map_err(config_error)?;
        let (fs, vault, note) = (Arc::clone(&self.fs), config.vault.clone(), file);
        let exists = blocking::run(move || {
            if !vault.is_dir() {
                return Err(config_error_vault_missing(&vault));
            }
            Ok(matches!(fs.metadata(&note), Ok(Some(meta)) if meta.is_file))
        })
        .await??;
        if !exists {
            return Ok(OpenNoteResult { opened: false });
        }
        let link = config
            .open_link(day, state.settings().vault_id().as_deref())
            .map_err(config_error)?;
        let request = OpenRequest::ShellExec {
            file: link,
            params: None,
            dir: None,
        };
        match state.opener().open(request).await {
            Ok(_) => Ok(OpenNoteResult { opened: true }),
            Err(e) => {
                warn!(code = ?e.code, reason = %e.log_reason, "Obsidian could not be started");
                Err(ApiError::failed("Could not start Obsidian.")
                    .with_source(VAULT_SOURCE)
                    .with_reason("openFailed"))
            }
        }
    }

    // ---- the topic ----

    fn publish_summaries(&self, summaries: Vec<HabitSummary>, force: bool) {
        let mut last = lock(&self.published);
        if !force && last.as_ref() == Some(&summaries) {
            return;
        }
        *last = Some(summaries.clone());
        drop(last);
        self.throttle.publish(&summaries);
    }

    /// Sends the habits when they differ from what the topic last carried. Nothing is computed
    /// while no page is connected; the next page fetches the list itself when it connects.
    async fn publish_if_changed(&self) {
        let Ok(state) = self.state() else { return };
        if !state.bus.has_receivers() {
            *lock(&self.published) = None;
            return;
        }
        if let Ok(index) = self.refresh().await {
            let summaries = self.summaries(&index);
            self.publish_summaries(summaries, false);
        }
    }

    /// Sends the habits whatever the last frame was (after a tick or an edit of the file).
    async fn publish_always(&self) {
        let Ok(state) = self.state() else { return };
        if !state.bus.has_receivers() {
            *lock(&self.published) = None;
            return;
        }
        if let Ok(index) = self.refresh().await {
            let summaries = self.summaries(&index);
            self.publish_summaries(summaries, true);
        }
    }

    // ---- background work ----

    /// Starts the one task of this module: it waits for changes (the daily notes folder,
    /// `habits.toml`, the settings) and wakes at no other time.
    pub fn start(self: &Arc<Self>, state: &AppState) {
        let token = state.shutdown.token().child_token();
        let handle =
            crate::core::task::spawn_named("life-watch", Arc::clone(self).run(token.clone()));
        *lock(&self.task) = Some((token, handle));
    }

    /// Ends the task.
    pub async fn stop(&self) {
        let task = lock(&self.task).take();
        if let Some((token, handle)) = task {
            token.cancel();
            let _ = handle.await;
        }
    }

    async fn run(self: Arc<Self>, stop: CancellationToken) {
        let Ok(state) = self.state() else { return };
        let mut settings = state.settings().watch();
        let mut habits_file = self.file.subscribe();
        let mut watch = self.watch_notes(&state);
        let mut visible = state.shell_facts.watch_visible();
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                changed = settings.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // Another vault, folder or format: look again, watch the new folder.
                    settings.borrow_and_update();
                    self.dirty.store(true, Ordering::SeqCst);
                    watch = self.watch_notes(&state);
                    self.publish_if_changed().await;
                }
                changed = habits_file.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let value = Arc::clone(&habits_file.borrow_and_update());
                    self.apply_file(&value);
                    self.publish_if_changed().await;
                }
                changed = visible.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // Shown again: the notes that changed while it was hidden are read now.
                    if *visible.borrow_and_update() {
                        self.publish_if_changed().await;
                    }
                }
                _ = next_change(&watch) => {
                    // The notes folder changed (Obsidian saved, a sync tool wrote). Correctness
                    // does not depend on this: a read looks again by itself.
                    if state.game_mode_active() {
                        continue;
                    }
                    self.dirty.store(true, Ordering::SeqCst);
                    if !state.shell_facts.window_visible() {
                        // Nobody looks: the notes are read when the window is shown.
                        continue;
                    }
                    self.publish_if_changed().await;
                }
            }
        }
    }

    /// A watch on the daily notes folder (only that folder, never the whole vault, which may
    /// be a git repository with a lot of traffic). None when the folder is the vault root or
    /// does not exist: reading still works, it just is not pushed.
    fn watch_notes(&self, state: &AppState) -> Option<DirWatch> {
        let resolved: Arc<Resolved> = state.settings().resolved();
        let config = DailyConfig::new(
            resolved.vault.as_deref(),
            &resolved.daily_note_folder,
            &resolved.daily_note_format,
        )
        .ok()?;
        if config.folder.is_empty() {
            return None;
        }
        let folder: PathBuf = config.folder_path();
        if !folder.is_dir() {
            return None;
        }
        state.hub.watch_dir(&folder, false, WATCH_QUIET).ok()
    }
}

/// How a habit that was just saved is found again in the list.
enum Found {
    Property(String),
    Id(String),
}

/// Waits for the next change of the watched folder; forever when nothing is watched.
async fn next_change(watch: &Option<DirWatch>) {
    match watch {
        Some(watch) => {
            watch.changed().await;
        }
        None => std::future::pending::<()>().await,
    }
}

fn config_error_vault_missing(vault: &Path) -> ApiError {
    vault_unavailable(
        "vaultMissing",
        format!("Obsidian vault not found at {}", vault.display()),
    )
}

/// The cached rows of a scope, or none when the database cannot answer.
async fn load_cached_notes(state: &AppState, scope: &str) -> BTreeMap<CivilDate, index::Note> {
    let scope = scope.to_owned();
    match state.db.call(move |db| store::load_notes(db, &scope)).await {
        Ok(notes) => notes,
        Err(e) => {
            warn!(error = %e, "the cache of the daily notes could not be read");
            BTreeMap::new()
        }
    }
}

/// `GET /vault`, on a blocking thread (a few metadata calls).
fn status_of(resolved: &Resolved, sandbox: bool, data_dir: &Path) -> VaultStatus {
    let vault = resolved.vault.as_deref();
    let config = DailyConfig::new(
        vault,
        &resolved.daily_note_folder,
        &resolved.daily_note_format,
    );
    let connected = vault.is_some_and(Path::is_dir);
    let daily_folder_found = match (&config, connected) {
        (Ok(config), true) => config.folder_path().is_dir(),
        _ => false,
    };
    let folder = normalise_folder(&resolved.daily_note_folder)
        .unwrap_or_else(|| resolved.daily_note_folder.clone());
    let problem = match &config {
        Err(ConfigError::BadFormat(text)) => Some(text.clone()),
        Err(ConfigError::BadFolder) => Some(
            "The daily notes folder is not a folder inside the vault. Change it in Settings."
                .to_owned(),
        ),
        Err(ConfigError::NoVault) => None,
        Ok(_) if connected && !daily_folder_found => Some(format!(
            "The daily notes folder {folder} was not found in the vault. Check it in Settings."
        )),
        Ok(_) => None,
    };
    VaultStatus {
        connected,
        path: vault.map(|path| path.to_string_lossy().into_owned()),
        daily_folder: Some(folder),
        daily_format: Some(resolved.daily_note_format.clone()),
        daily_folder_found,
        writable: connected && vault.is_some_and(|v| write_allowed(v, sandbox, data_dir)),
        problem,
    }
}

/// The state of the `vault` source in `/health`.
fn report_health(state: &AppState, status: &VaultStatus) {
    let (source_state, message) = if status.path.is_none() {
        (SourceState::Off, Some("No vault set".to_owned()))
    } else if !status.connected {
        (
            SourceState::Failed,
            Some("Vault folder not found".to_owned()),
        )
    } else if let Some(problem) = &status.problem {
        (SourceState::Failed, Some(problem.clone()))
    } else {
        (SourceState::Ok, None)
    };
    state.health.set_source(VAULT_SOURCE, source_state, message);
}
