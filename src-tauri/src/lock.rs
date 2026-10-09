//! Lock methods: what keeps a running session from being stopped. A session is three layers: what it
//! enforces (lists and actions, `lists::Lists`, `state::Actions`), when it is active (its
//! condition, `condition.rs`), and how it is locked (this).
//!
//! **A new lock method goes here**, as a variant, together with what it means in `Sessions`
//! (`session.rs`), an entry in the UI's `LOCKS` table and an option in the CLI (`cli/`, `CLI.md`).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::datetime::DateTime;
use crate::duration;

/// The longest text a text lock can ask to be typed: it is shown, and typed, by hand.
const MAX_TEXT: usize = 1000;
/// The longest random text, in characters.
pub const MAX_RANDOM: u32 = 1000;

/// The longest wait of a delay lock, as long as the unlock delay can be.
const MAX_DELAY: Duration = Duration::from_secs(24 * 3600);

/// What a timed lock's value is: how long it lasts, or when it ends.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimedKind {
    For,
    Until,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Lock {
    /// Not locked: the session can be stopped at any time.
    #[default]
    None,
    /// Locked until a time: the session cannot be stopped, and ends by itself when the time is up.
    /// `kind` says what `value` is: `For`, a while (`2h 30m`, `duration.rs`), or `Until`, a local
    /// date and time (`2026-10-06 18:00`, `datetime.rs`), the same as a lock for the while that ends
    /// then. Read in Windows' time zone when the session starts, and then measured on internet time;
    /// a time that has passed by then does not start.
    Timed { kind: TimedKind, value: String },
    /// Locked until Windows restarts (`boot.rs`): the session cannot be stopped before, and ends with
    /// the restart. A shut down with Fast Startup is not a restart (Windows hibernates then).
    UntilRestart,
    /// Locked while a condition of its own choosing is active (`condition`, a condition's id,
    /// `condition.rs`): it cannot be stopped then, and can be otherwise; it runs until stopped. A
    /// weekly schedule locks it at those times; a flash drive locks it while the drive is not
    /// connected, so the drive is the key. With the session's own condition, it is locked exactly
    /// while it is active.
    Condition { condition: String },
    /// Locked behind a wait, like unlocking (`unlock.rs`): stopping is asked for, counts down `delay`
    /// (`2h 30m`) on internet time, can be aborted, and once the wait is over the session can be
    /// stopped. Until then it cannot be, so it is locked for the delay at least.
    Delay { delay: String },
    /// Locked until a text of the user's own choosing is typed in: the session can only be stopped
    /// by typing `text` exactly as it is shown (the window shows it above an input that cannot be
    /// pasted into, and the service compares the two). It is no secret, so it is kept as it is.
    Text { text: String },
    /// Locked until a random text is typed in: `length` characters of a-z and 0-9 (`random_text`),
    /// drawn when the session starts and then held like a custom text (`Text`), so it cannot be
    /// learnt beforehand. As the other way to unlock (`unlock.rs`), `text` is the one drawn for it,
    /// drawn again after each unlock; a session's definition has none.
    RandomText {
        length: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Locked with a password of its own: it runs until stopped with that password (or the unlock
    /// password). Meant for someone else to set, so the user does not know it. Only a salted hash
    /// of it is kept (`unlock::new_hash`); neither is ever sent back to a client (`redacted`).
    Password {
        /// The password as typed, on its way to the service when it is set or changed; the service
        /// turns it into `hash` at once and never keeps or returns it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        /// What the service keeps (`salt$hash`). Missing from a client: a password lock saved
        /// without a new `password` keeps the hash it has.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hash: Option<String>,
    },
}

impl Lock {
    /// A timed lock for a while (`2h 30m`).
    pub fn timed_for(value: impl Into<String>) -> Lock {
        Lock::Timed { kind: TimedKind::For, value: value.into() }
    }

    /// A timed lock until a local date and time (`2026-10-06 18:00`).
    pub fn timed_until(value: impl Into<String>) -> Lock {
        Lock::Timed { kind: TimedKind::Until, value: value.into() }
    }

    /// As a client may see it: a password lock without its hash.
    pub fn redacted(&self) -> Lock {
        match self {
            Lock::Password { .. } => Lock::Password { password: None, hash: None },
            other => other.clone(),
        }
    }

    /// Whether it has settings worth keeping when another method is chosen (`Definition::kept_locks`).
    pub fn has_settings(&self) -> bool {
        !matches!(self, Lock::None | Lock::UntilRestart)
    }

    /// Whether the two are the same method, whatever their settings.
    pub fn same_method(&self, other: &Lock) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// The condition of a condition lock, by id.
    pub fn condition(&self) -> Option<&str> {
        match self {
            Lock::Condition { condition } => Some(condition),
            _ => None,
        }
    }

    /// The hash of a password lock.
    pub fn password_hash(&self) -> Option<&str> {
        match self {
            Lock::Password { hash, .. } => hash.as_deref(),
            _ => None,
        }
    }

    /// How long a timed lock for a while locks the session for; `None` for the other locks.
    pub fn duration(&self) -> Result<Option<Duration>, String> {
        match self {
            Lock::Timed { kind: TimedKind::For, value: duration } => {
                duration::parse(duration)?.map(Some).ok_or_else(|| "Enter how long to lock the session for, e.g. 2h 30m".into())
            }
            _ => Ok(None),
        }
    }

    /// How long stopping a delay lock takes; `None` for the other methods.
    pub fn delay(&self) -> Result<Option<Duration>, String> {
        match self {
            Lock::Delay { delay } => {
                let wait = duration::parse(delay)?.ok_or("Enter how long stopping the session takes, e.g. 2h 30m")?;
                if wait > MAX_DELAY {
                    return Err("Stopping can take at most 24 hours".into());
                }
                Ok(Some(wait))
            }
            _ => Ok(None),
        }
    }

    /// The local date and time a timed lock until a time ends at; `None` for the other locks.
    pub fn until(&self) -> Result<Option<DateTime>, String> {
        match self {
            Lock::Timed { kind: TimedKind::Until, value } => DateTime::parse(value).map(Some),
            _ => Ok(None),
        }
    }

    /// Whether what it was given can be read (a duration, a date and time), for a clear message
    /// when it is saved rather than when it starts.
    pub fn check(&self) -> Result<(), String> {
        self.duration()?;
        self.delay()?;
        self.until()?;
        if let Lock::RandomText { length, .. } = self {
            if !(1..=MAX_RANDOM).contains(length) {
                return Err(format!("A random text has 1 to {MAX_RANDOM} characters"));
            }
        }
        if let Lock::Text { text } = self {
            if text.trim().is_empty() {
                return Err("Give the text that has to be typed to stop the session".into());
            }
            if text.chars().count() > MAX_TEXT {
                return Err(format!("Keep the text under {MAX_TEXT} characters"));
            }
        }
        Ok(())
    }
}

/// `length` random characters of a-z and 0-9, each as likely as the others.
pub fn random_text(length: u32) -> Result<String, String> {
    const CHARACTERS: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut text = String::new();
    while text.len() < length as usize {
        let mut bytes = [0u8; 64];
        getrandom::fill(&mut bytes).map_err(|e| format!("cannot draw a random text: {e}"))?;
        // 252 is 7 × 36: a byte at or above it would favour the first characters.
        for byte in bytes.into_iter().filter(|&b| b < 252).take(length as usize - text.len()) {
            text.push(CHARACTERS[usize::from(byte % 36)] as char);
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_text_is_drawn_from_letters_and_digits() {
        let drawn = random_text(MAX_RANDOM).unwrap();
        assert_eq!(drawn.len(), MAX_RANDOM as usize);
        assert!(drawn.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{drawn}");
        assert_ne!(random_text(30).unwrap(), random_text(30).unwrap());
        assert_eq!(random_text(0).unwrap(), "");
        let random = |length| Lock::RandomText { length, text: None };
        assert!(random(1).check().is_ok() && random(MAX_RANDOM).check().is_ok());
        assert!(random(0).check().is_err() && random(MAX_RANDOM + 1).check().is_err());
        // Kept as a method of its own, the text only when there is one.
        assert_eq!(serde_json::to_value(random(12)).unwrap(), serde_json::json!({ "method": "random_text", "length": 12 }));
    }

    #[test]
    fn a_lock_says_how_long() {
        assert_eq!(Lock::None.duration(), Ok(None));
        assert_eq!(Lock::timed_for("2h 5m").duration(), Ok(Some(Duration::from_secs(7500))));
        assert!(Lock::timed_for("").duration().is_err());
        assert!(Lock::timed_for("soon").duration().is_err());
        assert_eq!(Lock::Condition { condition: "s".into() }.duration(), Ok(None));
        let until = Lock::timed_until("2026-10-06 18:00");
        assert_eq!(until.until().unwrap().map(|at| at.to_string()).as_deref(), Some("Tuesday 6 October 2026, 18:00"));
        assert_eq!(until.duration(), Ok(None));
        assert!(Lock::timed_until("tomorrow").check().is_err());
        assert!(Lock::UntilRestart.check().is_ok());
        let delayed = Lock::Delay { delay: "1h".into() };
        assert_eq!(delayed.delay(), Ok(Some(Duration::from_secs(3600))));
        assert!(delayed.check().is_ok() && delayed.has_settings());
        assert!(Lock::Delay { delay: "".into() }.check().is_err());
        assert!(Lock::Delay { delay: "2d".into() }.check().is_err());
        assert_eq!(delayed.duration(), Ok(None));
    }

    #[test]
    fn a_lock_travels_as_json() {
        assert_eq!(serde_json::to_string(&Lock::None).unwrap(), r#"{"method":"none"}"#);
        let timed = Lock::timed_for("10m");
        assert_eq!(serde_json::to_string(&timed).unwrap(), r#"{"method":"timed","kind":"for","value":"10m"}"#);
        assert_eq!(serde_json::from_str::<Lock>(r#"{"method":"timed","kind":"for","value":"10m"}"#).unwrap(), timed);
        assert_eq!(
            serde_json::to_string(&Lock::timed_until("2026-10-06 18:00")).unwrap(),
            r#"{"method":"timed","kind":"until","value":"2026-10-06 18:00"}"#
        );
        // One method, whichever way it is given: they keep their settings together.
        assert!(timed.same_method(&Lock::timed_until("2026-10-06 18:00")));
        assert_eq!(serde_json::to_string(&Lock::UntilRestart).unwrap(), r#"{"method":"until_restart"}"#);
        let typed = Lock::Text { text: "I will not scroll".into() };
        assert_eq!(serde_json::to_string(&typed).unwrap(), r#"{"method":"text","text":"I will not scroll"}"#);
        assert_eq!(serde_json::from_str::<Lock>(r#"{"method":"text","text":"I will not scroll"}"#).unwrap(), typed);
        assert!(typed.check().is_ok() && typed.has_settings());
        assert!(Lock::Text { text: "  \n ".into() }.check().is_err());
        assert!(Lock::Text { text: "x".repeat(MAX_TEXT + 1) }.check().is_err());
        assert!(serde_json::from_str::<Lock>(r#"{"method":"forever"}"#).is_err());
        assert_eq!(serde_json::to_string(&Lock::Condition { condition: "s".into() }).unwrap(), r#"{"method":"condition","condition":"s"}"#);
        // A client sends the password; what it gets back has neither the password nor the hash.
        let sent: Lock = serde_json::from_str(r#"{"method":"password","password":"secret"}"#).unwrap();
        assert_eq!(sent, Lock::Password { password: Some("secret".into()), hash: None });
        let kept = Lock::Password { password: None, hash: Some("ab$cd".into()) };
        assert_eq!(serde_json::to_string(&kept.redacted()).unwrap(), r#"{"method":"password"}"#);
    }
}
