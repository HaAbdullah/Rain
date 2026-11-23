//! The Life types: habits, their days and the state of the Obsidian vault.
//! Feature section 04 (section 3). The rules of foundation 9.2 apply: camelCase, no `null`,
//! plain `//` comments on fields.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::common::IconRef;

/// How often a habit is meant to be done. Daily means every day (`times` is 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Cadence {
    Daily,
    Weekly,
    Monthly,
}

/// A habit definition: what the user wrote in `habits.toml`. There are no number habits and no
/// "skip".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Habit {
    pub id: String,
    pub name: String,
    pub icon: IconRef,
    // The checkbox property in the daily note's frontmatter, for example "gym".
    pub property: String,
    #[ts(inline)]
    pub cadence: Cadence,
    // How many ticked days make a period complete: 1 for daily, 1 to 7 weekly, 1 to 31 monthly.
    pub times: u32,
}

/// The body of `POST /habits` and `PUT /habits/{id}`. The same fields as `Habit`; Rain chooses
/// the `id` when it creates a habit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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

/// One cell of the small history strip of a list row. Daily strips use `none`, `miss` and
/// `done`. Weekly and monthly strips use `miss`, `partial` and `done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HabitCell {
    /// No daily note that day (daily strips only).
    #[serde(rename = "none")]
    NoNote,
    /// The note exists but the property is not true, or the period has no tick.
    Miss,
    /// Some ticks, fewer than `times`.
    Partial,
    /// Done that day, or `times` reached in that period.
    Done,
}

/// What one list row needs. `GET /habits` and the `habits` topic carry a list of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HabitSummary {
    pub habit: Habit,
    // The date the backend used as today. The cells are placed relative to it.
    pub today: String,
    // True when today's note has the property set to true.
    pub today_done: bool,
    // Ticked days in the current period (daily: today only, 0 or 1).
    pub period_count: u32,
    // `times` (daily: 1).
    pub period_target: u32,
    // Oldest first, the last cell is the current day, week or month. Lengths: daily 70 (ten
    // weeks), weekly 12 (weeks), monthly 12 (months).
    pub cells: Vec<HabitCell>,
}

/// One existing daily note and which habits it has ticked. Days without a note are absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HabitDay {
    pub date: String,
    // Ids of the habits whose property is true in that note. With `?habit=` at most one id.
    pub done: Vec<String>,
}

/// Everything the full view of one habit shows besides the per-day grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct HabitStats {
    pub habit_id: String,
    pub today: String,
    // The first day in the history window whose note has this property (true or false). Absent:
    // no data yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    // Daily: days in a row. Weekly: weeks in a row with `times` reached. Monthly: months in a
    // row. The unit is the cadence.
    pub streak_current: u32,
    pub streak_best: u32,
    // 0 to 100. Absent when no period has been counted yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_percent: Option<u32>,
    pub done_this_week: u32,
    pub done_this_month: u32,
    pub done_this_year: u32,
    // The oldest day Rain will tick (today minus 30 days: the window is 31 days, today included).
    pub tickable_from: String,
    // The oldest day inside the history window (today minus 799 days).
    pub window_from: String,
}

/// The body of `PUT /habits/{id}/days/{date}`: the tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HabitTickBody {
    pub done: bool,
}

/// The state of the vault as the Life screen and Settings need it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct VaultStatus {
    // A vault is chosen, it exists and it is a folder.
    pub connected: bool,
    // The effective vault folder. Present even when `connected` is false (a chosen path that is
    // not a folder). Absent when no vault is chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    // The folder in effect, relative to the vault ("Daily"; "" is the vault root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_folder: Option<String>,
    // The Moment format in effect ("YYYY-MM-DD").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_format: Option<String>,
    // The daily notes folder exists. A tick cannot create it.
    pub daily_folder_found: bool,
    // A tick would be accepted. False in a sandbox build when the vault is outside the data folder.
    pub writable: bool,
    // One plain sentence when notes cannot be found for a reason the user can fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// A checkbox property that appears in the notes and that no habit uses yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PropertySuggestion {
    pub property: String,
    // Number of notes in the history window that have it as true or false.
    pub days: u32,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenNoteResult {
    pub opened: bool,
}

/// An event of the calendar. The route returns an empty list in this round (no calendar is
/// connected); the type stays for the day one is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::IconKind;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn habit() -> Habit {
        Habit {
            id: "gym".into(),
            name: "Gym".into(),
            icon: IconRef {
                kind: IconKind::Lucide,
                value: "dumbbell".into(),
                tint: None,
                fallback: None,
            },
            property: "gym".into(),
            cadence: Cadence::Weekly,
            times: 3,
        }
    }

    #[test]
    fn a_summary_uses_camel_case_and_the_none_cell() {
        let summary = HabitSummary {
            habit: habit(),
            today: "2026-11-22".into(),
            today_done: true,
            period_count: 3,
            period_target: 3,
            cells: vec![
                HabitCell::NoNote,
                HabitCell::Miss,
                HabitCell::Partial,
                HabitCell::Done,
            ],
        };
        let json = round_trip(&summary);
        assert_eq!(json["todayDone"], true);
        assert_eq!(json["periodTarget"], 3);
        assert_eq!(json["cells"], json!(["none", "miss", "partial", "done"]));
        assert_eq!(json["habit"]["cadence"], "weekly");
    }

    #[test]
    fn stats_leave_out_what_is_absent() {
        let stats = HabitStats {
            habit_id: "review".into(),
            today: "2026-11-22".into(),
            since: None,
            streak_current: 0,
            streak_best: 0,
            rate_percent: None,
            done_this_week: 0,
            done_this_month: 0,
            done_this_year: 0,
            tickable_from: "2026-10-23".into(),
            window_from: "2024-09-14".into(),
        };
        let json = round_trip(&stats);
        assert!(json.get("since").is_none() && json.get("ratePercent").is_none());
    }

    #[test]
    fn a_draft_may_not_carry_unknown_fields() {
        let draft = json!({
            "name": "Gym", "icon": { "kind": "lucide", "value": "dumbbell" },
            "property": "gym", "cadence": "daily", "times": 1, "kind": "number"
        });
        assert!(serde_json::from_value::<HabitDraft>(draft).is_err());
        let ok = json!({
            "name": "Gym", "icon": { "kind": "lucide", "value": "dumbbell" },
            "property": "gym", "cadence": "daily", "times": 1
        });
        let draft: HabitDraft = serde_json::from_value(ok).unwrap();
        assert_eq!(draft.id, None);
    }

    #[test]
    fn the_tick_body_is_exactly_done() {
        assert!(serde_json::from_value::<HabitTickBody>(json!({ "done": true })).is_ok());
        assert!(serde_json::from_value::<HabitTickBody>(json!({ "done": true, "x": 1 })).is_err());
        assert!(serde_json::from_value::<HabitTickBody>(json!({})).is_err());
    }
}
