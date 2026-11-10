//! The scheduler (SPECIFICATION.md §11.1): starts and stops the queue at set times on chosen days, and what to do
//! once the queue is done. Times are the computer's local time; the list checks them every second.

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

/// What happens once the queue has nothing left to download (Q4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenDone {
    #[default]
    Nothing,
    Quit,
    Sleep,
    ShutDown,
}

impl WhenDone {
    pub fn from_word(word: &str) -> Option<WhenDone> {
        Some(match word {
            "nothing" => WhenDone::Nothing,
            "quit" => WhenDone::Quit,
            "sleep" => WhenDone::Sleep,
            "shut-down" | "shut_down" => WhenDone::ShutDown,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        match self {
            WhenDone::Nothing => "nothing",
            WhenDone::Quit => "quit",
            WhenDone::Sleep => "sleep",
            WhenDone::ShutDown => "shut-down",
        }
    }
}

/// Kept in `settings.json` under `schedule`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    /// Q1: minutes after midnight; `None` is off.
    pub start: Option<u32>,
    /// Q2: minutes after midnight; `None` is off. A stop time no later than the start time is the next morning's.
    pub stop: Option<u32>,
    /// Q3, Monday first: the days the queue starts (or, with no start time, stops).
    pub days: [bool; 7],
    pub when_done: WhenDone,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule { start: None, stop: None, days: [true; 7], when_done: WhenDone::Nothing }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Start Queue.
    Start,
    /// Stop Queue, and the running downloads go back to waiting in the queue.
    Stop,
}

const DAY: i64 = 86_400;

/// The time `minutes` after midnight on `day`, local time. A time skipped by the clock going forward is an hour later.
fn at(day: NaiveDate, minutes: u32) -> Option<i64> {
    let time = day.and_hms_opt(minutes / 60, minutes % 60, 0)?;
    Local
        .from_local_datetime(&time)
        .earliest()
        .or_else(|| Local.from_local_datetime(&(time + chrono::Duration::hours(1))).earliest())
        .map(|t| t.timestamp())
}

fn local_date(time: i64) -> NaiveDate {
    Local.timestamp_opt(time, 0).earliest().map(|t| t.date_naive()).unwrap_or_default()
}

impl Schedule {
    pub fn clamped(mut self) -> Self {
        self.start = self.start.filter(|m| *m < 24 * 60);
        self.stop = self.stop.filter(|m| *m < 24 * 60);
        self
    }

    /// Whether it starts or stops anything at all.
    pub fn is_on(&self) -> bool {
        (self.start.is_some() || self.stop.is_some()) && self.days.contains(&true)
    }

    /// The events that belong to `day` (none if it isn't a chosen day); a stop may fall on the next day.
    fn events_of(&self, day: NaiveDate) -> Vec<(i64, Event)> {
        if !self.days[day.weekday().num_days_from_monday() as usize] {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(t) = self.start.and_then(|m| at(day, m)) {
            events.push((t, Event::Start));
        }
        if let Some(stop) = self.stop {
            let overnight = self.start.is_some_and(|start| stop <= start);
            let stop_day = if overnight { day.succ_opt() } else { Some(day) };
            if let Some(t) = stop_day.and_then(|d| at(d, stop)) {
                events.push((t, Event::Stop));
            }
        }
        events
    }

    fn events_around(&self, from: i64, to: i64) -> Vec<(i64, Event)> {
        let (mut day, last) = (local_date(from).pred_opt().unwrap_or_default(), local_date(to));
        let mut events = Vec::new();
        while day <= last {
            events.extend(self.events_of(day));
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        events
    }

    /// The latest event after `after` and no later than `upto` (looking back at most a week): what the queue should
    /// do now, after the app was closed or the computer slept through some events.
    pub fn last_event(&self, after: i64, upto: i64) -> Option<(i64, Event)> {
        if !self.is_on() || upto <= after {
            return None;
        }
        let after = after.max(upto - 8 * DAY);
        self.events_around(after, upto).into_iter().filter(|(t, _)| *t > after && *t <= upto).max_by_key(|(t, _)| *t)
    }

    /// The first event after `from`.
    pub fn next_event(&self, from: i64) -> Option<(i64, Event)> {
        if !self.is_on() {
            return None;
        }
        self.events_around(from, from + 8 * DAY).into_iter().filter(|(t, _)| *t > from).min_by_key(|(t, _)| *t)
    }

    /// The footer's words for the next event: "starts at 01:00", or "stops Tue at 07:00" when it isn't today.
    pub fn next_text(&self, from: i64) -> Option<String> {
        let (t, event) = self.next_event(from)?;
        let when = Local.timestamp_opt(t, 0).earliest()?;
        let verb = if event == Event::Start { "starts" } else { "stops" };
        Some(if when.date_naive() == local_date(from) {
            format!("{verb} at {}", when.format("%H:%M"))
        } else {
            format!("{verb} {} at {}", when.format("%a"), when.format("%H:%M"))
        })
    }
}

/// "01:30" → minutes after midnight.
pub fn parse_time(text: &str) -> Option<u32> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

pub fn time_text(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 9 November 2026.
    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()
    }

    fn t(day_offset: i64, hh: u32, mm: u32) -> i64 {
        at(monday() + chrono::Duration::days(day_offset), hh * 60 + mm).unwrap()
    }

    fn only(days: &[usize]) -> [bool; 7] {
        let mut d = [false; 7];
        for &i in days {
            d[i] = true;
        }
        d
    }

    #[test]
    fn a_window_starts_and_stops_on_its_days() {
        let s = Schedule { start: Some(60), stop: Some(7 * 60), days: only(&[0]), ..Schedule::default() };
        assert_eq!(s.last_event(t(0, 0, 0), t(0, 2, 0)), Some((t(0, 1, 0), Event::Start)));
        assert_eq!(s.last_event(t(0, 0, 0), t(0, 8, 0)), Some((t(0, 7, 0), Event::Stop)), "the latest one wins");
        assert_eq!(s.last_event(t(0, 1, 0), t(0, 2, 0)), None, "an event already seen doesn't fire again");
        assert_eq!(s.last_event(t(0, 8, 0), t(1, 23, 0)), None, "Tuesday isn't chosen");
        assert_eq!(s.next_event(t(0, 8, 0)), Some((t(7, 1, 0), Event::Start)), "next Monday");
    }

    #[test]
    fn a_stop_before_the_start_is_the_next_morning() {
        let s = Schedule { start: Some(23 * 60), stop: Some(6 * 60), days: only(&[0]), ..Schedule::default() };
        assert_eq!(s.last_event(t(0, 22, 0), t(0, 23, 30)), Some((t(0, 23, 0), Event::Start)));
        assert_eq!(s.last_event(t(0, 23, 30), t(1, 6, 30)), Some((t(1, 6, 0), Event::Stop)), "Tuesday morning");
        assert_eq!(s.last_event(t(-1, 0, 0), t(0, 6, 30)), None, "Sunday wasn't chosen, so no stop on Monday morning");
    }

    #[test]
    fn a_stop_alone_or_nothing_at_all() {
        let s = Schedule { stop: Some(9 * 60), ..Schedule::default() };
        assert_eq!(s.last_event(t(2, 8, 0), t(2, 10, 0)), Some((t(2, 9, 0), Event::Stop)));
        assert_eq!(Schedule::default().last_event(t(0, 0, 0), t(3, 0, 0)), None);
        let no_days = Schedule { start: Some(60), days: [false; 7], ..Schedule::default() };
        assert!(!no_days.is_on());
        assert_eq!(no_days.next_event(t(0, 0, 0)), None);
    }

    #[test]
    fn a_long_gap_looks_back_a_week_at_most() {
        let s = Schedule { start: Some(60), ..Schedule::default() };
        assert_eq!(s.last_event(0, t(0, 2, 0)), Some((t(0, 1, 0), Event::Start)));
    }

    #[test]
    fn the_footer_names_the_next_event() {
        let s = Schedule { start: Some(90), stop: Some(7 * 60), days: only(&[0, 1]), ..Schedule::default() };
        assert_eq!(s.next_text(t(0, 0, 0)).unwrap(), "starts at 01:30");
        assert_eq!(s.next_text(t(0, 2, 0)).unwrap(), "stops at 07:00");
        assert_eq!(s.next_text(t(0, 8, 0)).unwrap(), "starts Tue at 01:30");
    }

    #[test]
    fn times_and_words_read_back() {
        assert_eq!(parse_time("01:30"), Some(90));
        assert_eq!(parse_time("7:05"), Some(425));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(time_text(425), "07:05");
        assert_eq!(WhenDone::from_word("shut-down"), Some(WhenDone::ShutDown));
        let json = serde_json::to_string(&Schedule { start: Some(60), stop: Some(99_999), ..Schedule::default() }.clamped()).unwrap();
        let back: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!((back.start, back.stop), (Some(60), None));
    }
}
