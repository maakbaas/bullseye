//! A local date and time of day, as the "until" lock takes it (`lock::Lock::Until`): typed as
//! `2026-10-06 18:00`, written out as `Tuesday 6 October 2026, 18:00`. Which moment it is depends
//! on the time zone it is read in (`localtime::Zone::instant`), so it stays local time until a
//! block starts with it.

use std::fmt;

const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

impl DateTime {
    /// `2026-10-06 18:00`, also with a `T` between them, or without leading zeros (`2026-10-6 8:00`).
    pub fn parse(text: &str) -> Result<DateTime, String> {
        let wrong = || format!("\"{}\" is not a date and time such as 2026-10-06 18:00", text.trim());
        let (date, time) = text.trim().split_once([' ', 'T']).ok_or_else(wrong)?;
        let date: Vec<&str> = date.split('-').collect();
        let time: Vec<&str> = time.trim().split(':').collect();
        let [year, month, day] = date.as_slice() else { return Err(wrong()) };
        let [hour, minute] = time.as_slice() else { return Err(wrong()) };
        let number = |part: &str, digits: std::ops::RangeInclusive<usize>| {
            (digits.contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse::<i32>().ok()).flatten().ok_or_else(wrong)
        };
        let at = DateTime {
            year: number(year, 4..=4)?,
            month: number(month, 1..=2)? as u8,
            day: number(day, 1..=2)? as u8,
            hour: number(hour, 1..=2)? as u8,
            minute: number(minute, 2..=2)? as u8,
        };
        if !(1..=12).contains(&at.month) || at.day == 0 || at.day > days_in_month(at.year, at.month) || at.hour > 23 || at.minute > 59 {
            return Err(format!("{} is not a day and time that exists", at.text()));
        }
        Ok(at)
    }

    /// As typed, but always the same way: `2026-10-06 18:00`.
    pub fn text(&self) -> String {
        format!("{:04}-{:02}-{:02} {:02}:{:02}", self.year, self.month, self.day, self.hour, self.minute)
    }

    /// Days since 1970-01-01 (negative before).
    pub fn days(&self) -> i64 {
        // Howard Hinnant's days_from_civil.
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let of_era = year - era * 400;
        let month = i64::from(self.month);
        let of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
        era * 146_097 + of_era_days - 719_468
    }

    /// 0 is Monday.
    pub fn weekday(&self) -> usize {
        // 1970-01-01 was a Thursday (3, counting from Monday).
        (self.days() + 3).rem_euclid(7) as usize
    }

    /// Seconds since 1970-01-01 00:00 if this were UTC.
    pub fn seconds_as_utc(&self) -> i64 {
        self.days() * 86_400 + i64::from(self.hour) * 3600 + i64::from(self.minute) * 60
    }
}

/// `Tuesday 6 October 2026, 18:00`.
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let month = MONTHS[usize::from(self.month) - 1];
        write!(f, "{} {} {month} {}, {:02}:{:02}", DAYS[self.weekday()], self.day, self.year, self.hour, self.minute)
    }
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_and_time_is_read_and_written_out() {
        let at = DateTime::parse("2026-10-06 18:00").unwrap();
        assert_eq!(at, DateTime { year: 2026, month: 10, day: 6, hour: 18, minute: 0 });
        assert_eq!(at.to_string(), "Tuesday 6 October 2026, 18:00");
        assert_eq!(DateTime::parse(" 2026-10-6T8:05 ").unwrap().text(), "2026-10-06 08:05");
        assert_eq!(DateTime::parse("2028-02-29 00:00").unwrap().to_string(), "Tuesday 29 February 2028, 00:00");
    }

    #[test]
    fn what_is_not_a_date_and_time() {
        for text in ["", "tomorrow", "2026-10-06", "18:00", "2026-13-01 10:00", "2027-02-29 10:00", "2026-10-06 24:00", "2026-10-06 18:60", "26-10-06 18:00", "2026-10-06 18:0"] {
            assert!(DateTime::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn days_and_weekdays_count_from_1970() {
        assert_eq!(DateTime::parse("1970-01-01 00:00").unwrap().days(), 0);
        assert_eq!(DateTime::parse("1970-01-01 00:00").unwrap().weekday(), 3);
        // 2030-01-07 00:00 UTC is a Monday, 1 893 974 400 seconds after 1970.
        let monday = DateTime::parse("2030-01-07 00:00").unwrap();
        assert_eq!((monday.seconds_as_utc(), monday.weekday()), (1_893_974_400, 0));
        assert_eq!(DateTime::parse("1969-12-31 23:00").unwrap().seconds_as_utc(), -3600);
    }
}
