//! A weekly schedule: when a block with `definitions::Schedule::Weekly` is active. The schedule is
//! part of the block, like its lists and enforcers: copied when the block starts and fixed while it
//! runs. Outside the schedule the block keeps running (and stays locked, `lock.rs`) but enforces
//! nothing, so a block started at 16:00 with a 2-hour lock and a schedule of 08:00-17:00 blocks
//! until 17:00, and from then on nothing until the lock has ended the block at 18:00.
//!
//! A week is a list of spans per weekday, in minutes since local midnight. The UI edits it in a
//! weekly grid; the CLI and the UI's text field write it as text (`parse`, `describe`):
//! `Mon-Fri 08:00-17:00; Sat 10:00-12:00, 14:00-16:00`. Local time is the block's own time zone
//! (`localtime.rs`), measured on internet time (`clock.rs`).

use serde::{Deserialize, Serialize};

/// Minutes in a day: a span's end may be this (midnight at the end of the day).
pub const DAY: u16 = 24 * 60;

const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// From one minute of the day (inclusive) to another (exclusive), `0..=DAY`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span(pub u16, pub u16);

/// The times a block is active, per weekday, Monday first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Week {
    pub days: [Vec<Span>; 7],
}

/// A moment in local time, as far as a schedule cares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalTime {
    /// 0 is Monday.
    pub weekday: usize,
    /// Minutes since midnight.
    pub minute: u16,
    pub second: u8,
}

impl Week {
    /// The same schedule with each day's spans checked, sorted and merged where they touch, so two
    /// schedules that mean the same are equal. Refused if a span is empty or outside the day, or
    /// if there is no time at all in it (a block that is never active).
    pub fn normalized(mut self) -> Result<Week, String> {
        for (day, spans) in self.days.iter_mut().enumerate() {
            if let Some(Span(start, end)) = spans.iter().find(|Span(start, end)| start >= end || *end > DAY) {
                return Err(format!("{}: {}-{} is not a time span within the day", NAMES[day], clock(*start), clock(*end)));
            }
            spans.sort();
            let mut merged: Vec<Span> = Vec::new();
            for span in spans.drain(..) {
                match merged.last_mut() {
                    Some(last) if span.0 <= last.1 => last.1 = last.1.max(span.1),
                    _ => merged.push(span),
                }
            }
            *spans = merged;
        }
        if self.days.iter().all(Vec::is_empty) {
            return Err("The schedule has no time in it, so the block would never be active".into());
        }
        Ok(self)
    }

    /// Whether the block is active at this moment.
    pub fn is_on(&self, at: LocalTime) -> bool {
        self.days[at.weekday % 7].iter().any(|&Span(start, end)| start <= at.minute && at.minute < end)
    }

    /// Seconds until the schedule next turns the block on or off, if it ever does. Counted in local
    /// wall time, so it is off by an hour across a daylight saving change; it is only shown.
    pub fn next_change(&self, at: LocalTime) -> Option<u64> {
        let now = self.is_on(at);
        (1..=7 * u64::from(DAY)).find_map(|ahead| {
            let minute = u64::from(at.minute) + ahead;
            let later = LocalTime {
                weekday: (at.weekday + (minute / u64::from(DAY)) as usize) % 7,
                minute: (minute % u64::from(DAY)) as u16,
                second: 0,
            };
            (self.is_on(later) != now).then(|| ahead * 60 - u64::from(at.second))
        })
    }

    /// Reads a schedule written as text: rules separated by `;` or new lines, each a set of days and
    /// time spans. `Mon-Fri 08:00-17:00; Sat 10:00-12:00, 14:00-16:00`. Days are names (`mon`,
    /// `monday`), ranges (`fri-sun`), comma lists (`mon,wed`), `daily`, `weekdays` or `weekends`;
    /// a rule without days is every day, one without times is the whole day. Times are `8`, `8:30`,
    /// `08:30`, `24:00`, `8am`, `5:30pm`; a span that ends before it starts runs past midnight
    /// into the next day. `always` or an empty text is no schedule: always active.
    pub fn parse(text: &str) -> Result<Option<Week>, String> {
        let text = text.trim().to_lowercase();
        if text.is_empty() || text == "always" {
            return Ok(None);
        }
        let mut schedule = Week::default();
        for rule in text.split([';', '\n']).map(str::trim).filter(|rule| !rule.is_empty()) {
            parse_rule(rule, &mut schedule).map_err(|why| format!("In \"{rule}\": {why}"))?;
        }
        schedule.normalized().map(Some)
    }

    /// The schedule as text that `parse` reads back to the same schedule: days with the same times
    /// together, `Mon-Fri 08:00-17:00; Sat 10:00-12:00`.
    pub fn describe(&self) -> String {
        let mut groups: Vec<(Vec<usize>, &Vec<Span>)> = Vec::new();
        for (day, spans) in self.days.iter().enumerate().filter(|(_, spans)| !spans.is_empty()) {
            match groups.iter_mut().find(|(_, same)| *same == spans) {
                Some((days, _)) => days.push(day),
                None => groups.push((vec![day], spans)),
            }
        }
        groups
            .iter()
            .map(|(days, spans)| {
                let times = spans.iter().map(|&Span(start, end)| format!("{}-{}", clock(start), clock(end))).collect::<Vec<_>>();
                format!("{} {}", day_list(days), times.join(", "))
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// One rule, added to `schedule`.
fn parse_rule(rule: &str, schedule: &mut Week) -> Result<(), String> {
    // `08:00 - 17:00` is one span, as is `08:00-17:00`.
    let mut joined = String::new();
    for part in rule.split('-') {
        if !joined.is_empty() {
            joined.truncate(joined.trim_end().len());
            joined.push('-');
        }
        joined.push_str(if joined.is_empty() { part } else { part.trim_start() });
    }
    let mut words = joined.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).peekable();
    // Days come first, unless the rule starts with a time.
    let mut days = Vec::new();
    while let Some(word) = words.next_if(|w| !w.starts_with(|c: char| c.is_ascii_digit())) {
        days.extend(parse_days(word)?);
    }
    if days.is_empty() {
        days = (0..7).collect();
    }
    let mut spans = Vec::new();
    for word in words {
        let (start, end) = word.split_once('-').ok_or_else(|| format!("\"{word}\" is not a time span such as 08:00-17:00"))?;
        let (start, end) = (parse_time(start)?, parse_time(end)?);
        if start == DAY {
            return Err("a span cannot start at 24:00".into());
        }
        if start == end {
            return Err(format!("{word} is empty"));
        }
        spans.push((start, end));
    }
    if spans.is_empty() {
        spans.push((0, DAY));
    }
    for day in days {
        for &(start, end) in &spans {
            if start < end {
                schedule.days[day].push(Span(start, end));
            } else {
                // Past midnight: the rest of this day, and the start of the next.
                schedule.days[day].push(Span(start, DAY));
                if end > 0 {
                    schedule.days[(day + 1) % 7].push(Span(0, end));
                }
            }
        }
    }
    Ok(())
}

/// `mon`, `fri-sun`, `daily`, `weekdays`, `weekends`.
fn parse_days(word: &str) -> Result<Vec<usize>, String> {
    match word {
        "daily" | "everyday" | "all" => return Ok((0..7).collect()),
        "weekdays" => return Ok((0..5).collect()),
        "weekend" | "weekends" => return Ok(vec![5, 6]),
        _ => {}
    }
    match word.split_once('-') {
        Some((first, last)) => {
            let (first, last) = (day(first)?, day(last)?);
            // `fri-mon` goes round the end of the week.
            Ok((0..7).map(|step| (first + step) % 7).take((last + 7 - first) % 7 + 1).collect())
        }
        None => Ok(vec![day(word)?]),
    }
}

fn day(word: &str) -> Result<usize, String> {
    const FULL: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    FULL.iter()
        .position(|full| word.len() >= 2 && full.starts_with(word))
        .ok_or_else(|| format!("\"{word}\" is not a day; use e.g. mon, mon-fri, daily, weekdays or weekends"))
}

/// `8`, `8:30`, `08:30`, `24:00`, `8am`, `12pm`, `5:30pm`, as minutes since midnight.
fn parse_time(word: &str) -> Result<u16, String> {
    let invalid = || format!("\"{word}\" is not a time such as 08:00, 17:30 or 5pm");
    let (digits, half) = match word.strip_suffix("am").or_else(|| word.strip_suffix("a")) {
        Some(digits) => (digits, Some(0)),
        None => match word.strip_suffix("pm").or_else(|| word.strip_suffix("p")) {
            Some(digits) => (digits, Some(12)),
            None => (word, None),
        },
    };
    let (hours, minutes) = digits.split_once(':').unwrap_or((digits, "0"));
    if hours.is_empty() || hours.len() > 2 || (digits.contains(':') && minutes.len() != 2) {
        return Err(invalid());
    }
    let hours: u16 = hours.parse().map_err(|_| invalid())?;
    let minutes: u16 = minutes.parse().map_err(|_| invalid())?;
    let hours = match half {
        Some(offset) if (1..=12).contains(&hours) => hours % 12 + offset,
        Some(_) => return Err(invalid()),
        None => hours,
    };
    if minutes >= 60 || hours > 24 || (hours == 24 && minutes > 0) {
        return Err(invalid());
    }
    Ok(hours * 60 + minutes)
}

/// Minutes since midnight as `08:05`; the end of the day is `24:00`.
fn clock(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Days as `Daily`, `Mon-Fri`, `Mon,Wed,Fri` or `Mon-Wed,Sat`.
fn day_list(days: &[usize]) -> String {
    if days.len() == 7 {
        return "Daily".into();
    }
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &day in days {
        match runs.last_mut() {
            Some(run) if run.1 + 1 == day => run.1 = day,
            _ => runs.push((day, day)),
        }
    }
    runs.iter()
        .map(|&(first, last)| match last - first {
            0 => NAMES[first].to_string(),
            1 => format!("{},{}", NAMES[first], NAMES[last]),
            _ => format!("{}-{}", NAMES[first], NAMES[last]),
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(weekday: usize, hour: u16, minute: u16) -> LocalTime {
        LocalTime { weekday, minute: hour * 60 + minute, second: 0 }
    }

    fn office_hours() -> Week {
        Week::parse("Mon-Fri 08:00-17:00").unwrap().unwrap()
    }

    #[test]
    fn a_schedule_says_when_the_block_is_on() {
        let schedule = office_hours();
        assert!(schedule.is_on(at(0, 8, 0)));
        assert!(schedule.is_on(at(4, 16, 59)));
        assert!(!schedule.is_on(at(4, 17, 0)), "the end is not part of the span");
        assert!(!schedule.is_on(at(0, 7, 59)));
        assert!(!schedule.is_on(at(5, 12, 0)) && !schedule.is_on(at(6, 12, 0)));
    }

    #[test]
    fn the_next_change_is_found_across_days() {
        let schedule = office_hours();
        // Friday 16:00: off in an hour.
        assert_eq!(schedule.next_change(at(4, 16, 0)), Some(3600));
        // Friday 17:00: on again Monday 08:00.
        assert_eq!(schedule.next_change(at(4, 17, 0)), Some((2 * 24 + 15) * 3600));
        // Seconds into the minute count.
        assert_eq!(schedule.next_change(LocalTime { weekday: 0, minute: 16 * 60 + 59, second: 30 }), Some(30));
        // A schedule that is on all week never changes.
        let always = Week::parse("daily").unwrap().unwrap();
        assert!(always.is_on(at(3, 3, 3)));
        assert_eq!(always.next_change(at(3, 3, 3)), None);
    }

    #[test]
    fn texts_that_are_schedules() {
        assert_eq!(Week::parse(""), Ok(None));
        assert_eq!(Week::parse(" Always "), Ok(None));
        let mut expected = Week::default();
        for day in 0..5 {
            expected.days[day] = vec![Span(480, 1020)];
        }
        for same in [
            "mon-fri 08:00-17:00",
            "Weekdays 8-17",
            "monday-friday 8am-5pm",
            "mon,tue,wed,thu,fri 08:00 - 17:00",
            "mon-wed 8-12, 12:00-17:00; thu-fri 8-17",
        ] {
            assert_eq!(Week::parse(same), Ok(Some(expected.clone())), "{same}");
        }
        // No days is every day, no times is the whole day.
        let daily = Week::parse("9:30-10:15").unwrap().unwrap();
        assert!(daily.days.iter().all(|spans| spans == &vec![Span(570, 615)]));
        let saturday = Week::parse("sat").unwrap().unwrap();
        assert_eq!(saturday.days[5], vec![Span(0, DAY)]);
        // Overnight runs into the next day; Sunday night into Monday.
        let nights = Week::parse("fri-sun 22:00-02:00").unwrap().unwrap();
        assert_eq!(nights.days[4], vec![Span(1320, DAY)]);
        assert_eq!(nights.days[5], vec![Span(0, 120), Span(1320, DAY)]);
        assert_eq!(nights.days[0], vec![Span(0, 120)]);
        // 12am is midnight, 12pm noon; a range of days can wrap round the week.
        assert_eq!(Week::parse("sun-mon 12am-12pm").unwrap().unwrap().days[0], vec![Span(0, 720)]);
        assert_eq!(Week::parse("sun-mon 12am-12pm").unwrap().unwrap().days[6], vec![Span(0, 720)]);
    }

    #[test]
    fn texts_that_are_not() {
        for bad in
            ["someday 8-17", "mon 8-", "mon 25:00-26:00", "mon 8:5-9", "mon 9-9", "mon 24:00-02:00", "mon 13pm-14pm", "mon 8 17", "m 8-9"]
        {
            assert!(Week::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_description_reads_back_as_the_same_schedule() {
        for text in [
            "Mon-Fri 08:00-17:00",
            "Mon-Fri 08:00-12:00, 13:00-17:00; Sat 10:00-14:00",
            "Mon,Wed,Fri 06:00-07:30; Tue 00:00-24:00",
            "Daily 21:00-24:00",
            "Mon-Wed,Sat 09:00-10:00",
            "Mon,Tue 09:00-10:00",
        ] {
            let schedule = Week::parse(text).unwrap().unwrap();
            assert_eq!(schedule.describe(), text);
            assert_eq!(Week::parse(&schedule.describe()).unwrap().unwrap(), schedule);
        }
        let nights = Week::parse("fri 22-2").unwrap().unwrap();
        assert_eq!(nights.describe(), "Fri 22:00-24:00; Sat 00:00-02:00");
    }

    #[test]
    fn spans_are_checked_and_merged() {
        let mut schedule = Week::default();
        schedule.days[0] = vec![Span(600, 700), Span(480, 620), Span(700, 720)];
        assert_eq!(schedule.clone().normalized().unwrap().days[0], vec![Span(480, 720)]);
        schedule.days[1] = vec![Span(700, 600)];
        assert!(schedule.clone().normalized().is_err());
        schedule.days[1] = vec![Span(0, DAY + 1)];
        assert!(schedule.normalized().is_err());
        // A schedule with no time in it is never active, which is not a block.
        assert!(Week::default().normalized().is_err());
    }

    #[test]
    fn a_schedule_travels_as_json() {
        let schedule = office_hours();
        let json = serde_json::to_string(&schedule).unwrap();
        assert!(json.starts_with(r#"{"days":[[[480,1020]],"#), "{json}");
        assert_eq!(serde_json::from_str::<Week>(&json).unwrap(), schedule);
        // Not seven days is not a schedule.
        assert!(serde_json::from_str::<Week>(r#"{"days":[[]]}"#).is_err());
    }
}
