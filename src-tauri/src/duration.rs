//! Durations as people type and read them. Typed short (`5s`, `10m`, `3h`, `1d`, or several
//! together: `2h 5m`, `1d12h`) or in words (`2 hours 5 minutes`); shown by `countdown`, wherever the app or the CLI shows one.

use std::time::Duration;

/// Parses a duration: parts of a number and a unit, in any order, with or without spaces between
/// them (`2h 30m`, `2h30m`, `90m`, `1 day 2 hours`). Units are `s`/`sec`/`second(s)`,
/// `m`/`min`/`minute(s)`, `h`/`hr`/`hour(s)` and `d`/`day(s)`, any case. An empty text means no
/// duration.
pub fn parse(text: &str) -> Result<Option<Duration>, String> {
    match read(text)? {
        Some(Duration::ZERO) => Err(invalid_duration(text.trim().to_lowercase().as_str())),
        other => Ok(other),
    }
}

/// As `parse`, but a zero in any unit (`0s`, `0m`, `0h 0m`) is a duration too: for a setting where
/// none is a value (the unlock delay).
pub fn parse_allowing_zero(text: &str) -> Result<Option<Duration>, String> {
    read(text)
}

fn invalid_duration(text: &str) -> String {
    format!("Invalid duration \"{text}\"; use e.g. 10m (10 minutes), 2h 30m (2 hours 30 minutes) or 1d (1 day)")
}

fn read(text: &str) -> Result<Option<Duration>, String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Ok(None);
    }
    let invalid = || invalid_duration(&text);
    let mut total: u64 = 0;
    let mut rest = text.as_str();
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if digits == 0 {
            return Err(invalid());
        }
        let number: u64 = rest[..digits].parse().map_err(|_| invalid())?;
        rest = rest[digits..].trim_start();
        let letters = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
        let seconds = match &rest[..letters] {
            "s" | "sec" | "secs" | "second" | "seconds" => 1,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            "d" | "day" | "days" => 86400,
            _ => return Err(invalid()),
        };
        rest = &rest[letters..];
        total = number.checked_mul(seconds).and_then(|part| total.checked_add(part)).ok_or_else(invalid)?;
    }
    Ok(Some(Duration::from_secs(total)))
}

/// Seconds left, as short as a countdown wants: the largest unit that is not zero and the one
/// below it if that is not zero either, so never more than two units and no seconds from an hour
/// on: `3 days`, `1 day and 2 hours`, `2 hours and 5 minutes`, `5 minutes and 3 seconds`,
/// `42 seconds`. Every time the window and the CLI show is written by this.
pub fn countdown(seconds: u64) -> String {
    let units = [(seconds / 86400, "day"), (seconds % 86400 / 3600, "hour"), (seconds % 3600 / 60, "minute"), (seconds % 60, "second")];
    let plural = |(n, unit): (u64, &str)| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    let first = units.iter().position(|&(n, _)| n > 0).unwrap_or(units.len() - 1);
    match units.get(first + 1) {
        Some(&next) if next.0 > 0 => format!("{} and {}", plural(units[first]), plural(next)),
        _ => plural(units[first]),
    }
}

/// Time left, as a countdown shows it: one unit, rounded. A unit is used from one and a half of it
/// on, so below that the next one down counts: `2 minutes` for 2 minutes and 5 seconds, `89 seconds`
/// for 1 minute and 29 seconds, `36 hours` for a day and a half less a second, `2 days` from there.
pub fn left(seconds: u64) -> String {
    let units = [(86400, "day"), (3600, "hour"), (60, "minute"), (1, "second")];
    let &(size, unit) = units.iter().find(|&&(size, _)| seconds * 2 >= size * 3).unwrap_or(&units[3]);
    let n = (seconds + size / 2) / size;
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// A countdown after "every": a lone one is left out (`day`, not `1 day`), `2 days` and
/// `1 day and 12 hours` stay as they are.
pub fn period(seconds: u64) -> String {
    let words = countdown(seconds);
    match words.strip_prefix("1 ") {
        Some(rest) if !rest.contains(" and ") => rest.to_string(),
        _ => words,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_left_is_one_unit_from_one_and_a_half_of_it() {
        for (seconds, words) in [
            (0, "0 seconds"),
            (1, "1 second"),
            (89, "89 seconds"),
            (90, "2 minutes"),
            (125, "2 minutes"),
            (3599, "60 minutes"),
            (5399, "90 minutes"),
            (5400, "2 hours"),
            (36 * 3600 - 1, "36 hours"),
            (36 * 3600, "2 days"),
        ] {
            assert_eq!(left(seconds), words, "{seconds}");
        }
    }

    #[test]
    fn a_period_drops_a_lone_one() {
        assert_eq!(period(86400), "day");
        assert_eq!(period(3600), "hour");
        assert_eq!(period(60), "minute");
        assert_eq!(period(2 * 86400), "2 days");
        assert_eq!(period(36 * 3600), "1 day and 12 hours");
    }

    #[test]
    fn a_zero_is_a_duration_only_where_none_is_a_value() {
        for zero in ["0s", "0m", "0h 0m", "0 minutes"] {
            assert!(parse(zero).is_err(), "{zero:?}");
            assert_eq!(parse_allowing_zero(zero), Ok(Some(Duration::ZERO)), "{zero:?}");
        }
        assert_eq!(parse_allowing_zero("1m"), Ok(Some(Duration::from_secs(60))));
        assert_eq!(parse_allowing_zero(" "), Ok(None));
    }

    #[test]
    fn countdowns_are_short() {
        assert_eq!(countdown(0), "0 seconds");
        assert_eq!(countdown(1), "1 second");
        assert_eq!(countdown(59), "59 seconds");
        assert_eq!(countdown(60), "1 minute");
        assert_eq!(countdown(303), "5 minutes and 3 seconds");
        assert_eq!(countdown(3599), "59 minutes and 59 seconds");
        assert_eq!(countdown(3600), "1 hour");
        assert_eq!(countdown(3601), "1 hour");
        assert_eq!(countdown(2 * 3600 + 5 * 60 + 59), "2 hours and 5 minutes");
        assert_eq!(countdown(13 * 3600 + 7 * 60 + 30), "13 hours and 7 minutes");
        assert_eq!(countdown(86400 - 1), "23 hours and 59 minutes");
        assert_eq!(countdown(86400), "1 day");
        assert_eq!(countdown(86400 + 7200 + 60), "1 day and 2 hours");
        assert_eq!(countdown(3 * 86400 + 59), "3 days");
        assert_eq!(countdown(3 * 86400 + 600), "3 days");
    }

    #[test]
    fn single_units() {
        assert_eq!(parse(""), Ok(None));
        assert_eq!(parse("  "), Ok(None));
        assert_eq!(parse("5s"), Ok(Some(Duration::from_secs(5))));
        assert_eq!(parse(" 10M "), Ok(Some(Duration::from_secs(600))));
        assert_eq!(parse("3h"), Ok(Some(Duration::from_secs(10800))));
        assert_eq!(parse("1d"), Ok(Some(Duration::from_secs(86400))));
    }

    #[test]
    fn several_parts_and_words() {
        assert_eq!(parse("2h 5m"), Ok(Some(Duration::from_secs(2 * 3600 + 300))));
        assert_eq!(parse("2h5m"), Ok(Some(Duration::from_secs(2 * 3600 + 300))));
        assert_eq!(parse("1d 12h 30m 10s"), Ok(Some(Duration::from_secs(86400 + 12 * 3600 + 1800 + 10))));
        assert_eq!(parse("90m"), Ok(Some(Duration::from_secs(5400))));
        assert_eq!(parse("5 m"), Ok(Some(Duration::from_secs(300))));
        assert_eq!(parse("2 hours 5 minutes"), Ok(Some(Duration::from_secs(2 * 3600 + 300))));
        assert_eq!(parse("1 Hour, 1 minute, 1 second"), Ok(Some(Duration::from_secs(3661))));
        assert_eq!(parse("1 day"), Ok(Some(Duration::from_secs(86400))));
        assert_eq!(parse("30min"), Ok(Some(Duration::from_secs(1800))));
    }

    #[test]
    fn what_is_not_a_duration() {
        for bad in ["5", "m", "0m", "0h 0m", "5x", "1.5h", "-1h", "h5", "2h 5", "5 parsecs", "99999999999999999999d"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
