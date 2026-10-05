//! Local time for schedules (`schedule.rs`): the weekday and time of day that a moment in internet
//! time (`clock.rs`) is, in the block's time zone.
//!
//! It also goes the other way, for the "until" lock (`lock::Lock::Until`): the moment a local date
//! and time is (`Zone::instant`).
//!
//! The time zone is the one Windows had when the block started, and it stays the block's until the
//! block ends, also across a restart (`state::Saved::zone`). Standard users may change Windows'
//! time zone, so following it would let a schedule be moved out of the way; daylight saving is
//! still followed, as Windows' rules for that zone say.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::Time::{
    EnumDynamicTimeZoneInformation, FileTimeToSystemTime, GetDynamicTimeZoneInformation, SystemTimeToFileTime,
    SystemTimeToTzSpecificLocalTimeEx, TzSpecificLocalTimeToSystemTimeEx, DYNAMIC_TIME_ZONE_INFORMATION,
};

use serde::Serialize;

use crate::datetime::DateTime;
use crate::duration;
use crate::schedule::LocalTime;

/// 100-nanosecond intervals between 1601 (Windows' epoch) and 1970.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// The prefix of a fixed offset's name: `fixed:+60` is UTC+01:00 without daylight saving.
const FIXED: &str = "fixed:";

/// A time zone: one of Windows', by its key name (`W. Europe Standard Time`), or a fixed offset
/// from UTC (tests, and a zone Windows no longer knows).
#[derive(Clone)]
pub struct Zone {
    name: String,
    kind: Kind,
}

#[derive(Clone)]
enum Kind {
    Windows(Box<DYNAMIC_TIME_ZONE_INFORMATION>),
    /// Minutes east of UTC.
    Fixed(i32),
}

impl Zone {
    /// Windows' time zone right now.
    pub fn current() -> Zone {
        let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
        // SAFETY: `info` is a valid place to write to. TIME_ZONE_ID_INVALID is u32::MAX.
        if unsafe { GetDynamicTimeZoneInformation(&mut info) } == u32::MAX {
            return Zone::fixed(0);
        }
        Zone { name: key_name(&info), kind: Kind::Windows(Box::new(info)) }
    }

    /// A fixed offset from UTC, in minutes east.
    pub fn fixed(minutes: i32) -> Zone {
        Zone { name: format!("{FIXED}{minutes:+}"), kind: Kind::Fixed(minutes) }
    }

    /// The zone saved under this name (`name`). A Windows zone that cannot be found any more (it
    /// was removed by an update) is taken as Windows' current one.
    pub fn named(name: &str) -> Zone {
        if let Some(minutes) = name.strip_prefix(FIXED).and_then(|m| m.parse().ok()) {
            return Zone::fixed(minutes);
        }
        let mut index = 0;
        loop {
            let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
            // SAFETY: `info` is a valid place to write to. Anything but 0 is the end of the list.
            if unsafe { EnumDynamicTimeZoneInformation(index, &mut info) } != 0 {
                return Zone::current();
            }
            if key_name(&info) == name {
                return Zone { name: name.to_string(), kind: Kind::Windows(Box::new(info)) };
            }
            index += 1;
        }
    }

    /// What `named` takes back.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The local weekday and time of day of a moment, or `None` if Windows cannot say.
    pub fn local(&self, time: SystemTime) -> Option<LocalTime> {
        let unix = time.duration_since(UNIX_EPOCH).ok()?;
        match &self.kind {
            Kind::Fixed(minutes) => {
                let seconds = unix.as_secs() as i64 + i64::from(*minutes) * 60;
                let days = seconds.div_euclid(86400);
                let of_day = seconds.rem_euclid(86400);
                // 1970-01-01 was a Thursday (3, counting from Monday).
                Some(LocalTime { weekday: (days + 3).rem_euclid(7) as usize, minute: (of_day / 60) as u16, second: (of_day % 60) as u8 })
            }
            Kind::Windows(info) => {
                let ticks = FILETIME_UNIX_EPOCH + unix.as_secs() * 10_000_000 + u64::from(unix.subsec_nanos()) / 100;
                let file_time = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
                let mut utc = SYSTEMTIME::default();
                let mut local = SYSTEMTIME::default();
                // SAFETY: all pointers are to valid values of the right types.
                let converted = unsafe {
                    FileTimeToSystemTime(&file_time, &mut utc) != 0 && SystemTimeToTzSpecificLocalTimeEx(&**info, &utc, &mut local) != 0
                };
                converted.then(|| LocalTime {
                    // Windows counts from Sunday.
                    weekday: (usize::from(local.wDayOfWeek) + 6) % 7,
                    minute: local.wHour * 60 + local.wMinute,
                    second: local.wSecond as u8,
                })
            }
        }
    }

    /// The moment a local date and time is in this zone, or `None` if Windows cannot say (or it is
    /// before 1970). In the hour that daylight saving time skips or repeats, Windows decides.
    pub fn instant(&self, at: &DateTime) -> Option<SystemTime> {
        let seconds = match &self.kind {
            Kind::Fixed(minutes) => at.seconds_as_utc() - i64::from(*minutes) * 60,
            Kind::Windows(info) => {
                let local = SYSTEMTIME {
                    wYear: u16::try_from(at.year).ok()?,
                    wMonth: u16::from(at.month),
                    wDay: u16::from(at.day),
                    wHour: u16::from(at.hour),
                    wMinute: u16::from(at.minute),
                    ..SYSTEMTIME::default()
                };
                let mut utc = SYSTEMTIME::default();
                let mut file_time = FILETIME::default();
                // SAFETY: all pointers are to valid values of the right types.
                let converted = unsafe {
                    TzSpecificLocalTimeToSystemTimeEx(&**info, &local, &mut utc) != 0 && SystemTimeToFileTime(&utc, &mut file_time) != 0
                };
                if !converted {
                    return None;
                }
                let ticks = (u64::from(file_time.dwHighDateTime) << 32) | u64::from(file_time.dwLowDateTime);
                (ticks.checked_sub(FILETIME_UNIX_EPOCH)? / 10_000_000) as i64
            }
        };
        Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).ok()?))
    }
}

/// An until lock's end (`lock::Lock::Until`) as the window and the CLI show it.
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
    let at = DateTime::parse(text)?;
    let end = Zone::current().instant(&at).ok_or_else(|| format!("Cannot tell when {at} is in this time zone"))?;
    let left = end.duration_since(SystemTime::now()).map_err(|_| format!("{at} has passed"))?;
    // Rounded up to the minute, as the time was given in minutes.
    Ok(Until { when: at.to_string(), left: duration::words(left.as_secs().div_ceil(60) * 60) })
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
