//! The calendar date in the user's time zone. Foundation sections 4.3 and 11.6.
//!
//! The API carries instants in UTC (`rain_types::time`, no operating system call). A calendar
//! date (`ISODate`, a habit day, "has this job run today") depends on the zone of the PC, so
//! it is computed here and never by the frontend. Windows knows the zone and its daylight saving
//! rules; the calendar arithmetic is pure and tested on its own.

use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;

const SECONDS_PER_DAY: i64 = 86_400;

/// The local date of an instant: `2026-11-21`.
pub fn local_date(time: SystemTime) -> String {
    local_date_of_unix(unix_seconds(time))
}

/// The local date of a Unix time in seconds.
pub fn local_date_of_unix(unix_seconds: i64) -> String {
    date_with_offset(unix_seconds, utc_offset_seconds(unix_seconds))
}

/// The date that a clock set to `offset_seconds` east of UTC shows at `unix_seconds`. Pure: the
/// tests and the git history of a project use it with a chosen offset.
pub fn date_with_offset(unix_seconds: i64, offset_seconds: i64) -> String {
    let days = (unix_seconds + offset_seconds).div_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Seconds east of UTC that the PC's time zone has at this instant (including daylight saving).
/// 0 when Windows cannot say.
pub fn utc_offset_seconds(unix_seconds: i64) -> i64 {
    let days = unix_seconds.div_euclid(SECONDS_PER_DAY);
    let seconds_of_day = unix_seconds.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let Ok(year) = u16::try_from(year) else {
        return 0; // before year 0 or after 65535: outside what Windows handles
    };
    let utc = SYSTEMTIME {
        wYear: year,
        wMonth: month as u16,
        wDayOfWeek: 0, // ignored by the call
        wDay: day as u16,
        wHour: (seconds_of_day / 3600) as u16,
        wMinute: (seconds_of_day % 3600 / 60) as u16,
        wSecond: (seconds_of_day % 60) as u16,
        wMilliseconds: 0,
    };
    let mut local = SYSTEMTIME::default();
    // SAFETY: both pointers are valid for the call; no time zone structure means "the zone of
    // this PC".
    if unsafe { SystemTimeToTzSpecificLocalTime(None, &utc, &mut local) }.is_err() {
        return 0;
    }
    let local_days = days_from_civil(
        i64::from(local.wYear),
        i64::from(local.wMonth),
        i64::from(local.wDay),
    );
    let local_seconds = local_days * SECONDS_PER_DAY
        + i64::from(local.wHour) * 3600
        + i64::from(local.wMinute) * 60
        + i64::from(local.wSecond);
    local_seconds - unix_seconds
}

fn unix_seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    }
}

// The two functions below are the days-from-civil algorithm of Howard Hinnant (proleptic
// Gregorian calendar), which needs no table and no loop.

/// Days since 1970-01-01 of a calendar date.
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The calendar date of a number of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
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

    #[test]
    fn known_dates_convert_both_ways() {
        let cases = [
            (0, (1970, 1, 1)),
            (59, (1970, 3, 1)),
            (365, (1971, 1, 1)),
            (11_016, (2000, 2, 29)), // a leap day in a year divisible by 400
            (20_392, (2025, 10, 31)),
            (-1, (1969, 12, 31)),
            (-719_468, (0, 3, 1)),
        ];
        for (days, date) in cases {
            assert_eq!(civil_from_days(days), date, "days {days}");
            assert_eq!(
                days_from_civil(date.0, date.1, date.2),
                days,
                "date {date:?}"
            );
        }
    }

    #[test]
    fn every_day_of_four_years_round_trips() {
        for days in 10_957..10_957 + 4 * 366 {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn a_chosen_offset_moves_the_date_across_midnight() {
        // 2026-11-21 23:30 UTC.
        let instant = days_from_civil(2026, 11, 21) * SECONDS_PER_DAY + 23 * 3600 + 30 * 60;
        assert_eq!(date_with_offset(instant, 0), "2026-11-21");
        assert_eq!(
            date_with_offset(instant, 3600),
            "2026-11-22",
            "UTC+1 is past midnight"
        );
        assert_eq!(date_with_offset(instant, -5 * 3600), "2026-11-21");
        assert_eq!(date_with_offset(instant - 24 * 3600, 3600), "2026-11-21");
    }

    #[test]
    fn a_date_before_1970_is_formatted() {
        assert_eq!(date_with_offset(-86_400, 0), "1969-12-31");
    }

    #[test]
    fn the_local_date_is_within_a_day_of_the_utc_date() {
        let now = SystemTime::now();
        let utc = date_with_offset(unix_seconds(now), 0);
        let local = local_date(now);
        let to_days = |date: &str| {
            let part = |range: std::ops::Range<usize>| date[range].parse::<i64>().unwrap();
            days_from_civil(part(0..4), part(5..7), part(8..10))
        };
        assert!(
            (to_days(&utc) - to_days(&local)).abs() <= 1,
            "{utc} against {local}"
        );
    }

    #[test]
    fn the_offset_of_the_pc_is_a_plausible_zone() {
        let offset = utc_offset_seconds(unix_seconds(SystemTime::now()));
        assert!((-12 * 3600..=14 * 3600).contains(&offset), "{offset}");
        // Zones are a multiple of 15 minutes.
        assert_eq!(offset % (15 * 60), 0);
    }
}
