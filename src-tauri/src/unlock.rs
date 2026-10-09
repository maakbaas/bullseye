//! Unlocking: ending every running session at once, as the Settings page and `bullseye unlock`
//! do, and the passwords of password locks (`new_hash`).
//!
//! **This is the escape hatch, so it must never be unusable.** Unlocking takes a *countdown*: the
//! user chooses how long, from nothing to 24 hours (`UnlockDelay`: a night's sleep, and a fresh mind to abort with), and the default is nothing, so an
//! unconfigured app can always unlock. With a delay, unlocking is asked for (the countdown
//! starts), can be aborted at any time (back to locked), and once the countdown is over the user
//! unlocks or aborts. There is no password to lose: waiting is the only gate. The countdown runs
//! on internet time (`clock.rs`) and is kept on disk (`unlock.json`, which only SYSTEM can change),
//! so it survives a restart. A longer delay is free to set; a shorter one needs the unlock to be
//! ready, or setting it low would be a way around the wait. Whatever is wrong with the file (gone,
//! unreadable, nonsense), the delay is none: a fault opens the hatch, never closes it.
//!
//! The delay is always there as the fallback. Besides it there can be **one other way** to unlock
//! (`Way`), chosen like a session's lock but never the delay: unlocking is ready when the delay is over
//! **or** that way is satisfied (never both): a text or random text typed in, a password, or a
//! condition that is not active. Setting one makes unlocking easier, so
//! it needs unlocking to be ready or no session to be locked, like a shorter delay; removing it is
//! always free.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use pbkdf2::pbkdf2_hmac_array;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::condition::Rule;
use crate::duration;
use crate::localtime::Zone;
use crate::lock::{self, Lock};
use crate::state;
use crate::sync::LockExt;

/// The longest delay: 24 hours.
pub const MAX_DELAY: u64 = 24 * 3600;

/// OWASP's 2023 figure for PBKDF2-HMAC-SHA256.
const ROUNDS: u32 = 600_000;

fn hash(password: &str, salt: &[u8]) -> [u8; 32] {
    pbkdf2_hmac_array::<Sha256, 32>(password.as_bytes(), salt, ROUNDS)
}

/// Compared without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (a, b)| diff | (a ^ b)) == 0
}

/// A password as it is kept: a new random salt and the hash, `salt$hash` in hex. The password
/// itself is never kept. Used for the unlock password and the password of a password lock
/// (`lock::Lock::Password`).
pub fn new_hash(password: &str) -> Result<String, String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| format!("Cannot make a salt: {e}"))?;
    Ok(format!("{}${}", hex(&salt), hex(&hash(password, &salt))))
}

/// Whether `password` is the one `kept` (from `new_hash`) was made from.
pub fn check_hash(password: &str, kept: &str) -> bool {
    let Some((salt, expected)) = kept.split_once('$').and_then(|(s, h)| Some((unhex(s)?, unhex(h)?))) else { return false };
    same(&hash(password, &salt), &expected)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

/// One other way to unlock, besides the delay: the lock method it was chosen as (`lock`, a password
/// as its hash only) and, for the ones that depend on the world, what that was resolved to when it was
/// set (`opening`), so it does not change under a running session.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Way {
    pub lock: Lock,
    #[serde(default)]
    pub opening: Option<Opening>,
}

/// When a `Way` that needs no answer lets go: a condition, as in a session's lock: while it is active
/// it is locked (unlocking has to wait for the delay), and when it is not, unlocking is open.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Opening {
    /// Open while this condition is not active, in this time zone.
    While { rule: Rule, zone: String },
}

impl Way {
    /// Whether it lets go at `now` without an answer. What cannot be known counts as locked here
    /// (`Rule::active` errs towards blocking): the delay is the way out then, so the hatch stays open.
    pub fn is_open(&self, now: SystemTime) -> bool {
        match &self.opening {
            Some(Opening::While { rule, zone }) => !rule.active(&Zone::named(zone), now),
            None => false,
        }
    }

    /// Seconds until it lets go or locks again, where that is known (a condition's next change).
    pub fn left(&self, now: SystemTime) -> Option<u64> {
        match &self.opening {
            Some(Opening::While { rule, zone }) => rule.next_change(&Zone::named(zone), now),
            None => None,
        }
    }

    /// Whether `password` or `text` is the answer of a text or password way.
    pub fn answered(&self, password: Option<&str>, text: Option<&str>) -> bool {
        match &self.lock {
            Lock::Text { text: expected } | Lock::RandomText { text: Some(expected), .. } => {
                text.is_some_and(|given| lock::same_text(expected, given))
            }
            Lock::Password { hash: Some(hash), .. } => password.is_some_and(|given| check_hash(given, hash)),
            _ => false,
        }
    }
}

/// What `unlock.json` holds.
#[derive(Serialize, Deserialize, Default, Clone)]
struct Kept {
    /// The other way to unlock, if one is set.
    #[serde(default)]
    way: Option<Way>,
    /// The countdown, in seconds; 0 (the default) is none.
    #[serde(default)]
    delay: u64,
    /// When unlocking was asked for (milliseconds since the Unix epoch, internet time); none: not asked.
    #[serde(default)]
    requested: Option<u64>,
}

/// How long unlocking takes, and whether it is being waited for.
pub struct UnlockDelay {
    kept: Mutex<Kept>,
    /// `None`: kept for this run only (a test).
    path: Option<PathBuf>,
}

impl Default for UnlockDelay {
    /// No delay, kept for this run only.
    fn default() -> UnlockDelay {
        UnlockDelay { kept: Mutex::new(Kept::default()), path: None }
    }
}

/// A delay as typed: `2h 30m`, `1d`, or nothing (`0`, `0s`, `none`, `off`, empty); at most 24 hours.
pub fn parse_delay(text: &str) -> Result<u64, String> {
    let text = text.trim().to_lowercase();
    let seconds = if ["", "0", "0s", "none", "off"].contains(&text.as_str()) {
        0
    } else {
        duration::parse_allowing_zero(&text)?.map_or(0, |delay| delay.as_secs())
    };
    if seconds > MAX_DELAY {
        return Err("Unlocking can take at most 24 hours".into());
    }
    Ok(seconds)
}

impl UnlockDelay {
    /// The service's: `unlock.json` in its data folder, read back from the last run.
    pub fn new() -> UnlockDelay {
        UnlockDelay::at(state::dir().map(|dir| dir.join("unlock.json")))
    }

    /// Whatever is wrong with the file (missing, unreadable, nonsense), the delay is none. A delay
    /// beyond the longest is the longest.
    pub fn at(path: Option<PathBuf>) -> UnlockDelay {
        let kept = path
            .as_ref()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Kept>(&bytes).ok())
            .map(|kept| Kept { delay: kept.delay.min(MAX_DELAY), ..kept })
            .unwrap_or_default();
        UnlockDelay { kept: Mutex::new(kept), path }
    }

    /// The delay, in seconds.
    pub fn delay(&self) -> u64 {
        self.kept.locked().delay
    }

    /// Seconds until unlocking is ready, rounded up: `Some(0)` is ready now (always, with no delay;
    /// and when the time is up, until it is aborted), `None` is not asked for. A moment before the
    /// request (the clock was moved back) counts as the full wait, not more.
    pub fn remaining(&self, now: SystemTime) -> Option<u64> {
        let kept = self.kept.locked();
        if kept.delay == 0 {
            return Some(0);
        }
        let requested = state::from_unix(kept.requested?);
        let waited = now.duration_since(requested).unwrap_or_default().as_secs_f64();
        Some((kept.delay as f64 - waited).max(0.0).ceil() as u64)
    }

    /// Starts the countdown (asking again while it runs changes nothing).
    pub fn request(&self, now: SystemTime) -> Result<(), String> {
        let mut kept = self.kept.locked();
        if kept.requested.is_some() {
            return Ok(());
        }
        let at = state::to_unix(now).ok_or("Cannot tell the time")?;
        kept.requested = Some(at);
        // Not being able to write it down only costs the countdown a restart: it never stops one
        // from starting, as this is the way out.
        if let Err(why) = self.write(&kept) {
            eprintln!("unlock: {why}");
        }
        Ok(())
    }

    /// Stops the countdown, or ends the readiness: back to locked.
    pub fn abort(&self) -> Result<(), String> {
        let mut kept = self.kept.locked();
        kept.requested = None;
        if let Err(why) = self.write(&kept) {
            eprintln!("unlock: {why}");
        }
        Ok(())
    }

    /// Sets the delay. A longer one is free, as it only makes unlocking harder; a shorter one needs
    /// unlocking to be ready (`remaining` is 0), or it would be a way around the delay.
    pub fn set_delay(&self, seconds: u64, now: SystemTime) -> Result<(), String> {
        self.set(seconds, now, false)
    }

    /// As `set_delay`, for when no session is locked: there is nothing to unlock, and no way to ask
    /// for unlocking either (the window has no button for it), so a shorter delay is free too.
    pub fn set_delay_idle(&self, seconds: u64, now: SystemTime) -> Result<(), String> {
        self.set(seconds, now, true)
    }

    fn set(&self, seconds: u64, now: SystemTime, idle: bool) -> Result<(), String> {
        if seconds > MAX_DELAY {
            return Err("Unlocking can take at most 24 hours".into());
        }
        if !idle && seconds < self.delay() && self.remaining(now) != Some(0) {
            return Err("A shorter delay needs unlocking to be ready: request unlock, and wait for the countdown".into());
        }
        let mut kept = self.kept.locked();
        let next = Kept { delay: seconds, ..kept.clone() };
        self.write(&next)?;
        *kept = next;
        Ok(())
    }

    /// The other way to unlock, if one is set.
    pub fn way(&self) -> Option<Way> {
        self.kept.locked().way.clone()
    }

    /// Sets the other way (`None` removes it: always free). Setting one makes unlocking easier, so,
    /// unless `idle` (no session is locked, so nothing is got around), it needs unlocking to be ready.
    pub fn set_way(&self, way: Option<Way>, now: SystemTime, idle: bool) -> Result<(), String> {
        if way.is_some() && !idle && self.remaining(now) != Some(0) {
            return Err("Another way to unlock needs unlocking to be ready: request unlock and wait for the countdown, or set it while nothing is locked".into());
        }
        let mut kept = self.kept.locked();
        let next = Kept { way, ..kept.clone() };
        self.write(&next)?;
        *kept = next;
        Ok(())
    }

    /// Through a temporary file, so a crash leaves the old file or the new.
    fn write(&self, kept: &Kept) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_string_pretty(kept).map_err(|e| e.to_string())?;
        let dir = path.parent().ok_or_else(|| format!("no folder for {}", path.display()))?;
        fs::create_dir_all(dir).map_err(|e| format!("Cannot save: {e}"))?;
        let temporary = path.with_extension("new");
        fs::write(&temporary, json).map_err(|e| format!("Cannot save: {e}"))?;
        fs::rename(&temporary, path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("Cannot save: {e}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("unlock-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("unlock.json")
    }

    /// A moment `seconds` after a fixed one.
    fn after(seconds: u64) -> SystemTime {
        state::from_unix(1_000_000_000_000 + seconds * 1000)
    }

    // ---- The default: an unconfigured app can always unlock.

    #[test]
    fn with_no_delay_unlocking_is_always_ready() {
        let unlock = UnlockDelay::at(Some(file("none")));
        assert_eq!(unlock.delay(), 0);
        // Ready at any moment, asked for or not, however the clock moves.
        for moment in [after(0), after(1), after(1_000_000_000), SystemTime::UNIX_EPOCH, SystemTime::now()] {
            assert_eq!(unlock.remaining(moment), Some(0));
        }
        unlock.request(after(5)).unwrap();
        unlock.abort().unwrap();
        assert_eq!(unlock.remaining(after(5)), Some(0));
    }

    #[test]
    fn whatever_is_wrong_with_the_file_the_hatch_is_open() {
        let garbage: [&[u8]; 9] = [
            b"",
            b"{ not json",
            b"[]",
            b"null",
            b"{\"delay\": \"soon\"}",
            b"{\"delay\": -5}",
            b"{\"requested\": \"yesterday\"}",
            &[0xff, 0xfe, 0x00, 0x80],
            b"{\"delay\": 3600, \"requested\": 1e99}",
        ];
        for (n, bytes) in garbage.iter().enumerate() {
            let path = file(&format!("garbage{n}"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            let unlock = UnlockDelay::at(Some(path));
            assert_eq!(unlock.delay(), 0, "{:?}", String::from_utf8_lossy(bytes));
            assert_eq!(unlock.remaining(after(0)), Some(0));
        }
        // No file, and a path that is no file at all.
        assert_eq!(UnlockDelay::at(Some(file("missing"))).remaining(after(0)), Some(0));
        assert_eq!(UnlockDelay::at(Some(std::env::temp_dir())).remaining(after(0)), Some(0));
        assert_eq!(UnlockDelay::at(None).remaining(after(0)), Some(0));
    }

    #[test]
    fn a_delay_beyond_the_longest_in_the_file_is_the_longest() {
        let path = file("toolong");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"delay\": 99999999999, \"requested\": 1000000000000}").unwrap();
        let unlock = UnlockDelay::at(Some(path));
        assert_eq!(unlock.delay(), MAX_DELAY);
        // It is ready after the longest wait, never later.
        assert_eq!(unlock.remaining(after(MAX_DELAY)), Some(0));
    }

    // ---- The countdown.

    #[test]
    fn a_delay_is_a_countdown_that_can_be_aborted() {
        let unlock = UnlockDelay::at(Some(file("countdown")));
        unlock.set_delay(3600, after(0)).unwrap();
        // Not asked for: not ready.
        assert_eq!(unlock.remaining(after(0)), None);
        unlock.request(after(10)).unwrap();
        assert_eq!(unlock.remaining(after(10)), Some(3600));
        // Rounded up, never down to ready early.
        assert_eq!(unlock.remaining(after(10) + Duration::from_millis(1500)), Some(3599));
        assert_eq!(unlock.remaining(after(10 + 3599) + Duration::from_millis(1)), Some(1));
        // Asking again does not restart it.
        unlock.request(after(1000)).unwrap();
        assert_eq!(unlock.remaining(after(1010)), Some(2600));
        // Ready at the end, and it stays ready until it is aborted.
        assert_eq!(unlock.remaining(after(10 + 3600)), Some(0));
        assert_eq!(unlock.remaining(after(10 + 90_000)), Some(0));
        // Aborted: back to locked, and it can be asked for again from the start.
        unlock.abort().unwrap();
        assert_eq!(unlock.remaining(after(10 + 3600)), None);
        unlock.request(after(5000)).unwrap();
        assert_eq!(unlock.remaining(after(5000)), Some(3600));
    }

    #[test]
    fn aborting_without_a_request_is_harmless() {
        let unlock = UnlockDelay::at(Some(file("abort")));
        unlock.set_delay(60, after(0)).unwrap();
        unlock.abort().unwrap();
        unlock.abort().unwrap();
        assert_eq!(unlock.remaining(after(0)), None);
    }

    #[test]
    fn the_wait_is_never_longer_than_the_delay_and_always_ends() {
        // For every kind of delay, the hatch opens exactly when the delay is up.
        for delay in [1, 2, 59, 60, 61, 3599, 3600, 43_200, 86_399, MAX_DELAY] {
            let unlock = UnlockDelay::at(None);
            unlock.set_delay(delay, after(0)).unwrap();
            unlock.request(after(100)).unwrap();
            assert_eq!(unlock.remaining(after(100)), Some(delay), "delay {delay}");
            assert_eq!(unlock.remaining(after(100 + delay - 1)), Some(1), "delay {delay}");
            assert_eq!(unlock.remaining(after(100 + delay)), Some(0), "delay {delay}");
            assert_eq!(unlock.remaining(after(100 + delay + 1)), Some(0), "delay {delay}");
        }
    }

    #[test]
    fn moving_the_clock_cannot_lengthen_the_wait_beyond_the_delay() {
        let unlock = UnlockDelay::at(Some(file("clock")));
        unlock.set_delay(3600, after(0)).unwrap();
        unlock.request(after(1000)).unwrap();
        // The clock is moved back before the request: the wait is the whole delay, not more.
        assert_eq!(unlock.remaining(after(0)), Some(3600));
        assert_eq!(unlock.remaining(SystemTime::UNIX_EPOCH), Some(3600));
        // And far forward: ready.
        assert_eq!(unlock.remaining(after(1_000_000_000)), Some(0));
    }

    #[test]
    fn a_request_made_before_a_longer_delay_waits_for_the_longer_one() {
        let unlock = UnlockDelay::at(Some(file("longer")));
        unlock.set_delay(60, after(0)).unwrap();
        unlock.request(after(0)).unwrap();
        // Lengthening is free, and counts for a request already made: no ready at 60 seconds.
        unlock.set_delay(3600, after(30)).unwrap();
        assert_eq!(unlock.remaining(after(60)), Some(3540));
        assert_eq!(unlock.remaining(after(3600)), Some(0));
    }

    // ---- The delay cannot be set to get around itself.

    #[test]
    fn a_longer_delay_is_free_but_a_shorter_one_needs_unlocking_to_be_ready() {
        let unlock = UnlockDelay::at(Some(file("shorter")));
        unlock.set_delay(600, after(0)).unwrap();
        unlock.set_delay(3600, after(0)).unwrap();
        // Setting it lower is no way around the wait: refused until the countdown is over.
        assert!(unlock.set_delay(0, after(0)).is_err() && unlock.set_delay(60, after(0)).is_err());
        unlock.request(after(0)).unwrap();
        assert!(unlock.set_delay(0, after(100)).is_err() && unlock.set_delay(60, after(100)).is_err());
        assert_eq!(unlock.delay(), 3600);
        // Once ready, any shorter delay, down to none, goes.
        unlock.set_delay(60, after(3600)).unwrap();
        assert_eq!(unlock.delay(), 60);
        unlock.set_delay(0, after(3600)).unwrap();
        assert_eq!((unlock.delay(), unlock.remaining(after(3600))), (0, Some(0)));
    }

    #[test]
    fn the_delay_is_at_most_a_day() {
        let unlock = UnlockDelay::at(None);
        unlock.set_delay(MAX_DELAY, after(0)).unwrap();
        assert!(unlock.set_delay(MAX_DELAY + 1, after(0)).is_err());
        assert!(unlock.set_delay(u64::MAX, after(0)).is_err());
        assert_eq!(unlock.delay(), MAX_DELAY);
    }

    // ---- It survives, and a failing disk never closes it.

    #[test]
    fn the_countdown_and_the_delay_survive_a_restart() {
        let path = file("restart");
        let unlock = UnlockDelay::at(Some(path.clone()));
        unlock.set_delay(7200, after(0)).unwrap();
        unlock.request(after(5)).unwrap();
        let again = UnlockDelay::at(Some(path.clone()));
        assert_eq!((again.delay(), again.remaining(after(5))), (7200, Some(7200)));
        // The wait goes on from where it was, not from the restart.
        assert_eq!(again.remaining(after(5 + 7000)), Some(200));
        assert_eq!(again.remaining(after(5 + 7200)), Some(0));
        // And an abort is remembered too.
        again.abort().unwrap();
        assert_eq!(UnlockDelay::at(Some(path)).remaining(after(5)), None);
    }

    #[test]
    fn a_disk_that_cannot_be_written_never_stops_a_request() {
        // The data folder is a file: nothing can be saved there.
        let blocker = file("nowhere");
        fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        fs::write(&blocker, "this is a file").unwrap();
        let unlock = UnlockDelay::at(Some(blocker.join("unlock.json")));
        assert_eq!(unlock.remaining(after(0)), Some(0));
        // The delay cannot be saved, and is not taken as set...
        assert!(unlock.set_delay(60, after(0)).is_err());
        // ...but a countdown that is already set runs in memory without a disk.
        let fine =
            UnlockDelay { kept: Mutex::new(Kept { delay: 60, requested: None, way: None }), path: Some(blocker.join("unlock.json")) };
        fine.request(after(0)).unwrap();
        assert_eq!(fine.remaining(after(0)), Some(60));
        assert_eq!(fine.remaining(after(60)), Some(0));
        fine.abort().unwrap();
        assert_eq!(fine.remaining(after(60)), None);
    }

    #[test]
    fn asking_again_while_it_counts_down_does_not_restart_it() {
        let unlock = UnlockDelay::at(Some(file("again")));
        unlock.set_delay(100, after(0)).unwrap();
        unlock.request(after(10)).unwrap();
        for moment in [11, 50, 109] {
            unlock.request(after(moment)).unwrap();
        }
        assert_eq!(unlock.remaining(after(60)), Some(50));
        assert_eq!(unlock.remaining(after(110)), Some(0));
    }

    // ---- Typing a delay.

    #[test]
    fn a_delay_is_typed_like_a_duration() {
        for none in ["", " ", "0", "0s", "0m", "0h", "0d", "0h 0m", "0 minutes", " None ", "OFF"] {
            assert_eq!(parse_delay(none), Ok(0), "{none:?}");
        }
        assert_eq!(parse_delay("30s"), Ok(30));
        assert_eq!(parse_delay("2h 30m"), Ok(9000));
        assert_eq!(parse_delay("1d"), Ok(MAX_DELAY));
        assert_eq!(parse_delay("24 hours"), Ok(MAX_DELAY));
        assert_eq!(parse_delay("23h 59m 59s"), Ok(MAX_DELAY - 1));
        for wrong in ["2d", "1d 1s", "25h", "1d12h", "soon", "-1h", "1x", "99999999999999999999d"] {
            assert!(parse_delay(wrong).is_err(), "{wrong:?}");
        }
    }

    // ---- The passwords of password locks.

    #[test]
    fn a_lock_password_is_kept_only_as_a_salted_hash() {
        let kept = new_hash("correct horse").unwrap();
        assert!(!kept.contains("correct horse"));
        assert!(check_hash("correct horse", &kept));
        assert!(!check_hash("correct hors", &kept) && !check_hash("", &kept) && !check_hash("correct horse", "nonsense"));
        // A new salt each time: the same password does not give the same hash.
        assert_ne!(new_hash("correct horse").unwrap(), kept);
    }

    // ---- The other way to unlock.

    #[test]
    fn a_condition_way_is_open_while_its_condition_is_not_active() {
        use crate::schedule::Week;
        let office = Rule::Schedule { week: Week::parse("mon 08:00-17:00").unwrap().unwrap() };
        let way = Way {
            lock: Lock::Condition { condition: "c".into() },
            opening: Some(Opening::While { rule: office, zone: "fixed:+0".into() }),
        };
        // 1970-01-05 was a Monday.
        let monday = |hour: u64| std::time::UNIX_EPOCH + Duration::from_secs(4 * 86400 + hour * 3600);
        assert!(!way.is_open(monday(10)), "in office hours: locked");
        assert_eq!(way.left(monday(10)), Some(7 * 3600));
        assert!(way.is_open(monday(18)));
        assert_eq!(way.left(monday(18)), Some(6 * 86400 + 14 * 3600));
        // Nothing to answer: a condition way only lets go by itself.
        assert!(!way.answered(Some("anything"), Some("anything")));
        // A text or password way never lets go by itself, and has no time.
        let text = Way { lock: Lock::Text { text: "a\nb".into() }, opening: None };
        assert!(!text.is_open(monday(18)) && text.left(monday(18)).is_none());
        assert!(text.answered(None, Some("a\r\nb")) && !text.answered(Some("a\nb"), None));
    }
}
