//! Days, the 14 day strip and the activity score. Section 02 6.3 and 6.4. All pure.
//!
//! "A day" is a local calendar day: the strip is "per day" for the user. A day is handled as its
//! number (days since 1970-01-01), so that no date text has to be parsed in the middle of a
//! calculation. The UTC offset of a moment comes from the caller (`platform::time` in
//! production, a fixed offset in a test), because it changes with daylight saving time.

use crate::platform::time::days_from_civil;

/// How many days the strip has.
pub const STRIP_DAYS: usize = 14;

/// `YYYY-MM-DD` as a day number. `None` for text of another shape.
pub fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// An `ISODateTime` (`2026-11-21T09:30:00.000Z`) as Unix milliseconds. `None` for other text.
pub fn iso_to_unix_ms(text: &str) -> Option<i64> {
    let (date, time) = text.trim_end_matches('Z').split_once('T')?;
    let days = parse_date(date)?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut parts = clock.split(':');
    let hour: i64 = parts.next()?.parse().ok()?;
    let minute: i64 = parts.next()?.parse().ok()?;
    let second: i64 = parts.next()?.parse().ok()?;
    let millis: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
    Some(((days * 86_400) + hour * 3600 + minute * 60 + second) * 1000 + millis)
}

/// The local day of a Unix time, given the offset from UTC that holds at that moment.
pub fn local_day(unix_seconds: i64, offset_seconds: i64) -> i64 {
    (unix_seconds + offset_seconds).div_euclid(86_400)
}

/// Counts the commit times into the 14 days that end on `today` (a day number). Index 13 is
/// today. A time before the window or after today is dropped. `offset_at` gives the UTC offset
/// at a Unix time.
pub fn bucket(times: &[i64], today: i64, offset_at: &dyn Fn(i64) -> i64) -> Vec<u32> {
    let mut counts = vec![0u32; STRIP_DAYS];
    for &time in times {
        let age = today - local_day(time, offset_at(time));
        if (0..STRIP_DAYS as i64).contains(&age) {
            counts[STRIP_DAYS - 1 - age as usize] += 1;
        }
    }
    counts
}

/// The strip of an earlier day moved to today: it shifts left by the days that have passed, and
/// the new days are zeros. Right only when no commit can have been made since (the caller has
/// checked that the HEAD files are unchanged).
pub fn shift(counts: &[u32], days_passed: i64) -> Vec<u32> {
    if days_passed <= 0 {
        return counts.to_vec();
    }
    let passed = usize::try_from(days_passed)
        .unwrap_or(STRIP_DAYS)
        .min(STRIP_DAYS);
    let mut moved: Vec<u32> = counts.iter().skip(passed).copied().collect();
    moved.resize(STRIP_DAYS, 0);
    moved
}

/// `round(sum of counts[i] * (i + 1) / 14)`: today counts fully, 13 days ago a fourteenth. The
/// formula of the prototype, so that Home and Projects order as before.
pub fn activity_score(counts: &[u32]) -> u32 {
    let weighted: u64 = counts
        .iter()
        .enumerate()
        .map(|(index, &count)| u64::from(count) * (index as u64 + 1))
        .sum();
    // Rounding half up in integers.
    ((weighted * 2 + STRIP_DAYS as u64) / (STRIP_DAYS as u64 * 2)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    #[test]
    fn civil_dates_become_day_numbers() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(iso_to_unix_ms("1970-01-02T00:00:01.250Z"), Some(86_401_250));
        assert_eq!(iso_to_unix_ms("junk"), None);
        assert_eq!(
            parse_date("2026-11-22"),
            Some(days_from_civil(2026, 11, 22))
        );
        assert_eq!(parse_date("2026-02-30x"), None);
        assert_eq!(parse_date("nonsense"), None);
        assert_eq!(parse_date("2026-13-01"), None);
    }

    #[test]
    fn local_day_follows_the_offset() {
        // 23:59 and 00:00 at UTC+2 are two different local days, one minute apart.
        let midnight_local = 10 * DAY - 2 * 3600; // 00:00 of day 10 at UTC+2
        assert_eq!(local_day(midnight_local - 60, 7200), 9);
        assert_eq!(local_day(midnight_local, 7200), 10);
        assert_eq!(local_day(-1, 0), -1);
    }

    #[test]
    fn bucket_puts_times_into_their_local_days() {
        let offset = |_: i64| 7200;
        let today = 100;
        let at = |day: i64, hour: i64| day * DAY + hour * 3600 - 7200;
        let times = [
            at(100, 0),  // today, 00:00 local
            at(100, 23), // today
            at(99, 23),  // yesterday, 23:00 local
            at(87, 5),   // 13 days ago: the first bar
            at(86, 5),   // 14 days ago: dropped
            at(101, 1),  // tomorrow: dropped
        ];
        let counts = bucket(&times, today, &offset);
        assert_eq!(counts.len(), 14);
        assert_eq!(counts[13], 2);
        assert_eq!(counts[12], 1);
        assert_eq!(counts[0], 1);
        assert_eq!(counts.iter().sum::<u32>(), 4);
    }

    #[test]
    fn bucket_survives_a_change_of_offset() {
        // The offset is -1 h before 01:00 UTC of day 50 and 0 after: each time uses its own.
        let offset = |time: i64| if time < 50 * DAY + 3600 { -3600 } else { 0 };
        let before = 50 * DAY + 1800; // 00:30 UTC = 23:30 local of day 49
        let after = 50 * DAY + 7200; // 02:00 UTC = 02:00 local of day 50
        let counts = bucket(&[before, after], 50, &offset);
        assert_eq!((counts[12], counts[13]), (1, 1));
        assert_eq!(bucket(&[], 50, &offset), vec![0; 14]);
    }

    #[test]
    fn shift_moves_the_strip_left_and_fills_with_zeros() {
        let counts: Vec<u32> = (1..=14).collect();
        assert_eq!(shift(&counts, 0), counts);
        let moved = shift(&counts, 2);
        assert_eq!(&moved[..12], &counts[2..]);
        assert_eq!(&moved[12..], &[0, 0]);
        assert_eq!(shift(&counts, 40), vec![0; 14]);
        assert_eq!(shift(&counts, -3), counts);
    }

    #[test]
    fn the_score_is_the_formula_of_the_prototype() {
        assert_eq!(activity_score(&[0; 14]), 0);
        let mut today = [0u32; 14];
        today[13] = 1;
        assert_eq!(activity_score(&today), 1);
        let mut old = [0u32; 14];
        old[0] = 1;
        assert_eq!(activity_score(&old), 0);
        // 3 commits today and 2 a week ago: (3 * 14 + 2 * 7) / 14 = 4.
        let mut mixed = [0u32; 14];
        mixed[13] = 3;
        mixed[6] = 2;
        assert_eq!(activity_score(&mixed), 4);
        // 1 commit at index 6 (weight 7): 7 / 14 = 0.5 rounds up to 1.
        let mut half = [0u32; 14];
        half[6] = 1;
        assert_eq!(activity_score(&half), 1);
    }
}
