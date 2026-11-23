//! Cron schedules: parsing, the next occurrence and the label. Section 03 6.11.2.
//!
//! This is the only file that knows `chrono` (foundation 1.6). The rest of Rain passes
//! `SystemTime` and strings. Times are the local time of the PC, so `0 6 * * *` stays at 06:00
//! across a daylight saving change.

use std::str::FromStr;
use std::time::SystemTime;

use chrono::{DateTime, Local, TimeZone};
use croner::Cron;

/// The message for every schedule that is not five fields (or that `croner` rejects).
pub const FIVE_FIELDS: &str = "Use five fields: minute hour day month weekday";

/// Parses the five-field text. Six or seven fields, `@daily` style macros and text that `croner`
/// rejects give the same plain message.
pub fn parse(text: &str) -> Result<Cron, String> {
    let fields = text.split_whitespace().count();
    if fields != 5 || text.trim_start().starts_with('@') {
        return Err(FIVE_FIELDS.to_owned());
    }
    Cron::from_str(text).map_err(|_| FIVE_FIELDS.to_owned())
}

/// The first occurrence strictly after `after`, in the local time zone. `None` for a schedule
/// that never happens again (for example 31 February).
pub fn next_after(cron: &Cron, after: SystemTime) -> Option<SystemTime> {
    next_after_in(cron, &DateTime::<Local>::from(after)).map(SystemTime::from)
}

/// The same for any time zone, so that a test can use a zone with daylight saving.
pub fn next_after_in<Tz: TimeZone>(cron: &Cron, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    cron.find_next_occurrence(after, false).ok()
}

/// Reads an `ISODateTime` that the API wrote back into a time. `None` for text that is not one.
pub fn parse_iso(text: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| SystemTime::from(time.with_timezone(&chrono::Utc)))
}

// ---- the label ----

/// One field of a schedule, as far as the label needs to tell.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Field {
    Any,
    Step(u32),
    Number(u32),
    List(Vec<u32>),
    Range(u32, u32),
    Other,
}

fn field(text: &str) -> Field {
    if text == "*" {
        return Field::Any;
    }
    if let Some(step) = text.strip_prefix("*/") {
        return step.parse().map_or(Field::Other, Field::Step);
    }
    if let Ok(number) = text.parse() {
        return Field::Number(number);
    }
    if let Some((from, to)) = text.split_once('-')
        && let (Ok(from), Ok(to)) = (from.parse(), to.parse())
    {
        return Field::Range(from, to);
    }
    if text.contains(',') {
        let numbers: Result<Vec<u32>, _> = text.split(',').map(str::parse).collect();
        return numbers.map_or(Field::Other, Field::List);
    }
    Field::Other
}

const DAYS: [&str; 7] = [
    "Sundays",
    "Mondays",
    "Tuesdays",
    "Wednesdays",
    "Thursdays",
    "Fridays",
    "Saturdays",
];
const DAY_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// "Every day at 06:00", "Weekdays at 09:30", "Every 15 minutes" ... Anything the function does
/// not know falls back to the library's description, and text that is not a schedule is returned
/// as it is.
pub fn label(expression: &str) -> String {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    let [minute, hour, day, month, weekday] = parts[..] else {
        return expression.to_owned();
    };
    let (minute_f, hour_f) = (field(minute), field(hour));
    let known = (field(day), field(month), field(weekday));

    let text = match (&minute_f, &hour_f, &known) {
        (Field::Any, Field::Any, (Field::Any, Field::Any, Field::Any)) => {
            Some("Every minute".to_owned())
        }
        (Field::Step(n), Field::Any, (Field::Any, Field::Any, Field::Any)) => {
            Some(format!("Every {n} minutes"))
        }
        (Field::Number(0), Field::Any, (Field::Any, Field::Any, Field::Any)) => {
            Some("Every hour".to_owned())
        }
        (Field::Number(m), Field::Any, (Field::Any, Field::Any, Field::Any)) => {
            Some(format!("Every hour at :{m:02}"))
        }
        (Field::Number(0), Field::Step(n), (Field::Any, Field::Any, Field::Any)) => {
            Some(format!("Every {n} hours"))
        }
        (Field::Number(m), hours, (day, Field::Any, weekday)) => {
            at_times(*m, hours).and_then(|times| daily_label(&times, day, weekday))
        }
        _ => None,
    };
    text.unwrap_or_else(|| fallback(expression))
}

/// The clock times of an hour field with one fixed minute: `06:00`, or `09:00 and 17:00`.
fn at_times(minute: u32, hours: &Field) -> Option<String> {
    let hours: Vec<u32> = match hours {
        Field::Number(h) => vec![*h],
        Field::List(list) => list.clone(),
        _ => return None,
    };
    if minute > 59 || hours.iter().any(|h| *h > 23) {
        return None;
    }
    let times: Vec<String> = hours
        .iter()
        .map(|h| format!("{h:02}:{minute:02}"))
        .collect();
    Some(times.join(" and "))
}

fn daily_label(times: &str, day: &Field, weekday: &Field) -> Option<String> {
    match (day, weekday) {
        (Field::Any, Field::Any) => Some(format!("Every day at {times}")),
        (Field::Any, Field::Range(1, 5)) => Some(format!("Weekdays at {times}")),
        (Field::Any, Field::List(days)) if is_weekend(days) => Some(format!("Weekends at {times}")),
        (Field::Any, Field::Number(d)) if *d <= 7 => {
            Some(format!("{} at {times}", DAYS[*d as usize % 7]))
        }
        (Field::Any, Field::List(days)) if days.iter().all(|d| *d <= 7) => {
            Some(format!("{} at {times}", day_names(days)))
        }
        (Field::Number(d), Field::Any) if (1..=31).contains(d) => {
            Some(format!("Monthly on the {} at {times}", ordinal(*d)))
        }
        _ => None,
    }
}

fn is_weekend(days: &[u32]) -> bool {
    let mut sorted: Vec<u32> = days.iter().map(|d| d % 7).collect();
    sorted.sort_unstable();
    sorted.dedup();
    sorted == [0, 6]
}

/// "Mon, Wed and Fri".
fn day_names(days: &[u32]) -> String {
    let names: Vec<&str> = days.iter().map(|d| DAY_SHORT[*d as usize % 7]).collect();
    match names.split_last() {
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// What the library says, without its full stop. Text that does not parse stays as it is.
fn fallback(expression: &str) -> String {
    match Cron::from_str(expression) {
        Ok(cron) => cron.describe().trim_end_matches('.').to_owned(),
        Err(_) => expression.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Timelike};
    use chrono_tz::Tz;
    use std::time::Duration;

    #[test]
    fn labels_for_the_usual_schedules() {
        let cases = [
            ("0 6 * * *", "Every day at 06:00"),
            ("30 9 * * 1-5", "Weekdays at 09:30"),
            ("*/15 * * * *", "Every 15 minutes"),
            ("* * * * *", "Every minute"),
            ("0 * * * *", "Every hour"),
            ("5 * * * *", "Every hour at :05"),
            ("0 */2 * * *", "Every 2 hours"),
            ("0 6 * * 1", "Mondays at 06:00"),
            ("0 10 * * 6,0", "Weekends at 10:00"),
            ("0 10 * * 1,3,5", "Mon, Wed and Fri at 10:00"),
            ("0 0 1 * *", "Monthly on the 1st at 00:00"),
            ("0 0 22 * *", "Monthly on the 22nd at 00:00"),
            ("0 9,17 * * *", "Every day at 09:00 and 17:00"),
        ];
        for (expression, expected) in cases {
            assert_eq!(label(expression), expected, "{expression}");
        }
    }

    #[test]
    fn an_unusual_schedule_falls_back_and_text_that_is_not_a_schedule_stays() {
        let odd = label("0 6 15 3 *");
        assert!(!odd.is_empty() && !odd.ends_with('.'));
        assert_eq!(label("not a schedule"), "not a schedule");
        assert_eq!(label("a b"), "a b");
    }

    #[test]
    fn cron_validate_accepts_five_fields_only() {
        assert!(parse("0 6 * * *").is_ok());
        assert!(parse("*/5 1-3 * 1,2 mon-fri").is_ok());
        for bad in [
            "0 0 6 * * *",
            "0 6 * *",
            "@daily",
            "",
            "61 * * * *",
            "0 25 * * *",
            "x * * * *",
        ] {
            assert_eq!(parse(bad).unwrap_err(), FIVE_FIELDS, "{bad:?}");
        }
    }

    fn utc(h: u32, m: u32) -> SystemTime {
        let time = chrono::Utc
            .with_ymd_and_hms(2026, 11, 21, h, m, 0)
            .single()
            .unwrap();
        SystemTime::from(time)
    }

    #[test]
    fn the_next_occurrence_is_strictly_after_the_given_time() {
        let cron = parse("* * * * *").unwrap();
        let next = next_after(&cron, utc(9, 30)).unwrap();
        assert_eq!(
            next.duration_since(utc(9, 30)).unwrap(),
            Duration::from_secs(60)
        );
        let half = utc(9, 30) + Duration::from_millis(400);
        assert_eq!(
            next_after(&cron, half)
                .unwrap()
                .duration_since(utc(9, 30))
                .unwrap(),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn both_day_fields_mean_either_when_both_are_given() {
        // Classic cron: "the 13th, or a Friday". The test pins what the library does.
        let cron = parse("0 0 13 * 5").unwrap();
        let tz: Tz = "UTC".parse().unwrap();
        let start = tz.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        let first = next_after_in(&cron, &start).unwrap();
        assert_eq!(
            first.date_naive(),
            NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
        );
    }

    #[test]
    fn sunday_is_zero_and_seven() {
        let tz: Tz = "UTC".parse().unwrap();
        let start = tz.with_ymd_and_hms(2026, 11, 21, 0, 0, 0).unwrap(); // a Saturday
        for text in ["0 8 * * 0", "0 8 * * 7"] {
            let next = next_after_in(&parse(text).unwrap(), &start).unwrap();
            assert_eq!(
                next.date_naive(),
                NaiveDate::from_ymd_opt(2026, 11, 22).unwrap()
            );
        }
    }

    #[test]
    fn next_after_dst() {
        let zone: Tz = "Europe/Berlin".parse().unwrap();
        // 06:00 stays 06:00 local across the change of 2026-03-29 (02:00 became 03:00).
        let daily = parse("0 6 * * *").unwrap();
        let before = zone.with_ymd_and_hms(2026, 3, 28, 7, 0, 0).unwrap();
        let next = next_after_in(&daily, &before).unwrap();
        assert_eq!(
            (next.date_naive(), next.hour()),
            (NaiveDate::from_ymd_opt(2026, 3, 29).unwrap(), 6)
        );

        // 02:30 does not exist that day: it runs once, at 03:00.
        let gap = parse("30 2 * * *").unwrap();
        let before = zone.with_ymd_and_hms(2026, 3, 28, 12, 0, 0).unwrap();
        let next = next_after_in(&gap, &before).unwrap();
        assert_eq!(
            (next.date_naive(), next.hour()),
            (NaiveDate::from_ymd_opt(2026, 3, 29).unwrap(), 3)
        );

        // 02:30 happens twice on 2026-10-25; the schedule runs once.
        let repeat = parse("30 2 * * *").unwrap();
        let first = zone.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
        let once = next_after_in(&repeat, &first).unwrap();
        let again = next_after_in(&repeat, &once).unwrap();
        assert_eq!(
            once.date_naive(),
            NaiveDate::from_ymd_opt(2026, 10, 25).unwrap()
        );
        assert_eq!(
            again.date_naive(),
            NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()
        );
    }

    #[test]
    fn iso_text_of_the_api_reads_back() {
        let time = parse_iso("2026-11-21T09:30:00.250Z").unwrap();
        assert_eq!(
            rain_types::time::iso_from_system_time(time),
            "2026-11-21T09:30:00.250Z"
        );
        assert_eq!(parse_iso("yesterday"), None);
    }
}
