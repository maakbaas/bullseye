//! Durations as people type and read them. Typed short (`5s`, `10m`, `3h`, `1d`, or several
//! together: `2h 5m`, `1d12h`) or in words (`2 hours 5 minutes`); shown in words
//! (`2 hours 5 minutes`), wherever the app or the CLI shows one.

use std::time::Duration;

/// Parses a duration: parts of a number and a unit, in any order, with or without spaces between
/// them (`2h 30m`, `2h30m`, `90m`, `1 day 2 hours`). Units are `s`/`sec`/`second(s)`,
/// `m`/`min`/`minute(s)`, `h`/`hr`/`hour(s)` and `d`/`day(s)`, any case. An empty text means no
/// duration.
pub fn parse(text: &str) -> Result<Option<Duration>, String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Ok(None);
    }
    let invalid = || format!("Invalid duration \"{text}\"; use e.g. 10m (10 minutes), 2h 30m (2 hours 30 minutes) or 1d (1 day)");
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
    if total == 0 {
        return Err(invalid());
    }
    Ok(Some(Duration::from_secs(total)))
}

/// Seconds in words, to the three largest units from the first that is not zero, leaving out the
/// ones that are: `1 day, 2 hours and 5 minutes`, `2 hours and 5 seconds`, `1 minute`, `0 seconds`.
/// The window and the CLI show every time this way.
pub fn words(seconds: u64) -> String {
    let units = [(seconds / 86400, "day"), (seconds % 86400 / 3600, "hour"), (seconds % 3600 / 60, "minute"), (seconds % 60, "second")];
    let first = units.iter().position(|&(n, _)| n > 0).unwrap_or(units.len() - 1);
    let parts: Vec<String> = units[first..]
        .iter()
        .take(3)
        .enumerate()
        .filter(|&(i, &(n, _))| n > 0 || i == 0)
        .map(|(_, &(n, unit))| format!("{n} {unit}{}", if n == 1 { "" } else { "s" }))
        .collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn durations_in_words() {
        assert_eq!(words(0), "0 seconds");
        assert_eq!(words(1), "1 second");
        assert_eq!(words(59), "59 seconds");
        assert_eq!(words(60), "1 minute");
        assert_eq!(words(61), "1 minute and 1 second");
        assert_eq!(words(3600), "1 hour");
        assert_eq!(words(2 * 3600 + 5), "2 hours and 5 seconds");
        assert_eq!(words(2 * 3600 + 5 * 60 + 3), "2 hours, 5 minutes and 3 seconds");
        assert_eq!(words(86400 + 7200 + 60 + 5), "1 day, 2 hours and 1 minute");
        // Seconds are past the three largest units from a day on.
        assert_eq!(words(86400 + 5), "1 day");
        assert_eq!(words(3 * 86400), "3 days");
    }
}
