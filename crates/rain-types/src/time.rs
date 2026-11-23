//! Timestamps for the API, using only `std`. Foundation section 4.3.
//!
//! An instant (`ISODateTime`) is RFC 3339 in UTC, always with `Z` and exactly 3 fraction digits:
//! `2026-11-21T09:30:00.000Z`. Calendar dates in the user's local zone need the operating
//! system, so the backend produces them (`platform::time::local_date`), not this crate.

use std::time::{SystemTime, UNIX_EPOCH};

const MILLIS_PER_SECOND: i64 = 1_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// The current time as an `ISODateTime`. Backend code asks its `Clock` instead, so tests can
/// control time; this function is for code that has no clock (the shell, the hook, tools).
pub fn now_iso() -> String {
    iso_from_system_time(SystemTime::now())
}

/// Formats a `SystemTime` as an `ISODateTime`. Anything finer than a millisecond is cut off.
pub fn iso_from_system_time(time: SystemTime) -> String {
    let nanos_since_epoch: i128 = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_nanos() as i128,
        Err(before) => -(before.duration().as_nanos() as i128),
    };
    // `div_euclid` rounds toward the past, so a time before 1970 keeps a positive millisecond part.
    let millis = nanos_since_epoch.div_euclid(1_000_000) as i64;

    let seconds = millis.div_euclid(MILLIS_PER_SECOND);
    let millis_of_second = millis.rem_euclid(MILLIS_PER_SECOND);
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let seconds_of_day = seconds.rem_euclid(SECONDS_PER_DAY);

    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        seconds_of_day / 3600,
        seconds_of_day % 3600 / 60,
        seconds_of_day % 60,
    );
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis_of_second:03}Z"
    )
}

/// Converts a day count since 1970-01-01 into (year, month, day) of the proleptic Gregorian
/// calendar. This is Howard Hinnant's `civil_from_days` algorithm: it works on "eras" of 400
/// years, which have the same length (146097 days), so leap years need no special case.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468; // shift the epoch to 0000-03-01
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153; // 0 = March ... 11 = February
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at_millis_after_epoch(millis: u64) -> String {
        iso_from_system_time(UNIX_EPOCH + Duration::from_millis(millis))
    }

    #[test]
    fn the_epoch_is_formatted_with_three_fraction_digits() {
        assert_eq!(at_millis_after_epoch(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn a_known_instant_is_formatted_correctly() {
        // 1_700_000_000 seconds after the epoch is 2023-11-14 22:13:20 UTC.
        assert_eq!(
            at_millis_after_epoch(1_700_000_000_123),
            "2023-11-14T22:13:20.123Z"
        );
    }

    #[test]
    fn a_leap_day_is_formatted_correctly() {
        assert_eq!(
            at_millis_after_epoch(1_709_164_800_000),
            "2024-02-29T00:00:00.000Z"
        );
        assert_eq!(
            at_millis_after_epoch(1_709_251_199_999),
            "2024-02-29T23:59:59.999Z"
        );
        assert_eq!(
            at_millis_after_epoch(1_709_251_200_000),
            "2024-03-01T00:00:00.000Z"
        );
    }

    #[test]
    fn the_end_of_a_year_rolls_over() {
        // 2026-12-31 23:59:59.999 UTC, then one millisecond later.
        assert_eq!(
            at_millis_after_epoch(1_798_761_599_999),
            "2026-12-31T23:59:59.999Z"
        );
        assert_eq!(
            at_millis_after_epoch(1_798_761_600_000),
            "2027-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn sub_millisecond_parts_are_cut_off() {
        let time = UNIX_EPOCH + Duration::new(1, 999_999);
        assert_eq!(iso_from_system_time(time), "1970-01-01T00:00:01.000Z");
    }

    #[test]
    fn an_instant_before_1970_is_formatted_correctly() {
        let time = UNIX_EPOCH - Duration::from_millis(1);
        assert_eq!(iso_from_system_time(time), "1969-12-31T23:59:59.999Z");
        let time = UNIX_EPOCH - Duration::from_secs(86_400 * 365);
        assert_eq!(iso_from_system_time(time), "1969-01-01T00:00:00.000Z");
    }

    #[test]
    fn now_has_the_documented_shape() {
        let now = now_iso();
        assert_eq!(now.len(), "2026-11-21T09:30:00.000Z".len());
        assert!(now.ends_with('Z'));
        assert_eq!(&now[10..11], "T");
        assert_eq!(&now[19..20], ".");
    }
}
