//! Diary days as local calendar dates (`YYYY-MM-DD`), and the month key a day is filed under.
//!
//! Never use `model::today()` for tracker data: it is the UTC day, so a 23:30 meal in the evening
//! west of Greenwich would be filed under tomorrow.

use crate::model::{civil_from_days, days_from_civil};

/// A valid local calendar date, `YYYY-MM-DD`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Day(String);

impl Day {
    /// Exactly `YYYY-MM-DD` and a real calendar date.
    pub fn parse(s: &str) -> Result<Day, String> {
        let bad = || format!("\"{s}\" isn't a valid date (use YYYY-MM-DD).");
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
            return Err(bad());
        }
        let (y, m, d): (i32, u32, u32) = (s[..4].parse().map_err(|_| bad())?, s[5..7].parse().map_err(|_| bad())?, s[8..].parse().map_err(|_| bad())?);
        // a date that rolls over (February 30) comes back as a different date
        if !(1..=12).contains(&m) || d == 0 || civil_from_days(days_from_civil(y, m, d)) != (y, m, d) {
            return Err(bad());
        }
        Ok(Day(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The `YYYY-MM` key of the month file this day belongs to.
    pub fn month(&self) -> &str {
        &self.0[..7]
    }
}

/// The local calendar day of a UTC time, given the local offset in minutes east of UTC (for
/// example -300 for US Eastern standard time). The webview supplies the offset from the local date
/// parts (never `toISOString`); this function exists so the 23:30 and DST rules are testable
/// without a time-zone crate.
pub fn local_day(utc_secs: i64, utc_offset_minutes: i32) -> Day {
    let days = (utc_secs + utc_offset_minutes as i64 * 60).div_euclid(86400);
    let (y, m, d) = civil_from_days(days as i32);
    Day(format!("{y:04}-{m:02}-{d:02}"))
}

/// Exactly `YYYY-MM` with a month from 01 to 12, so a key can be used in a file name safely.
pub fn is_month_key(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7 && b[4] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || c.is_ascii_digit()) && (1..=12).contains(&s[5..].parse::<u32>().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, m: u32, d: u32, hh: i64, mm: i64) -> i64 {
        days_from_civil(y, m, d) as i64 * 86400 + hh * 3600 + mm * 60
    }

    #[test]
    fn day_23_30_files_on_the_local_day() {
        // 23:30 local on 13 November at UTC-5 is already 14 November in UTC
        let t = utc(2026, 11, 14, 4, 30);
        assert_eq!(local_day(t, -300).as_str(), "2026-11-13");
        assert_eq!(local_day(t, -300).month(), "2026-11");
        // 23:30 on 31 January belongs to January's file, not February's
        let jan = local_day(utc(2026, 2, 1, 4, 30), -300);
        assert_eq!((jan.as_str(), jan.month()), ("2026-01-31", "2026-01"));
        // east of UTC the local day can be ahead of the UTC day
        assert_eq!(local_day(utc(2026, 11, 13, 20, 0), 330).as_str(), "2026-11-14");
        // before 1970 still floors
        assert_eq!(local_day(-1, 0).as_str(), "1969-12-31");
    }

    #[test]
    fn day_across_dst_forward_and_back() {
        // spring forward 2026-03-08: 23:30 local is day 2026-03-08 either side of the change
        assert_eq!(local_day(utc(2026, 3, 9, 4, 30), -300).as_str(), "2026-03-08");
        assert_eq!(local_day(utc(2026, 3, 9, 3, 30), -240).as_str(), "2026-03-08");
        // fall back 2026-11-01: 01:30 happens twice and both are the same day
        assert_eq!(local_day(utc(2026, 11, 1, 5, 30), -240).as_str(), "2026-11-01");
        assert_eq!(local_day(utc(2026, 11, 1, 6, 30), -300).as_str(), "2026-11-01");
    }

    #[test]
    fn parse_day_rejects_bad_dates() {
        for bad in ["2026-02-30", "2026-13-01", "2026-00-10", "2026-01-00", "26-1-1", "../x", "2026-1-01", "2026/01/01", "", "2026-01-011", "+026-01-01"] {
            let e = Day::parse(bad).unwrap_err();
            assert!(e.contains("isn't a valid date"), "{bad}: {e}");
        }
        assert_eq!(Day::parse("2028-02-29").unwrap().month(), "2028-02");
        assert!(Day::parse("2026-02-29").is_err());
        assert!(is_month_key("2026-11") && is_month_key("2026-01"));
        for bad in ["2026-13", "2026-00", "2026-1", "../x", "2026-11-01", "2026-1a", "abcd-ef"] {
            assert!(!is_month_key(bad), "{bad}");
        }
    }
}
