//! A Pine session string such as `"0930-1600"`, `"1800-0600"` (overnight),
//! `"0930-1130,1300-1600"` or `"0930-1600:23456"` (Monday to Friday; Sunday is 1). `"24x7"` is
//! always open. A port of `Builtins/PineSession.swift`.

use super::calendar::Components;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Window {
    start: u32,
    end: u32,
    days: BTreeSet<u32>,
}

impl Window {
    fn contains(&self, minute: u32, weekday: u32) -> bool {
        if self.start == self.end {
            return self.days.contains(&weekday);
        }
        if self.start < self.end {
            return minute >= self.start && minute < self.end && self.days.contains(&weekday);
        }
        if minute >= self.start {
            return self.days.contains(&weekday);
        }
        // The after-midnight part of an overnight window belongs to the day it opened on.
        minute < self.end
            && self
                .days
                .contains(&if weekday == 1 { 7 } else { weekday - 1 })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradingSession {
    windows: Vec<Window>,
}

fn all_days() -> BTreeSet<u32> {
    (1..=7).collect()
}

impl TradingSession {
    /// None when the text is not a session.
    pub fn parse(text: &str) -> Option<Self> {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.eq_ignore_ascii_case("24x7") {
            return Some(Self {
                windows: vec![Window {
                    start: 0,
                    end: 0,
                    days: all_days(),
                }],
            });
        }
        let windows: Option<Vec<Window>> = compact.split(',').map(window).collect();
        windows
            .filter(|windows| !windows.is_empty())
            .map(|windows| Self { windows })
    }

    /// Whether the moment described by `components` falls inside the session.
    pub fn contains(&self, components: &Components) -> bool {
        let minute = (components.hour * 60 + components.minute) as u32;
        let weekday = components.weekday as u32;
        self.windows
            .iter()
            .any(|window| window.contains(minute, weekday))
    }
}

fn window(text: &str) -> Option<Window> {
    let sides: Vec<&str> = text.split(':').collect();
    if sides.len() > 2 {
        return None;
    }
    let (start, end) = sides[0].split_once('-')?;
    let (start, end) = (minute_of_day(start)?, minute_of_day(end)?);
    let mut days = all_days();
    if let Some(digits) = sides.get(1) {
        let parsed: Option<BTreeSet<u32>> = digits
            .chars()
            .map(|c| c.to_digit(10).filter(|d| (1..=7).contains(d)))
            .collect();
        days = parsed.filter(|set| !digits.is_empty() && !set.is_empty())?;
    }
    Some(Window { start, end, days })
}

fn minute_of_day(text: &str) -> Option<u32> {
    if text.len() != 4 || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: u32 = text.parse().ok()?;
    let (hour, minute) = (value / 100, value % 100);
    (minute < 60 && hour <= 24 && (hour < 24 || minute == 0)).then_some(hour * 60 + minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: i64, minute: i64, weekday: i64) -> Components {
        Components {
            year: 2024,
            month: 3,
            day: 5,
            hour,
            minute,
            second: 0,
            weekday,
        }
    }

    fn session(text: &str) -> TradingSession {
        TradingSession::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn a_plain_window_is_half_open() {
        let regular = session("0930-1600");
        assert!(!regular.contains(&at(9, 29, 3)));
        assert!(regular.contains(&at(9, 30, 3)));
        assert!(regular.contains(&at(15, 59, 3)));
        assert!(
            !regular.contains(&at(16, 0, 3)),
            "the end minute is outside"
        );
        assert!(
            regular.contains(&at(12, 0, 1)),
            "every day unless restricted"
        );
    }

    #[test]
    fn days_restrict_a_window() {
        let weekdays = session("0930-1600:23456");
        assert!(weekdays.contains(&at(10, 0, 2)), "Monday");
        assert!(weekdays.contains(&at(10, 0, 6)), "Friday");
        assert!(!weekdays.contains(&at(10, 0, 1)), "Sunday is 1");
        assert!(!weekdays.contains(&at(10, 0, 7)), "Saturday");
    }

    #[test]
    fn an_overnight_window_spills_into_the_next_day() {
        let overnight = session("1800-0600:23456");
        assert!(overnight.contains(&at(20, 0, 2)), "Monday evening");
        assert!(
            overnight.contains(&at(3, 0, 3)),
            "Tuesday 03:00 belongs to Monday's window"
        );
        assert!(
            !overnight.contains(&at(3, 0, 2)),
            "Monday 03:00 would need Sunday's window"
        );
        assert!(
            overnight.contains(&at(3, 0, 7)),
            "Saturday 03:00 belongs to Friday's window"
        );
        assert!(!overnight.contains(&at(12, 0, 3)));
        let sunday_wraps = session("2200-0200:7");
        assert!(
            sunday_wraps.contains(&at(1, 0, 1)),
            "Sunday 01:00 belongs to Saturday's window"
        );
    }

    #[test]
    fn several_windows_and_the_always_open_session() {
        let split = session("0930-1130,1300-1600");
        assert!(split.contains(&at(10, 0, 3)) && split.contains(&at(14, 0, 3)));
        assert!(!split.contains(&at(12, 0, 3)));
        let always = session("24x7");
        assert!(always.contains(&at(0, 0, 1)) && always.contains(&at(23, 59, 7)));
        assert!(
            session(" 24X7 ").contains(&at(5, 0, 4)),
            "case and spaces are ignored"
        );
        let midnight = session("0000-2400");
        assert!(midnight.contains(&at(23, 59, 4)));
        let all_day = session("0900-0900");
        assert!(all_day.contains(&at(3, 0, 4)), "equal ends mean all day");
    }

    #[test]
    fn malformed_sessions_are_rejected() {
        for bad in [
            "",
            "0930",
            "0930-",
            "09:30-16:00",
            "0930-1600:",
            "0930-1600:8",
            "0930-1600:0",
            "0930-1600:12a",
            "2500-0100",
            "0960-1000",
            "2401-2402",
            "930-1600",
            "0930-1600:1:2",
            "0930-1130,",
            "abcd-efgh",
        ] {
            assert!(TradingSession::parse(bad).is_none(), "{bad:?}");
        }
    }
}
