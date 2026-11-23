//! What the Life screen shows, computed from the note index: the list rows (today, progress,
//! a small history strip) and, for one habit, streaks, rate and totals. Section 04 6.5 and 6.6.
//!
//! Pure functions: no file access, no clock. They take the notes and today. The backend
//! computes everything so that the week and month rules exist once and can be tested; the
//! frontend only draws.
//!
//! A week runs from Monday to Sunday, a month is a calendar month. A habit is a checkbox
//! property: a day is "done" when that day's note has the property set to true.

use std::collections::BTreeMap;

use rain_types::{Cadence, Habit, HabitCell, HabitStats, HabitSummary};

use super::dates::CivilDate;
use super::index::{Note, window_from};
use super::{
    LIFE_LIST_DAYS, LIFE_LIST_MONTHS, LIFE_LIST_WEEKS, RATE_DAYS, RATE_MONTHS, RATE_WEEKS,
    TICK_WINDOW_DAYS,
};

/// The notes and the property of one habit.
pub struct Lookup<'a> {
    pub notes: &'a BTreeMap<CivilDate, Note>,
    pub property: &'a str,
}

impl Lookup<'_> {
    fn has_note(&self, date: CivilDate) -> bool {
        self.notes.contains_key(&date)
    }

    fn done(&self, date: CivilDate) -> bool {
        self.notes
            .get(&date)
            .is_some_and(|note| note.value(self.property) == Some(true))
    }

    /// Done days from `from` to `to`, both included. A `to` before `from` counts nothing.
    fn done_between(&self, from: CivilDate, to: CivilDate) -> u32 {
        let mut count = 0;
        let mut day = from;
        while day <= to {
            count += u32::from(self.done(day));
            day = day.add_days(1);
        }
        count
    }
}

// ---- units: a day, a Monday week or a calendar month, depending on the cadence ----

/// The first day of the unit (a day, a week or a month) that contains `date`.
fn unit_start(cadence: Cadence, date: CivilDate) -> CivilDate {
    match cadence {
        Cadence::Daily => date,
        Cadence::Weekly => date.monday_of(),
        Cadence::Monthly => date.first_of_month(),
    }
}

/// The unit that follows the one starting at `start`.
fn next_start(cadence: Cadence, start: CivilDate) -> CivilDate {
    match cadence {
        Cadence::Daily => start.add_days(1),
        Cadence::Weekly => start.add_days(7),
        Cadence::Monthly => start.add_months(1),
    }
}

/// The unit that comes `back` units before the one starting at `start`.
fn start_back(cadence: Cadence, start: CivilDate, back: i64) -> CivilDate {
    match cadence {
        Cadence::Daily => start.add_days(-back),
        Cadence::Weekly => start.add_days(-7 * back),
        Cadence::Monthly => start.add_months(-(back as i32)),
    }
}

/// The last day of the unit that starts at `start`.
fn unit_end(cadence: Cadence, start: CivilDate) -> CivilDate {
    next_start(cadence, start).add_days(-1)
}

/// How many ticked days complete a unit.
fn target_of(habit: &Habit) -> u32 {
    match habit.cadence {
        Cadence::Daily => 1,
        Cadence::Weekly | Cadence::Monthly => habit.times.max(1),
    }
}

// ---- the list row ----

/// The cell of one unit for a weekly or monthly strip (and for a daily one that has a note).
fn period_cell(count: u32, target: u32) -> HabitCell {
    if count >= target {
        HabitCell::Done
    } else if count > 0 {
        HabitCell::Partial
    } else {
        HabitCell::Miss
    }
}

/// One list row: today's state, progress in the current period, and the history strip.
pub fn summarise(habit: &Habit, lookup: &Lookup, today: CivilDate) -> HabitSummary {
    let cadence = habit.cadence;
    let target = target_of(habit);
    let current = unit_start(cadence, today);
    let period_count = lookup.done_between(current, today);

    let cells = match cadence {
        Cadence::Daily => (0..LIFE_LIST_DAYS)
            .rev()
            .map(|back| {
                let day = today.add_days(-back);
                if !lookup.has_note(day) {
                    HabitCell::NoNote
                } else if lookup.done(day) {
                    HabitCell::Done
                } else {
                    HabitCell::Miss
                }
            })
            .collect(),
        Cadence::Weekly | Cadence::Monthly => {
            let count = if cadence == Cadence::Weekly {
                LIFE_LIST_WEEKS
            } else {
                LIFE_LIST_MONTHS
            };
            (0..count)
                .rev()
                .map(|back| {
                    let start = start_back(cadence, current, back);
                    // Days after today are not counted.
                    let end = unit_end(cadence, start).min(today);
                    period_cell(lookup.done_between(start, end), target)
                })
                .collect()
        }
    };

    HabitSummary {
        habit: habit.clone(),
        today: today.to_string(),
        today_done: lookup.done(today),
        period_count: period_count.min(target.max(period_count)),
        period_target: target,
        cells,
    }
}

// ---- the full view ----

/// Streaks, rate and totals of one habit.
pub fn stats(habit: &Habit, lookup: &Lookup, today: CivilDate) -> HabitStats {
    let cadence = habit.cadence;
    let target = target_of(habit);
    let window_start = window_from(today);
    let tickable_from = today.add_days(-(TICK_WINDOW_DAYS - 1));
    let totals = HabitStats {
        habit_id: habit.id.clone(),
        today: today.to_string(),
        since: None,
        streak_current: 0,
        streak_best: 0,
        rate_percent: None,
        done_this_week: lookup.done_between(today.monday_of(), today),
        done_this_month: lookup.done_between(today.first_of_month(), today),
        done_this_year: lookup.done_between(today.first_of_year(), today),
        tickable_from: tickable_from.to_string(),
        window_from: window_start.to_string(),
    };

    // The first day of the window whose note has the property, true or false.
    let since = lookup
        .notes
        .range(window_start..=today)
        .find(|(_, note)| note.value(lookup.property).is_some())
        .map(|(date, _)| *date);
    let Some(since) = since else {
        return totals; // no data yet: no streak, no rate
    };

    // Every unit that lies fully inside the window, then the current one (always the last).
    let current = unit_start(cadence, today);
    let mut units = Vec::new(); // (start, end, met)
    let mut start = unit_start(cadence, window_start);
    if start < window_start {
        start = next_start(cadence, start);
    }
    while start <= current {
        let end = unit_end(cadence, start);
        let count = lookup.done_between(start, end.min(today));
        units.push((start, end, count >= target));
        start = next_start(cadence, start);
    }
    let Some(&(_, _, current_met)) = units.last() else {
        return HabitStats {
            since: Some(since.to_string()),
            ..totals
        };
    };

    // An open current unit never counts against the user: it adds 1 when met and is skipped
    // (neither adds nor breaks) when not.
    let finished = &units[..units.len() - 1];
    let mut streak_current = u32::from(current_met);
    for (_, _, met) in finished.iter().rev() {
        if *met {
            streak_current += 1;
        } else {
            break;
        }
    }
    let mut streak_best = 0;
    let mut run = 0;
    for (_, _, met) in finished {
        run = if *met { run + 1 } else { 0 };
        streak_best = streak_best.max(run);
    }
    if current_met {
        // The current unit continues the run that ends just before it.
        streak_best = streak_best.max(run + 1);
    }
    let streak_best = streak_best.max(streak_current);

    // The rate: the last K units, units that end before the first data dropped, and the current
    // one dropped unless it is met.
    let last_k = match cadence {
        Cadence::Daily => RATE_DAYS,
        Cadence::Weekly => RATE_WEEKS,
        Cadence::Monthly => RATE_MONTHS,
    };
    let skip = units.len().saturating_sub(last_k);
    let counted: Vec<bool> = units
        .iter()
        .enumerate()
        .skip(skip)
        .filter(|(_, (_, end, _))| *end >= since)
        .filter(|(index, (_, _, met))| *index + 1 < units.len() || *met)
        .map(|(_, (_, _, met))| *met)
        .collect();
    let rate_percent = (!counted.is_empty()).then(|| {
        let met = counted.iter().filter(|met| **met).count();
        ((100.0 * met as f64) / counted.len() as f64).round() as u32
    });

    HabitStats {
        since: Some(since.to_string()),
        streak_current,
        streak_best,
        rate_percent,
        ..totals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::{IconKind, IconRef};

    fn date(text: &str) -> CivilDate {
        CivilDate::parse(text).expect("a real date")
    }

    fn habit(id: &str, cadence: Cadence, times: u32) -> Habit {
        Habit {
            id: id.into(),
            name: id.into(),
            icon: IconRef {
                kind: IconKind::Lucide,
                value: "square-check".into(),
                tint: None,
                fallback: None,
            },
            property: id.into(),
            cadence,
            times,
        }
    }

    /// The notes of the fixture `life-basic` (section 10.3): today is Sunday 2026-11-22.
    fn basic_notes() -> BTreeMap<CivilDate, Note> {
        let rows: &[(&str, &[(&str, bool)])] = &[
            ("2026-11-09", &[("gym", true), ("read", false)]),
            ("2026-11-11", &[("gym", true), ("read", true)]),
            ("2026-11-12", &[]),
            ("2026-11-13", &[]),
            ("2026-11-15", &[("gym", false), ("read", true)]),
            ("2026-11-16", &[("gym", true), ("read", false)]),
            ("2026-11-17", &[("read", true)]),
            ("2026-11-18", &[("gym", true), ("read", true)]),
            ("2026-11-19", &[("read", true)]),
            ("2026-11-20", &[("gym", true), ("read", true)]),
            ("2026-11-21", &[("read", true)]),
            ("2026-11-22", &[("read", true)]),
        ];
        rows.iter()
            .map(|(day, bools)| {
                let note = Note {
                    mtime_ns: 1,
                    size: 1,
                    bools: bools.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
                };
                (date(day), note)
            })
            .collect()
    }

    fn summary(notes: &BTreeMap<CivilDate, Note>, habit: &Habit, today: &str) -> HabitSummary {
        let lookup = Lookup {
            notes,
            property: &habit.property,
        };
        summarise(habit, &lookup, date(today))
    }

    fn full_stats(notes: &BTreeMap<CivilDate, Note>, habit: &Habit, today: &str) -> HabitStats {
        let lookup = Lookup {
            notes,
            property: &habit.property,
        };
        stats(habit, &lookup, date(today))
    }

    use HabitCell::{Done, Miss, NoNote, Partial};

    #[test]
    fn the_weekly_row_of_the_worked_example() {
        let row = summary(
            &basic_notes(),
            &habit("gym", Cadence::Weekly, 3),
            "2026-11-22",
        );
        assert_eq!(
            (row.period_count, row.period_target, row.today_done),
            (3, 3, false)
        );
        let mut want = vec![Miss; 10];
        want.extend([Partial, Done]);
        assert_eq!(row.cells, want);
        assert_eq!(row.today, "2026-11-22");
    }

    #[test]
    fn the_daily_row_of_the_worked_example() {
        let row = summary(
            &basic_notes(),
            &habit("read", Cadence::Daily, 1),
            "2026-11-22",
        );
        assert_eq!(
            (row.period_count, row.period_target, row.today_done),
            (1, 1, true)
        );
        assert_eq!(row.cells.len(), 70);
        assert!(row.cells[..56].iter().all(|c| *c == NoNote));
        assert_eq!(
            &row.cells[56..],
            [
                Miss, NoNote, Done, Miss, Miss, NoNote, Done, Miss, Done, Done, Done, Done, Done,
                Done
            ]
        );
    }

    #[test]
    fn the_monthly_row_counts_the_calendar_month() {
        let row = summary(
            &basic_notes(),
            &habit("gym", Cadence::Monthly, 2),
            "2026-11-22",
        );
        assert_eq!((row.period_count, row.period_target), (5, 2)); // 9, 11, 16, 18, 20
        assert_eq!(row.cells.len(), 12);
        assert_eq!(row.cells[11], Done);
        assert!(row.cells[..11].iter().all(|c| *c == Miss));
        let half = summary(
            &basic_notes(),
            &habit("gym", Cadence::Monthly, 9),
            "2026-11-22",
        );
        assert_eq!(half.cells[11], Partial);
    }

    #[test]
    fn a_habit_with_no_data_has_empty_cells_and_a_zero_count() {
        let row = summary(
            &basic_notes(),
            &habit("review", Cadence::Monthly, 2),
            "2026-11-22",
        );
        assert_eq!(
            (row.period_count, row.period_target, row.today_done),
            (0, 2, false)
        );
        assert!(row.cells.iter().all(|c| *c == Miss));
    }

    #[test]
    fn a_week_starts_on_monday_and_a_month_on_the_first() {
        let mut notes = basic_notes();
        // Monday 2026-11-23: the new week has no tick yet, the old week is still 3 of 3.
        let gym = habit("gym", Cadence::Weekly, 3);
        let monday = summary(&notes, &gym, "2026-11-23");
        assert_eq!(monday.period_count, 0);
        assert_eq!(monday.cells[10], Done);
        assert_eq!(monday.cells[11], Miss);
        // Tick it: a tick on the first day of a month counts in the new month only.
        notes.insert(
            date("2026-12-01"),
            Note {
                mtime_ns: 1,
                size: 1,
                bools: vec![("gym".into(), true)],
            },
        );
        let month = habit("gym", Cadence::Monthly, 2);
        let december = summary(&notes, &month, "2026-12-01");
        assert_eq!(december.period_count, 1);
        assert_eq!(december.cells[10], Done); // November
        assert_eq!(december.cells[11], Partial);
    }

    #[test]
    fn days_after_today_are_not_counted() {
        let mut notes = basic_notes();
        // A note in the future (a template or a mistake) must not count in this week.
        notes.insert(
            date("2026-11-25"),
            Note {
                mtime_ns: 1,
                size: 1,
                bools: vec![("gym".into(), true)],
            },
        );
        let row = summary(&notes, &habit("gym", Cadence::Weekly, 3), "2026-11-20");
        // Week 16 to 22 up to Friday 20: 16, 18 and 20 are ticked.
        assert_eq!(row.period_count, 3);
        let later = summary(&notes, &habit("gym", Cadence::Weekly, 3), "2026-11-22");
        assert_eq!(later.period_count, 3);
    }

    // ---- stats ----

    #[test]
    fn the_daily_stats_of_the_worked_example() {
        let s = full_stats(
            &basic_notes(),
            &habit("read", Cadence::Daily, 1),
            "2026-11-22",
        );
        assert_eq!(s.since.as_deref(), Some("2026-11-09"));
        assert_eq!((s.streak_current, s.streak_best), (6, 6));
        assert_eq!(s.rate_percent, Some(57));
        assert_eq!(
            (s.done_this_week, s.done_this_month, s.done_this_year),
            (6, 8, 8)
        );
        assert_eq!(s.tickable_from, "2026-10-23");
        assert_eq!(s.window_from, "2024-09-14");
        assert_eq!(s.habit_id, "read");
    }

    #[test]
    fn the_weekly_stats_of_the_worked_example() {
        let s = full_stats(
            &basic_notes(),
            &habit("gym", Cadence::Weekly, 3),
            "2026-11-22",
        );
        assert_eq!(s.since.as_deref(), Some("2026-11-09"));
        assert_eq!((s.streak_current, s.streak_best), (1, 1));
        assert_eq!(s.rate_percent, Some(50));
        assert_eq!(
            (s.done_this_week, s.done_this_month, s.done_this_year),
            (3, 5, 5)
        );
    }

    #[test]
    fn a_habit_no_note_has_gives_zeros_and_no_since_or_rate() {
        let s = full_stats(
            &basic_notes(),
            &habit("review", Cadence::Monthly, 2),
            "2026-11-22",
        );
        assert_eq!(s.since, None);
        assert_eq!((s.streak_current, s.streak_best), (0, 0));
        assert_eq!(s.rate_percent, None);
        assert_eq!(
            (s.done_this_week, s.done_this_month, s.done_this_year),
            (0, 0, 0)
        );
        assert_eq!(s.tickable_from, "2026-10-23");
    }

    /// A daily habit with notes from `first` on: `pattern` gives done (true) or not for each day.
    fn daily_notes(first: &str, pattern: &[bool]) -> BTreeMap<CivilDate, Note> {
        pattern
            .iter()
            .enumerate()
            .map(|(offset, done)| {
                (
                    date(first).add_days(offset as i64),
                    Note {
                        mtime_ns: 1,
                        size: 1,
                        bools: vec![("h".into(), *done)],
                    },
                )
            })
            .collect()
    }

    #[test]
    fn an_open_current_day_neither_adds_nor_breaks_a_streak() {
        let h = habit("h", Cadence::Daily, 1);
        // 5 done days, then today still open (note exists, not done yet).
        let notes = daily_notes("2026-11-17", &[true, true, true, true, true, false]);
        let s = full_stats(&notes, &h, "2026-11-22");
        assert_eq!(
            s.streak_current, 5,
            "today is not done yet, the five before still count"
        );
        assert_eq!(s.streak_best, 5);
        // The rate leaves the open day out: 5 of 5.
        assert_eq!(s.rate_percent, Some(100));
        // Once today is done it counts.
        let done = daily_notes("2026-11-17", &[true, true, true, true, true, true]);
        let s = full_stats(&done, &h, "2026-11-22");
        assert_eq!(
            (s.streak_current, s.streak_best, s.rate_percent),
            (6, 6, Some(100))
        );
    }

    #[test]
    fn a_missed_day_breaks_the_current_streak_but_not_the_best() {
        let h = habit("h", Cadence::Daily, 1);
        let notes = daily_notes(
            "2026-11-10",
            &[
                true, true, true, true, false, true, true, false, true, true, true, true,
            ],
        );
        let s = full_stats(&notes, &h, "2026-11-21");
        assert_eq!(s.streak_current, 4);
        assert_eq!(s.streak_best, 4);
        // A day without a note breaks the streak too (note missing on 2026-11-13).
        let mut gap = daily_notes("2026-11-10", &[true, true, true, true, true, true]);
        gap.remove(&date("2026-11-13"));
        let s = full_stats(&gap, &h, "2026-11-15");
        assert_eq!(s.streak_current, 2);
        assert_eq!(s.streak_best, 3);
    }

    #[test]
    fn weekly_streaks_count_weeks_with_the_target_reached() {
        let h = habit("h", Cadence::Weekly, 2);
        // Weeks (Monday starts): Nov 2 met (2), Nov 9 met (3), Nov 16 open with one tick.
        let mut notes = BTreeMap::new();
        for day in [
            "2026-11-03",
            "2026-11-05",
            "2026-11-10",
            "2026-11-11",
            "2026-11-13",
            "2026-11-17",
        ] {
            notes.insert(
                date(day),
                Note {
                    mtime_ns: 1,
                    size: 1,
                    bools: vec![("h".into(), true)],
                },
            );
        }
        let s = full_stats(&notes, &h, "2026-11-19");
        assert_eq!(
            s.streak_current, 2,
            "the open week adds nothing and breaks nothing"
        );
        assert_eq!(s.streak_best, 2);
        assert_eq!(s.rate_percent, Some(100));
        assert_eq!(s.done_this_week, 1);
        // The week closes with the second tick: the streak is 3.
        notes.insert(
            date("2026-11-18"),
            Note {
                mtime_ns: 1,
                size: 1,
                bools: vec![("h".into(), true)],
            },
        );
        let s = full_stats(&notes, &h, "2026-11-19");
        assert_eq!((s.streak_current, s.streak_best), (3, 3));
    }

    #[test]
    fn monthly_streaks_and_the_rate_use_calendar_months() {
        let h = habit("h", Cadence::Monthly, 2);
        let mut notes = BTreeMap::new();
        for day in ["2026-09-04", "2026-09-20", "2026-10-02", "2026-11-03"] {
            notes.insert(
                date(day),
                Note {
                    mtime_ns: 1,
                    size: 1,
                    bools: vec![("h".into(), true)],
                },
            );
        }
        let s = full_stats(&notes, &h, "2026-11-22");
        // September met, October has one tick (not met), November open with one.
        assert_eq!((s.streak_current, s.streak_best), (0, 1));
        // Counted: September (met), October (not met); November is open and not met: left out.
        assert_eq!(s.rate_percent, Some(50));
        assert_eq!(s.done_this_month, 1);
        assert_eq!(s.done_this_year, 4);
    }

    #[test]
    fn the_rate_ignores_units_before_the_first_data() {
        let h = habit("h", Cadence::Daily, 1);
        // Data only for the last three days; the 87 days before `since` are not counted.
        let notes = daily_notes("2026-11-20", &[true, false, true]);
        let s = full_stats(&notes, &h, "2026-11-22");
        assert_eq!(s.since.as_deref(), Some("2026-11-20"));
        assert_eq!(s.rate_percent, Some(67)); // 2 of 3
    }

    #[test]
    fn a_window_that_starts_in_the_middle_of_a_week_skips_the_partial_week() {
        let h = habit("h", Cadence::Weekly, 1);
        // Today is 2026-11-22; the window starts 2024-09-14, a Saturday: the week of Sep 9 is
        // not walked, the week of Sep 16 is the first unit.
        let notes = BTreeMap::from([(
            date("2024-09-14"),
            Note {
                mtime_ns: 1,
                size: 1,
                bools: vec![("h".into(), true)],
            },
        )]);
        let s = full_stats(&notes, &h, "2026-11-22");
        assert_eq!(s.since.as_deref(), Some("2024-09-14"));
        // That tick is outside every walked unit: nothing is met.
        assert_eq!(s.streak_best, 0);
        assert_eq!(s.rate_percent, Some(0));
    }
}
