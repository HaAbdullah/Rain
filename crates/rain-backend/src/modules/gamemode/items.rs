//! The six rows of the Game mode checklist. Section 06 3.2, 6.7.4 and 6.11.
//!
//! The ids, labels, icons and kinds are constants, in the order in which the wind-down stops
//! them. What changes is the count and the effect, which are measured.

use rain_types::{IconKind, IconRef, WindDownEffect, WindDownItem, WindDownKind, WindDownStatus};

use crate::core::winddown::Counts;

pub const SESSIONS: &str = "wind-sessions";
pub const OTHER_APPS: &str = "wind-other-apps";
pub const JOBS: &str = "wind-jobs";
pub const IDLE_JOBS: &str = "wind-idle-jobs";
pub const SERVICE: &str = "wind-service";
pub const WINDOW: &str = "wind-window";

/// The words of a row, and of the recap line about the same kind of thing.
pub fn label(kind: WindDownKind) -> &'static str {
    match kind {
        WindDownKind::Session => "Claude sessions",
        WindDownKind::OtherApp => "Other apps",
        WindDownKind::Job => "Scheduled jobs",
        WindDownKind::IdleJob => "Idle jobs",
        WindDownKind::Service => "Background service",
        WindDownKind::Window => "Rain window",
    }
}

fn row(
    id: &str,
    kind: WindDownKind,
    icon: &str,
    count: u32,
    effect: WindDownEffect,
) -> WindDownItem {
    WindDownItem {
        id: id.to_owned(),
        label: label(kind).to_owned(),
        icon: IconRef {
            kind: IconKind::Lucide,
            value: icon.to_owned(),
            tint: None,
            fallback: None,
        },
        kind,
        count,
        effect,
        status: WindDownStatus::Pending,
        progress: None,
    }
}

/// What a group of runs does to the user when it is stopped: it loses work only if a run is in
/// progress.
fn effect_of(counts: Counts) -> WindDownEffect {
    if counts.running > 0 {
        WindDownEffect::Interrupts
    } else {
        WindDownEffect::Safe
    }
}

/// The six rows for the numbers that were just measured. The sessions row is always
/// `leftRunning`: game mode never stops a session. The other programs are closed in the first
/// step of the wind-down, and never interrupt: a program that shows a window is only asked, and a
/// program that is asked something stays (section 06 6.11).
pub fn measure(sessions: u32, other_apps: u32, jobs: Counts, idle: Counts) -> Vec<WindDownItem> {
    vec![
        row(
            SESSIONS,
            WindDownKind::Session,
            "bot",
            sessions,
            WindDownEffect::LeftRunning,
        ),
        row(
            OTHER_APPS,
            WindDownKind::OtherApp,
            "layout-grid",
            other_apps,
            WindDownEffect::Safe,
        ),
        row(
            JOBS,
            WindDownKind::Job,
            "calendar-clock",
            jobs.enabled,
            effect_of(jobs),
        ),
        row(
            IDLE_JOBS,
            WindDownKind::IdleJob,
            "cpu",
            idle.enabled,
            effect_of(idle),
        ),
        row(
            SERVICE,
            WindDownKind::Service,
            "server",
            1,
            WindDownEffect::Safe,
        ),
        row(
            WINDOW,
            WindDownKind::Window,
            "app-window",
            1,
            WindDownEffect::Safe,
        ),
    ]
}

/// The rows while game mode is off: nothing is counted and nothing interrupts.
pub fn idle_rows() -> Vec<WindDownItem> {
    let mut items = measure(0, 0, Counts::default(), Counts::default());
    // The sessions row says "left running" only when sessions are counted.
    items[0].effect = WindDownEffect::Safe;
    for item in &mut items[4..] {
        item.count = 0;
    }
    items
}

/// How many rows lose unfinished work. The heading of the screen says "2 things will be
/// interrupted"; the sessions row never counts.
pub fn interrupting(items: &[WindDownItem]) -> usize {
    items
        .iter()
        .filter(|item| item.effect == WindDownEffect::Interrupts)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(enabled: u32, running: u32) -> Counts {
        Counts { enabled, running }
    }

    #[test]
    fn there_are_always_six_rows_in_stop_order() {
        let ids: Vec<_> = measure(0, 0, counts(0, 0), counts(0, 0))
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(
            ids,
            [SESSIONS, OTHER_APPS, JOBS, IDLE_JOBS, SERVICE, WINDOW]
        );
    }

    #[test]
    fn a_row_interrupts_only_when_a_run_is_in_progress() {
        let items = measure(2, 5, counts(3, 0), counts(1, 1));
        assert_eq!(items[2].count, 3);
        assert_eq!(items[2].effect, WindDownEffect::Safe);
        assert_eq!(items[3].effect, WindDownEffect::Interrupts);
        assert_eq!(interrupting(&items), 1);
    }

    #[test]
    fn the_sessions_row_is_left_running_and_never_counted_as_interrupted() {
        let items = measure(4, 0, counts(0, 0), counts(0, 0));
        assert_eq!(items[0].count, 4);
        assert_eq!(items[0].effect, WindDownEffect::LeftRunning);
        assert_eq!(interrupting(&items), 0);
    }

    #[test]
    fn the_other_apps_row_counts_programs_and_never_interrupts() {
        let items = measure(0, 7, counts(0, 0), counts(0, 0));
        assert_eq!(items[1].kind, WindDownKind::OtherApp);
        assert_eq!(items[1].count, 7);
        assert_eq!(items[1].effect, WindDownEffect::Safe);
        assert_eq!(interrupting(&items), 0);
    }

    #[test]
    fn the_off_rows_count_nothing() {
        let items = idle_rows();
        assert!(items.iter().all(|item| item.count == 0));
        assert!(items.iter().all(|item| item.effect == WindDownEffect::Safe));
        assert!(
            items
                .iter()
                .all(|item| item.status == WindDownStatus::Pending)
        );
    }
}
