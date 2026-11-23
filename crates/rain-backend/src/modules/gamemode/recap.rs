//! Builds the recap of the last game mode from its record. Section 06 6.7.7.
//!
//! Pure: the store gives rows, this gives the type of the API.

use rain_types::{GameModeRecap, RecapItem, WindDownKind};

use super::items;
use super::store::{ItemRow, RecordRow};

/// At most this many job names go to the screen.
const NAMES_SHOWN: usize = 5;

/// `at` is when the wind-down ended, or when it began if it never ended.
pub fn build(record: &RecordRow, rows: &[ItemRow]) -> GameModeRecap {
    let mut items: Vec<RecapItem> = Vec::new();
    for row in rows {
        // One recap line for each kind and outcome (all stopped jobs are one line).
        let line = items
            .iter_mut()
            .find(|item| item.kind == row.kind && item.outcome == row.outcome);
        let line = match line {
            Some(line) => line,
            None => {
                items.push(RecapItem {
                    kind: row.kind,
                    label: items::label(row.kind).to_owned(),
                    count: 0,
                    outcome: row.outcome,
                    names: Vec::new(),
                });
                items.last_mut().expect("a line was just pushed")
            }
        };
        line.count += row.count;
        // A session title can hold prompt text, so sessions never carry a name.
        if row.kind != WindDownKind::Session
            && line.names.len() < NAMES_SHOWN
            && !line.names.contains(&row.name)
        {
            line.names.push(row.name.clone());
        }
    }
    // Sessions first, then jobs, then idle jobs, as on the screen.
    items.sort_by_key(|item| match item.kind {
        WindDownKind::Session => 0,
        WindDownKind::Job => 1,
        _ => 2,
    });
    GameModeRecap {
        at: record
            .finished_at
            .clone()
            .unwrap_or_else(|| record.started_at.clone()),
        complete: record.complete,
        items,
        launched_title: record.then_launch_title.clone(),
        launch_error: record.launch_error.clone(),
        launched_in_job: record.launched_in_job.then_some(true),
        seen: record.seen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::RecapOutcome;

    fn record(complete: bool) -> RecordRow {
        RecordRow {
            started_at: "2026-11-22T10:00:00.000Z".into(),
            finished_at: complete.then(|| "2026-11-22T10:00:09.000Z".into()),
            complete,
            then_launch_title: None,
            launch_error: None,
            launched_in_job: false,
            seen: false,
        }
    }

    fn row(kind: WindDownKind, name: &str, outcome: RecapOutcome, count: u32) -> ItemRow {
        ItemRow {
            kind,
            name: name.into(),
            outcome,
            count,
        }
    }

    #[test]
    fn jobs_are_one_line_per_outcome_with_at_most_five_names() {
        let rows: Vec<_> = (1..=7)
            .map(|n| {
                row(
                    WindDownKind::Job,
                    &format!("Job {n}"),
                    RecapOutcome::Stopped,
                    1,
                )
            })
            .chain([row(WindDownKind::Job, "Stuck", RecapOutcome::Failed, 1)])
            .collect();
        let recap = build(&record(true), &rows);
        assert_eq!(recap.items.len(), 2);
        assert_eq!(recap.items[0].count, 7);
        assert_eq!(recap.items[0].names.len(), 5);
        assert_eq!(recap.items[1].outcome, RecapOutcome::Failed);
        assert_eq!(recap.items[1].names, ["Stuck"]);
    }

    #[test]
    fn sessions_carry_a_count_and_never_a_name() {
        let rows = [
            row(WindDownKind::Job, "Demo", RecapOutcome::Forced, 1),
            row(
                WindDownKind::Session,
                "Claude sessions",
                RecapOutcome::LeftRunning,
                3,
            ),
        ];
        let recap = build(&record(true), &rows);
        assert_eq!(
            recap.items[0].kind,
            WindDownKind::Session,
            "sessions come first"
        );
        assert_eq!(recap.items[0].count, 3);
        assert!(recap.items[0].names.is_empty());
        assert_eq!(recap.items[1].names, ["Demo"]);
    }

    #[test]
    fn an_incomplete_record_says_so_and_dates_from_its_start() {
        let recap = build(&record(false), &[]);
        assert!(!recap.complete);
        assert_eq!(recap.at, "2026-11-22T10:00:00.000Z");
        assert!(recap.items.is_empty());
    }

    #[test]
    fn the_game_and_the_seen_mark_come_from_the_record() {
        let mut finished = record(true);
        finished.then_launch_title = Some("Game A".into());
        finished.launch_error = Some("It did not start.".into());
        finished.seen = true;
        let recap = build(&finished, &[]);
        assert_eq!(recap.launched_title.as_deref(), Some("Game A"));
        assert_eq!(recap.launch_error.as_deref(), Some("It did not start."));
        assert!(recap.seen);
        assert_eq!(recap.at, "2026-11-22T10:00:09.000Z");
    }
}
