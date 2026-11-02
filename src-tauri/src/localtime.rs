//! Local time for schedules (`schedule.rs`): the weekday and time of day that a moment in internet
//! time (`clock.rs`) is, in the session's time zone.
//!
//! It also goes the other way, for the "until" lock (`lock::Lock::Timed`): the moment a local date
//! and time is (`Zone::instant`).
//!
//! The time zone is the one Windows had when the session started, and it stays the session's until the
//! session ends, also across a restart (`state::Saved::zone`). Standard users may change Windows'
//! time zone, so following it would let a schedule be moved out of the way; daylight saving is
//! still followed, as Windows' rules for that zone say. A zone is kept with those rules (`Zone::name`),
//! so the sync server, which is not Windows, reads a schedule as the computer that started it does.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::Time::{GetDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION};

use serde::Serialize;

use crate::datetime::{days_in_month, DateTime};
use crate::duration;
use crate::schedule::LocalTime;

/// The prefix of a fixed offset's name: `fixed:+60` is UTC+01:00 without daylight saving.
const FIXED: &str = "fixed:";

/// The prefix of a zone's name with its rules (`Rules::text`), then `|` and Windows' key name.
const RULES: &str = "rules:";

/// A time zone: Windows' as it was taken (`current`), by its rules, or a fixed offset from UTC
/// (tests, and the sync server when no computer has said its zone). Its name holds all it is, so a
/// zone reads the same wherever it is read: on the computer it was taken on, on another, and on the
/// sync server, which is not Windows (`named`).
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    name: String,
    kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Rules(Rules),
    /// Minutes east of UTC.
    Fixed(i32),
}

/// A zone as Windows has it (`TIME_ZONE_INFORMATION`): UTC is local time plus `bias` minutes, plus
/// `standard_bias` in standard time and `daylight_bias` in daylight saving time, which starts at
/// `daylight` (in local standard time) and ends at `standard` (in local daylight saving time).
/// Without `daylight`, there is no daylight saving time. The rules Windows has for this year are
/// taken for every year.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rules {
    bias: i32,
    standard_bias: i32,
    daylight_bias: i32,
    standard: Option<Change>,
    daylight: Option<Change>,
}

/// When daylight saving time starts or ends (`SYSTEMTIME` as `TIME_ZONE_INFORMATION` uses it): in
/// `month`, on the `week`th (1 to 5, 5 the last) `weekday` (0 is Sunday), at `second` of the day.
/// With a `year`, Windows means a fixed date: the `week` is the day of the month then.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Change {
    year: u16,
    month: u8,
    weekday: u8,
    week: u8,
    second: u32,
}

impl Change {
    fn of(time: &SYSTEMTIME) -> Option<Change> {
        (1..=12).contains(&time.wMonth).then(|| Change {
            year: time.wYear,
            month: time.wMonth as u8,
            weekday: (time.wDayOfWeek % 7) as u8,
            week: time.wDay.clamp(1, 31) as u8,
            // 23:59:59.999 is the end of the day.
            second: u32::from(time.wHour) * 3600
                + u32::from(time.wMinute) * 60
                + u32::from(time.wSecond)
                + u32::from(time.wMilliseconds >= 500),
        })
    }

    /// Seconds since 1970 of the change in `year`, as if its local time were UTC.
    fn in_year(&self, year: i32) -> i64 {
        let date = |day: u8| DateTime { year, month: self.month, day, hour: 0, minute: 0 };
        let last = days_in_month(year, self.month);
        let day = if self.year != 0 {
            self.week.min(last)
        } else {
            // Counting from Sunday, as Windows does: `weekday()` counts from Monday.
            let first = (date(1).weekday() + 1) % 7;
            let mut day = 1 + (usize::from(self.weekday) + 7 - first) % 7 + 7 * (usize::from(self.week) - 1);
            while day > usize::from(last) {
                day -= 7;
            }
            day as u8
        };
        date(day).seconds_as_utc() + i64::from(self.second)
    }

    fn text(change: Option<Change>) -> String {
        let Change { year, month, weekday, week, second } = change.unwrap_or(Change { year: 0, month: 0, weekday: 0, week: 0, second: 0 });
        format!("{year}/{month}/{weekday}/{week}/{second}")
    }

    fn parse(text: &str) -> Option<Option<Change>> {
        let parts: Vec<u32> = text.split('/').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        let [year, month, weekday, week, second] = parts.as_slice() else { return None };
        if *month == 0 {
            return Some(None);
        }
        let valid = (1..=12).contains(month) && *weekday < 7 && (1..=31).contains(week) && *second <= 86_400;
        valid.then_some(Some(Change {
            year: *year as u16,
            month: *month as u8,
            weekday: *weekday as u8,
            week: *week as u8,
            second: *second,
        }))
    }
}

impl Rules {
    fn of(info: &DYNAMIC_TIME_ZONE_INFORMATION) -> Rules {
        let saving = !info.DynamicDaylightTimeDisabled;
        let (standard, daylight) = match (Change::of(&info.StandardDate), Change::of(&info.DaylightDate)) {
            (Some(standard), Some(daylight)) if saving => (Some(standard), Some(daylight)),
            _ => (None, None),
        };
        Rules { bias: info.Bias, standard_bias: info.StandardBias, daylight_bias: info.DaylightBias, standard, daylight }
    }

    /// `bias,standard bias,daylight bias,standard,daylight`, the changes as `Change::text`.
    fn text(&self) -> String {
        let Rules { bias, standard_bias, daylight_bias, standard, daylight } = self;
        format!("{bias},{standard_bias},{daylight_bias},{},{}", Change::text(*standard), Change::text(*daylight))
    }

    fn parse(text: &str) -> Option<Rules> {
        let parts: Vec<&str> = text.split(',').collect();
        let [bias, standard_bias, daylight_bias, standard, daylight] = parts.as_slice() else { return None };
        let minutes = |text: &str| text.parse::<i32>().ok().filter(|m| m.abs() <= 24 * 60);
        let (standard, daylight) = match (Change::parse(standard)?, Change::parse(daylight)?) {
            (Some(standard), Some(daylight)) => (Some(standard), Some(daylight)),
            _ => (None, None),
        };
        Some(Rules {
            bias: minutes(bias)?,
            standard_bias: minutes(standard_bias)?,
            daylight_bias: minutes(daylight_bias)?,
            standard,
            daylight,
        })
    }

    /// Whether `utc` (seconds since 1970) is in daylight saving time.
    fn saving(&self, utc: i64) -> bool {
        let (Some(standard), Some(daylight)) = (self.standard, self.daylight) else { return false };
        let year = year_of((utc - i64::from(self.bias + self.standard_bias) * 60).div_euclid(86_400));
        let starts = daylight.in_year(year) + i64::from(self.bias + self.standard_bias) * 60;
        let ends = standard.in_year(year) + i64::from(self.bias + self.daylight_bias) * 60;
        if starts < ends {
            (starts..ends).contains(&utc)
        } else {
            // South of the equator: from the start to the end of the year, and up to the end.
            utc >= starts || utc < ends
        }
    }

    /// Minutes east of UTC at `utc`.
    fn offset(&self, utc: i64) -> i32 {
        -(self.bias + if self.saving(utc) { self.daylight_bias } else { self.standard_bias })
    }
}

/// The year of a day counted from 1970-01-01.
fn year_of(days: i64) -> i32 {
    // Howard Hinnant's civil_from_days, as far as the year.
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * of_year + 2) / 153;
    (year_of_era + era * 400 + i64::from(month >= 10)) as i32
}

impl Zone {
    /// Windows' time zone right now, with the rules Windows has for it.
    pub fn current() -> Zone {
        let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
        // SAFETY: `info` is a valid place to write to. TIME_ZONE_ID_INVALID is u32::MAX.
        if unsafe { GetDynamicTimeZoneInformation(&mut info) } == u32::MAX {
            return Zone::fixed(0);
        }
        Zone::of_rules(Rules::of(&info), &key_name(&info))
    }

    fn of_rules(rules: Rules, key: &str) -> Zone {
        Zone { name: format!("{RULES}{}|{key}", rules.text()), kind: Kind::Rules(rules) }
    }

    /// A fixed offset from UTC, in minutes east.
    pub fn fixed(minutes: i32) -> Zone {
        Zone { name: format!("{FIXED}{minutes:+}"), kind: Kind::Fixed(minutes) }
    }

    /// The zone saved under this name (`name`), read the same on any computer and on the sync
    /// server. A name that is not one is taken as Windows' current zone.
    pub fn named(name: &str) -> Zone {
        Zone::parse(name).unwrap_or_else(Zone::current)
    }

    /// The zone of a name, if it is one.
    pub fn parse(name: &str) -> Option<Zone> {
        if let Some(minutes) = name.strip_prefix(FIXED).and_then(|m| m.parse().ok()) {
            return Some(Zone::fixed(minutes));
        }
        let (rules, key) = name.strip_prefix(RULES)?.split_once('|')?;
        Some(Zone::of_rules(Rules::parse(rules)?, key))
    }

    /// What `named` takes back.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Minutes east of UTC at `utc` (seconds since 1970).
    fn offset(&self, utc: i64) -> i32 {
        match &self.kind {
            Kind::Fixed(minutes) => *minutes,
            Kind::Rules(rules) => rules.offset(utc),
        }
    }

    /// The local weekday and time of day of a moment, or `None` before 1970.
    pub fn local(&self, time: SystemTime) -> Option<LocalTime> {
        let utc = i64::try_from(time.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
        let seconds = utc + i64::from(self.offset(utc)) * 60;
        let days = seconds.div_euclid(86400);
        let of_day = seconds.rem_euclid(86400);
        // 1970-01-01 was a Thursday (3, counting from Monday).
        Some(LocalTime { weekday: (days + 3).rem_euclid(7) as usize, minute: (of_day / 60) as u16, second: (of_day % 60) as u8 })
    }

    /// The moment a local date and time is in this zone, or `None` before 1970. A time that
    /// daylight saving time skips is taken in standard time (an hour later than it says), and a
    /// time it repeats is the later one, in standard time.
    pub fn instant(&self, at: &DateTime) -> Option<SystemTime> {
        let local = at.seconds_as_utc();
        let seconds = match &self.kind {
            Kind::Fixed(minutes) => local - i64::from(*minutes) * 60,
            Kind::Rules(rules) => {
                let standard = local + i64::from(rules.bias + rules.standard_bias) * 60;
                let daylight = local + i64::from(rules.bias + rules.daylight_bias) * 60;
                if !rules.saving(standard) || !rules.saving(daylight) {
                    standard
                } else {
                    daylight
                }
            }
        };
        Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).ok()?))
    }
}

/// An until lock's end (`lock::Lock::Timed`) as the window and the CLI show it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Until {
    /// `Tuesday 6 October 2026, 18:00`.
    pub when: String,
    /// How long from now, in words, by the Windows clock (the service measures on internet time).
    pub left: String,
}

/// An until lock's end as typed (`2026-10-06 18:00`), in Windows' time zone now; refused when it is
/// not a date and time or has passed.
pub fn until(text: &str) -> Result<Until, String> {
    until_in(text, &Zone::current())
}

/// As `until`, in `zone` (the sync server's page: the browser's).
pub fn until_in(text: &str, zone: &Zone) -> Result<Until, String> {
    let at = DateTime::parse(text)?;
    let end = zone.instant(&at).ok_or_else(|| format!("Cannot tell when {at} is in this time zone"))?;
    let left = end.duration_since(SystemTime::now()).map_err(|_| format!("{at} has passed"))?;
    Ok(Until { when: at.to_string(), left: duration::left(left.as_secs()) })
}

fn key_name(info: &DYNAMIC_TIME_ZONE_INFORMATION) -> String {
    let name = &info.TimeZoneKeyName;
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    String::from_utf16_lossy(&name[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2030-01-07 00:00:00 UTC, a Monday.
    const MONDAY: u64 = 1_893_974_400;

    #[test]
    fn a_fixed_offset_gives_the_weekday_and_time() {
        let at = |seconds: u64| UNIX_EPOCH + Duration::from_secs(seconds);
        assert_eq!(Zone::fixed(0).local(at(MONDAY)), Some(LocalTime { weekday: 0, minute: 0, second: 0 }));
        assert_eq!(Zone::fixed(0).local(at(MONDAY + 16 * 3600 + 61)), Some(LocalTime { weekday: 0, minute: 16 * 60 + 1, second: 1 }));
        // An hour east of UTC, it is already 01:00 on Monday; an hour west, still Sunday.
        assert_eq!(Zone::fixed(60).local(at(MONDAY)), Some(LocalTime { weekday: 0, minute: 60, second: 0 }));
        assert_eq!(Zone::fixed(-60).local(at(MONDAY)), Some(LocalTime { weekday: 6, minute: 23 * 60, second: 0 }));
    }

    #[test]
    fn a_local_date_and_time_is_a_moment_in_its_zone() {
        let monday = DateTime::parse("2030-01-07 00:00").unwrap();
        let at = |seconds: u64| UNIX_EPOCH + Duration::from_secs(seconds);
        assert_eq!(Zone::fixed(0).instant(&monday), Some(at(MONDAY)));
        // Midnight an hour east of UTC is an hour earlier.
        assert_eq!(Zone::fixed(60).instant(&monday), Some(at(MONDAY - 3600)));
        // Windows' own zone, there and back.
        let zone = Zone::current();
        let moment = zone.instant(&monday).unwrap();
        assert_eq!(zone.local(moment), Some(LocalTime { weekday: 0, minute: 0, second: 0 }));
    }

    /// W. Europe Standard Time as Windows has it: UTC+1, summer time from the last Sunday of March at
    /// 2:00 to the last Sunday of October at 3:00.
    const EUROPE: &str = "rules:-60,0,-60,0/10/0/5/10800,0/3/0/5/7200|W. Europe Standard Time";
    /// AUS Eastern Standard Time: UTC+10, summer time from the first Sunday of October at 2:00 to the
    /// first Sunday of April at 3:00.
    const SYDNEY: &str = "rules:-600,0,-60,0/4/0/1/10800,0/10/0/1/7200|AUS Eastern Standard Time";

    fn utc(text: &str) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(DateTime::parse(text).unwrap().seconds_as_utc() as u64)
    }

    fn time(zone: &Zone, at: &str) -> String {
        let local = zone.local(utc(at)).unwrap();
        format!("{} {:02}:{:02}", local.weekday, local.minute / 60, local.minute % 60)
    }

    #[test]
    fn windows_rules_follow_summer_time_on_both_sides_of_the_equator() {
        let europe = Zone::named(EUROPE);
        assert_eq!(europe.name(), EUROPE);
        // 2030-03-31 and 2030-10-27 are the last Sundays (6) of March and October.
        assert_eq!(time(&europe, "2030-01-08 08:30"), "1 09:30");
        assert_eq!(time(&europe, "2030-03-31 00:59"), "6 01:59");
        assert_eq!(time(&europe, "2030-03-31 01:00"), "6 03:00");
        assert_eq!(time(&europe, "2030-07-01 08:30"), "0 10:30");
        assert_eq!(time(&europe, "2030-10-27 00:59"), "6 02:59");
        assert_eq!(time(&europe, "2030-10-27 01:00"), "6 02:00");
        // Summer time in January, not in July.
        let sydney = Zone::named(SYDNEY);
        assert_eq!(time(&sydney, "2030-01-08 08:30"), "1 19:30");
        assert_eq!(time(&sydney, "2030-07-01 08:30"), "0 18:30");
        // 2030-04-07 is the first Sunday of April: 3:00 summer time is 16:00 UTC the day before.
        assert_eq!(time(&sydney, "2030-04-06 15:59"), "6 02:59");
        assert_eq!(time(&sydney, "2030-04-06 16:00"), "6 02:00");
        // A fixed date (with a year), every year; and no summer time at all.
        let fixed_date = Zone::named("rules:-60,0,-60,2020/10/0/20/10800,2020/3/0/20/7200|Odd");
        assert_eq!(time(&fixed_date, "2030-03-20 00:59"), "2 01:59");
        assert_eq!(time(&fixed_date, "2030-03-20 01:00"), "2 03:00");
        let plain = Zone::named("rules:-330,0,-60,0/0/0/0/0,0/0/0/0/0|India Standard Time");
        assert_eq!(time(&plain, "2030-07-01 00:00"), "0 05:30");
    }

    #[test]
    fn a_local_time_in_a_zone_with_summer_time_is_a_moment() {
        let europe = Zone::named(EUROPE);
        let instant = |text: &str| europe.instant(&DateTime::parse(text).unwrap());
        assert_eq!(instant("2030-01-08 09:30"), Some(utc("2030-01-08 08:30")));
        assert_eq!(instant("2030-07-01 10:30"), Some(utc("2030-07-01 08:30")));
        // Skipped: in standard time. Repeated: the later one.
        assert_eq!(instant("2030-03-31 02:30"), Some(utc("2030-03-31 01:30")));
        assert_eq!(instant("2030-10-27 02:30"), Some(utc("2030-10-27 01:30")));
        assert_eq!(instant("1969-12-31 23:00"), None);
    }

    #[test]
    fn what_is_not_a_zone_is_windows_current_one() {
        for name in [
            "W. Europe Standard Time",
            "rules:-60,0,-60|x",
            "rules:-60,0,-60,0/10/0/5/10800,0/3/0/5/7200",
            "rules:-60,0,-60,0/13/0/5/10800,0/3/0/5/7200|x",
            "rules:-60,0,-60,0/10/0/5,0/3/0/5/7200|x",
            "rules:-6000,0,-60,0/10/0/5/10800,0/3/0/5/7200|x",
            "rules:a,0,-60,0/10/0/5/10800,0/3/0/5/7200|x",
            "fixed:x",
        ] {
            assert_eq!(Zone::parse(name), None, "{name}");
            assert_eq!(Zone::named(name), Zone::current(), "{name}");
        }
        // Half the rules is none.
        let half = Zone::named("rules:-60,0,-60,0/0/0/0/0,0/3/0/5/7200|x");
        assert_eq!(time(&half, "2030-07-01 08:30"), "0 09:30");
    }

    #[test]
    fn windows_own_rules_are_taken_as_they_are() {
        let mut info = DYNAMIC_TIME_ZONE_INFORMATION { Bias: -60, DaylightBias: -60, ..Default::default() };
        info.StandardDate = SYSTEMTIME { wMonth: 10, wDay: 5, wHour: 3, ..Default::default() };
        info.DaylightDate = SYSTEMTIME { wMonth: 3, wDay: 5, wHour: 1, wMinute: 59, wSecond: 59, wMilliseconds: 999, ..Default::default() };
        for (c, name) in "Europe".encode_utf16().enumerate() {
            info.TimeZoneKeyName[c] = name;
        }
        let zone = Zone::of_rules(Rules::of(&info), &key_name(&info));
        assert_eq!(zone.name(), "rules:-60,0,-60,0/10/0/5/10800,0/3/0/5/7200|Europe");
        info.DynamicDaylightTimeDisabled = true;
        assert_eq!(Rules::of(&info).text(), "-60,0,-60,0/0/0/0/0,0/0/0/0/0");
    }

    #[test]
    fn a_zone_comes_back_by_its_name() {
        assert_eq!(Zone::named(Zone::fixed(-330).name()).name(), "fixed:-330");
        let current = Zone::current();
        assert!(!current.name().is_empty());
        let again = Zone::named(current.name());
        assert_eq!(again.name(), current.name());
        // The same zone says the same local time.
        let now = SystemTime::now();
        assert_eq!(again.local(now), current.local(now));
        assert!(current.local(now).is_some_and(|t| t.weekday < 7 && t.minute < 24 * 60));
    }
}
