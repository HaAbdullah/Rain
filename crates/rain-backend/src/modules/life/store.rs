//! The files and tables of this module: `habits.toml` (the habit definitions, owned by Rain)
//! and the two SQLite tables (the cache of the notes, and the audit trail of ticks).
//! Section 04 sections 4, 5 and 6.7.
//!
//! `habits.toml` is edited with `toml_edit`, so the comments and the order of the user's file
//! survive. A hand edit is allowed: reordering, renaming, a new habit. A habit that breaks a
//! rule is skipped on its own with an issue shown in `/health`, and the other habits keep
//! working.

use std::collections::{BTreeMap, HashSet};

use rain_types::{Cadence, ConfigIssue, Habit, HabitDraft, IconKind, IconRef};
use rusqlite::{Connection, params};
use serde::Deserialize;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use super::dates::CivilDate;
use super::frontmatter;
use super::index::Note;
use super::{HABITS_MAX, SOURCE, WRITE_LOG_KEEP};
use crate::core::error::{ApiError, ApiResult};
use crate::store::hub::ConfigHub;
use crate::store::toml_file::{TomlFile, set_value};

/// The file name in the data folder.
pub const FILE_NAME: &str = "habits.toml";

/// The text Rain writes at the top of a new `habits.toml`.
const HEADER: &str = "\
# Rain habits. One [[habit]] per habit. Rain edits this file when you add, change or delete a
# habit in the Life screen; you may edit it by hand too (Rain notices within a second).
# Keep comments up here: a comment directly above a [[habit]] is removed with that habit.
#
# A habit is a checkbox property at the top of your daily note, for example  gym: true
#
#   id        a short name Rain uses inside; letters, digits and dashes; never changes
#   name      what the Life screen shows
#   icon      a Lucide icon name, for example \"dumbbell\"
#   property  the property name in the note's frontmatter
#   cadence   \"daily\", \"weekly\" or \"monthly\"
#   times     for weekly: ticked days per week (1 to 7); for monthly: per month (1 to 31)
schema_version = 1
";

// ---- the file as the code reads it ----

/// What `habits.toml` holds. Every value is kept as a loose TOML value so that one bad entry
/// (a text where a number belongs) skips that habit and not the whole file.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct HabitsFile {
    pub schema_version: Option<u32>,
    pub habit: Vec<RawHabit>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct RawHabit {
    id: Option<toml::Value>,
    name: Option<toml::Value>,
    icon: Option<toml::Value>,
    property: Option<toml::Value>,
    cadence: Option<toml::Value>,
    times: Option<toml::Value>,
}

/// The rules of the schema that the types cannot state: none, because every entry is checked on
/// its own by `load`. (A syntax error or a newer schema is the file's problem, not an entry's.)
fn no_file_rules(_: &HabitsFile) -> Result<(), String> {
    Ok(())
}

pub async fn open(hub: &ConfigHub) -> ApiResult<TomlFile<HabitsFile>> {
    TomlFile::open(hub, FILE_NAME, no_file_rules)
        .await
        .map_err(|e| e.with_source(SOURCE))
}

// ---- turning entries into habits ----

/// The icon of an entry, from the text of `icon`. A plain name is a Lucide icon. `brand:<slug>`
/// and `letter:<text>` keep the other two kinds that the dialog's `IconRef` can name.
fn icon_from_text(text: &str) -> Option<IconRef> {
    let (kind, value) = if let Some(slug) = text.strip_prefix("brand:") {
        (IconKind::Brand, slug)
    } else if let Some(letters) = text.strip_prefix("letter:") {
        (IconKind::Letter, letters)
    } else {
        (IconKind::Lucide, text)
    };
    icon_is_valid(kind, value).then(|| IconRef {
        kind,
        value: value.to_owned(),
        tint: None,
        fallback: None,
    })
}

fn icon_is_valid(kind: IconKind, value: &str) -> bool {
    match kind {
        IconKind::Lucide | IconKind::Brand => {
            (1..=40).contains(&value.len())
                && value
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }
        IconKind::Letter => (1..=2).contains(&value.chars().count()),
        IconKind::Url => false,
    }
}

/// How an icon is written in the file.
fn icon_to_text(icon: &IconRef) -> String {
    match icon.kind {
        IconKind::Brand => format!("brand:{}", icon.value),
        IconKind::Letter => format!("letter:{}", icon.value),
        IconKind::Lucide | IconKind::Url => icon.value.clone(),
    }
}

/// `Gym Time!` becomes `gym-time`: lower case, anything outside `a-z0-9` becomes `-`, runs
/// collapse, no `-` at the ends, at most 40 characters. `habit` when nothing is left.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.trim_matches('-').chars().take(40).collect();
    out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "habit".to_owned()
    } else {
        out
    }
}

fn id_is_valid(id: &str) -> bool {
    let mut bytes = id.bytes();
    (1..=40).contains(&id.len())
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn text_of(value: &Option<toml::Value>) -> Option<&str> {
    value.as_ref().and_then(toml::Value::as_str)
}

/// The id of an entry: its `id`, else made from its name, else from its property.
fn derived_id(raw: &RawHabit) -> Option<String> {
    match &raw.id {
        Some(value) => value.as_str().map(str::to_owned),
        None => text_of(&raw.name)
            .or_else(|| text_of(&raw.property))
            .map(slug),
    }
}

/// One entry as a habit, or the reason it is skipped.
fn convert(raw: &RawHabit) -> Result<Habit, String> {
    let id = derived_id(raw).ok_or("it has no id, name or property")?;
    if !id_is_valid(&id) {
        return Err(format!(
            "the id '{id}' may only hold a-z, 0-9 and dashes (40 at most)"
        ));
    }
    let name = match &raw.name {
        None => id.clone(),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|name| (1..=60).contains(&name.chars().count()))
            .map(str::to_owned)
            .ok_or("the name must be text of 1 to 60 characters")?,
    };
    let icon = match &raw.icon {
        None => icon_from_text("square-check"),
        Some(value) => value.as_str().and_then(icon_from_text),
    }
    .ok_or("the icon must be a Lucide icon name such as \"dumbbell\"")?;
    let property = match &raw.property {
        None => id.clone(),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|p| p.chars().count() <= 60 && frontmatter::check_key(p).is_ok())
            .map(str::to_owned)
            .ok_or("the property is not a name that can be used in a note")?,
    };
    let cadence = match text_of(&raw.cadence) {
        None if raw.cadence.is_none() => Cadence::Daily,
        Some("daily") => Cadence::Daily,
        Some("weekly") => Cadence::Weekly,
        Some("monthly") => Cadence::Monthly,
        _ => return Err("the cadence must be \"daily\", \"weekly\" or \"monthly\"".to_owned()),
    };
    let times = match &raw.times {
        None => 1,
        Some(value) => value
            .as_integer()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("times must be a whole number")?,
    };
    if !times_are_valid(cadence, times) {
        return Err(match cadence {
            Cadence::Daily => "a daily habit has times = 1".to_owned(),
            Cadence::Weekly => "times must be 1 to 7 for a weekly habit".to_owned(),
            Cadence::Monthly => "times must be 1 to 31 for a monthly habit".to_owned(),
        });
    }
    Ok(Habit {
        id,
        name,
        icon,
        property,
        cadence,
        times,
    })
}

pub fn times_are_valid(cadence: Cadence, times: u32) -> bool {
    match cadence {
        Cadence::Daily => times == 1,
        Cadence::Weekly => (1..=7).contains(&times),
        Cadence::Monthly => (1..=31).contains(&times),
    }
}

/// The habits of the file in file order, and the issues of the entries that were skipped.
/// At most `HABITS_MAX` entries are read.
pub fn load(file: &HabitsFile) -> (Vec<Habit>, Vec<ConfigIssue>) {
    let mut habits: Vec<Habit> = Vec::new();
    let mut issues = Vec::new();
    let mut ids = HashSet::new();
    let mut properties = HashSet::new();
    let skip = |issues: &mut Vec<ConfigIssue>, number: usize, why: String| {
        issues.push(ConfigIssue {
            file: FILE_NAME.to_owned(),
            line: None,
            message: format!("Habit {number} is skipped: {why}"),
        });
    };
    for (index, raw) in file.habit.iter().enumerate() {
        let number = index + 1;
        if number > HABITS_MAX {
            skip(
                &mut issues,
                number,
                format!("Rain keeps at most {HABITS_MAX} habits"),
            );
            continue;
        }
        match convert(raw) {
            Err(why) => skip(&mut issues, number, why),
            Ok(habit) if !ids.insert(habit.id.clone()) => {
                skip(
                    &mut issues,
                    number,
                    format!("another habit already has the id '{}'", habit.id),
                );
            }
            Ok(habit) if !properties.insert(habit.property.clone()) => {
                skip(
                    &mut issues,
                    number,
                    format!(
                        "another habit already uses the property '{}'",
                        habit.property
                    ),
                );
            }
            Ok(habit) => habits.push(habit),
        }
    }
    (habits, issues)
}

// ---- checking what the user typed in the dialog ----

/// A draft that passed the rules, in the form the file stores.
pub struct Clean {
    pub name: String,
    pub icon: IconRef,
    pub property: String,
    pub cadence: Cadence,
    pub times: u32,
}

fn bad(reason: &'static str, message: &str) -> ApiError {
    ApiError::invalid(message)
        .with_source(SOURCE)
        .with_reason(reason)
}

/// Checks a draft (section 3, table of rules). `path_id` is the id in the path of a `PUT`.
pub fn check_draft(draft: &HabitDraft, path_id: Option<&str>) -> ApiResult<Clean> {
    match (&draft.id, path_id) {
        (Some(_), None) => return Err(bad("badId", "A new habit cannot have an id")),
        (Some(id), Some(path)) if id != path => {
            return Err(bad("badId", "The id of the habit cannot be changed"));
        }
        _ => {}
    }
    let name = draft.name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(bad("badName", "Give the habit a name"));
    }
    let icon_ok = matches!(
        draft.icon.kind,
        IconKind::Lucide | IconKind::Brand | IconKind::Letter
    ) && icon_is_valid(draft.icon.kind, &draft.icon.value);
    if !icon_ok {
        return Err(bad("badIcon", "That icon is not allowed"));
    }
    let property = draft.property.trim();
    if property.chars().count() > 60 || frontmatter::check_key(property).is_err() {
        return Err(bad(
            "badProperty",
            "That property name cannot be used in a note",
        ));
    }
    if !times_are_valid(draft.cadence, draft.times) {
        return Err(bad(
            "badTimes",
            "Times must be 1 to 7 a week or 1 to 31 a month",
        ));
    }
    Ok(Clean {
        name: name.to_owned(),
        icon: IconRef {
            kind: draft.icon.kind,
            value: draft.icon.value.clone(),
            tint: None,
            fallback: None,
        },
        property: property.to_owned(),
        cadence: draft.cadence,
        times: draft.times,
    })
}

// ---- editing the file ----

/// A string as a basic (double quoted) TOML string, the way the header shows them.
fn text_value(text: &str) -> Value {
    Value::from(text)
}

fn cadence_text(cadence: Cadence) -> &'static str {
    match cadence {
        Cadence::Daily => "daily",
        Cadence::Weekly => "weekly",
        Cadence::Monthly => "monthly",
    }
}

fn str_of<'a>(table: &'a Table, key: &str) -> Option<&'a str> {
    table.get(key).and_then(Item::as_str)
}

/// The id of a table in the document, worked out like `derived_id` does for an entry.
fn table_id(table: &Table) -> Option<String> {
    match table.get("id") {
        Some(item) => item.as_str().map(str::to_owned),
        None => str_of(table, "name")
            .or_else(|| str_of(table, "property"))
            .map(slug),
    }
}

/// The property of a table: its `property`, else its id.
fn table_property(table: &Table) -> Option<String> {
    str_of(table, "property")
        .map(|p| p.trim().to_owned())
        .or_else(|| table_id(table))
}

fn changed(message: &'static str) -> ApiError {
    ApiError::conflict("configChanged", message).with_source(SOURCE)
}

/// The array of `[[habit]]` tables of the document, made when it is not there yet.
fn habit_tables(doc: &mut DocumentMut) -> ApiResult<&mut ArrayOfTables> {
    let item = doc
        .entry("habit")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    item.as_array_of_tables_mut()
        .ok_or_else(|| changed("habits.toml changed. Try again."))
}

fn duplicate_property(property: &str) -> ApiError {
    ApiError::conflict(
        "duplicateProperty",
        format!("Another habit already uses the property '{property}'"),
    )
    .with_source(SOURCE)
}

/// Appends a habit. Used inside `TomlFile::edit`. The first write of a new file puts the
/// explaining header on top.
pub fn append_habit(doc: &mut DocumentMut, clean: &Clean, name_for_id: &str) -> ApiResult<()> {
    if doc.to_string().trim().is_empty() {
        *doc = HEADER.parse::<DocumentMut>().map_err(|_| {
            ApiError::failed("Could not save habits.toml").with_reason("saveFailed")
        })?;
    }
    let tables = habit_tables(doc)?;
    if tables.len() >= HABITS_MAX {
        return Err(
            ApiError::conflict("tooManyHabits", "Rain keeps at most 50 habits").with_source(SOURCE),
        );
    }
    if tables
        .iter()
        .any(|table| table_property(table).as_deref() == Some(clean.property.as_str()))
    {
        return Err(duplicate_property(&clean.property));
    }
    // A free id: the slug of the name, then `-2`, `-3` ...
    let taken: HashSet<String> = tables.iter().filter_map(table_id).collect();
    let base = slug(name_for_id);
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        let suffix = format!("-{n}");
        let room = 40usize.saturating_sub(suffix.len());
        id = format!(
            "{}{suffix}",
            base.chars()
                .take(room)
                .collect::<String>()
                .trim_end_matches('-')
        );
        n += 1;
    }
    let mut table = Table::new();
    set_value(&mut table, "id", text_value(&id));
    set_value(&mut table, "name", text_value(&clean.name));
    set_value(&mut table, "icon", text_value(&icon_to_text(&clean.icon)));
    set_value(&mut table, "property", text_value(&clean.property));
    set_value(
        &mut table,
        "cadence",
        text_value(cadence_text(clean.cadence)),
    );
    if clean.cadence != Cadence::Daily {
        set_value(&mut table, "times", Value::from(i64::from(clean.times)));
    }
    tables.push(table);
    Ok(())
}

/// Changes the habit with this id. Only the keys that really changed are written, so a trailing
/// comment on a value and a key the user left out stay as they are.
pub fn update_habit(doc: &mut DocumentMut, id: &str, clean: &Clean) -> ApiResult<()> {
    let tables = habit_tables(doc)?;
    let position = tables
        .iter()
        .position(|table| table_id(table).as_deref() == Some(id))
        .ok_or_else(|| changed("habits.toml changed. Try again."))?;
    if tables.iter().enumerate().any(|(i, table)| {
        i != position && table_property(table).as_deref() == Some(clean.property.as_str())
    }) {
        return Err(duplicate_property(&clean.property));
    }
    let table = tables
        .get_mut(position)
        .ok_or_else(|| changed("habits.toml changed. Try again."))?;
    // What the loader would see now, so that a value the file leaves out is not "added back"
    // when it does not change.
    let before = convert(&raw_of(table)).ok();
    let old = |pick: fn(&Habit) -> String| before.as_ref().map(pick);
    let new_name = clean.name.clone();
    let new_icon = icon_to_text(&clean.icon);
    let new_property = clean.property.clone();
    let new_cadence = cadence_text(clean.cadence).to_owned();
    let pairs: [(&str, String, Option<String>); 4] = [
        ("name", new_name, old(|h| h.name.clone())),
        ("icon", new_icon, old(|h| icon_to_text(&h.icon))),
        ("property", new_property, old(|h| h.property.clone())),
        (
            "cadence",
            new_cadence,
            old(|h| cadence_text(h.cadence).to_owned()),
        ),
    ];
    for (key, new, current) in pairs {
        if current.as_deref() != Some(new.as_str()) {
            set_value(table, key, text_value(&new));
        }
    }
    let current_times = before.as_ref().map(|h| h.times);
    if current_times != Some(clean.times) {
        set_value(table, "times", Value::from(i64::from(clean.times)));
    }
    Ok(())
}

/// Reads one table of the document as the loader reads an entry.
fn raw_of(table: &Table) -> RawHabit {
    let text = |key: &str| str_of(table, key).map(|s| toml::Value::String(s.to_owned()));
    RawHabit {
        id: table
            .get("id")
            .and_then(Item::as_str)
            .map(|s| toml::Value::String(s.to_owned())),
        name: text("name"),
        icon: text("icon"),
        property: text("property"),
        cadence: text("cadence"),
        times: table
            .get("times")
            .and_then(Item::as_integer)
            .map(toml::Value::Integer),
    }
}

/// Removes the habit with this id. A table that is already gone is fine.
pub fn remove_habit(doc: &mut DocumentMut, id: &str) -> ApiResult<()> {
    let Some(item) = doc.get_mut("habit") else {
        return Ok(());
    };
    let Some(tables) = item.as_array_of_tables_mut() else {
        return Ok(());
    };
    let position = tables
        .iter()
        .position(|table| table_id(table).as_deref() == Some(id));
    if let Some(position) = position {
        tables.remove(position);
    }
    if tables.is_empty() {
        doc.remove("habit");
    }
    Ok(())
}

// ---- SQLite ----

/// The cached rows of one scope, as the index keeps them. A row that cannot be understood is
/// left out (the next check reads that note again).
pub fn load_notes(
    connection: &Connection,
    scope: &str,
) -> rusqlite::Result<BTreeMap<CivilDate, Note>> {
    let mut statement =
        connection.prepare("SELECT date, mtime_ns, size, bools FROM life_note WHERE scope = ?1")?;
    let rows = statement.query_map([scope], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut notes = BTreeMap::new();
    for row in rows {
        let (date, mtime_ns, size, bools) = row?;
        let (Some(date), Ok(size), Ok(map)) = (
            CivilDate::parse(&date),
            u64::try_from(size),
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&bools),
        ) else {
            continue;
        };
        let bools = map
            .into_iter()
            .filter_map(|(name, value)| value.as_bool().map(|flag| (name, flag)))
            .collect();
        notes.insert(
            date,
            Note {
                mtime_ns,
                size,
                bools,
            },
        );
    }
    Ok(notes)
}

/// Writes what a check changed, in one transaction, and drops the rows of other scopes and of
/// days before the window.
pub fn save_notes(
    connection: &mut Connection,
    scope: &str,
    changed: &[(CivilDate, Note)],
    removed: &[CivilDate],
    window_from: CivilDate,
) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    {
        let mut put = transaction.prepare(
            "INSERT OR REPLACE INTO life_note (scope, date, mtime_ns, size, bools)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (date, note) in changed {
            let bools: serde_json::Map<String, serde_json::Value> = note
                .bools
                .iter()
                .map(|(name, flag)| (name.clone(), serde_json::Value::Bool(*flag)))
                .collect();
            put.execute(params![
                scope,
                date.to_string(),
                note.mtime_ns,
                i64::try_from(note.size).unwrap_or(i64::MAX),
                serde_json::Value::Object(bools).to_string()
            ])?;
        }
        let mut delete =
            transaction.prepare("DELETE FROM life_note WHERE scope = ?1 AND date = ?2")?;
        for date in removed {
            delete.execute(params![scope, date.to_string()])?;
        }
    }
    transaction.execute(
        "DELETE FROM life_note WHERE scope <> ?1 OR date < ?2",
        params![scope, window_from.to_string()],
    )?;
    transaction.commit()
}

/// One vault write, for the audit trail. No note text, no path.
pub struct WriteRow {
    pub at: String,
    pub date: String,
    pub habit_id: String,
    pub property: String,
    pub done: bool,
    pub outcome: &'static str,
}

/// Adds a row and keeps the newest `WRITE_LOG_KEEP`, in one transaction.
pub fn insert_write(connection: &mut Connection, row: &WriteRow) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO life_write (at, date, habit_id, property, done, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.at,
            row.date,
            row.habit_id,
            row.property,
            row.done,
            row.outcome
        ],
    )?;
    transaction.execute(
        "DELETE FROM life_write WHERE id <= (SELECT id FROM life_write ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        [WRITE_LOG_KEEP],
    )?;
    transaction.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> HabitsFile {
        toml::from_str(text).expect("valid TOML")
    }

    fn draft(name: &str, property: &str, cadence: Cadence, times: u32) -> HabitDraft {
        HabitDraft {
            id: None,
            name: name.into(),
            icon: IconRef {
                kind: IconKind::Lucide,
                value: "dumbbell".into(),
                tint: None,
                fallback: None,
            },
            property: property.into(),
            cadence,
            times,
        }
    }

    fn clean(name: &str, property: &str, cadence: Cadence, times: u32) -> Clean {
        check_draft(&draft(name, property, cadence, times), None).expect("a valid draft")
    }

    fn doc(text: &str) -> DocumentMut {
        text.parse().expect("a TOML document")
    }

    // ---- load ----

    #[test]
    fn defaults_fill_what_an_entry_leaves_out() {
        let (habits, issues) = load(&parse("[[habit]]\nname = \"Read more\"\n"));
        assert!(issues.is_empty());
        let habit = &habits[0];
        assert_eq!(habit.id, "read-more");
        assert_eq!(habit.name, "Read more");
        assert_eq!(habit.icon.value, "square-check");
        assert_eq!(habit.property, "read-more");
        assert_eq!((habit.cadence, habit.times), (Cadence::Daily, 1));
    }

    #[test]
    fn an_entry_with_only_an_id_uses_it_for_name_and_property() {
        let (habits, _) = load(&parse("[[habit]]\nid = \"walk\"\n"));
        assert_eq!(
            (habits[0].name.as_str(), habits[0].property.as_str()),
            ("walk", "walk")
        );
    }

    #[test]
    fn a_bad_entry_is_skipped_alone_with_an_issue_and_the_others_stay() {
        let text = r#"
schema_version = 1
[[habit]]
id = "gym"
property = "gym"
cadence = "weekly"
times = 3
[[habit]]
id = "gym"
property = "other"
[[habit]]
id = "twin"
property = "gym"
[[habit]]
id = "weird"
cadence = "hourly"
[[habit]]
id = "many"
cadence = "weekly"
times = 8
[[habit]]
id = "daily2"
times = 2
[[habit]]
id = "BAD ID"
[[habit]]
id = "typed"
times = "three"
cadence = "weekly"
[[habit]]
id = "fine"
property = "fine"
"#;
        let (habits, issues) = load(&parse(text));
        let ids: Vec<_> = habits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["gym", "fine"]);
        assert_eq!(issues.len(), 7);
        assert!(issues[0].message.contains("Habit 2") && issues[0].message.contains("id 'gym'"));
        assert!(issues[1].message.contains("property 'gym'"));
        assert!(issues.iter().all(|i| i.file == "habits.toml"));
    }

    #[test]
    fn at_most_fifty_habits_are_read() {
        let text: String = (0..51)
            .map(|n| format!("[[habit]]\nid = \"h{n}\"\n"))
            .collect();
        let (habits, issues) = load(&parse(&text));
        assert_eq!(habits.len(), 50);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("Habit 51"));
    }

    #[test]
    fn brand_and_letter_icons_are_kept_and_unknown_shapes_are_refused() {
        let (habits, issues) = load(&parse(
            "[[habit]]\nid=\"a\"\nicon=\"brand:github\"\n[[habit]]\nid=\"b\"\nicon=\"letter:G\"\n[[habit]]\nid=\"c\"\nicon=\"Bad Icon\"\n",
        ));
        assert_eq!(habits[0].icon.kind, IconKind::Brand);
        assert_eq!(habits[1].icon.kind, IconKind::Letter);
        assert_eq!(habits.len(), 2);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Gym Time!"), "gym-time");
        assert_eq!(slug("  --Read  "), "read");
        assert_eq!(slug("日本語"), "habit");
        assert_eq!(slug(&"a".repeat(60)).len(), 40);
        assert_eq!(slug(&format!("{}-b", "a".repeat(39))), "a".repeat(39));
    }

    // ---- the draft rules ----

    #[test]
    fn a_draft_is_checked_field_by_field_with_its_reason() {
        let reason = |d: HabitDraft, path: Option<&str>| {
            check_draft(&d, path).err().and_then(|e| e.body.reason)
        };
        assert_eq!(reason(draft("Gym", "gym", Cadence::Daily, 1), None), None);
        assert_eq!(
            reason(draft("  ", "gym", Cadence::Daily, 1), None).as_deref(),
            Some("badName")
        );
        assert_eq!(
            reason(draft(&"x".repeat(61), "gym", Cadence::Daily, 1), None).as_deref(),
            Some("badName")
        );
        assert_eq!(
            reason(draft("Gym", "a:b", Cadence::Daily, 1), None).as_deref(),
            Some("badProperty")
        );
        assert_eq!(
            reason(draft("Gym", "", Cadence::Daily, 1), None).as_deref(),
            Some("badProperty")
        );
        assert_eq!(
            reason(draft("Gym", &"p".repeat(61), Cadence::Daily, 1), None).as_deref(),
            Some("badProperty")
        );
        assert_eq!(
            reason(draft("Gym", "gym", Cadence::Daily, 2), None).as_deref(),
            Some("badTimes")
        );
        assert_eq!(
            reason(draft("Gym", "gym", Cadence::Weekly, 8), None).as_deref(),
            Some("badTimes")
        );
        assert_eq!(
            reason(draft("Gym", "gym", Cadence::Weekly, 0), None).as_deref(),
            Some("badTimes")
        );
        assert_eq!(
            reason(draft("Gym", "gym", Cadence::Monthly, 31), None),
            None
        );
        assert_eq!(
            reason(draft("Gym", "gym", Cadence::Monthly, 32), None).as_deref(),
            Some("badTimes")
        );
        let mut url = draft("Gym", "gym", Cadence::Daily, 1);
        url.icon = IconRef {
            kind: IconKind::Url,
            value: "media/x".into(),
            tint: None,
            fallback: None,
        };
        assert_eq!(reason(url, None).as_deref(), Some("badIcon"));
        let mut upper = draft("Gym", "gym", Cadence::Daily, 1);
        upper.icon.value = "Dumbbell".into();
        assert_eq!(reason(upper, None).as_deref(), Some("badIcon"));
        let mut with_id = draft("Gym", "gym", Cadence::Daily, 1);
        with_id.id = Some("gym".into());
        assert_eq!(reason(with_id.clone(), None).as_deref(), Some("badId"));
        assert_eq!(reason(with_id.clone(), Some("gym")), None);
        assert_eq!(reason(with_id, Some("other")).as_deref(), Some("badId"));
        // the name and property are trimmed
        let ok = check_draft(&draft("  Gym ", " gym ", Cadence::Daily, 1), None).unwrap();
        assert_eq!((ok.name.as_str(), ok.property.as_str()), ("Gym", "gym"));
    }

    // ---- editing ----

    #[test]
    fn the_first_create_writes_the_header_and_the_habit() {
        let mut document = doc("");
        append_habit(
            &mut document,
            &clean("Gym", "gym", Cadence::Weekly, 3),
            "Gym",
        )
        .unwrap();
        let text = document.to_string();
        assert!(text.starts_with("# Rain habits."), "{text}");
        assert!(text.contains("schema_version = 1"));
        assert!(text.contains(
            "[[habit]]\nid = \"gym\"\nname = \"Gym\"\nicon = \"dumbbell\"\nproperty = \"gym\"\ncadence = \"weekly\"\ntimes = 3\n"
        ), "{text}");
        let (habits, issues) = load(&parse(&text));
        assert!(issues.is_empty());
        assert_eq!(habits[0].times, 3);
    }

    #[test]
    fn a_daily_habit_is_written_without_times_and_a_second_is_separated_by_a_blank_line() {
        let mut document = doc("");
        append_habit(
            &mut document,
            &clean("Gym", "gym", Cadence::Weekly, 3),
            "Gym",
        )
        .unwrap();
        append_habit(
            &mut document,
            &clean("Read", "read", Cadence::Daily, 1),
            "Read",
        )
        .unwrap();
        let text = document.to_string();
        assert!(text.ends_with(
            "times = 3\n\n[[habit]]\nid = \"read\"\nname = \"Read\"\nicon = \"dumbbell\"\nproperty = \"read\"\ncadence = \"daily\"\n"
        ), "{text}");
    }

    #[test]
    fn a_create_keeps_the_users_comments_and_order() {
        let original = "# my notes\nschema_version = 1\n\n# the first one\n[[habit]]\nid = \"a\" # why\nproperty = \"a\"\n";
        let mut document = doc(original);
        append_habit(&mut document, &clean("B", "b", Cadence::Daily, 1), "B").unwrap();
        let text = document.to_string();
        assert!(text.starts_with(original), "{text}");
        assert!(text.contains("id = \"b\""));
    }

    #[test]
    fn ids_are_made_from_the_name_and_numbered_when_taken() {
        let mut document = doc("");
        for (name, property) in [("Gym", "gym"), ("Gym", "gym2"), ("Gym", "gym3")] {
            append_habit(
                &mut document,
                &clean(name, property, Cadence::Daily, 1),
                name,
            )
            .unwrap();
        }
        let (habits, _) = load(&parse(&document.to_string()));
        let ids: Vec<_> = habits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["gym", "gym-2", "gym-3"]);
    }

    #[test]
    fn a_duplicate_property_and_the_fifty_first_habit_are_refused_with_their_reasons() {
        let mut document = doc("");
        append_habit(
            &mut document,
            &clean("Gym", "gym", Cadence::Daily, 1),
            "Gym",
        )
        .unwrap();
        let again = append_habit(
            &mut document,
            &clean("Other", "gym", Cadence::Daily, 1),
            "Other",
        );
        let error = again.unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("duplicateProperty"));
        assert!(error.body.message.contains("'gym'"));

        let mut full = doc("");
        for n in 0..50 {
            let property = format!("p{n}");
            append_habit(&mut full, &clean("H", &property, Cadence::Daily, 1), "H").unwrap();
        }
        let error =
            append_habit(&mut full, &clean("H", "p50", Cadence::Daily, 1), "H").unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("tooManyHabits"));
    }

    #[test]
    fn an_edit_changes_only_the_keys_that_changed_and_keeps_a_trailing_comment() {
        let original = "schema_version = 1\n\n[[habit]]\nid = \"gym\"\nname = \"Gym\" # my gym\nicon = \"dumbbell\"\nproperty = \"gym\"\ncadence = \"weekly\"\ntimes = 3\n";
        let mut document = doc(original);
        update_habit(
            &mut document,
            "gym",
            &clean("Gym", "gym", Cadence::Weekly, 4),
        )
        .unwrap();
        assert_eq!(
            document.to_string(),
            original.replace("times = 3", "times = 4"),
            "only times changed, the comment stayed"
        );
        // A rename keeps the comment on that value too.
        update_habit(
            &mut document,
            "gym",
            &clean("Gym time", "gym", Cadence::Weekly, 4),
        )
        .unwrap();
        assert!(
            document
                .to_string()
                .contains("name = \"Gym time\" # my gym"),
            "{document}"
        );
    }

    #[test]
    fn a_key_the_user_left_out_is_not_added_back_when_it_does_not_change() {
        // No icon and no times key: the defaults are "square-check" and 1.
        let original = "[[habit]]\nid = \"read\"\nname = \"Read\"\nproperty = \"read\"\n";
        let mut document = doc(original);
        let mut same = draft("Read", "read", Cadence::Daily, 1);
        same.icon.value = "square-check".into();
        update_habit(&mut document, "read", &check_draft(&same, None).unwrap()).unwrap();
        assert_eq!(document.to_string(), original);
        // Changing the icon adds that one key.
        update_habit(
            &mut document,
            "read",
            &clean("Read", "read", Cadence::Daily, 1),
        )
        .unwrap();
        assert!(document.to_string().contains("icon = \"dumbbell\""));
        assert!(!document.to_string().contains("times"));
    }

    #[test]
    fn an_edit_can_change_the_cadence_and_finds_a_habit_by_its_derived_id() {
        let mut document = doc("[[habit]]\nname = \"Read more\"\n");
        update_habit(
            &mut document,
            "read-more",
            &clean("Read more", "read-more", Cadence::Monthly, 4),
        )
        .unwrap();
        let (habits, issues) = load(&parse(&document.to_string()));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!((habits[0].cadence, habits[0].times), (Cadence::Monthly, 4));
    }

    #[test]
    fn an_edit_refuses_a_duplicate_property_and_a_habit_that_is_gone() {
        let mut document = doc(
            "[[habit]]\nid = \"a\"\nproperty = \"a\"\n[[habit]]\nid = \"b\"\nproperty = \"b\"\n",
        );
        let error =
            update_habit(&mut document, "a", &clean("A", "b", Cadence::Daily, 1)).unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("duplicateProperty"));
        let error =
            update_habit(&mut document, "zzz", &clean("Z", "z", Cadence::Daily, 1)).unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("configChanged"));
        // keeping its own property is not a duplicate
        update_habit(&mut document, "a", &clean("A2", "a", Cadence::Daily, 1)).unwrap();
    }

    #[test]
    fn a_delete_removes_the_table_and_the_comment_directly_above_it() {
        let original = "# header\nschema_version = 1\n\n# the gym habit\n[[habit]]\nid = \"gym\"\n\n[[habit]]\nid = \"read\"\n";
        let mut document = doc(original);
        remove_habit(&mut document, "gym").unwrap();
        let text = document.to_string();
        assert!(
            !text.contains("the gym habit") && !text.contains("gym"),
            "{text}"
        );
        assert!(text.contains("# header") && text.contains("id = \"read\""));
        // already gone is fine, and the last one leaves no empty array
        remove_habit(&mut document, "gym").unwrap();
        remove_habit(&mut document, "read").unwrap();
        assert!(!document.to_string().contains("habit"));
    }

    #[test]
    fn a_syntax_error_is_the_files_problem_not_a_skipped_entry() {
        assert!(toml::from_str::<HabitsFile>("[[habit]\nid = ").is_err());
        // an entry with the wrong type is still read, and skipped by `load`
        let (habits, issues) = load(&parse("[[habit]]\nid = 5\n"));
        assert!(habits.is_empty());
        assert_eq!(issues.len(), 1);
    }

    // ---- SQLite ----

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().expect("an in-memory database");
        connection
            .execute_batch(include_str!("migrations/0001_create_life_tables.sql"))
            .expect("the migration runs");
        connection
    }

    fn note(mtime: i64, bools: &[(&str, bool)]) -> Note {
        Note {
            mtime_ns: mtime,
            size: 7,
            bools: bools.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
        }
    }

    fn date(text: &str) -> CivilDate {
        CivilDate::parse(text).expect("a real date")
    }

    #[test]
    fn notes_are_saved_loaded_replaced_and_removed_per_scope() {
        let mut db = connection();
        let rows = vec![
            (date("2026-11-20"), note(1, &[("gym", true)])),
            (
                date("2026-11-21"),
                note(2, &[("gym", false), ("read", true)]),
            ),
        ];
        save_notes(&mut db, "aaaa", &rows, &[], date("2024-09-14")).unwrap();
        let loaded = load_notes(&db, "aaaa").unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[&date("2026-11-21")].value("read"), Some(true));
        assert_eq!(loaded[&date("2026-11-20")].size, 7);

        // replace one row, remove the other
        save_notes(
            &mut db,
            "aaaa",
            &[(date("2026-11-20"), note(5, &[("gym", false)]))],
            &[date("2026-11-21")],
            date("2024-09-14"),
        )
        .unwrap();
        let loaded = load_notes(&db, "aaaa").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[&date("2026-11-20")].mtime_ns, 5);

        // another scope deletes the rows of the old one, and old dates go
        save_notes(
            &mut db,
            "bbbb",
            &[
                (date("2024-01-01"), note(1, &[])),
                (date("2026-11-01"), note(1, &[])),
            ],
            &[],
            date("2024-09-14"),
        )
        .unwrap();
        assert!(load_notes(&db, "aaaa").unwrap().is_empty());
        let kept = load_notes(&db, "bbbb").unwrap();
        assert_eq!(
            kept.keys().map(ToString::to_string).collect::<Vec<_>>(),
            ["2026-11-01"]
        );
    }

    #[test]
    fn the_audit_trail_keeps_the_newest_rows_only() {
        let mut db = connection();
        for n in 0..(WRITE_LOG_KEEP + 5) {
            let row = WriteRow {
                at: format!("2026-11-22T10:00:{n:02}.000Z"),
                date: "2026-11-22".into(),
                habit_id: "gym".into(),
                property: "gym".into(),
                done: n % 2 == 0,
                outcome: "updated",
            };
            insert_write(&mut db, &row).unwrap();
        }
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM life_write", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, WRITE_LOG_KEEP);
        let oldest: i64 = db
            .query_row("SELECT MIN(id) FROM life_write", [], |r| r.get(0))
            .unwrap();
        assert_eq!(oldest, 6, "the first five rows were dropped");
    }
}
