//! Calendar arithmetic for the daily notes: pure, std only, no time zone. Section 04 6.13.
//!
//! "Today" comes from the clock and `platform::time::local_date` as text; this file turns that
//! text into a `CivilDate` and does the sums (days, Monday weeks, months). A day is a calendar
//! date, so a daylight saving change cannot disturb any of it.

use std::fmt;

/// A date in the proleptic Gregorian calendar. The field order makes the derived ordering the
/// calendar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    pub y: i32,
    pub m: u8,
    pub d: u8,
}

impl fmt::Display for CivilDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap_year(year) => 29,
        _ => 28,
    }
}

impl CivilDate {
    /// Only a real date in the form `YYYY-MM-DD`: exactly ten characters, four digits of year
    /// from 1970 on (the notes of a person are never older), a real month and a real day.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<u32> {
            let part = &bytes[range];
            if part.iter().all(u8::is_ascii_digit) {
                std::str::from_utf8(part).ok()?.parse().ok()
            } else {
                None
            }
        };
        let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        let y = i32::try_from(y).ok()?;
        if !(1970..=9999).contains(&y) || !(1..=12).contains(&m) {
            return None;
        }
        let (m, d) = (m as u8, d as u8);
        (1..=days_in_month(y, m))
            .contains(&d)
            .then_some(Self { y, m, d })
    }

    /// Days since 1970-01-01 (negative before). Howard Hinnant's days-from-civil algorithm.
    pub fn to_days(self) -> i64 {
        let y = i64::from(self.y) - i64::from(self.m <= 2);
        let era = y.div_euclid(400);
        let year_of_era = y.rem_euclid(400);
        let month_index = (i64::from(self.m) + 9) % 12; // March = 0
        let day_of_year = (153 * month_index + 2) / 5 + i64::from(self.d) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// The inverse of `to_days`.
    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_index = (5 * day_of_year + 2) / 153;
        let d = (day_of_year - (153 * month_index + 2) / 5 + 1) as u8;
        let m = if month_index < 10 {
            month_index + 3
        } else {
            month_index - 9
        } as u8;
        let y = year_of_era + era * 400 + i64::from(m <= 2);
        Self { y: y as i32, m, d }
    }

    pub fn add_days(self, n: i64) -> Self {
        Self::from_days(self.to_days() + n)
    }

    /// `other - self` in days.
    pub fn days_until(self, other: Self) -> i64 {
        other.to_days() - self.to_days()
    }

    /// Monday = 0 ... Sunday = 6. 1970-01-01 was a Thursday.
    pub fn weekday(self) -> u8 {
        (self.to_days() + 3).rem_euclid(7) as u8
    }

    pub fn monday_of(self) -> Self {
        self.add_days(-i64::from(self.weekday()))
    }

    pub fn first_of_month(self) -> Self {
        Self { d: 1, ..self }
    }

    pub fn first_of_year(self) -> Self {
        Self { m: 1, d: 1, ..self }
    }

    /// Adds calendar months and clamps the day to the end of the target month
    /// (31 January + 1 month = 28 or 29 February).
    pub fn add_months(self, n: i32) -> Self {
        let index = self.y * 12 + i32::from(self.m) - 1 + n;
        let (y, m) = (index.div_euclid(12), (index.rem_euclid(12) + 1) as u8);
        Self {
            y,
            m,
            d: self.d.min(days_in_month(y, m)),
        }
    }

    /// The ISO week number (1 to 53) and the year that week belongs to.
    pub fn iso_week_and_year(self) -> (u32, i32) {
        // The Thursday of this week decides the year.
        let thursday = self.add_days(3 - i64::from(self.weekday()));
        let first = thursday.first_of_year();
        let week = (first.days_until(thursday) / 7 + 1) as u32;
        (week, thursday.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> CivilDate {
        CivilDate::parse(text).expect("a real date")
    }

    #[test]
    fn parse_accepts_real_dates_only() {
        assert_eq!(date("2026-11-22").to_string(), "2026-11-22");
        assert_eq!(date("2024-02-29").to_string(), "2024-02-29");
        for bad in [
            "2026-02-30",
            "2025-02-29",
            "2100-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-11-00",
            "26-11-22",
            "2026-1-22",
            "2026/11/22",
            "2026-11-22 ",
            "",
            "abcd-ef-gh",
            "+026-11-22",
            "1969-12-31",
        ] {
            assert_eq!(CivilDate::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn days_round_trip_and_match_known_values() {
        assert_eq!(date("1970-01-01").to_days(), 0);
        assert_eq!(date("2000-03-01").to_days(), 11_017);
        assert_eq!(date("2026-11-22").to_days(), 20_779);
        for days in (0..40_000).step_by(37) {
            assert_eq!(CivilDate::from_days(days).to_days(), days);
        }
    }

    #[test]
    fn add_days_crosses_months_years_and_leap_days() {
        assert_eq!(date("2024-02-28").add_days(1), date("2024-02-29"));
        assert_eq!(date("2024-02-29").add_days(1), date("2024-03-01"));
        assert_eq!(date("2100-02-28").add_days(1), date("2100-03-01"));
        assert_eq!(date("2026-12-31").add_days(1), date("2027-01-01"));
        assert_eq!(date("2027-01-01").add_days(-1), date("2026-12-31"));
        assert_eq!(date("2026-11-22").add_days(-799), date("2024-09-14"));
        assert_eq!(date("2026-11-22").add_days(-30), date("2026-10-23"));
    }

    #[test]
    fn weekdays_and_mondays() {
        assert_eq!(date("2026-11-22").weekday(), 6); // a Sunday
        assert_eq!(date("2026-11-23").weekday(), 0);
        assert_eq!(date("2026-11-22").monday_of(), date("2026-11-16"));
        assert_eq!(date("2026-11-16").monday_of(), date("2026-11-16"));
        assert_eq!(date("2021-01-03").monday_of(), date("2020-12-28"));
    }

    #[test]
    fn iso_weeks_including_week_53_and_the_year_border() {
        assert_eq!(date("2021-01-03").iso_week_and_year(), (53, 2020));
        assert_eq!(date("2020-12-31").iso_week_and_year(), (53, 2020));
        assert_eq!(date("2026-01-01").iso_week_and_year(), (1, 2026));
        assert_eq!(date("2024-12-30").iso_week_and_year(), (1, 2025));
        assert_eq!(date("2026-11-03").iso_week_and_year(), (45, 2026));
    }

    #[test]
    fn months_clamp_to_the_end_of_the_target_month() {
        assert_eq!(date("2026-01-31").add_months(1), date("2026-02-28"));
        assert_eq!(date("2024-01-31").add_months(1), date("2024-02-29"));
        assert_eq!(date("2026-11-22").add_months(-11), date("2025-12-22"));
        assert_eq!(date("2026-11-22").add_months(2), date("2027-01-22"));
        assert_eq!(date("2026-03-31").add_months(-1), date("2026-02-28"));
    }

    #[test]
    fn ordering_is_calendar_order() {
        assert!(date("2026-11-22") > date("2026-11-09"));
        assert!(date("2027-01-01") > date("2026-12-31"));
    }
}
