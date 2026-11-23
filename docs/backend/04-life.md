# Rain backend: section 04, life (vault, habits, daily notes, calendar)

| | |
|---|---|
| Area | life: the Obsidian vault status, habit definitions, habit days, ticking a habit (the only vault write in Rain), opening the daily note, the calendar route that returns nothing for now, and the REDESIGNED Life screen |
| Modules | `life` (backend); `src/features/life` and parts of `src/features/home`, `src/features/settings`, `src/keys`, `src/actions`, `src/nav` (frontend) |
| Stage | 7 (foundation section 14.3), built in four steps 7a to 7d (section 9.6) |
| Author | life designer, 2026-11-22 |
| Status | draft 1. Binds on `decisions.md` (sections 4 and 5) and `00-foundation.md` (draft 2). Where this file needs a different shared rule it says so under "Open points". |

Reading order for the builder: section 1 (what and why), section 3 (types), section 6 (algorithms), section 9 (frontend). Everything else is lookup.

---------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 What this area does

1. Reads the user's Obsidian daily notes (read-only) and finds the habit checkboxes in their YAML frontmatter (`gym: true`).
2. Keeps the habit definitions (name, icon, property, cadence, times) in `habits.toml` in the data folder, and creates, edits and deletes them.
3. Computes, in Rust, everything the Life screen shows: the compact list rows (today's checkbox, progress, a small heatmap), and for one habit the streaks, completion rate, totals and the per-day grid.
4. Writes ONE boolean property of ONE daily note when the user ticks or unticks a habit. This is the only vault write in Rain. It lives in one file, `modules/life/vault_write.rs`.
5. Opens a daily note in Obsidian (never creates one).
6. Keeps the calendar route (`GET /calendar/events`) and returns `[]`.

What it does not do: no checklist rows in the note body, no number habits, no "skip", no calendar connection, no vault write other than the tick, no search entries for habits (decided, section 1.4), no habit reorder UI.

### 1.2 `RainApi` methods and topics

| Method | Route | Change against `api.ts` today |
|---|---|---|
| `getVaultStatus` | `GET /vault` | new shape (`VaultStatus` loses `missingProperties`, gains folder, format, writable) |
| `rereadVault` | `POST /vault/reread` | NEW. Ctrl+R "Re-read vault" |
| `listHabitSuggestions` | `GET /vault/properties` | NEW. Checkbox properties found in the notes that no habit uses yet |
| `listHabits` | `GET /habits` | returns `HabitSummary[]` (definition plus today and compact history) instead of `Habit[]` |
| `saveHabit` | `POST /habits` (create, 201) or `PUT /habits/{id}` | NEW |
| `deleteHabit` | `DELETE /habits/{id}` | NEW |
| `getHabitStats` | `GET /habits/{id}/stats` | NEW. Full view: streaks, rate, totals |
| `listHabitDays` | `GET /habit-days?from=&to=&habit=` | `HabitDay` is `{ date, done: ID[] }`, one row per EXISTING note, optional `habit` filter |
| `setHabitDone` | `PUT /habits/{id}/days/{date}` | NEW. The tick |
| `listEvents` | `GET /calendar/events?from=&to=` | route renamed by the registry; returns `[]` |
| `openDailyNote` | `POST /daily-notes/{date}/open` | return type named `OpenNoteResult` (same shape) |
| `subscribe('habits')` | `GET /stream` | NEW topic `habits` |

Modules owned: `life`. Decisions of `decisions.md` implemented: section 4 "Life screen, redesigned by the user", "Calendar: none for now", "Daily notes on this PC"; section 5 "A weekly habit is N times per week...", "Habit definitions ... live in the data folder", "Habits are only checkboxes"; section 6 "Never write to the vault except through the one habit-property function. Never log note text."

### 1.3 The rule for the vault write, in one paragraph

A tick in Rain changes exactly one property in exactly one daily note, only as the direct result of the user's tick. Every other byte of the note stays the same. If the note does not exist it is created holding only the ticked property. The function that does it is `life::vault_write::set_habit_property`. The source-scanning test of foundation 13.3 can enforce this because of the file layout in section 6.8.

### 1.4 Decisions at a glance

| # | Question | Decision | Reason (one line) |
|---|---|---|---|
| L1 | Bare note: all habits or only the ticked one? | Only the ticked property. | "Holds only the habit properties" is satisfied minimally; no property is written that the user did not set. |
| L2 | What does unticking write? | `false` into an existing property. If the note or the property does not exist, nothing is written. | Obsidian's own checkbox writes `false`; deleting a line would break "every other byte unchanged"; absent already means "not done". |
| L3 | Which days may be ticked? | The list: today only. The full view: the last 31 days in total: today and the 30 days before it (`TICK_WINDOW_DAYS = 31` counts today, so the oldest tickable day is today minus 30). Never a future day. | Forgetting yesterday is the common case; 31 days always covers the current week and month; an old note is never created by accident. |
| L4 | Weekly and monthly habits | "N times per week/month". A tick is stored in the note of the day the user ticked, which is today or, in the full view, one of the last 31 days (L3); a past-day tick never touches today's note. Settled by decisions.md section 5, ruling of 2026-11-22 ("limited to the last 31 days" is read as 31 days in total: today and the 30 days before it). Week = Monday to Sunday, month = calendar month. Progress = number of ticked days in the period. | Matches decisions.md section 5. Monday start is fixed (no setting this round). |
| L5 | Who computes streaks, rates, totals? | The backend. The frontend only draws. | One definition for week and month rules, testable with a fake clock and fixtures. |
| L6 | Main Life screen | One list, three groups (Daily, Weekly, Monthly), no strip, no right pane. | "Minimal": a pane would repeat what a row says. The full view is the detail. |
| L7 | Full view and navigation | Screen-local state of the Life route (`life.open` in `useViewState`), not a new route. A new Esc rung "leave the sub-view" (`ScreenController.back`). | `nav.back()` returns to the previous screen, not to the list; a route would also need hash and trail rules. |
| L8 | Habits in search | NOT searchable. | Typing "gym" then Enter in the global search must never write to the vault. |
| L9 | History window | The last 800 days (`LIFE_INDEX_DAYS`). Streaks, rate and totals look back at most that far. | Bounds the file checks to about 20 ms; the user's vault is younger than that. |
| L10 | Where are the notes found? | Date driven: format the date, join the folder, check that one path. No directory scan, no parsing of file names. | Needs only a date formatter, which the Moment tokens allow exactly. |
| L11 | Is the watcher needed for correctness? | No. A read re-checks the files when the last check is older than 2 s. The watcher only pushes the `habits` topic. | `notify` is one day old; correctness must not depend on it. |
| L12 | `missingProperties` | Removed. | With ticking, a habit whose property no note has yet is normal, not a warning. |
| L13 | Calendar | `listEvents` stays, returns `[]`; Home and Life no longer call it; Settings row is an info row "Not connected". | decisions.md section 4. |

---------------------------------------------------------------------------------------------------

## 2. Routes

All paths are after `/api/v1`. Dates in paths and queries are `YYYY-MM-DD` (local calendar date). Errors use the one body of foundation 4.5. "Idempotent" follows 4.6.

| Method | Path | Request | Response | Success | Errors (code, reason) | Idempotent | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/vault` | none | `VaultStatus` | 200 | none (every problem is a field of the body) | yes | none |
| POST | `/vault/reread` | none | none | 204 | none | yes | `habits` (only if the snapshot changed) |
| GET | `/vault/properties` | none | `PropertySuggestion[]` (empty when no vault) | 200 | none | yes | none |
| GET | `/habits` | none | `HabitSummary[]` (file order is kept inside each group; all groups in one list) | 200 | none (vault problems do not fail it: cells are then `none`) | yes | none |
| POST | `/habits` | `HabitDraft` (no `id`; an `id` is `400 badId`) | `Habit` | 201 | 400 invalid: `badName`, `badProperty`, `badIcon`, `badTimes`, `badId`; 409 conflict: `duplicateProperty`, `tooManyHabits`, `configBroken`, `configChanged`, `configNewer`; 500 failed: `saveFailed` | no. A second identical POST fails with `409 duplicateProperty`, so a double click creates one habit | `habits` |
| PUT | `/habits/{id}` | `HabitDraft` (`id` absent or equal to the path) | `Habit` | 200 | as POST, plus 404 notFound: `unknownHabit` | yes | `habits` |
| DELETE | `/habits/{id}` | none | none | 204 (also when the id is already gone) | 500 failed: `saveFailed`; 409 conflict: `configBroken`, `configChanged`, `configNewer` | yes | `habits` |
| GET | `/habits/{id}/stats` | none | `HabitStats` | 200 | 404 notFound: `unknownHabit` | yes | none |
| GET | `/habit-days` | query `from`, `to` (required), `habit` (optional id) | `HabitDay[]`, oldest first, only days that HAVE a note | 200 | 400 invalid: `badDate`, `badRange` (`from` after `to`, or more than 1200 days); 404 notFound: `unknownHabit` | yes | none |
| PUT | `/habits/{id}/days/{date}` | `HabitTickBody` `{ "done": boolean }` | `HabitSummary` of that habit | 200 | 404 notFound: `unknownHabit`; 400 invalid: `badDate`, `futureDate`, `tooOld`; 503 unavailable (source `vault`): `noVault`, `vaultMissing`, `badDailyFormat`, `noDailyFolder`; 409 conflict: `sandbox`, `noteBusy`, `notBoolean`, `duplicateKey`, `unreadableNote`, `notUtf8`; 500 failed: `writeFailed` | yes (the same body twice leaves the note equal; the second call writes nothing) | `habits` |
| GET | `/calendar/events` | query `from`, `to` | `CalendarEvent[]` (always `[]`) | 200 | 400 invalid: `badDate` | yes | none |
| POST | `/daily-notes/{date}/open` | none | `OpenNoteResult` | 200 | 400 invalid: `badDate`; 503 unavailable (source `vault`): `noVault`, `vaultMissing`, `badDailyFormat`; 500 failed: `openFailed` | yes (opens the note again) | none |

Notes on the table:

- `GET /habits` never answers an error because of the vault: the list is built from `habits.toml`, and the history cells are `none` when no note can be read. The screen decides what to say from `GET /vault`. This keeps one failure from hiding the other.
- The route `/habits/{id}/days/{date}` is a PUT of one day's state, not a POST of an event, so a retry is safe.
- `DELETE /habits/{id}` removes the definition only. No note is touched.
- `GET /vault/properties` is under `/vault`, not `/habits`, so that no habit id can ever clash with a route word.
- Registry (foundation 14.4) rows kept: `getVaultStatus`, `listHabits`, `listHabitDays`, `listEvents`, `openDailyNote`. Rows added: the six NEW methods above (`rereadVault`, `listHabitSuggestions`, `saveHabit`, `deleteHabit`, `getHabitStats`, `setHabitDone`); foundation 14.4 now lists them. `listHabitDays` gains the optional query `habit`.

### 2.1 Live topics

| Topic | Payload | Whole list or one item | Max rate | Publisher and when |
|---|---|---|---|---|
| `habits` | `HabitSummary[]` (the same value as `GET /habits`) | whole list | 4 per second (`Throttle`, newest wins) | `life::Service`. (a) After a tick (always). (b) After `habits.toml` changed (create, edit, delete, or a hand edit seen by `TomlFile::subscribe`). (c) After the daily-notes watcher or `POST /vault/reread` or a Settings change found the notes changed: the new snapshot is compared with the last published one and sent only if different. A frame is computed only when a client is connected (`bus.has_receivers()`, see open points). Payload size: 50 habits at most, about 700 bytes each, far under `FRAME_MAX`. |

The frontend resource `habits` follows this topic (`useResource('habits', ..., { live: 'habits' })`), so a tick made in Obsidian shows in Rain within about one second while the window is open.

---------------------------------------------------------------------------------------------------

## 3. Types

File: `crates/rain-types/src/life.rs`. One line per type is added to the `decls!` list in `bindings.rs`: `Cadence, Habit, HabitDraft, HabitCell, HabitSummary, HabitDay, HabitStats, HabitTickBody, VaultStatus, PropertySuggestion, OpenNoteResult, CalendarEvent`. `Topic` struct, in `crates/rain-types/src/events.rs` (foundation 5.3), added in stage 7: `pub struct Habits; impl Topic for Habits { const NAME: &'static str = "habits"; type Payload = Vec<HabitSummary>; }` and the name `habits` is added to the `TOPIC_NAMES` list.

All structs follow foundation 9.2: `#[serde(rename_all = "camelCase")]`, `#[ts(optional_fields)]`, `Option` fields with `#[serde(default, skip_serializing_if = "Option::is_none")]`, string-union enums inline in fields with `#[ts(inline)]`, request bodies with `#[serde(deny_unknown_fields)]`, dates and ids as `String`.

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::common::IconRef;

/// How often a habit is meant to be done. Daily means every day (times is 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Cadence { Daily, Weekly, Monthly }

/// A habit definition: what the user wrote in habits.toml. Number habits and 'skip' no longer exist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Habit {
    pub id: String,
    pub name: String,
    pub icon: IconRef,
    /// The checkbox property in the daily note's frontmatter, for example "gym".
    pub property: String,
    #[ts(inline)]
    pub cadence: Cadence,
    /// How many ticked days make a period complete. 1 for daily. 1 to 7 weekly. 1 to 31 monthly.
    pub times: u32,
}

/// The body of POST /habits and PUT /habits/{id}. Same fields as Habit; `id` is chosen by Rain.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct HabitDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub icon: IconRef,
    pub property: String,
    #[ts(inline)]
    pub cadence: Cadence,
    pub times: u32,
}

/// One cell of the small history strip of a list row.
/// Daily strips use none, miss, done. Weekly and monthly strips use miss, partial, done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HabitCell {
    #[serde(rename = "none")] NoNote,   // no daily note that day (daily strips only)
    Miss,                               // note exists but the property is not true / the period has no tick
    Partial,                            // some ticks, fewer than `times`
    Done,                               // done that day / `times` reached in that period
}

/// What one list row needs. `GET /habits` and the `habits` topic carry a list of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HabitSummary {
    pub habit: Habit,
    /// The date the backend used as today. The cells are placed relative to it.
    pub today: String,
    /// True when today's note has the property set to true.
    pub today_done: bool,
    /// Ticked days in the current period (daily: today only, 0 or 1).
    pub period_count: u32,
    /// `times` (daily: 1).
    pub period_target: u32,
    /// Oldest first, the last cell is the current day / week / month.
    /// Lengths: daily 70 (10 weeks), weekly 12 (weeks), monthly 12 (months).
    pub cells: Vec<HabitCell>,
}

/// One existing daily note and which habits it has ticked. Days without a note are absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HabitDay {
    pub date: String,
    /// Ids of the habits whose property is true in that note. With ?habit= at most one id.
    pub done: Vec<String>,
}

/// Everything the full view shows besides the per-day grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct HabitStats {
    pub habit_id: String,
    pub today: String,
    /// First day in the history window whose note has this property (true or false). Absent: no data yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Daily: days in a row. Weekly: weeks in a row with `times` reached. Monthly: months in a row. The unit is the cadence.
    pub streak_current: u32,
    pub streak_best: u32,
    /// 0 to 100. Absent when no period has been counted yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_percent: Option<u32>,
    pub done_this_week: u32,
    pub done_this_month: u32,
    pub done_this_year: u32,
    /// The oldest day Rain will tick (today minus 30 days: the window is 31 days in total, today included). The grid disables older cells.
    pub tickable_from: String,
    /// The oldest day inside the history window (today minus 799 days).
    pub window_from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HabitTickBody { pub done: bool }

/// The state of the vault as the Life screen and Settings need it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct VaultStatus {
    /// A vault is chosen and it exists and is a folder.
    pub connected: bool,
    /// The effective vault folder: `Resolved.vault` of section 06 (the override, else the single vault Obsidian has registered; absent when there are several and none is chosen, or none exists). It is present even when `connected` is false (a chosen path that is not a folder), so `{path}` of `vaultMissing` has a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The folder in effect, relative to the vault ("Daily"; "" is the vault root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_folder: Option<String>,
    /// The Moment format in effect ("YYYY-MM-DD").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_format: Option<String>,
    /// The daily notes folder exists. A tick cannot create it.
    pub daily_folder_found: bool,
    /// A tick would be accepted. False in a sandbox build when the vault is outside the data folder.
    pub writable: bool,
    /// One plain sentence when notes cannot be found for a reason the user can fix. Absent when all is well.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// A checkbox property that appears in the notes and is not used by any habit yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PropertySuggestion {
    pub property: String,
    /// Number of notes in the history window that have it as true or false.
    pub days: u32,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenNoteResult { pub opened: bool }

/// Unchanged from today's types.ts. The route returns an empty list this round.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub calendar: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}
```

Generated TypeScript (the generator prints it; shown here so the frontend work can start from it; the exact text is whatever `npm run types` writes):

```ts
export type Cadence = "daily" | "weekly" | "monthly";
export type Habit = { id: string, name: string, icon: IconRef, property: string, cadence: "daily" | "weekly" | "monthly", times: number, };
export type HabitDraft = { id?: string, name: string, icon: IconRef, property: string, cadence: "daily" | "weekly" | "monthly", times: number, };
export type HabitCell = "none" | "miss" | "partial" | "done";
export type HabitSummary = { habit: Habit, today: string, todayDone: boolean, periodCount: number, periodTarget: number, cells: Array<HabitCell>, };
export type HabitDay = { date: string, done: Array<string>, };
export type HabitStats = { habitId: string, today: string, since?: string, streakCurrent: number, streakBest: number, ratePercent?: number,
  doneThisWeek: number, doneThisMonth: number, doneThisYear: number, tickableFrom: string, windowFrom: string, };
export type HabitTickBody = { done: boolean, };
export type VaultStatus = { connected: boolean, path?: string, dailyFolder?: string, dailyFormat?: string,
  dailyFolderFound: boolean, writable: boolean, problem?: string, };
export type PropertySuggestion = { property: string, days: number, lastSeen: string, };
export type OpenNoteResult = { opened: boolean, };
export type CalendarEvent = { id: string, title: string, start: string, end: string, allDay: boolean, calendar: string, color: string, location?: string, };
```

### 3.1 What changes against `src/data/types.ts`, and why

| Type | Today | After | Why |
|---|---|---|---|
| `Habit` | `{ id, name, icon, property, kind: 'boolean' \| 'number', target?, unit? }` | `{ id, name, icon, property, cadence, times }` | Number habits are gone (decisions.md section 5); cadence and times are new (section 4). |
| `HabitValue` | `boolean \| number \| 'skip' \| null` | deleted | No number, no skip, no null: a day is done or not. The `'skip'` trap of foundation 9.2 disappears with the type. |
| `HabitDay` | `{ date, noteExists, values: Record<ID, HabitValue> }`, one row per day | `{ date, done: ID[] }`, one row per existing note | A compact form; "no row" means "no note". `noteExists` is implied. |
| `VaultStatus` | `{ connected, path?, missingProperties }` | see section 3 | `missingProperties` removed (L12); folder, format and writable added. |
| new | | `HabitSummary`, `HabitCell`, `Cadence`, `HabitDraft`, `HabitStats`, `HabitTickBody`, `PropertySuggestion`, `OpenNoteResult` | The list needs no year of days per habit: a summary is about 700 bytes. |
| `CalendarEvent` | hand written | generated, identical | Ported with the stage so `types.ts` loses it. |

Number-size checks (foundation 9.2 last row): the largest value of every `u32` here is below 2^32; no `u64` is used.

Validation rules of `HabitDraft` (enforced by the backend, section 6.7):

| Field | Rule | Reason tag |
|---|---|---|
| `id` | absent on POST; absent or equal to the path on PUT | `badId` |
| `name` | trimmed, 1 to 60 characters, no control characters | `badName` |
| `icon` | `kind` is `lucide`, `brand` or `letter` (not `url`: no media here); `value` matches `^[a-z0-9-]{1,40}$` for lucide and brand, 1 to 2 characters for letter | `badIcon` |
| `property` | trimmed; passes `frontmatter::check_key` (no YAML syntax characters, no edge whitespace); at most 60 characters | `badProperty` |
| `cadence` | one of the three (serde rejects anything else with `badBody`) | |
| `times` | daily: exactly 1; weekly: 1 to 7; monthly: 1 to 31 | `badTimes` |
| uniqueness | no other habit uses the same `property` (compared exactly) | `duplicateProperty` (409) |
| count | at most 50 habits (`HABITS_MAX`) | `tooManyHabits` (409) |

---------------------------------------------------------------------------------------------------

## 4. Data files

### 4.1 `habits.toml` (owned by this section, row of foundation 10.2)

Location: `<data folder>\habits.toml`. Written only by `life::store` through `TomlFile` (foundation 10.4).

| Key | Type | Default when missing | Validation |
|---|---|---|---|
| `schema_version` | integer | 1 | higher than 1: loaded read-only (foundation 10.3 rule 2) |
| `[[habit]]` | array of tables | none | at most 50 are read; extra ones are ignored with a `ConfigIssue` |
| `habit.id` | string | derived from `name` on load (not written back until the next write) | `^[a-z0-9][a-z0-9-]{0,39}$`, unique in the file. A duplicate id: the later entry is skipped with a `ConfigIssue`. |
| `habit.name` | string | the id | 1 to 60 characters |
| `habit.icon` | string, a Lucide name | `"square-check"` | `^[a-z0-9-]{1,40}$`. A name that Rain's icon map does not know is shown as a letter tile; it is not an error. |
| `habit.property` | string | the id | same rule as the draft. A duplicate of an earlier habit's property: skipped with a `ConfigIssue`. |
| `habit.cadence` | `"daily"` \| `"weekly"` \| `"monthly"` | `"daily"` | anything else: skipped with a `ConfigIssue` |
| `habit.times` | integer | 1 | daily: must be 1 or absent; weekly 1 to 7; monthly 1 to 31. Outside: skipped with a `ConfigIssue` |

An invalid entry is skipped on its own, with a `ConfigIssue { file: "habits.toml", line, message }`; the other habits keep working, and a habit save or delete is not blocked by the skipped entry (its text stays unchanged; `configBroken` is for a file that does not parse). (A TOML syntax error is different: the whole file fails and the last good list is kept, foundation 10.4.) The service reports the per-entry problems with `state.health.set_file_issues("habits.toml", issues)` (open points). Order of the list in the UI = order in the file, inside each cadence group.

Commented example (this exact text is what Rain writes as the file header on the first create; a missing file means "no habits"):

```toml
# Rain habits. One [[habit]] per habit. Rain edits this file when you add, change or delete a
# habit in the Life screen; you may edit it by hand too (Rain notices within a second).
# Keep comments up here: a comment directly above a [[habit]] is removed with that habit.
#
# A habit is a checkbox property at the top of your daily note, for example  gym: true
#
#   id        a short name Rain uses inside; letters, digits and dashes; never changes
#   name      what the Life screen shows
#   icon      a Lucide icon name, for example "dumbbell"
#   property  the property name in the note's frontmatter
#   cadence   "daily", "weekly" or "monthly"
#   times     for weekly: ticked days per week (1 to 7); for monthly: per month (1 to 31)
schema_version = 1

[[habit]]
id = "gym"
name = "Gym"
icon = "dumbbell"
property = "gym"
cadence = "weekly"
times = 3

[[habit]]
id = "read"
name = "Read"
icon = "book-open"
property = "read"
cadence = "daily"
```

Who writes and when:

| Action | What Rain writes |
|---|---|
| create | appends one `[[habit]]` table with the keys in the order id, name, icon, property, cadence, times (`times` is written for weekly and monthly only). On the first write the file is created with the header above. |
| edit | for the table whose `id` matches: sets only the keys that changed, keeping the value decor (trailing comments) with `set_*_keep_decor` of the spike `toml_store.rs`. A key that the user removed by hand and that is unchanged is not added back. |
| delete | removes the table. The comment lines directly above it go with it (foundation 10.4). |

A hand edit may do anything that stays valid: reorder, rename, change a property, add a habit with a new `id`. Changing an `id` makes it a different habit for the cursor state in the UI and nothing else (no table of this module is keyed by habit id). The file is never deleted by Rain.

### 4.2 Files Rain reads but does not own

| File | Read for | Rule |
|---|---|---|
| `<vault>\.obsidian\daily-notes.json` | `folder` and `format` of Obsidian's daily notes | NOT read by this section: section 06 reads it once and delivers `Resolved.daily_note_folder` and `Resolved.daily_note_format` (06 6.1.2). Never written. |
| `%APPDATA%\obsidian\obsidian.json` | the id of the vault for the `obsidian://open` link | NOT read here: the id comes from the shared reader `settings().vault_id()` of section 06 (06 6.1.7), so `obsidian.json` is read in one place. Missing: no id (the link falls back to `path=`). Never written. |
| the daily notes | the first 64 KiB of each note (`HEAD_READ_MAX`) to find the top-level boolean properties | `File::open` read-only. Note text is never stored, logged or sent. |

The Settings fields that steer all this belong to section 06 (`vault_path`, `daily_note_folder`, `daily_note_format` in `settings.toml`): see the contract-change table, row S.9.

---------------------------------------------------------------------------------------------------

## 5. Tables

Migration list of the module: `pub const MIGRATIONS: &[Migration] = &[Migration { id: 1, name: "create_life_tables", sql: include_str!("migrations/0001_create_life_tables.sql") }]`.

```sql
-- migrations/0001_create_life_tables.sql

-- CACHE (rebuildable). One row per daily note that existed at the last check.
-- `scope` identifies the vault path, the folder and the format together, so a changed setting
-- never mixes old rows with new ones. `bools` holds ONLY the names and values of the top-level
-- true/false properties of the note, as JSON, for example {"gym":true,"read":false}. No note text.
CREATE TABLE life_note (
  scope    TEXT    NOT NULL,   -- 16 hex characters: hash of lower-cased vault path + "|" + folder + "|" + format
  date     TEXT    NOT NULL,   -- YYYY-MM-DD
  mtime_ns INTEGER NOT NULL,   -- last write time of the file, nanoseconds since 1970
  size     INTEGER NOT NULL,   -- length in bytes
  bools    TEXT    NOT NULL,   -- JSON object, see above
  PRIMARY KEY (scope, date)
) WITHOUT ROWID;

-- HISTORY. One row per vault write Rain made (an audit trail so the user can see what Rain changed).
-- No note text, no property value other than the boolean, no path.
CREATE TABLE life_write (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  at       TEXT    NOT NULL,   -- RFC 3339 UTC, as the API
  date     TEXT    NOT NULL,   -- the note's date
  habit_id TEXT    NOT NULL,
  property TEXT    NOT NULL,
  done     INTEGER NOT NULL,   -- 1 or 0
  outcome  TEXT    NOT NULL    -- 'created' | 'updated' | 'unchanged'
);
CREATE INDEX idx_life_write_at ON life_write(at);
```

| Table | Cache or history | Retention |
|---|---|---|
| `life_note` | cache (deleting `rain.db` loses nothing) | Rows of another `scope` are deleted when the index is built. Rows for dates older than the window (800 days) are deleted at the same time. A row is replaced when the file's `(mtime_ns, size)` differs, and deleted when the file is gone. At most about 800 rows. |
| `life_write` | history | The newest 500 rows (`WRITE_LOG_KEEP`); older rows are deleted in the same transaction that inserts a new one. Nothing reads this table in this round except the integration tests and the user (with any SQLite viewer); no route exposes it. |

All access goes through `db.call(...)` (foundation 10.5). Table prefix `life_`.

---------------------------------------------------------------------------------------------------

## 6. Algorithms

### 6.1 Module layout (this is what makes the vault rule enforceable)

```
crates/rain-backend/src/modules/life/
  mod.rs            ENTRY, MIGRATIONS, constants (section 6.9), re-export of Service
  routes.rs         axum handlers. The tick handler is the ONLY place that calls vault_write::set_habit_property
  service.rs        Service: state, index, summaries, stats, habit CRUD, `tick`, open note
  store.rs          TomlFile<HabitsFile> for habits.toml, SQL for life_note and life_write
  dates.rs          pure civil-date arithmetic and Monday-week helpers (std only)
  moment.rs         pure Moment-format renderer (section 6.3)
  daily.rs          path for a date from `Resolved` (vault, folder, format); the `obsidian://` link; no file reading of its own
  index.rs          the in-memory note index and the (mtime, size) refresh (read-only file access)
  stats.rs          pure: summaries, streaks, rate, totals from a day lookup (no I/O, no clock)
  watch.rs          the `ConfigHub::watch_dir` subscription on the daily notes folder
  frontmatter.rs    PURE text functions: read_bool, read_bools, set_bool, set_bool_existing, new_note, check_key. No std::fs, no io.
  vault_write.rs    the ONLY file that writes inside the vault: set_habit_property, its temp file and rename
  migrations/0001_create_life_tables.sql
```

Rules checked by `tests/source_rules.rs` (they extend foundation 13.3 rule 1; the scan is a plain text search):

1. A file under `modules/life/` other than `vault_write.rs` contains none of: `fs::write`, `File::create`, `OpenOptions`, `fs::rename`, `fs::remove_file`, `fs::remove_dir`, `fs::create_dir`, `fs::copy`, `set_permissions`, `set_modified`. (Foundation wording: "on a path derived from the vault path"; a text scan cannot see that, so for `life/` the ban is total.) Any other file of the crate that contains one of those calls AND the words `vault_path`, `vault_root` or `life::` fails too (the foundation's crude scan for other modules; `store/` and `core/` write only the data folder and the local folder and never mention the vault).
2. `frontmatter.rs` contains neither `fs::` nor `std::io` nor `File`. It is pure text in, text out; it cannot write by construction.
3. `vault_write.rs` does not use `crate::store` or `crate::core::blocking` (it has its own temp-file code and is called from inside `blocking::run` by the route).
4. The name `set_habit_property` appears only in `vault_write.rs`, `life/service.rs` (in `tick`) and files under `tests/`. (Foundation 13.3 text.)
5. The strings `obsidian://new`, `obsidian://daily`, `obsidian://search` and `obsidian://hook-get-address` appear nowhere in the crate; `obsidian://open` appears only in `daily.rs`.
6. `life/` contains no `tracing` macro with a field named `text`, `content`, `detail`, `prompt`, `body` or `secret` (foundation 13.4), and no `tracing` call with `property` or `path` of a note.

### 6.2 Resolving the vault and the daily notes (`daily.rs`)

Inputs: `settings().resolved()` of section 06 (`vault`, `daily_note_folder`, `daily_note_format`). Section 06 (6.1.2) already applies the override, then `daily-notes.json`, then `YYYY-MM-DD`, so this section reads no `daily-notes.json` and has no fallback of its own; it keeps only the format check.

```
vault_root   = Resolved.vault                      (the effective vault: override, else the only vault Obsidian has registered; None = no vault, also when there are several and none is chosen)
connected    = vault_root is Some and metadata(vault_root).is_dir()   (Some but not a folder: `vaultMissing`; None: `noVault`)
folder_raw   = Resolved.daily_note_folder          (never empty-by-error; "" means the vault root)
format       = Resolved.daily_note_format          (effective, never empty)
folder       = normalise(folder_raw)               (defensive: 06 already normalised it)
```

`normalise`: replace `\` by `/`, split on `/`, drop empty parts and `.`, reject (problem, no notes) any part that is `..`, contains `:`, or any control character. An absolute path or a drive letter is therefore rejected. Join with `/`. `""` means the vault root.

`file_for(date) = vault_root / folder / render(format, date)` where `render` is section 6.3 and its output is split on `/` into path parts (each non-empty, not `..`), with `.md` appended to the last part. A format whose output yields a bad part gives `badDailyFormat`.

`scope = hex16(fnv1a64(lowercase(vault_root) + "|" + folder + "|" + format))` (any stable hash; not a security feature).

`VaultStatus`: `connected`; `path` = `Resolved.vault` (the effective vault, also when `connected` is false; there is no `suggestedPath`, no "Use this vault" button and no suggestion code, because section 06 already makes the only registered vault the effective one; with several vaults and none chosen there is no vault, decisions.md ruling of 2026-11-22); `dailyFolder`/`dailyFormat` = the values in effect; `dailyFolderFound` = `metadata(vault_root/folder).is_dir()`; `writable` per section 6.8 rule W1; `problem`:

| Condition | `problem` text |
|---|---|
| no path set, or several vaults registered and none chosen | absent (the screen says "Choose your vault"; nothing is read or written) |
| path set, not a folder | absent (the screen says "Vault not found") |
| format has a token Rain cannot read | "The daily note format uses {token}, which Rain cannot read. Change it in Settings." |
| folder part invalid | "The daily notes folder is not a folder inside the vault. Change it in Settings." |
| folder not found | "The daily notes folder {folder} was not found in the vault. Check it in Settings." (the folder name may be shown, it is a configured path) |

Cost: three small metadata and read calls on a blocking thread, well under 5 ms. Nothing is cached for `GET /vault`.

### 6.3 The Moment-style format (`moment.rs`)

Obsidian's daily notes plugin formats the file name with Moment.js. Rain implements only the formatting direction (date to text) with English names. A pure function: `render(format: &str, date: CivilDate) -> Result<String, MomentError>`; `MomentError::Unsupported(String)` carries the token. `validate(format)` renders a probe date and returns the error text; section 06 calls it through `life::Service::check_note_format(&str) -> Result<(), String>` before saving the setting.

Scanner: left to right.
1. `[` starts a literal that runs to the next `]` (brackets are not printed). An unclosed `[` is `Unsupported("[")`.
2. A run of one ASCII letter, taken whole, is matched against the table below. If the letter is a Moment token letter and the run length is not in the table, the result is `Unsupported(run)`. `Do` is the letter D followed by `o`.
3. Every other character (digits, `-`, space, `.`, `_`, `,`, `/`, `(`, `)`) is copied as is. Letters that are not Moment token letters are copied as is (Moment does the same).

| Token | Output | Example for 2026-11-03 (a Tuesday) |
|---|---|---|
| `YYYY` | 4-digit year | 2026 |
| `YY` | last two digits | 26 |
| `MMMM` | month name | November |
| `MMM` | month short name | Nov |
| `MM` | month, 2 digits | 11 |
| `M` | month | 11 |
| `DD` | day, 2 digits | 03 |
| `D` | day | 3 |
| `Do` | day with suffix (1st 2nd 3rd 4th ... 11th 12th 13th ... 21st 22nd 23rd 31st) | 3rd |
| `dddd` | weekday name | Tuesday |
| `ddd` | weekday short | Tue |
| `dd` | weekday two letters | Tu |
| `d` | weekday number, Sunday = 0 | 2 |
| `E` | ISO weekday, Monday = 1 | 2 |
| `WW` | ISO week, 2 digits | 45 |
| `W` | ISO week | 45 |
| `GGGG` | ISO week-year, 4 digits | 2026 |
| `GG` | ISO week-year, 2 digits | 26 |

Moment token letters that are NOT supported and give `Unsupported`: `Q A a H h k m s S Z z X x w e g L l N y`, and `Y` or `D` runs of other lengths than those in the table (for example `DDD`, `DDDD`, `Y`, `YYY`). Reason: a daily note name has no time of day; the others need a locale (week numbers by locale, localised formats) that Rain does not carry. English names only; an Obsidian set to another language writes other names and those notes are not found (the Settings override is the way out; documented in the Settings hint).

Test vectors (Moment documentation and the edge dates; a file `tests/fixtures/life/moment-vectors.txt` read by the unit test, one per line `format | date | expected`):

| Format | Date | Result |
|---|---|---|
| `YYYY-MM-DD` | 2026-11-22 | `2026-11-22` |
| `YYYY/MM/YYYY-MM-DD` | 2026-01-05 | `2026/01/2026-01-05` |
| `YYYY-MM-DD dddd` | 2026-11-22 | `2026-11-22 Sunday` |
| `[Daily] YYYY.MM.DD` | 2024-02-29 | `Daily 2024.02.29` |
| `MMMM Do, YYYY` | 2026-11-01 / 02 / 03 / 04 / 11 / 12 / 13 / 21 / 22 / 23 / 31 | `November 1st, 2026` ... `2nd`, `3rd`, `4th`, `11th`, `12th`, `13th`, `21st`, `22nd`, `23rd`, `31st` |
| `GGGG-[W]WW-E` | 2021-01-03 | `2020-W53-7` |
| `GGGG-[W]WW-E` | 2026-01-01 | `2026-W01-4` |
| `GGGG-[W]WW-E` | 2024-12-30 | `2025-W01-1` |
| `ddd, D MMM YY` | 2026-11-03 | `Tue, 3 Nov 26` |
| `YYYY-MM-DD HH:mm` | any | `Unsupported("HH")` |
| `YYYY-MM-DD [` | any | `Unsupported("[")` |
| `YYYY-DDD` | any | `Unsupported("DDD")` |

### 6.4 The note index (`index.rs`)

Purpose: for a window of 800 days ending today, know for each date whether a note exists and which top-level boolean properties it has.

```rust
struct Note { mtime_ns: i64, size: u64, bools: Vec<(String, bool)> }     // no text
struct Index {
    scope: String,
    today: CivilDate,                       // the day the window was built for
    notes: BTreeMap<CivilDate, Note>,       // only dates that have a file
    checked_at: Instant,                    // end of the last full check
    dirty: bool,                            // set by the watcher, the tick, reread and Settings changes
}
```

`Service` keeps `Mutex<Option<Index>>` (tokio mutex; held only while merging results, never across I/O).

Read path (`refresh_if_needed`, called by every route that reads notes):

1. Build the config (section 6.2). No vault or a bad format: the index is an empty one (every cell `none`); stop.
2. If game mode is active (`state.game_mode_active`): serve the index as it is (or empty), do not check files. (Foundation 11.5.)
3. If the scope differs from `index.scope`, or `today` changed, or `dirty`, or `checked_at` is older than 2 s (`RESTAT_MIN_INTERVAL`): run the check.
4. The check runs in `core::blocking::run`:
   - For each date from `today - 799` to `today` (800 dates): `path = file_for(date)`; `fs::metadata(path)`:
     - not found, or not a file: no note.
     - found with `(mtime_ns, size)` equal to the cached row: keep the cached `bools`.
     - found, different or not cached: open the file, read up to 64 KiB (`HEAD_READ_MAX`), decode with `String::from_utf8_lossy`, `frontmatter::read_bools(head)` gives the top-level true/false properties. A read error (sharing violation, access denied) keeps the old row if there is one, else the date counts as "no note" and the check counts one `unreadable` (logged once per check as a count, never a name).
   - Load the cached rows of this `scope` from `life_note` on the first check after start (one query); after the check write the changed rows and the deletions in ONE transaction (`INSERT OR REPLACE`, `DELETE`), and delete rows of other scopes and older than the window.
5. Replace the index, clear `dirty`, set `checked_at`.

Cost: 800 `metadata` calls, about 15 to 25 microseconds each for a missing file and about 40 for a present one: roughly 15 to 30 ms for the first check on this PC (about 120 notes), then the same 800 calls with no file reads at every check older than 2 s. It runs on the blocking pool, never on a worker. (No measurement exists yet: the number is an estimate from Windows file metadata costs; stage 7a measures it and writes it into the stage report; if it is over 50 ms the window shrinks to 400 days, one constant.)

Why not a directory scan: it would need the reverse of the format (name to date). The date-driven check needs only the formatter and makes a nested format such as `YYYY/MM/YYYY-MM-DD` work without any extra watch.

Why `(mtime, size)` and not a content hash: it avoids reading the 120 files at each check. Both come from the same `metadata` call. A change that keeps both equal (an edit within the same file time tick and the same size) is not seen until the next real change; with a 100 ns file time this is a theoretical case.

`read_bools` (new, in `frontmatter.rs`, read only, pure): same classification as `read_bool` (`classify`, `parse_key`, `parse_value` of the spike). Returns every top-level key whose value is a plain `true`/`false` (also `True`, `TRUE`, `False`, `FALSE`) and that occurs once in the block. A key that occurs twice is left out (not an error: a note must not break the check). No frontmatter, unterminated fence or an implausible block: an empty list. A value that is not a plain boolean (`yes`, a string, a number) is left out. Keys are returned as written (case kept, quotes removed).

### 6.5 Summaries (`stats.rs`)

Pure functions over a lookup `done(date) -> bool` and `has_note(date) -> bool`, and `today: CivilDate`. Week = Monday to Sunday (`dates::monday_of`). Month = calendar month.

`done(date)` is true when the note exists and its `bools` hold the habit's property as `true`.

Per cadence:

| | Daily | Weekly | Monthly |
|---|---|---|---|
| `period_target` | 1 | `times` | `times` |
| `period_count` | 1 if `done(today)` else 0 | number of done days from this week's Monday to today | number of done days from the 1st of this month to today |
| `cells` length | 70: one per day, `today - 69` ... `today` | 12: one per week, the Monday of the current week minus 11 weeks ... the current week | 12: one per month, current month minus 11 ... current month |
| cell state | `none` when no note, `done` when `done(date)`, else `miss` | count of done days in the week (days after today are not counted): `done` when `count >= times`, `partial` when `count > 0`, else `miss` | same, per month |
| `today_done` | `done(today)` | same | same |

Worked example (fixture `life-basic`, section 10.3; today is Sunday 2026-11-22): the habit `gym` (weekly, 3) has `period_count` 3, `period_target` 3, and cells `[miss x10, partial, done]` (week 9 to 15 Nov has 2 ticks, week 16 to 22 Nov has 3). The habit `read` (daily) has `cells` = 56 times `none`, then for 9 to 22 Nov: `miss, none, done, miss, miss, none, done, miss, done, done, done, done, done, done`.

### 6.6 Stats of one habit (`stats.rs`, route `GET /habits/{id}/stats`)

`since` = the earliest date in the window whose note has the property (true or false). No such date: `since` absent, `streak_current` 0, `streak_best` 0, `rate_percent` absent, all totals 0.

A "unit" is a day (daily), a Monday-to-Sunday week (weekly) or a calendar month (monthly). A unit is "met" when its count is at least `times` (daily: the day is done). The current unit is "open": it is `met` only when it reaches the target; it never counts against the user.

| Value | Rule |
|---|---|
| `streak_current` | Walk units backwards from the current one. The current unit adds 1 if met and is skipped (neither adds nor breaks) if not met. Every earlier unit adds 1 if met, otherwise the walk stops. Units before the window start are not walked. |
| `streak_best` | The longest run of consecutive met units among the units fully inside the window and the current unit (when met). At least `streak_current`. |
| `rate_percent` | Take the last K units (daily 90 days, weekly 12 weeks, monthly 12 months), drop units that end before `since`, drop the current unit unless it is met. If none is left: absent. Else `round(100 * met / counted)`. A day without a note counts as not met. |
| `done_this_week`, `done_this_month`, `done_this_year` | Done days from this week's Monday, the 1st of this month, 1 January, to today (all cadences). |
| `tickable_from` | `today - (TICK_WINDOW_DAYS - 1)` = `today - 30 days` |
| `window_from` | `today - 799 days` |

Worked example (`life-basic`): `read` has `since` 2026-11-09, `streak_current` 6 (22 to 17 Nov; 16 Nov is false), `streak_best` 6, `rate_percent` 57 (8 done days of 14 counted), totals 6 / 8 / 8. `gym`: `since` 2026-11-09, `streak_current` 1, `streak_best` 1, `rate_percent` 50 (week 9 to 15 not met, current week met: 1 of 2), totals 3 / 5 / 5. `review` (monthly, 2): no note has it, so no `since`, all zeros, rate absent.

### 6.7 Habit definitions (`store.rs`, `service.rs`)

Load: `TomlFile<HabitsFile>` on `habits.toml` (foundation 10.4). `HabitsFile { schema_version: Option<u32>, habit: Vec<RawHabit> }`, every field `#[serde(default)]`. The service converts `RawHabit` to `Habit` with the validation of section 4.1 and keeps `Arc<Vec<Habit>>`, rebuilt on every `TomlFile::subscribe` notification (hand edits, own writes), then publishes `habits`.

Create (`POST /habits`):
1. Validate the draft (section 3 table).
2. `id` = slug of `name`: lower case, characters outside `a-z0-9` become `-`, runs of `-` collapse, trimmed of `-`, at most 40 characters, `habit` if empty; if taken append `-2`, `-3`, and so on.
3. `TomlFile::edit`: if another habit already has this `property`: `409 duplicateProperty`; if 50 exist: `409 tooManyHabits`. Else append the table.
4. Return the new `Habit` (201). The list resource is refreshed by the topic.

Edit (`PUT`): validate; unknown id `404`; the same duplicate check against the OTHER habits; update the keys of the table found by `id`; a table that has vanished meanwhile (hand edit): `409 configChanged`. Delete: remove the table; an unknown id is `204`.

Changing the `property` of a habit changes which notes count (its history moves to the new property); the dialog says so (section 9.4). Nothing in any note is touched.

### 6.8 The tick (`routes.rs`, `service.rs`, `vault_write.rs`, `frontmatter.rs`)

#### Which function is copied from where

Copy `spikes/crates/src/frontmatter.rs` (702 lines: the pure functions, `NoteError`, `check_key`, `split_lines`, `classify`, `parse_key`, `find_key`, `parse_value`, and ALL its tests: 38 exact-output cases, 12 refusal cases plus 10 bad key names, reader cases, `new_note`, the file wrapper cases, the 20 000-note randomised invariant test with `yaml-rust2` as test oracle on 6 seeds, the 4 mutation checks) and split it:

| Spike item | Goes to | Change |
|---|---|---|
| `NoteError`, `read_bool`, `set_bool`, `new_note`, `check_key`, the line parser | `life/frontmatter.rs` | unchanged. Add `read_bools` (6.4) and `set_bool_existing` (below). |
| `tick_habit_in_file`, `Tick`, `to_io` | `life/vault_write.rs` as `set_habit_property`, `Tick`, `WriteError` | see below |
| `crate::toml_store::write_atomic` (called by the spike) | replaced by the private `write_replace` in `vault_write.rs` | The vault file must not depend on shared store code (rule 3 of 6.1). |
| the tests of the file wrapper | `vault_write.rs` tests | adapted |
| `yaml-rust2` | dev-dependency of `rain-backend` (tests only) | the foundation already lists it as test oracle |

`set_bool_existing(note, key, value) -> Result<String, NoteError>`: the same as `set_bool`, except that a note with no frontmatter, or with frontmatter that lacks the key, is returned unchanged. All refusals are the same. It exists for unticking (L2).

#### The function (the only vault write in Rain)

```rust
// life/vault_write.rs. THE ONLY FILE IN RAIN THAT WRITES INSIDE THE VAULT. See 6.1 for the scan that keeps it so.
pub enum Tick { Created, Updated, Unchanged }
pub enum WriteError {
    Note(NoteError),       // InvalidKey, DuplicateKey, NotABoolean, UnreadableFrontmatter: the note was left alone
    NotUtf8,               // the file is not valid UTF-8: left alone
    NoFolder,              // the note's folder does not exist: Rain never creates a folder
    Busy,                  // the note kept changing, or stayed locked: nothing written
    Io(std::io::Error),
}
/// Sets one boolean property of one daily note. `path` is the full path of the note. Called from inside `blocking::run`.
pub fn set_habit_property(path: &Path, key: &str, value: bool) -> Result<Tick, WriteError>;
```

Steps (`value = true` is a tick, `false` an untick):

1. Up to 3 rounds (`STALE_ROUNDS`):
2. `meta = fs::metadata(path)`.
   - Not found:
     - `value == false`: return `Unchanged` (nothing exists to untick; no file is created).
     - `value == true`: the text is `new_note(&[(key, true)], "\n")` (LF, no BOM, only this property, no template). Create with `OpenOptions::new().write(true).create_new(true)`; write; `sync_all`; return `Created`. If the open fails with `AlreadyExists`, someone created the note meanwhile: next round. If it fails with `NotFound` (the folder is missing): `NoFolder`. No folder is ever created.
   - Another error kind: `Io`.
3. Read the whole file (`fs::read`) and decode with `String::from_utf8`; failure is `NotUtf8` (the file is not touched).
4. `new = set_bool(&text, key, true)` for a tick, `set_bool_existing(&text, key, false)` for an untick. A `NoteError` becomes `WriteError::Note` (nothing written).
5. If `new == text`: return `Unchanged` (no write, no new file time).
6. Stale check: `meta2 = fs::metadata(path)`; if `len` or `modified` differs from `meta`: next round.
7. `write_replace(path, &new)`: temp file `.<note name>.rain-tmp-<pid>` in the SAME folder, `write_all`, `sync_all`, `fs::rename(temp, path)` (replace). If the rename fails with os error 5, 32 or 33 (Obsidian, a sync tool or an antivirus holds the note without delete sharing), retry up to 8 times, 25 ms apart (`NOTE_RENAME_RETRIES`; the sleep is `std::thread::sleep` inside this blocking closure with `#[allow(clippy::disallowed_methods)]` and a comment citing timer row T4); after the last failure remove the temp file and return `Busy`. The temp file is removed on every failure path.
8. Return `Updated`.
9. After 3 rounds without success: `Busy`.

What the function guarantees (and the tests assert):

| Case | Result |
|---|---|
| property `false` and a tick | only the token `false` becomes `true`; spacing, trailing comment, key quoting, line ends, BOM, key order, every other line stay byte for byte. `True` stays `True` when already true. |
| property missing, note has frontmatter | one line `key: true` is inserted before the closing `---`, with the opening fence's own line ending (CRLF note: CRLF line). |
| note has no frontmatter (also an empty file, or an unterminated `---`) | `---`, `key: true`, `---` are put at the very top, after a BOM if there is one, in the note's own line ending (LF if the note has none). The rest of the note follows unchanged. |
| note does not exist, tick | a new file holding exactly `---\nkey: true\n---\n`. |
| note does not exist, untick | nothing happens. |
| property exists, untick | the token becomes `false` (never a deleted line). |
| property missing, untick | nothing happens. |
| value is not a plain boolean (`yes`, `"true"`, empty, list, block scalar) | refused: `NotABoolean`. |
| key twice | refused: `DuplicateKey`. |
| fenced block that is not property-like | refused: `UnreadableFrontmatter`. |
| file is not UTF-8 | refused: `NotUtf8`. |
| a lone CR line ending (old Mac) | not treated as a line end; such a note is handled as one line and, if it has no `---` on the first line, gets a block on top. Documented as unproven (appendix C). |
| already the requested value | no write at all (file time unchanged). |

#### Concurrency with Obsidian

Obsidian keeps the open note in memory and saves about two seconds after the last keystroke. Rain reads the file, edits the text in memory, re-checks length and file time just before the replace, and replaces the whole file atomically. Four outcomes:

- Obsidian has the note open and idle: it notices the changed file and reloads it (its normal behaviour for an external change). This is the main case.
- Obsidian saved between Rain's read and the stale check: the check fails, Rain reads again and edits the new text (up to 3 rounds).
- Obsidian saves in the few microseconds between the stale check and the rename: one of the two writes wins; Obsidian merges external changes into an open editor when it can. This race is accepted and listed in appendix C (it needs a real Obsidian with unsaved edits to test).
- The note is locked without delete sharing: `Busy` after about 0.2 s; the user sees "The note keeps changing" and can press Enter again.

The temp file is dot-prefixed (hidden from Obsidian's file list) and lives for a few milliseconds. A sync or git tool may see it only if it scans at that moment; after a crash a stray `.<note>.rain-tmp-<pid>` can remain and is harmless (Rain never deletes anything else in the vault, so it does not clean it up; the user may delete it).

#### The routing of one tick

```rust
// life/routes.rs: no logic, one call
async fn tick(State(s): State<AppState>, ApiPath((id, date)): ApiPath<(String, String)>, ApiJson(body): ApiJson<HabitTickBody>)
    -> ApiResult<Json<HabitSummary>> {
    s.life().tick(&id, &date, body.done).await.map(Json)
}
```

`life::Service::tick` (in `life/service.rs`; the ONLY place that references `set_habit_property`, called only from the route handler above; foundation 13.3 rule 1) does the whole job in this order. 1. Validate: parse the date (`badDate`); `date > today` is `futureDate`; `date < today - 30` (that is, older than `TICK_WINDOW_DAYS - 1` days before today) is `tooOld`; find the habit (`unknownHabit`); take `Resolved` (`noVault`, `vaultMissing`, `badDailyFormat`); apply the sandbox rule W1; check that the note's folder exists (`noDailyFolder`); take the write lock (an `OwnedMutexGuard`, two ticks never overlap). 2. The one write, on a blocking thread: `blocking::run(move || vault_write::set_habit_property(&path, &key, value))`. 3. On `Ok(tick)`: re-check just that date (one `metadata`, one head read), update the index row and `life_note`, insert a `life_write` row (outcome `created`, `updated` or `unchanged`), `tracing::info!(habit = %id, date = %date, outcome = ?tick, "habit tick")`, publish `habits`, return the new `HabitSummary` and drop the guard. On `Err`: map with the table in section 8; nothing else changes. There is no `plan_tick` and no `finish_tick`.

Rule W1 (sandbox): the write is allowed when `!env.sandbox()` or when the canonical path of the vault folder lies inside the canonical path of the data folder (a fixture vault). Otherwise `409 conflict`, reason `sandbox`. `VaultStatus.writable` is the same test. In a debug build, tests and every run with `RAIN_DATA_DIR` set, the real vault is therefore never written.


### 6.9 Opening the daily note (`daily.rs`, `service.rs`)

1. Parse the date (`badDate`). Resolve the config; `noVault`, `vaultMissing`, `badDailyFormat` as in 6.2.
2. `fs::metadata(file_for(date))`: not found or not a file: return `{ "opened": false }`. THE CHECK COMES FIRST so that Obsidian is never asked for a missing note (an `obsidian://open` link to a missing file may create it; untested, so never risked).
3. Build the link. With the vault id from `settings().vault_id()` (section 06 6.1.7; no argument, the id of the effective vault; it matches the vault path to an `obsidian.json` entry case-insensitively, with `/` and `\` treated alike, trailing slash ignored): `obsidian://open?vault=<id>&file=<relative path without .md>`. Without an id: `obsidian://open?path=<absolute path>` (UNPROVEN for Windows paths; see appendix C). Percent-encode every byte except `A-Z a-z 0-9 - . _ ~`; a `/` between folder parts becomes `%2F`.
4. Start it with the one `Opener` of foundation 11.6 (`state.opener()`, `OpenRequest::ShellExec { file: uri, params: None, dir: None }`; its production implementation makes the one shell call, `platform::shell::shell_execute`; never `cmd /c start`). The tests use the one `FakeOpener` of section 01, as foundation 11.6 requires for every launch. A failure: `500 failed`, reason `openFailed`.
5. Return `{ "opened": true }`. Rain cannot know whether Obsidian showed the note.

Never used: `obsidian://new`, `obsidian://daily` (it creates the note), the Obsidian CLI (not installed on this PC; research B10).

### 6.10 Suggestions (`GET /vault/properties`)

From the index (refreshed as in 6.4): for every top-level boolean property in the window, `days` = number of notes that have it, `last_seen` = newest such date. Remove properties used by a habit and names that fail `check_key`. Sort by `days` descending, then `last_seen` descending, then name; at most 20 (`SUGGESTIONS_MAX`). No vault: `[]`.

### 6.11 Calendar

`calendar_events(from, to)`: validate both as dates (`badDate`) and return `[]`. No fetch, no secret, no crate (decisions.md section 4).

### 6.12 Constants (module `life/mod.rs`; asked to be added to foundation Appendix A)

| Name | Value | Used in |
|---|---|---|
| `LIFE_INDEX_DAYS` | 800 | 6.4 window |
| `LIFE_LIST_DAYS`, `LIFE_LIST_WEEKS`, `LIFE_LIST_MONTHS` | 70, 12, 12 | 6.5 cells |
| `TICK_WINDOW_DAYS` | 31 (days in total, today included: today and the 30 days before it) | 6.8 |
| `RATE_DAYS`, `RATE_WEEKS`, `RATE_MONTHS` | 90, 12, 12 | 6.6 |
| `RESTAT_MIN_INTERVAL` | 2 s | 6.4 |
| `HEAD_READ_MAX` | 64 KiB | 6.4 |
| `HABIT_DAYS_RANGE_MAX` | 1200 days | route `habit-days` |
| `HABITS_MAX` | 50 | 3 |
| `SUGGESTIONS_MAX` | 20 | 6.10 |
| `WRITE_LOG_KEEP` | 500 | 5 |
| `STALE_ROUNDS` | 3 | 6.8 |
| `NOTE_RENAME_RETRIES` | 8 attempts, 25 ms apart | 6.8 |

### 6.13 Dates (`dates.rs`)

`CivilDate { y: i32, m: u8, d: u8 }`: `parse("YYYY-MM-DD")` (real dates only), `Display`, `add_days(n)` and `days_between` through the days-from-civil algorithm (proleptic Gregorian), `weekday()` (Monday = 0), `monday_of()`, `iso_week_and_year()`, `first_of_month()`, `add_months(n)`. Today = `platform::time::local_date(clock.now())` (foundation 4.3). No time zone code in the module: only calendar arithmetic. Tests: leap days (2024-02-29, 2100-02-28 + 1), year ends, ISO week 53 (2020-12-31 and 2021-01-03), DST change days (arithmetic is calendar based, so unaffected).

---------------------------------------------------------------------------------------------------

## 7. Background work

| Task | Trigger | Timer row (foundation 11.5) | Cost | In game mode | How it stops |
|---|---|---|---|---|---|
| Daily notes watcher (`watch.rs`) | `ConfigHub::watch_dir(folder, false, 300 ms)` (foundation 10.4; `ReadDirectoryChangesW` underneath, `NonRecursive`; this section builds no debouncer of its own), any event kind. On an event: set `dirty`; if at least one client is connected, spawn one task that checks the notes (6.4), builds the snapshot and publishes `habits` only if it differs from the last one. | none. The 300 ms debounce is T4. | One watcher and one debouncer thread, zero CPU idle. Only the daily notes folder is watched, never the vault root, because the vault is a git repository with heavy churn in `.git`. If the folder is the vault root (`folder = ""`) or does not exist, no watcher is started; correctness does not need it (L11). | Events are ignored (no check, no publish). | Dropped in `stop()`, and re-created when Settings change the vault, folder or format. |
| Settings change | `settings().watch()` (a `watch` channel of `Resolved`, no timer) | none | re-resolve (6.2), drop the index, re-create the watcher, publish `habits` if a client is connected | not run | none |
| Habit file change | `TomlFile::subscribe` (no timer) | none | rebuild the habit list, publish | not run | none |
| Note check on request | each read route (6.4) | none | up to about 30 ms on a blocking thread, at most once per 2 s | skipped (cached data served) | ends by itself |
| Rename retry of a tick | inside one tick request, after a sharing violation | T4 (a short wait inside one operation) | at most 8 times 25 ms | the tick is still accepted (user action) | ends by itself |

There is no polling timer in this module. Idle cost with no client and no change: zero wake-ups.

Frontend timers: none new. Midnight is handled by the existing `useNow` (the screen refetches `habits` when `today` changes, section 9.3).

---------------------------------------------------------------------------------------------------

## 8. Errors

All bodies are `{ code, message, source?, reason? }`. Source is `vault` for vault problems and `habits` for the rest. Messages never contain note text. A configured property name, folder or vault path may appear.

| Situation | Status | code | reason | message | What the UI shows |
|---|---|---|---|---|---|
| unknown habit id | 404 | notFound | `unknownHabit` | "Habit not found" | notice (a stale screen); the list reloads |
| date not `YYYY-MM-DD` or not a real date | 400 | invalid | `badDate` | "That is not a valid date" | none in normal use (client bug) |
| range wrong or over 1200 days | 400 | invalid | `badRange` | "The date range is not valid" | same |
| tick for a future day | 400 | invalid | `futureDate` | "That day has not happened yet" | notice |
| tick older than 31 days | 400 | invalid | `tooOld` | "Rain only changes the last 31 days. Change older days in Obsidian." | notice |
| habit draft invalid | 400 | invalid | `badName`, `badProperty`, `badIcon`, `badTimes`, `badId` | "Give the habit a name", "That property name cannot be used in a note", "That icon is not allowed", "Times must be 1 to 7 a week or 1 to 31 a month", "A new habit cannot have an id" | inline text in the habit dialog |
| property already used | 409 | conflict | `duplicateProperty` | "Another habit already uses the property '{property}'" | inline in the dialog |
| 50 habits exist | 409 | conflict | `tooManyHabits` | "Rain keeps at most 50 habits" | inline |
| `habits.toml` changed under the edit | 409 | conflict | `configChanged` | "habits.toml changed. Try again." | inline / notice |
| `habits.toml` written by a newer Rain | 409 | conflict | `configNewer` | "habits.toml was written by a newer Rain and is read only" | inline / notice |
| `habits.toml` does not parse (`POST`, `PUT`, `DELETE /habits`; a skipped bad habit does not count) | 409 | conflict | `configBroken` | "habits.toml has an error. Fix the file, then try again" | inline / notice; `GET /health` lists the issue |
| `habits.toml` cannot be written | 500 | failed | `saveFailed` | "Could not save habits.toml" | inline / notice |
| no vault path set | 503 | unavailable | `noVault` | "No Obsidian vault is set. Choose one in Settings." | notice; the screen already shows the empty state |
| vault folder missing | 503 | unavailable | `vaultMissing` | "Obsidian vault not found at {path}" | notice |
| format has an unsupported token | 503 | unavailable | `badDailyFormat` | "The daily note format uses {token}, which Rain cannot read. Change it in Settings." | notice |
| the note's folder is missing | 503 | unavailable | `noDailyFolder` | "The folder for that note does not exist. Rain never creates folders in your vault." | notice |
| sandbox build, vault outside the data folder | 409 | conflict | `sandbox` | "Ticking is switched off in this development build." | notice; checkboxes are disabled (`writable: false`) |
| the note kept changing or stayed locked | 409 | conflict | `noteBusy` | "The note keeps changing. Try again in a moment." | notice |
| the property is not true/false in that note | 409 | conflict | `notBoolean` | "The property '{property}' in that note is not a checkbox, so Rain left the note alone." | notice |
| the property appears twice | 409 | conflict | `duplicateKey` | "The property '{property}' appears twice in that note, so Rain left the note alone." | notice |
| the top of the note is not YAML-like | 409 | conflict | `unreadableNote` | "The top of that note is not in a format Rain understands, so Rain left the note alone." | notice |
| the note is not UTF-8 | 409 | conflict | `notUtf8` | "That note is not valid UTF-8, so Rain left the note alone." | notice |
| any other I/O error while writing | 500 | failed | `writeFailed` | "Could not save the note." | notice |
| Obsidian could not be started | 500 | failed | `openFailed` | "Could not start Obsidian." | notice (an error through `runAction`) |
| the module failed to start | 503 | unavailable | `moduleFailed` | (foundation 11.7) | error rows on the screen |
| the database is newer than the code | 503 | unavailable | `dbTooNew` | (foundation 10.5) | same |

Degraded sources and `GET /health` `sources`:

| Source name | state | message |
|---|---|---|
| `vault` | `off` | "No vault set" (path empty) |
| `vault` | `failed` | "Vault folder not found" or "Daily notes folder not found" or the format problem |
| `vault` | `ok` | (none) |

`habits.toml` per-entry problems are `ConfigIssue`s (foundation 6.9), not sources. `GET /health` is where a user finds why a habit is missing from the list.

---------------------------------------------------------------------------------------------------

## 9. Frontend changes

All paths are relative to the repository root unless they start with `docs`. Line numbers are those of the files today (commit `e4c7f70`).

### 9.1 The design in words

Life becomes a list of habits. Nothing else.

```
+--------------------------------------------------------------------------------------------------+
| Daily  2                                                                                         |
|  [dumbbell]  Gym ......................  [ 70 small cells, 10 weeks, today at the right ]   [x]  |
|  [book]      Read .....................  [ 70 small cells ]                                  [ ]  |
| Weekly  1                                                                                        |
|  [dumbbell]  Gym                  2 / 3  [ 12 cells, one per week ]                          [x]  |
| Monthly  1                                                                                       |
|  [pen]       Review               1 / 2  [ 12 cells, one per month ]                         [ ]  |
+--------------------------------------------------------------------------------------------------+
   Enter ticks today        Ctrl+Enter opens the habit        Click the box ticks; click the row opens
```

- No strip, no pane, no calendar. 40 px rows like every other list (`--row-h`), group headers "Daily", "Weekly", "Monthly" with counts (`SectionHeader` via `ItemList`).
- Row (left to right): habit icon 20 px; name 14 px (`truncate`); for weekly and monthly the progress "2 / 3" in 13 px muted (it turns `text-foreground` when the target is reached); the heat strip; today's checkbox.
- Heat strip, daily: 70 cells (10 weeks) in one line, oldest on the left, today on the right with the 1 px foreground outline. Cell 10 px, gap 2 px (838 px at most; the cells shrink with the window through `grid-template-columns: repeat(70, minmax(0, 10px))`). Weekly and monthly: 12 cells of 14 px, gap 3 px, the last cell is the current week or month and has the outline. Colours: `done` = `--heat-3`, `partial` = `--heat-1`, `miss` = `--heat-0`, `none` = hollow with a 1 px `--selected` border (the old "no note" look). `--heat-2` and `--heat-4` are no longer used by Life.
- The strips are right aligned, so the checkboxes line up in one column whatever the group.
- No tooltips on single cells; the full view has the detail.

The full view of one habit (opened with Ctrl+Enter or a click on the row):

```
 [<-]  [dumbbell]  Gym   Weekly, 3 times                                          [x] today    [pencil]
 Streak 1 week    Best 1 week    Rate 50%    This week 3    This month 5    This year 5
 [ year grid: 53 week columns x 7 weekday rows, 12 px cells, month labels, M W F labels ]     < 2026 >
 Sun 22 Nov: not done
```

- No pane. The column is the main area (about 1224 px wide at the default window; the grid is 820 px).
- `[<-]` is a ghost icon button (`arrow-left`, tooltip "Back" with the `Esc` keycap) and does the same as Esc.
- The checkbox is TODAY's, as on the list. The pencil opens the habit dialog.
- Stats row: label 12 px `--text-3` above value 20 px. Streak and Best carry the unit of the cadence ("5 days", "1 week", "2 months"). Rate shows an en-dash style "-" when absent. Totals are done days.
- Year grid: the old `YearHeatmap` grid for ONE habit. Cell colours: done `--heat-3`, miss `--heat-0` (note exists, not done), none hollow (no note). Future days are not drawn. Today has the 1 px outline, the selected day a 2 px `--ring` outline. Days older than `tickableFrom` are drawn at 50% opacity. Days before `windowFrom` are always hollow.
- Cursor: Left/Right move a week, Up/Down a day, PageUp/PageDown a year (clamped to `windowFrom` ... today). It starts on today.
- Read-out line under the grid: "Sun 22 Nov: done" / "not done" / "no note" (13 px muted), `.selectable`.
- Enter ticks or unticks the selected day. Ctrl+Enter opens that day's note. Esc (or Backspace on an empty query) goes back to the list. The list keeps its selection.

Why no right-hand pane on the list (L6): a row already says everything a pane could (name, today, trend). A pane would force a 340 px column on every row for information that only matters for one habit at a time; that information is the full view, one keypress away and given the whole screen.

Why the full view is not a new route (L7): see decisions table. It is a sub-view of Life, held in `useViewState('life.open', { habitId: '' })`, so it survives a short hide and is forgotten with the other view states after the "Return to Home" delay.

### 9.2 Files: new, rewritten, deleted

| File | Action | What |
|---|---|---|
| `features/life/LifeScreen.vue` (322 lines) | rewrite | see 9.3 |
| `features/life/HabitRow.vue` | new | one list row |
| `features/life/HabitStrip.vue` | new | the heat strip of a row |
| `features/life/HabitView.vue` | new | the full view |
| `features/life/HabitYear.vue` | new | the year grid of one habit; replaces `YearHeatmap.vue` |
| `features/life/HabitEditorDialog.vue` | new | create and edit dialog (9.4) |
| `features/life/HeatCell.vue` (32 lines) | edit | the `state` prop type (import on line 3, prop on line 6) becomes `HabitCell` from `@/data/types`; the `FILL` map (lines 11 to 19) becomes `none: 'border border-selected'`, `miss: 'bg-heat-0'`, `partial: 'bg-heat-1'`, `done: 'bg-heat-3'`; delete the `skip` dash `span` (line 30); the `today` and `selected` props stay |
| `features/life/actions.ts` (51 lines) | rewrite | 9.5 |
| `features/life/types.ts` | new | `HabitDayItem` (target item of the full view) |
| `features/life/rows.ts` | new | pure: `groupHabits(list, query)` |
| `features/life/yearGrid.ts` | new | pure: `yearCells(year, today)` gives `{ date, column, row }[]`, month labels; extracted from `YearHeatmap.vue` lines 33 to 74 so it can be unit tested |
| `features/life/useYearCursor.ts` | new | replaces `useDayCursor.ts` (year-only cursor, no rows, no chips) |
| `features/life/dates.ts` (54 lines) | edit | delete `isWeekend` (lines 45 to 48); keep the rest |
| `features/life/MonthHeatmap.vue`, `YearHeatmap.vue`, `DayPanel.vue`, `Agenda.vue`, `useDayCursor.ts`, `heat.ts` | delete | `heat.ts` (cell, share, streak, completion) is replaced by the backend; the month grid and the agenda are gone; the day panel is gone |

Reused as they are: `HeatCell.vue` (edited), `dates.ts`, `ScreenFrame`, `ItemList`, `EmptyState`, `ErrorRow`, `SectionHeader`, `RIcon`, `KeyHint`, `Checkbox` (`@/components/ui/checkbox`), `Dialog` parts, `ToggleGroup`.

### 9.3 `LifeScreen.vue` (rewrite)

Structure:

```ts
const nav = useNav(); const query = useSectionQuery('life'); const now = useNow()
const today = computed(() => toISODate(new Date(now.value)))
const view = useViewState('life.open', { habitId: '' })        // '' = the list
const vault   = useResource('vault', () => api.getVaultStatus())
const habits  = useResource('habits', () => api.listHabits(), { live: 'habits' })
watch(today, () => void habits.reload())                        // after midnight
// "Show in Life" from Home or the command: nav.go('life', undefined, { habit: id }) opens that habit (read once, like AppsScreen.wantedPoolId)
```

States, in this order (the same precedence as today's screen, lines 237 to 274):

1. `vault.error ?? habits.error`: `ErrorRow` "Vault unavailable" with Retry (`reload` of both).
2. not ready (`vault.data` or `habits.data` undefined): skeleton of 5 rows after 150 ms, else nothing.
3. `!vault.connected`: `EmptyState` icon `calendar-check`.
   - with `path`: title "Vault not found", hint "Nothing at {path}. Check the path in Settings." button "Open settings".
   - else: title "Choose your vault", hint "Point Rain at your Obsidian vault to see your habits.", button "Open settings".
4. `habits` is empty: `EmptyState` icon `calendar-check`, title "No habits yet", hint "A habit is a checkbox property at the top of your daily note.", button "New habit" with `action-keys="Ctrl+KeyN"` -> `habitEditor.create()`.
5. a query with no match: `EmptyState` icon `search-x` "No habit matches".
6. `view.habitId !== ''` and the habit exists: `<HabitView>`. If the id no longer exists (deleted, renamed by hand) the view state is cleared and the list shows.
7. else the list: `<ItemList :groups :selected-id click-runs @activate="open" @context>` with `HabitRow` in the `row` slot. `click-runs` is true so ONE click calls `@activate`, which here OPENS THE FULL VIEW (not the primary action). The checkbox button inside the row calls `@click.stop` so it never reaches the row.

Above the list, only when there is something to say (`ErrorRow` is not used because there is nothing to retry):

- `vault.problem` set: one 40 px muted line with `triangle-alert` 16 px, the sentence, and a ghost button "Open settings".
- `!vault.writable`: one muted line "Ticking is switched off in this development build." (shown only in sandbox builds).

Groups: `groupHabits(habits.data, query)` returns `SelGroup<HabitSummary & { id: string }>[]` with ids `daily`, `weekly`, `monthly`, titles "Daily", "Weekly", "Monthly", counts, items filtered by `matches(query, habit.name)`, in the order of the list. The item id is the habit id. Selection: `useSelection(groups, { resetOn: query, persistKey: 'life.selected' })`.

`useScreen` (list):

```ts
useScreen({
  target: computed(() => open.value ? dayTarget.value : selectedHabit.value
      ? { type: 'habit', id: selectedHabit.value.habit.id, item: selectedHabit.value } : null),
  move: (dir) => open.value ? cursor.move(dir) : move(dir),       // list: returns false for left/right so the caret keeps them
  page: (dir) => open.value ? cursor.page(dir) : page(dir),
  group: (dir) => { if (!open.value) group(dir) },
  back: () => { if (view.habitId === '') return false; view.habitId = ''; return true },
  stepOut: () => (view.habitId === '' ? false : (view.habitId = '', true)),
})
```

`today` changes at midnight: the `watch(today)` above refetches; the stats and days resources of `HabitView` do the same (their keys are re-read).

### 9.4 Components

**`HabitRow.vue`**, props `{ item: HabitSummary; selected: boolean; writable: boolean }`, emits `tick`. Markup: a `div` like `ListRow` (`flex h-(--row-h) items-center gap-3 rounded-lg px-2`, `bg-selected` when selected, `hover:bg-hover` otherwise); `RIcon :icon="item.habit.icon" :size="20"`; name (`min-w-0 flex-1 truncate text-sm`); for `cadence !== 'daily'` a `span` with `${periodCount} / ${periodTarget}` (`w-12 text-right text-13`, `text-foreground` when `periodCount >= periodTarget`, else `text-muted-foreground`); `<HabitStrip :cells :cadence />`; a `Checkbox` (`:model-value="item.todayDone"`, `:disabled="!writable"`, `tabindex="-1"`, `:aria-label="`${name}, today`"`, `@click.stop`, `@update:model-value="emit('tick')"`, `@mousedown="keepFocus"`). `tick` runs `runPrimary({ type: 'habit', ... })` in the screen.

**`HabitStrip.vue`**, props `{ cells: HabitCell[]; cadence: Cadence }`. A `div` with `display: grid; grid-template-columns: repeat(N, minmax(0, Wpx)); gap: Gpx` where N is `cells.length` and (W, G) is (10, 2) for daily and (14, 3) for the others; one `HeatCell` per cell; the last has `today` true. `role="img"` with `aria-label` like "Last 10 weeks".

**`HabitView.vue`**, props `{ habit: HabitSummary; today: ISODate }`, emits `back`, `edit`, `tick-today`. Owns:

```ts
const stats = useResource('habitStats:open', () => api.getHabitStats(props.habit.habit.id))
const year = ref(Number(props.today.slice(0, 4)))
const days = useResource('habitDays:open',
  () => api.listHabitDays(`${year.value}-01-01`, minDate(`${year.value}-12-31`, props.today), props.habit.habit.id))
watch([() => props.habit.habit.id, year], () => { void stats.reload(); void days.reload() })
watch(() => props.habit, () => { void stats.reload(); void days.reload() })    // a tick or an external change (the habits topic) refreshes both
```

`useYearCursor({ today, min: windowFrom })` holds `{ date }` in `useViewState('life.cursor', { date: '' })`; `''` means today (follows midnight). `move(dir)`: left/right +-7 days, up/down +-1 day; `page(dir)`: +-12 months (`addMonths`); all clamped to `[windowFrom, today]`; `selectedYear` follows the date. `cellState(date)`: `rows.has(date) ? (rows.get(date).done.includes(id) ? 'done' : 'miss') : 'none'` where `rows` is `Map(date -> HabitDay)`. The `HabitDayItem` target: `{ habit, date, done, noteExists: rows.has(date), tickable: date >= stats.tickableFrom }`.

Failure states inside the view: `stats.error` or `days.error`: `ErrorRow` "Habit unavailable" with Retry; loading: skeleton after 150 ms.

**`HabitYear.vue`**: props `{ year: number; today: ISODate; selected: ISODate; stateOf(date): HabitCell; tickableFrom: ISODate; windowFrom: ISODate }`; emits `select(date)`. It is the grid of `YearHeatmap.vue` lines 85 to 137 without the chip group (lines 87 to 106) and with `cells` from `yearCells(...)` of `yearGrid.ts`. The year label with two chevron buttons sits in the view's header row. A click on a cell emits `select`.

**`HabitEditorDialog.vue`** (modelled on `apps/PoolEditorDialog.vue`, same structure and key rule): open while `habitEditor.draft` is not null; `useOverlay(open)`; `@keydown.capture.ctrl.enter.stop.prevent="save"`.

| Field | Control | Behaviour |
|---|---|---|
| Name | `Input` (autofocus) | required |
| Property | `Input`, mono 12 px | filled from the name (`Gym` -> `gym`: lower case, spaces to `-`) until the user edits it; hint under it: "The checkbox property in your daily note. Changing it changes which notes count." |
| Found in your notes | up to 8 chips from `listHabitSuggestions()` (fetched when the dialog opens, held in a local ref, not a shared resource) "gym - 6 days"; clicking one fills Property and, if Name is empty, a name made from it | hidden when the list is empty |
| Icon | a grid of 28 icon buttons (`RIcon` 16 px), one selected (ring). The set: `dumbbell book-open brain glass-water footprints person-standing notebook-pen moon pill sun heart-pulse bike droplets leaf salad coffee bed guitar graduation-cap flame smile utensils mountain music sunrise timer target waves` (all are in `icon-map.ts` today) | default `square-check` for a new habit is not offered in the grid; the first grid icon is preselected |
| How often | `ToggleGroup type="single"` with "Daily", "Weekly", "Monthly" (never empty) | |
| Times | `Input type=number`, min 1, max 7 or 31, shown for weekly and monthly with the label "times a week" / "times a month" | daily sends 1 |

Buttons: "Cancel" (ghost), "Save" with `KeyHint Ctrl+Enter` (disabled while `saving` or when the name or property is empty). Save calls `api.saveHabit(...)`, then `refreshResources('habits', 'habitStats', 'habitDays')`, `notice.success('Saved {name}')`, closes. A rejection shows `errorMessage(reason, 'Could not save the habit')` inline (`role="alert"`, `text-status-failed`), keeps the dialog open and clears `saving` (the Save button is disabled while saving, so a double click cannot send two creates; the backend also refuses a duplicate property).

`habitEditor` (in `actions.ts`): `{ draft: ref<HabitDraft | null>(null), create(), edit(habit) }` exactly like `poolEditor`, `create` sets `{ name: '', icon: { kind: 'lucide', value: 'dumbbell' }, property: '', cadence: 'daily', times: 1 }`.

### 9.5 Actions and key bindings

`features/life/actions.ts` (full replacement):

```ts
// Actions for a habit row and for a day of the open habit (spec 4.3), the habit dialog hook,
// and "open today's note", which Ctrl+T and a command also use.
import { ref } from 'vue'
import { hideAfterSuccess } from '@/actions/common'
import type { ActionContext, ActionProvider, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import { useViewState } from '@/composables/useViewState'
import type { Habit, HabitDraft, HabitSummary, ISODate } from '@/data/types'
import { toISODate } from '@/lib/time'

/** The open habit of the Life screen. '' = the list. Read by LifeScreen and set by the actions. */
export const lifeView = useViewState('life.open', { habitId: '' })

export const habitEditor = {
  draft: ref<HabitDraft | null>(null),
  create() { habitEditor.draft.value = { name: '', icon: { kind: 'lucide', value: 'dumbbell' }, property: '', cadence: 'daily', times: 1 } },
  edit(habit: Habit) { habitEditor.draft.value = { ...habit, icon: { ...habit.icon } } },
}

async function openNote(date: ISODate, missing: string, ctx: ActionContext): Promise<void> {
  const { opened } = await ctx.api.openDailyNote(date)
  if (opened) await hideAfterSuccess(ctx)
  else ctx.notice.info(missing)
}

/** Opens today's daily note, then hides. Used by Ctrl+T, the command and the habit actions. Never creates a note. */
export function openTodaysNote(ctx: ActionContext): Promise<void> {
  return openNote(toISODate(new Date()), 'No note for today', ctx)
}

/** One tick at a time per habit: a held Enter must not send a second request while one is on its way. */
const pending = new Set<string>()
async function setDone(ctx: ActionContext, habit: Habit, date: ISODate, done: boolean) {
  const key = `${habit.id}:${date}`
  if (pending.has(key)) return
  pending.add(key)
  try {
    await ctx.api.setHabitDone(habit.id, date, done)
    refreshResources('habits', 'habitDays', 'habitStats')
  } finally {
    pending.delete(key)
  }
}

/** The editor dialog is mounted by the Life screen, so go there first. */
function editHabit(habit: Habit, ctx: ActionContext) {
  if (ctx.nav.displayed.value !== 'life') ctx.nav.go('life')
  habitEditor.edit(habit)
}

const reread: RainAction = {            // shared by both providers
  id: 'reread', label: 'Re-read vault', icon: 'refresh-cw', slot: 'more', keys: 'Ctrl+KeyR',
  async run({ api, notice }) {
    await api.rereadVault()
    refreshResources('vault', 'habits', 'habitDays', 'habitStats')
    notice.info('Reading the vault again')
  },
}

export const habitActions: ActionProvider<TargetOf<'habit'>> = ({ item: summary }) => [
  { id: 'tick', label: summary.todayDone ? 'Untick today' : 'Tick today', icon: summary.todayDone ? 'square' : 'square-check', slot: 'primary',
    run: (ctx) => setDone(ctx, summary.habit, summary.today, !summary.todayDone) },
  { id: 'open-habit', label: 'Open habit', icon: 'chart-column', slot: 'secondary',
    run() { lifeView.habitId = summary.habit.id } },
  { id: 'edit-habit', label: 'Edit habit', icon: 'pencil', slot: 'more', run: (ctx) => editHabit(summary.habit, ctx) },
  { id: 'open-today', label: "Open today's note", icon: 'notebook-pen', slot: 'more', run: openTodaysNote },
  reread,
  { id: 'delete-habit', label: 'Delete habit', icon: 'trash-2', slot: 'more', danger: true,
    async run({ api, confirm, notice }) {
      const confirmed = await confirm({ title: `Delete ${summary.habit.name}?`,
        body: `The habit is removed from Rain. Your notes keep their '${summary.habit.property}' property.`,
        confirmLabel: 'Delete habit', danger: true })
      if (!confirmed) return
      await api.deleteHabit(summary.habit.id)
      if (lifeView.habitId === summary.habit.id) lifeView.habitId = ''
      refreshResources('habits', 'habitDays', 'habitStats')
      notice.success(`Deleted ${summary.habit.name}`)
    } },
]

export const habitDayActions: ActionProvider<TargetOf<'habitDay'>> = ({ item: day }) => [
  { id: 'tick', label: day.done ? 'Untick this day' : 'Tick this day', icon: day.done ? 'square' : 'square-check', slot: 'primary',
    async run(ctx) {
      if (!day.tickable) ctx.notice.info('That day is too old for Rain to change. Open its note instead.')
      else await setDone(ctx, day.habit, day.date, !day.done)
    } },
  { id: 'open-note', label: "Open that day's note", icon: 'notebook-pen', slot: 'secondary',
    async run(ctx) {
      if (!day.noteExists) ctx.notice.info('No note for this day')
      else await openNote(day.date, 'No note for this day', ctx)
    } },
  { id: 'open-today', label: "Open today's note", icon: 'calendar-check', slot: 'more', run: openTodaysNote },
  { id: 'edit-habit', label: 'Edit habit', icon: 'pencil', slot: 'more', run: (ctx) => editHabit(day.habit, ctx) },
  reread,
]
```

(`RainAction` is imported from `@/actions/types`; the sketch omits it for brevity.)

`features/life/types.ts`: `export interface HabitDayItem { habit: Habit; date: ISODate; done: boolean; noteExists: boolean; tickable: boolean }`.

Changes in shared files:

| File and place | Change |
|---|---|
| `actions/types.ts` line 15 (`HabitDay` import), line 36 (`'day'` in `ItemType`), line 60 (`{ type: 'day'; ... }`) | import `HabitSummary` and `HabitDayItem`; `ItemType` swaps `'day'` for `'habit' \| 'habitDay'`; `ActionTarget` swaps the `day` line for `{ type: 'habit'; id: ID; item: HabitSummary }` and `{ type: 'habitDay'; id: ID; item: HabitDayItem }` |
| `actions/registry.ts` line 7 and line 23 | import `habitActions, habitDayActions`; `habit: habitActions, habitDay: habitDayActions` replace `day: dayActions` |
| `actions/commands.ts` line 31 | `life: ['habits', 'streak', 'vault', 'daily note']` (drop `calendar`, `agenda`) |
| `actions/commands.ts` after the `new-pool` command (line 91 to 99) | add `{ id: 'new-habit', title: 'New habit', icon: 'square-check', keywords: ['habit', 'life', 'create', 'checkbox'], run({ nav }) { nav.go('life'); habitEditor.create() } }`; add `import { habitEditor, openTodaysNote } from '@/features/life/actions'` (line 8) |
| `composables/useScreen.ts` lines 12 to 27 | add to `ScreenController`: `/** Esc: leave a sub-view of the screen. Returns false when there is none. */ back?(): boolean` |
| `keys/keymap.ts` lines 126 to 130 (`escape`) | `function escape({ route, search, screen }: KeyContext) { if (screen?.back?.()) return; if (search.query.value !== '') search.clear(); else if (route.name === 'gameMode') void cancelGameMode(); else void shell.hide() }` and update its comment (the ladder is now: overlay, sub-view, query, hide) |
| `keys/keymap.ts` line 159 (the `back` binding, `label: 'Back: overlay, query, hide'`) | `label: 'Back: overlay, open habit, query, hide'` (id and combo unchanged) |
| `keys/keymap.ts` lines 287 to 297 (`new`) | `label: 'New pool (Apps), custom game (Games), habit (Life)'`; `when` adds `\|\| nav.displayed.value === 'life'`; `run`: `else if (nav.displayed.value === 'life') habitEditor.create()`; add `habitEditor` to the import on line 15 |
| `keys/keymap.ts` line 233 to 242 (`step-out`) | no change: Life's `stepOut` is what closes the full view; update the `label` to `'Step out (empty query): folder up, close habit, else remove the chip'` |
| `keys/keymap.ts` line 219 and 223 (`page` label) | `'Page up / down; year in a habit'` |
| `keys/keymap.ts` lines 225 to 232 (`group` label) | `'Previous / next group (Life: Daily, Weekly, Monthly)'` |
| `keys/keymap.ts` lines 314 to 328 (`today-note`) | no change |
| `nav/routes.ts` lines 96 to 107 | `path: '/life'`, delete `subs` and `defaultSub`; `routeOfPath('/life/month')` from an old browser hash falls back to `{ name: 'life' }` (no code change needed: `routeOfPath` returns `{ name }` when the route has no subs) |
| `features/home/TodayPanel.vue` | 9.7 |
| `features/settings/*` | 9.8 |
| `data/api.ts` | 9.9 |

Key map for the Life screen after this change (`ui-spec` 4.3 text in section 9.10):

| Where | Key | Action |
|---|---|---|
| list | Up / Down | move the selection (PageUp/PageDown a page, Ctrl+Up/Down the previous or next group) |
| list | Enter | tick or untick TODAY |
| list | Ctrl+Enter | open the habit's full view |
| list | Ctrl+K | action panel: Edit habit, Open today's note, Re-read vault, Delete habit (confirm) |
| list | Ctrl+N | new habit (the dialog) |
| list | Ctrl+R | re-read the vault |
| list | mouse: click the checkbox | tick or untick today |
| list | mouse: click the row | open the full view |
| list | mouse: right click | the action panel |
| full view | Left / Right | one week back / forward |
| full view | Up / Down | one day back / forward |
| full view | PageUp / PageDown | one year back / forward |
| full view | Enter | tick or untick the selected day (only the last 31 days) |
| full view | Ctrl+Enter | open that day's note |
| full view | Esc, Backspace on an empty query | back to the list |
| anywhere | Ctrl+T | open today's note, then hide |

### 9.6 The stages inside stage 7 (the app builds and runs after each)

| Step | Backend | Frontend | Router | The app meanwhile |
|---|---|---|---|---|
| 7a read side and the write function | `rain-types/life.rs` and `decls!`; the `life` module: `dates`, `moment`, `daily`, `index`, `stats`, `store` (habits.toml, `life_note`), `frontmatter` and `vault_write` WITH all copied tests plus the new ones (no route calls the write yet); routes `GET /vault`, `/vault/reread`, `/vault/properties`, `GET/POST/PUT/DELETE /habits`, `/habits/{id}/stats`, `/habit-days`, `/calendar/events`, `/daily-notes/{date}/open`; topic `habits` from CRUD and watcher; `source_rules` additions | none | none (all `mock`) | unchanged; the backend is tested with the harness |
| 7b the tick | `PUT /habits/{id}/days/{date}` with `Service::tick`, the audit table, sandbox rule W1; integration tests with the fixtures | none | none | unchanged |
| 7c the screen | none | everything in 9.2 to 9.10; `types.ts` barrel loses the life types; mock life slice and seed deleted (9.9) | the life methods and the `habits` topic flip to `real` | Life, Home and Settings run on the real backend; the rest on the mock |
| 7d acceptance | none | none | none | section 11 on the real PC, first on a copy of the vault in a debug build, then in a release build on the real vault by the owner |

Reason for this order: the dangerous part (the write) is built and tested alone first; the screen is rewritten once, against a backend that already works.

### 9.7 Home: `features/home/TodayPanel.vue`

The panel keeps its habit tiles and the "Today's note" button, loses the agenda. It stays read-only (the vault is written only on the Life screen, where the effect is visible). Full replacement of the script and the middle of the template:

```vue
<script setup lang="ts">
// The read-only side of Home: today's date, one tile per habit and the "Today's note" button.
// Ticking happens on the Life screen, never here. It is not part of the keyboard cursor.
import { NotebookPenIcon } from '@lucide/vue'
import { computed } from 'vue'
import { commands } from '@/actions/commands'
import { runPrimary } from '@/actions/context'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useResource } from '@/composables/useResource'
import { api } from '@/data'
import { keepFocus } from '@/lib/focus'
import { formatDayLabel, toISODate } from '@/lib/time'
import { useNav } from '@/nav/useNav'

const MAX_HABITS = 7
const nav = useNav()
const from = toISODate(new Date())

// The same resource and topic as the Life screen: one fetch feeds both.
const habits = useResource('habits', () => api.listHabits(), { live: 'habits' })
const shown = computed(() => (habits.data.value ?? []).slice(0, MAX_HABITS))
const doneCount = computed(() => shown.value.filter((item) => item.todayDone).length)
const todayNote = commands.find((command) => command.id === 'today-note')
function openTodayNote() {
  if (todayNote) runPrimary({ type: 'command', id: todayNote.id, item: todayNote })
}
</script>
```

Template: header block and first `Separator` unchanged (lines 80 to 85); the habit block (lines 87 to 126) becomes:

- `habits.error.value` -> `ErrorRow message="Habits unavailable" @retry="habits.reload()"`;
- `habits.showSkeleton.value` -> the seven `Skeleton` squares;
- `habits.data.value && shown.length === 0` -> the link button, text "No habits yet. Add one in Life", `@click="nav.go('life')"`;
- else the tiles: for each item a `Tooltip` + `button` of 28 px: done `bg-heat-3 text-foreground`, not done `bg-surface-2 text-muted-foreground`, `aria-label` and tooltip `${item.habit.name}: ${item.todayDone ? 'done' : 'not yet'}`, `@click="nav.go('life')"`, icon `RIcon`. The `MinusIcon` skip branch (line 120) is deleted. Under the tiles one line `text-xs text-muted-foreground`: "{doneCount} of {shown.length} done today" (only when `shown.length > 0`).

Delete: the second `Separator` and the whole events block (lines 128 to 165), the imports `MinusIcon`, `CalendarEvent`, `Habit`, `HabitValue`, `formatClock`, `useNow`, the constants `MAX_EVENTS`, `DAY_MS`, the `to`, `events`, `days`, `nextEvents`, `isDone`, `valueText`, `isCurrent`, `habitsFailed`, `habitsLoading`, `reloadHabits` and the `settings` resource (it was only used for the calendar line, line 139). The button at the bottom (lines 167 to 178) stays; its `mt-auto` keeps it at the bottom.

Home shows no events. Nothing else on Home reads life data (`homeRows.ts` only has the `job` icon `calendar-clock`; checked by search).

### 9.8 Settings

Section 06 owns `settingDefs.ts`, `settingValues.ts`, `SettingRow.vue`, `SettingsScreen.vue` and `settings/actions.ts`, with the final list in its section 9.2. This section only states which rows it needs: the vault row, `note-format` and `note-folder` (text rows for `dailyNoteFormat` and `dailyNoteFolder`) and the calendar info row (label "Calendar", hint "Coming later", value "Not connected", no "Mock data" badge). All of them, with their hints, are in 06 section 9.2. The vault glyph of Settings and the state of Life agree because both use `Resolved.vault` (section 06).

After a change of `vaultPath`, `dailyNoteFolder` or `dailyNoteFormat`, `SettingsScreen.save` also refreshes `vault`, `habits`, `habitDays` and `habitStats`, so Life never shows data of the old vault. The code is in 06 section 9.2 (the `save` function, stage 7). The backend does the same on its side (6.2 settings subscription).

### 9.9 Contract, router, mock

Contract-change table (foundation 15.1 template):

| # | Item | From | To | Reason | Frontend files touched | Breaking for the mock? | Stage |
|---|---|---|---|---|---|---|---|
| S.1 | `Habit` | `{ id, name, icon, property, kind, target?, unit? }` | `{ id, name, icon, property, cadence, times }` | no number habits; cadence | `data/types.ts` (type moves to the generated file); `features/life/*`; `features/home/TodayPanel.vue` | yes: the mock life slice is deleted in the same stage | 7c |
| S.2 | `HabitValue` | exported union | deleted | no skip, no number, no null | `data/types.ts`; `features/life/DayPanel.vue` (deleted), `features/life/heat.ts` (deleted), `features/home/TodayPanel.vue` | yes (deleted with the slice) | 7c |
| S.3 | `HabitDay` | `{ date, noteExists, values }` | `{ date, done: ID[] }` | compact; absent row = no note | `data/types.ts`; `actions/types.ts`; `features/life/*` | yes | 7c |
| S.4 | `VaultStatus` | `{ connected, path?, missingProperties }` | section 3 | `missingProperties` is meaningless once ticking exists; Life needs folder, format, writable, suggestion | `data/types.ts`; `features/life/LifeScreen.vue`; `features/settings/SettingRow.vue` | yes | 7c |
| S.5 | `listHabits` | `Habit[]` | `HabitSummary[]` | the list row needs today and a compact history | `data/api.ts`; `features/life/LifeScreen.vue`; `features/home/TodayPanel.vue` | yes | 7c |
| S.6 | `listHabitDays` | `(from, to)` | `(from, to, habitId?)`, only existing notes | the full view loads one habit's year | `data/api.ts`; `features/life/HabitView.vue` | yes | 7c |
| S.7 | new methods | none | `rereadVault`, `listHabitSuggestions`, `saveHabit`, `deleteHabit`, `getHabitStats`, `setHabitDone` | habit CRUD, full view, the tick, Ctrl+R | `data/api.ts`; `data/rest/client.ts`; `data/routing.ts` | yes (see below) | 7c |
| S.8 | topic | six topics (and `health`) | adds `habits: HabitSummary[]` | live update after a tick and after an edit in Obsidian | `data/api.ts` (`TopicPayload`); `data/rest/topic-check.ts` (compile check; `habits` is added to `Topic` and to `TOPIC_NAMES`, not to `UnportedTopic`, foundation 9.3); `data/routing.ts` (`topics.habits`); `data/mock/engine.ts` (`listeners`) | yes: `mock/engine.ts` declares `listeners: { [T in Topic]: Set<...> }`, so the new key of `TopicPayload` needs the entry `habits: new Set(),` in stage 7 (with the comment "The mock never publishes this; the router sends it to the backend."), or `vue-tsc` fails | 7c |
| S.9 | `Settings` | `{ vaultPath, dailyNoteFormat, calendarConnected, ... }` | adds `dailyNoteFolder: string`; `dailyNoteFormat` default is `''` (use Obsidian's); `calendarConnected` is read only and always `false` (a patch that sets it is `invalid`) | override of Obsidian's folder; calendar has no connect flow | `data/types.ts` (generated by section 06); `features/settings/*` (9.8) | no (the mock keeps its own settings) | 1b (shape, section 06), 7c (fields) |
| S.10 | `ActionTarget` / `ItemType` | `'day'` | `'habit'`, `'habitDay'` | two different things are selected now | `actions/types.ts`, `actions/registry.ts`, `features/life/actions.ts` | no | 7c |
| S.11 | `ScreenController` | no sub-view back | adds `back?(): boolean` | Esc leaves the full view | `composables/useScreen.ts`, `keys/keymap.ts` (`escape`) | no | 7c |
| S.12 | route `/life/:sub` | sub-views `month`, `year` | `/life`, no sub-views | no month or year view any more | `nav/routes.ts` | no | 7c |
| S.13 | `listEvents` route | `/events` | `/calendar/events` (the registry's rename), returns `[]` | the registry; no calendar this round | `data/api.ts` (comment); the two callers are deleted | no | 7c |
| S.14 | `openDailyNote` | `Promise<{ opened: boolean }>` | `Promise<OpenNoteResult>` | named type | `data/api.ts` | no | 7c |

`data/api.ts` (lines 6 to 9, 70 to 74): imports lose `Habit`, `HabitDay`; add `HabitDraft, HabitStats, HabitSummary, HabitDay, Habit, OpenNoteResult, PropertySuggestion`. `TopicPayload` gets `habits: HabitSummary[]`. The life block becomes:

```ts
  // life
  getVaultStatus(): Promise<VaultStatus>                                       // GET    /vault
  rereadVault(): Promise<void>                                                 // POST   /vault/reread
  listHabitSuggestions(): Promise<PropertySuggestion[]>                        // GET    /vault/properties
  listHabits(): Promise<HabitSummary[]>                                        // GET    /habits
  saveHabit(draft: HabitDraft): Promise<Habit>                                 // POST   /habits | PUT /habits/:id
  deleteHabit(id: ID): Promise<void>                                           // DELETE /habits/:id
  getHabitStats(id: ID): Promise<HabitStats>                                   // GET    /habits/:id/stats
  listHabitDays(from: ISODate, to: ISODate, habitId?: ID): Promise<HabitDay[]> // GET    /habit-days?from=&to=&habit=
  setHabitDone(id: ID, date: ISODate, done: boolean): Promise<HabitSummary>    // PUT    /habits/:id/days/:date   (body { done })
  listEvents(from: ISODate, to: ISODate): Promise<CalendarEvent[]>             // GET    /calendar/events?from=&to=   (always [] this round)
  openDailyNote(date: ISODate): Promise<OpenNoteResult>                        // POST   /daily-notes/:date/open
```

`data/rest/client.ts` (one line each, foundation 8.3): `getVaultStatus: () => call('GET', '/vault')`, `rereadVault: () => call('POST', '/vault/reread')`, `listHabitSuggestions: () => call('GET', '/vault/properties')`, `listHabits: () => call('GET', '/habits')`, `saveHabit: (d) => d.id ? call('PUT', `/habits/${enc(d.id)}`, d) : call('POST', '/habits', d)`, `deleteHabit: (id) => call('DELETE', `/habits/${enc(id)}`)`, `getHabitStats: (id) => call('GET', `/habits/${enc(id)}/stats`)`, `listHabitDays: (from, to, habit) => call('GET', `/habit-days${qs({ from, to, habit })}`)`, `setHabitDone: (id, date, done) => call('PUT', `/habits/${enc(id)}/days/${date}`, { done })`, `listEvents: (from, to) => call('GET', `/calendar/events${qs({ from, to })}`)`, `openDailyNote: (date) => call('POST', `/daily-notes/${date}/open`)`. `call` runs `resolveMedia`; no habit field is a media reference, so nothing changes there.

`data/routing.ts`: the eleven life methods and `topics.habits` change from `'mock'` to `'real'` in step 7c. A missing entry is a compile error, as designed.

Mock (foundation 14.3 row 7 deletes `mock/slices/life.ts` and `mock/seed/life.ts`):

| File | Change |
|---|---|
| `data/mock/slices/life.ts` | deleted |
| `data/mock/seed/life.ts` | deleted (it exports `VAULT_PATH` used by `seed/settings.ts` line 7 and 15) |
| `data/mock/seed/settings.ts` lines 7, 15, 35 | the `VAULT_PATH` import goes; `vaultPath: ''`; the `empty` scenario keeps `vaultPath: ''` |
| `data/mock/store.ts` lines 5, 15, 44 to 48 | remove `Habit`, `HabitDay`, `CalendarEvent` imports, the `seedLife` import and the fields `vaultPath`, `missingProperties`, `habits`, `habitDays`, `events` of the state and of its reseed |
| `data/mock/control.ts` line 64 | `pathExists` no longer special-cases `state.vaultPath`; the function itself is deleted in stage 3 (`checkPaths`) |
| `data/mock/index.ts` lines 13 and 26 | remove the `lifeSlice` import and its spread |
| `data/mock/engine.ts` lines 143 to 152 (`listeners`) | add `habits: new Set(),` after the entries of `settings` and `health`, with the comment "The mock never publishes this; the router sends it to the backend." (the same as sections 02 and 06 do for their topics) |

Because that removes five `RainApi` methods from `createMockApi()`, and six more methods are new in this stage and were never in the mock, its return type no longer satisfies `RainApi`. The same commit adds the names of all eleven life methods to `RetiredMockMethod` of `MockApi`: `getVaultStatus`, `listHabits`, `listHabitDays`, `listEvents`, `openDailyNote`, `rereadVault`, `listHabitSuggestions`, `saveHabit`, `deleteHabit`, `getHabitStats`, `setHabitDone` (foundation 8.6; the type already exists since stage 2 and section 05 extended it in stage 3; the router never calls a retired method because `routing` says `'real'`). The same commit keeps the mock adaptation of foundation 8.5/8.6: `createMockApi(): MockApi`, and the all-mock branch of `src/data/index.ts` reads `createMockApi() as RainApi` ("retired methods are undefined here; their screens show an error row; stage 10 deletes the branch"). Consequence, said plainly: with `VITE_RAIN_MOCK=1` (`npm run dev:ui`) the Life screen shows its error row after stage 7 (nothing answers); the flag is deleted in stage 10.

Files whose `Habit...` imports change: `actions/types.ts`, `data/api.ts`, `data/types.ts`. Found by `grep -rn "Habit\|HabitDay\|HabitValue\|VaultStatus\|CalendarEvent\|listEvents" src`; the only other hits are inside the files deleted or rewritten above.

### 9.10 `docs/ui-spec.md`: replacement text (the lead applies it in stage 7)

Do not edit the spec from this section; the lead applies these. Each block says which part it replaces.

**Replaces section 7.6 (lines 869 to 888) in full:**

> ### 7.6 Life (`features/life/LifeScreen.vue`)
>
> Layout: `ScreenFrame pane="none"`, no strip. Rain reads the vault and writes exactly one thing: when you tick a habit, it sets that one property in that day's daily note and changes nothing else. There is no calendar and no agenda on this screen.
>
> - **List.** `ItemList` with three groups, "Daily", "Weekly", "Monthly" (group header with a count; empty groups are not drawn). One row per habit, 40 px: habit icon 20, name 14 px, for weekly and monthly "2 / 3" (13 px; `--foreground` when the target is reached, else `--muted-foreground`), the heat strip, today's `Checkbox`. The strips are right aligned so the checkboxes form one column.
> - **Heat strip.** Daily: the last 70 days, one 10 px cell per day, gap 2, today last with a 1 px `--foreground` outline. Weekly: the last 12 weeks, one 14 px cell per week (Monday to Sunday), gap 3. Monthly: the last 12 months, one 14 px cell per month. Cells (radius 3): done `--heat-3`; partial (some ticks, fewer than the target; weekly and monthly only) `--heat-1`; miss `--heat-0`; no note that day (daily only) transparent with a 1 px `--selected` border. The current period has the 1 px outline. Cells shrink with the window.
> - **Keys.** Enter ticks or unticks today. Ctrl+Enter opens the habit's full view. Ctrl+K: Edit habit, Open today's note, Re-read vault, Delete habit (confirm). Ctrl+N: new habit. Ctrl+R: re-read the vault. Typing with the chip filters habits by name. Left and Right belong to the caret. Mouse: the checkbox ticks, a click on the row opens the full view, a right click opens the action panel.
> - **Full view (a sub-view of Life, not a route).** Header: back button (`arrow-left`, tooltip "Back" + `Esc`), icon, name, "Weekly, 3 times", today's `Checkbox`, edit button (`pencil`). Under it a row of figures (label 12 px `--text-3`, value 20 px): Streak, Best (in days, weeks or months), Rate, This week, This month, This year. Then the year grid for one habit: 53 week columns x 7 rows, 12 px cells, gap 3, month labels above, M / W / F on the left; done `--heat-3`, not done `--heat-0`, no note hollow; future days are not drawn; days older than 31 days at 50% opacity; today has the 1 px outline and the selected day the 2 px `--ring` outline. Beside the year label two chevron buttons change the year. Under the grid one read-out line: "Sun 22 Nov: done", "not done" or "no note".
> - **Full view keys.** Left/Right: one week. Up/Down: one day. PageUp/PageDown: one year. Enter: tick or untick the selected day, only within the last 31 days (an older day shows the notice "That day is too old for Rain to change. Open its note instead."). Ctrl+Enter: open that day's note. Esc or Backspace on an empty query: back to the list; the list keeps its selection.
> - **New / edit habit dialog.** Name, Property (the checkbox property in the daily note; filled from the name; chips "gym - 6 days" for checkbox properties already found in the notes), Icon (a grid of 28), How often (Daily, Weekly, Monthly), Times (weekly and monthly only: "times a week" / "times a month"). Cancel, and Save + `KeyHint` Ctrl Enter. Same dialog rules as the pool editor.
> - **States.** Vault not set: `EmptyState` `calendar-check` "Choose your vault", button "Open settings". Vault path wrong: "Vault not found", "Nothing at {path}. Check the path in Settings." No habits: "No habits yet", "A habit is a checkbox property at the top of your daily note.", button "New habit" + `KeyHint` Ctrl N. A problem with the daily notes (folder or format): one muted line above the list with the sentence and "Open settings". Ticking switched off (development build): one muted line "Ticking is switched off in this development build." and disabled checkboxes. A tick that fails shows an error notice with the reason ("The property 'gym' in that note is not a checkbox, so Rain left the note alone.").
> - **Writing rules (for the reader of the spec).** A tick sets `property: true` in the note of that day. If the property exists the `true`/`false` word is replaced and nothing else changes. If it is missing one line is added at the end of the properties. If the note has no properties, a block is added at the top. If the note does not exist it is created and holds only that property (the Obsidian template is not applied). Unticking writes `false` when the property exists and does nothing otherwise. Rain never creates a folder, never changes a property that is not true/false, and refuses instead of guessing.

**Replaces the lock icon and its tooltip:** the lock icon (`lock`, "Rain reads your vault and never writes to it") is deleted. There is no longer a strip on this screen. No replacement tooltip: the writing rule is stated in Settings' hint for the vault row instead (the text is in section 06 9.2, which owns `settingDefs.ts`).

**Replaces 2.3 (Escape), items 1 to 3 (lines 97 to 101):**

> 1. Close the open overlay (action panel, dialog, shortcut sheet, select), or leave a focused Settings field.
> 2. Leave a sub-view of the screen (Life: the open habit goes back to the list).
> 3. Clear a non-empty query.
> 4. Hide the window.

**Replaces rule 10 of section 2 (line 27) and rule 8 of section 4.1 (line 263):**

> 10. **Esc ladder is four steps:** close overlay, leave a sub-view, clear query, hide. Shift+Esc hides at once. Ctrl+1 goes Home.
> 8. **Esc goes back, four steps at most.** (The sub-view step exists only on Life.)

**Replaces in 4.2 (lines 272, 283, 284, 285 and 291):**

| Group | Key | Action | Applies when |
|---|---|---|---|
| Window | Esc | Back: overlay, open habit, query, hide | Always |
| Move | PageUp / PageDown | Page up / down; a year in the open habit | Always |
| Move | Ctrl+Up / Ctrl+Down | Previous / next group (Life: Daily, Weekly, Monthly) | Grouped screens |
| Move | Backspace (empty query) | Step out: Disk folder up, close the open habit, else remove chip | Always |
| Act | Ctrl+N | New: pool (Apps), custom game (Games), habit (Life) | Apps, Games, Life |

**Replaces the `Day cell` row of 4.3 (line 311) by two rows:**

| Item | Where | Enter | Ctrl+Enter | More in Ctrl+K |
|---|---|---|---|---|
| Habit | Life list | Tick / untick today | Open habit | Edit habit, Open today's note, Re-read vault, Delete habit (confirm) |
| Habit day | Life, open habit | Tick / untick that day (last 31 days) | Open that day's note | Open today's note, Edit habit, Re-read vault |

**Replaces the Life row of the route table (line 188):** `| /life | life | Life | calendar-check | Ctrl+6 | top | none |`. Section 9 item 9 (line 26): "One fixed detail pane in Apps, Projects, Games and Workflows." (Life no longer has one.)

**Replaces TodayPanel items 2 and 3 (lines 781 to 783):**

> 2. Habits: one row of 28x28 tiles, one per habit (max 7). Done: icon `--foreground` on `--heat-3`. Not yet: icon `--muted-foreground` on `--surface-2`. Tooltip: name and "done" or "not yet". Click goes to Life. Under the tiles "3 of 5 done today" (12 px muted). No habits: "No habits yet. Add one in Life", linking to Life. Home never ticks a habit.
> 3. Bottom: `Button variant="outline"` "Today's note" with `notebook-pen` and `KeyHint` Ctrl T.

(these replace items 2, 3 and 4 of the list; in the sentence above it (line 779) "three blocks" becomes "two blocks"; the decision row at line 47 keeps its history, "habits and agenda cost one press" becomes "habits cost one press".)

**Settings rows (ui-spec 7.9):** not replaced here. Section 06 9.2 owns the table and its hints (the vault row, the two daily note rows and the row "Calendar"); this section gives no text for them.

**Replaces in 6.2 (line 682):** resource keys `habits`, `habitDays:open`, `habitStats:open`, `vault`, and no `events:<from>:<to>`. **Replaces the `day` line of the `ActionTarget` type in 6.4 (line 713):** `| { type: 'habit'; id: ID; item: HabitSummary } | { type: 'habitDay'; id: ID; item: HabitDayItem }`. **Replaces the contract blocks of the appendix (lines 1053 to 1057 and 1143 to 1149)** by the blocks of sections 9.9 and 3 above. **Replaces the "Not in the prototype" line 1310** ("Writing to the vault or the calendar in any form."): "Writing to the vault except the one property a habit tick sets; writing to the calendar in any form."

---------------------------------------------------------------------------------------------------

## 10. Tests

### 10.1 Unit tests (next to the code)

| Module | What is proven |
|---|---|
| `frontmatter.rs` | ALL spike tests copied unchanged: 38 exact outputs, 12 refusals plus 10 bad key names, reader cases, `new_note`, the 20 000-note randomised invariant test on 6 seeds with `yaml-rust2` as oracle, the 4 mutation checks. NEW: `set_bool_existing` returns the note unchanged for no frontmatter and for a missing key and edits an existing key like `set_bool` (8 cases incl. CRLF, BOM, trailing comment); `read_bools` (12 cases: empty, no frontmatter, unterminated fence, duplicates excluded, `yes` and strings excluded, quoted keys, `True`, nested keys ignored, body lines ignored, CRLF, BOM, implausible block returns empty); a randomised check that `read_bools` agrees with the `yaml-rust2` oracle on the same 20 000 notes. |
| `vault_write.rs` | on a temp folder: create (exact bytes `---\ngym: true\n---\n`), update, unchanged (file time unchanged), untick of a missing note creates nothing, untick of a missing key writes nothing, missing folder gives `NoFolder` and creates no folder, bad UTF-8 gives `NotUtf8` and the file is byte equal, each refusal leaves the file byte equal and leaves no temp file, stale detection (a file changed between read and check: a fake hook changes it, the second round edits the new text), 3 stale rounds give `Busy`, the temp file never remains, a read-only temp for locked rename (an exclusive handle on the target, share mode without delete: `Busy` after about 0.2 s, target unchanged, temp removed; released inside the retry window: succeeds). |
| `moment.rs` | every vector of the table in 6.3 from `moment-vectors.txt`; every unsupported letter; an unclosed bracket; ordinal suffixes 1 to 31 |
| `dates.rs` | leap years, month ends, `add_days` over a year change, `monday_of`, ISO week 53 and 1, `add_months` (31 Jan + 1 month = 28/29 Feb), parse refuses 2026-02-30 |
| `daily.rs` | `normalise` (`\`, leading and trailing slashes, `.`, rejects `..`, `C:`, control characters); nested format paths; percent-encoding of the link (spaces, `&`, `#`, non-ASCII, `/`) |
| `stats.rs` | the worked examples of 6.5 and 6.6 (`life-basic`); daily, weekly, monthly cells; period boundaries on Monday and on the 1st; streak with an open current period; best streak; rate with the current period excluded and included; no `since`; a window start in the middle of a week |
| `store.rs` | `habits.toml`: load defaults, per-entry skips with issues (duplicate id, duplicate property, bad cadence, bad times, 51st habit), create keeps comments and CRLF, edit keeps a trailing comment on a value, delete removes the comment block above, newer schema is read only, a syntax error keeps the last good list |
| `index.rs` | (with a `VaultFs` fake) unchanged `(mtime, size)` is not re-read; a changed file is re-read once; a deleted file disappears; an unreadable file keeps the old row; rows of another scope and of old dates are dropped; the `life_note` rows equal the index after a check |

### 10.2 Traits and fakes

| Trait | Production | Fake | Used for |
|---|---|---|---|
| `VaultFs` (read side only: `metadata`, `read_head`) | `std::fs` | in-memory map with settable mtimes, sizes, content and errors | `index.rs`, `daily.rs` unit tests |
| `settings().vault_id()` (the shared reader of section 06; the fake in these tests returns fixed ids) | reads `%APPDATA%\obsidian\obsidian.json` | fixed values | open note |
| `Opener` (foundation 11.6; no trait of this section) | `platform::shell::shell_execute` | the one `FakeOpener` of section 01 records the `OpenRequest`s (the `obsidian://` links); can fail | `openDailyNote` |
| `Clock` | foundation | fixed time | today |

The write function uses the real file system (temp folders); it is the thing under test and has no fake.

### 10.3 Fixtures (`tests/fixtures/life-basic/data/`)

Invented data only. The harness replaces `{DATA}`.

```
settings.toml          vault_path = "{DATA}\\vault"      (daily_note_folder and format absent)
habits.toml            gym (weekly, 3), read (daily), review (monthly, 2)
vault\.obsidian\daily-notes.json    { "folder": "Daily" }
vault\Daily\2026-11-09.md   ---\ngym: true\nread: false\n---\n Monday notes
              2026-11-11.md   gym: true, read: true
              2026-11-12.md   no frontmatter, one paragraph
              2026-11-13.md   empty file (0 bytes)
              2026-11-15.md   gym: false, read: true
              2026-11-16.md   gym: true, read: false
              2026-11-17.md   read: true
              2026-11-18.md   gym: true, read: true
              2026-11-19.md   read: true
              2026-11-20.md   gym: true, read: true
              2026-11-21.md   read: true
              2026-11-22.md   read: true
              (no 2026-11-10.md and no 2026-11-14.md)
              2026-11-03 1.md  a duplicate-name copy that must be ignored
              notes.txt        a stray file that must be ignored
```

Fixed clock: 2026-11-22 10:00 local (a Sunday). Golden outputs (expected-json files next to the fixture) are the worked examples of 6.5 and 6.6.

More fixtures for the write tests (`tests/fixtures/life-write/`): one file per shape, each also used with the property test: LF note with `gym: false`; CRLF note; BOM note; no frontmatter; empty; `gym: True # comment`; `"gym": false`; `gym: yes`; `gym:` empty; `gym: [a]`; duplicate key; unterminated `---`; a fenced block of prose; a note with a block scalar containing `gym: false`; non-UTF-8 bytes; a note of 1 MiB (the whole file is read for a write); a note whose body has `---` rules. Plus `life-nested` (format `YYYY/MM/YYYY-MM-DD`, folders `Daily/2026/11`) and `life-badformat` (`YYYY-MM-DD HH:mm`).

### 10.4 Integration tests (`tests/life.rs`, harness of foundation 12.2)

For every route: success, `invalid` body or query, `notFound`, idempotency as in the table of section 2.

| Test | Proves |
|---|---|
| `vault_status_*` | connected, folder found, `writable` true for a vault inside the data folder and false outside (the sandbox rule), `problem` for each cause in 6.2, empty settings, several vaults registered and none chosen (fake `settings()` gives `vault = None`: not connected, no path, nothing read or written) |
| `habits_summary_life_basic` | `GET /habits` equals the golden file (cells, counts, today) |
| `habit_stats_life_basic` | `GET /habits/{id}/stats` equals the golden file; unknown id is 404 `unknownHabit` |
| `habit_days_*` | only days with notes, the `habit` filter, `badRange`, the decoys (`2026-11-03 1.md`, `notes.txt`) are never rows |
| `habit_crud` | create (201, id slug, header comment in the file), double create is 409 `duplicateProperty`, edit, delete (204 twice), `tooManyHabits`, hand edit of the file shows within 1 s and the topic arrives, comments survive |
| `tick_updates_one_line` | for each shape in `life-write`: tick, then the note differs from the original in exactly the expected bytes (byte compare), a second tick writes nothing (mtime equal), untick flips to `false`, `habits` frame arrives with the new cells |
| `tick_creates_bare_note` | missing note: file equals `---\nread: true\n---\n`; a second habit ticked the same day adds exactly one line; untick of a missing note creates no file |
| `tick_never_creates_folder` | missing `Daily` folder: 503 `noDailyFolder`, no folder appears |
| `tick_refusals` | `gym: yes`, duplicate key, unreadable block, bad UTF-8: 409 with the right reason, file byte equal |
| `tick_window` | today and today minus 30 accepted; minus 31 `tooOld`; tomorrow `futureDate`; `2026-02-30` `badDate`; a tick for a past day changes only that day's note and never today's |
| `tick_sandbox_refusal` | vault outside the data folder: 409 `sandbox`, nothing written, `writable` false |
| `tick_concurrent` | two ticks of the same note at once are serialised (both succeed, both properties present); 50 parallel ticks of 2 habits end with a valid note |
| `topic_habits` | a frame on tick, on a hand edit of `habits.toml`, on a file change made by the test in the vault (watcher) and on `POST /vault/reread`; `resync` on connect; no frame when nothing changed; at most 4 per second |
| `settings_change` | changing `daily_note_folder` in `settings.toml` re-resolves: the next `GET /habits` reads the other folder; the `life_note` rows of the old scope are gone |
| `open_note` | existing note: the fake opener got exactly `obsidian://open?vault=<id>&file=Daily%2F2026-11-22`; missing note: `opened: false` and the opener was NOT called; no vault: 503; opener fails: 500 `openFailed`; nested format percent-encoding |
| `calendar_empty` | `[]` for any valid range; `badDate` for a bad one |
| `game_mode_cache_only` | with `game_mode_active` set, reads serve the cache and the watcher does not publish |
| `no_note_text_in_logs` | run the whole suite with a log capture; the captured log never contains a line of any fixture note (the fixture notes carry a unique marker string) |
| `source_rules` (in `tests/source_rules.rs`) | the six rules of section 6.1 |

### 10.5 Frontend tests (vitest, foundation 12.3)

| File | Proves |
|---|---|
| `src/features/life/rows.test.ts` | `groupHabits`: group order Daily, Weekly, Monthly; empty groups kept as empty (the list hides them); filter by name; list order kept |
| `src/features/life/yearGrid.test.ts` | `yearCells(2026, today)`: Jan 1 2026 (a Thursday) is column 0 row 3; 2026-11-22 is column 46 row 6; future days absent; month labels at the first column of each month; a leap year (2024) |
| `src/features/life/dates.test.ts` | `addDays`, `addMonths` (clamp to the last day), `daysBetween` across a DST change, `weekdayRow` |
| `src/data/rest/client.test.ts` (additions) | method, path, query and body of each of the eleven life methods against a fake bridge: `PUT /habits/gym/days/2026-11-22` with `{ done: true }`, `saveHabit` picks POST or PUT by `id`, `listHabitDays` leaves out an undefined `habit`, ids are percent-encoded |
| `src/data/router.test.ts` (additions) | every life method and the `habits` topic go to `real` |

Screens and composables are not unit tested (foundation 12.3); the acceptance checks cover them.

### 10.6 What cannot be tested automatically, and how it is checked

| Item | Check by hand |
|---|---|
| Obsidian with the note open and idle: it reloads the changed file | tick in Rain while the same note is open in Obsidian (a copy of the vault in a debug build); the property shows ticked in Obsidian without a restart |
| Obsidian with unsaved edits in the note | type a word in the open note, tick within two seconds, watch the result; record what happened in the stage report (appendix C item) |
| A sync tool and the short-lived dot-prefixed temp file | with `obsidian-git` active on the copy, tick twenty times quickly; the git status shows only the notes |
| `obsidian://open` brings Obsidian to the front from the windowless backend | Ctrl+T with Obsidian in the background (open point 6) |
| A format with a nested folder in real Obsidian | set `YYYY/MM/YYYY-MM-DD` in a scratch vault |

---------------------------------------------------------------------------------------------------

## 11. Acceptance checks

Run steps 1 to 6 in a debug build (a sandbox) on a COPY of the vault inside the data folder; steps 7 and 8 in a release build on the real vault, by the owner.

| # | Action | Expected |
|---|---|---|
| 1 | `npm run check` after each of 7a, 7b, 7c | all eight checks pass; `source_rules` passes; `npm run types:check` is current |
| 2 | Debug build. Copy the vault folder to `<RAIN_DATA_DIR>\vault`. In Settings set the vault to that folder. Open Life. | The list shows the habits of `habits.toml` in the groups Daily, Weekly, Monthly; cells match the notes; no calendar anywhere; Settings shows "Not connected" for Google Calendar with no button. |
| 3 | With no `habits.toml`: open Life. | "No habits yet" and a "New habit" button. Ctrl+N opens the dialog; the "Found in your notes" chips list `gym - N days` from the copy; save a habit `gym` (weekly, 3). The file `habits.toml` appears with the header comment and one `[[habit]]`. |
| 4 | Select the habit, press Enter. Open the note of today in the copy with any editor. | Today's note holds the property `gym: true` and EVERY other byte equals the original (compare with the vault's git: `git -C <copy> diff` shows one added line, or three when the note had no properties; a new bare note shows as a new file `---`, `gym: true`, `---`). The row's checkbox is ticked and the progress reads "1 / 3". Press Enter again: the note now says `gym: false` and nothing else changed. |
| 5 | Ctrl+Enter on the row. Press Left, Right, Up, Down, PageUp, Enter on a day within the last 31 days, then on an older day, Esc. | The full view opens with streak, best, rate and three totals; the cursor moves by week and by day; Enter ticks a past day (a bare note is created if there was none; the git status of the copy shows it); an older day shows the notice "That day is too old for Rain to change. Open its note instead."; Esc returns to the list with the same row selected; a second Esc clears a typed query or hides. |
| 6 | Edit the note in Obsidian (tick `gym` there) while Rain is open on Life. | The row updates within about one second without a click. A fixed sandbox check: in a debug build with the real vault path set, ticking shows the notice "Ticking is switched off in this development build." and the checkboxes are disabled; the real vault is unchanged. |
| 7 | Release build, real vault: open Life, tick `gym`, then in a terminal `git -C <vault> diff` (the user's own command; Rain never runs git in the vault). | exactly one `+gym: true` line (or the three-line block) in today's note; nothing else. Press Ctrl+T: Obsidian shows today's note (record whether it comes to the front: open point 6). |
| 8 | Stop the backend (`npm run backend:stop`) and open Life; restart it and press Retry. Idle: `rain-backend.exe` with no client for 60 s. | Life shows its error row and recovers; idle stays under 30 MB and 0.1 s CPU gained (the `life` module has no timers). |

What "empty" looks like, per state: no vault path: "Choose your vault"; wrong path: "Vault not found"; no habits: "No habits yet"; habits but a daily notes folder that does not exist: the list shows with all cells hollow and the muted line "The daily notes folder Daily was not found in the vault. Check it in Settings."; a filter with no match: "No habit matches".

---------------------------------------------------------------------------------------------------

## 12. Open points

Only what cannot be decided inside this area. Each: question, who decides, what the code does until then, what it blocks.

| # | Question | Who | Until decided | Blocks |
|---|---|---|---|---|
| 1 | SETTLED in foundation 13.3: `set_habit_property` is referenced only in `life/service.rs`, in `Service::tick`, which only the route handler calls. No `plan_tick` / `finish_tick`. | none | as written in 6.1 rule 4 and 6.8 | nothing |
| 2 | Section 06 must add to `Settings` / `settings.toml`: `dailyNoteFolder` (`daily_note_folder`, default empty), make the `dailyNoteFormat` default empty ("use Obsidian's"), keep `calendarConnected` read only and always false, and call `life::Service::check_note_format(&str) -> Result<(), String>` before saving a format (an unsupported token must be `invalid`, reason `badFormat` (400, section 06)). Life reads `settings().resolved()` and `settings().watch()` (06 6.1.7). | section 06 and the architect | life reads the three fields by those names | step 7a |
| 3 | `EventBus::has_receivers() -> bool` (a one-line accessor over `receiver_count()`), so a module can skip computing a snapshot nobody will receive. | architect | the module always computes (cost: one cached summary, under 1 ms) | nothing |
| 4 | SETTLED in foundation 11.1: `Health::set_file_issues(file, issues)` exists. | none | used | nothing |
| 5 | The Esc ladder becomes four steps with a "leave a sub-view" step (`ScreenController.back`). ui-spec 2.3, 2.10 rule, 4.1 rule 8 change (text in 9.10). The user's own words ("Esc goes back", decisions.md section 4) settle the behaviour; only the ui-spec text change stays with the architect. | architect | built as written | step 7c |
| 6 | Does `obsidian://open` started by the windowless backend bring Obsidian to the front? Windows may only flash its taskbar button because the foreground right belongs to the shell process. If it does not work: the shell grants the right first (`AllowSetForegroundWindow(ASFW_ANY)` before the call, one shell-local path such as `POST /local/allow-foreground`, outside the `api_request` allow-list change). | architect, after the check in acceptance step 7 | the plain `ShellExecuteW` from the backend | nothing else |
| 7 | SETTLED in foundation 10.4: `ConfigHub::watch_dir(path, recursive, quiet)`; this section builds no debouncer of its own. | none | used | nothing |
| 8 | SETTLED in foundation 8.6: `MockApi = Omit<RainApi, RetiredMockMethod>`, created in stage 2 (`getHealth`) and extended by each stage; stage 7 adds the eleven life methods (9.9). | none | as 9.9 | nothing |
| 9 | SETTLED (foundation 11.6): there is no `NoteOpener`. `openDailyNote` uses the one `Opener` of `core/launch_contract.rs` (`state.opener()`) with `OpenRequest::ShellExec`, and the tests use the one `FakeOpener` of section 01 | settled | the one `Opener` | nothing |
| 10 | Registry and Appendix A: the six new methods are in 14.4 (done); add the constants of 6.12 to Appendix A. | architect | this file is the source | the REST client of 7c |
| 11 | The week starts on Monday and is not configurable. A user who wants Sunday needs a Settings field. | the user | Monday, fixed | nothing this round |
| 12 | The history window is 800 days: a vault older than that loses its oldest streaks and totals. | the user | 800 days (`LIFE_INDEX_DAYS`, one constant) | nothing this round |
| 13 | A Rain-created bare note is not given the Obsidian template (decisions.md section 4). The user's Obsidian "daily note" command then opens the existing bare note and does not apply the template. Documented behaviour; a future option could apply it. | the user | as decided | nothing |

---------------------------------------------------------------------------------------------------

## Appendix. Check against the task list

| Required by the task | Where |
|---|---|
| Habit definitions in the data folder with create, edit, delete and a dialog modelled on the pool editor; vault scan suggests candidates | 4.1, 6.7, 6.10, 9.4 |
| Daily notes folder and format from the vault, Settings override, Moment tokens, cache by file time and size, watch only the daily notes folder, never log note text | 6.2 to 6.4, 7, 5, 6.1 rule 6 |
| One tick route; one property of one note; bare note content; unticking; concurrency; atomic replace; line endings; BOM; no frontmatter; which dates; layout for the source scan; sandbox | 1.4 (L1 to L3), 6.8, 6.1, 8 |
| Main screen: groups, rows, heatmap sizes, what weekly and monthly show, keys, mouse, no agenda, no pane | 9.1, 9.3 to 9.5 |
| Full view: year heatmap, streaks, rate, totals, ticking past days, navigation model and Esc ladder | 9.1, 9.3, 9.5, 1.4 (L7) |
| ui-spec replacement text, lock icon, Home today panel, actions and keymap, search decision, reused and deleted components | 9.10, 9.7, 9.5, 1.4 (L8), 9.2 |
| Types: habit shape, `HabitDay`, summary, removal of number habits and skip | 3, 3.1 |
| Calendar: route kept, returns `[]`, Settings "not connected", Home no events | 6.11, 9.7, 9.8 |
