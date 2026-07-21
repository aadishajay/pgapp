//! Dependency-free Gregorian calendar math — just enough to lay out a
//! month grid (`render::calendar_html`) and default to "today"
//! (`server`'s Calendar dispatch) without pulling in `chrono`/`time`.
//! `days_from_civil`/`civil_from_days` are Howard Hinnant's well-known
//! constant-time algorithms (correct for the proleptic Gregorian
//! calendar across the full `i32` year range); everything else here is
//! built on top of those two.

/// Days since the Unix epoch (1970-01-01) for a given Gregorian
/// calendar date. Inverse of [`civil_from_days`].
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The Gregorian calendar date for a given day count since the Unix
/// epoch. Inverse of [`days_from_civil`].
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

pub fn is_leap_year(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Day of week for a Gregorian date: 0 = Sunday, ..., 6 = Saturday.
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    let days = days_from_civil(y, m, d);
    // 1970-01-01 (day 0) was a Thursday (weekday 4).
    (((days + 4) % 7 + 7) % 7) as u32
}

/// Today's (year, month, day) in UTC, from the system clock.
pub fn today_ymd() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_from_days((secs / 86400) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_roundtrips() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn known_dates_roundtrip() {
        for (y, m, d) in [(2026, 7, 21), (2000, 2, 29), (1999, 12, 31), (2024, 2, 29), (1600, 1, 1), (2400, 2, 29)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d), "roundtrip failed for {y}-{m}-{d}");
        }
    }

    #[test]
    fn weekday_matches_known_anchor() {
        // 1970-01-01 was a Thursday.
        assert_eq!(weekday(1970, 1, 1), 4);
        // 2000-01-01 was a Saturday.
        assert_eq!(weekday(2000, 1, 1), 6);
    }

    #[test]
    fn leap_year_days_in_month() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2026, 7), 31);
        assert_eq!(days_in_month(2026, 4), 30);
    }
}
