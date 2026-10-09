//! Proleptic Gregorian calendar math in UTC, ported from `PineCalendar.swift`. Days are counted
//! from the Unix epoch using Howard Hinnant's civil-calendar algorithms (reusing `format.rs`).

use crate::format::{civil_from_days, days_from_civil};

const MILLIS_PER_DAY: i64 = 86_400_000;
const MAX_MILLIS: f64 = 9e18;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Components {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    /// Pine's `dayofweek`: Sunday is 1 … Saturday is 7.
    pub weekday: i64,
}

/// Milliseconds since the Unix epoch. Out-of-range months and days roll over, as in Pine.
/// None when the result would not fit an `i64`.
#[allow(clippy::too_many_arguments)]
pub fn make(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    offset_minutes: i64,
) -> Option<i64> {
    let month_index = month - 1;
    let y = year + month_index.div_euclid(12);
    let m = month_index.rem_euclid(12) + 1;
    let days = days_from_civil(y, m, 1) + (day - 1);
    if (days as f64 * MILLIS_PER_DAY as f64).abs() >= MAX_MILLIS {
        return None;
    }
    let seconds = ((days * 24 + hour) * 60 + minute - offset_minutes) * 60 + second;
    Some(seconds * 1000)
}

pub fn components(milliseconds: i64) -> Components {
    let days = milliseconds.div_euclid(MILLIS_PER_DAY);
    let seconds_of_day = milliseconds.div_euclid(1000).rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday; Sunday-based index 4.
    let weekday = (days + 4).rem_euclid(7) + 1;
    Components {
        year,
        month,
        day,
        hour: seconds_of_day / 3600,
        minute: seconds_of_day % 3600 / 60,
        second: seconds_of_day % 60,
        weekday,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_round_trips_through_components() {
        // 2023-11-14 22:13:20 UTC
        let ms = make(2023, 11, 14, 22, 13, 20, 0).unwrap();
        let c = components(ms);
        assert_eq!(
            (c.year, c.month, c.day, c.hour, c.minute, c.second),
            (2023, 11, 14, 22, 13, 20)
        );
        assert_eq!(c.weekday, 3); // Tuesday
    }

    #[test]
    fn month_and_day_roll_over() {
        assert_eq!(make(2023, 13, 1, 0, 0, 0, 0), make(2024, 1, 1, 0, 0, 0, 0));
        assert_eq!(make(2023, 0, 1, 0, 0, 0, 0), make(2022, 12, 1, 0, 0, 0, 0));
    }
}
