//! Lock methods: what keeps a running block from being stopped. A block is three layers: what it
//! enforces (lists and enforcers, `block::Lists`, `state::Enforcers`), when it is active (its
//! schedule, `definitions::Schedule`), and how it is locked (this).
//!
//! **A new lock method goes here**, as a variant, together with what it means in `Blocklist`
//! (`block.rs`), an entry in the UI's `LOCKS` table and an option in the CLI (`cli.rs`, `CLI.md`).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::datetime::DateTime;
use crate::duration;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Lock {
    /// Not locked: the block can be stopped at any time.
    #[default]
    None,
    /// Locked for a while (`2h 30m`, `duration.rs`): the block cannot be stopped, and ends by
    /// itself when the time is up. Measured on internet time.
    Duration { duration: String },
    /// Locked until a local date and time (`2026-10-06 18:00`, `datetime.rs`): the same as a
    /// duration lock that ends then. Read in Windows' time zone when the block starts, and then
    /// measured on internet time; a time that has passed by then does not start.
    Until { until: String },
    /// Locked until Windows restarts (`boot.rs`): the block cannot be stopped before, and ends with
    /// the restart. A shut down with Fast Startup is not a restart (Windows hibernates then).
    UntilRestart,
    /// Locked while a schedule of its own choosing is active (`schedule`, a schedule's id): it
    /// cannot be stopped then, and can be outside those times; it runs until stopped. With the
    /// block's own schedule, it is locked exactly while it is active.
    Scheduled { schedule: String },
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
    /// As a client may see it: a password lock without its hash.
    pub fn redacted(&self) -> Lock {
        match self {
            Lock::Password { .. } => Lock::Password { password: None, hash: None },
            other => other.clone(),
        }
    }

    /// The schedule of a scheduled lock, by id.
    pub fn schedule(&self) -> Option<&str> {
        match self {
            Lock::Scheduled { schedule } => Some(schedule),
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

    /// How long a duration lock locks the block for; `None` for the other methods.
    pub fn duration(&self) -> Result<Option<Duration>, String> {
        match self {
            Lock::Duration { duration } => duration::parse(duration)?.map(Some).ok_or_else(|| "Enter how long to lock the block for, e.g. 2h 30m".into()),
            _ => Ok(None),
        }
    }

    /// The local date and time an until lock ends at; `None` for the other methods.
    pub fn until(&self) -> Result<Option<DateTime>, String> {
        match self {
            Lock::Until { until } => DateTime::parse(until).map(Some),
            _ => Ok(None),
        }
    }

    /// Whether what it was given can be read (a duration, a date and time), for a clear message
    /// when it is saved rather than when it starts.
    pub fn check(&self) -> Result<(), String> {
        self.duration()?;
        self.until()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_says_how_long() {
        assert_eq!(Lock::None.duration(), Ok(None));
        assert_eq!(Lock::Duration { duration: "2h 5m".into() }.duration(), Ok(Some(Duration::from_secs(7500))));
        assert!(Lock::Duration { duration: "".into() }.duration().is_err());
        assert!(Lock::Duration { duration: "soon".into() }.duration().is_err());
        assert_eq!(Lock::Scheduled { schedule: "s".into() }.duration(), Ok(None));
        let until = Lock::Until { until: "2026-10-06 18:00".into() };
        assert_eq!(until.until().unwrap().map(|at| at.to_string()).as_deref(), Some("Tuesday 6 October 2026, 18:00"));
        assert_eq!(until.duration(), Ok(None));
        assert!(Lock::Until { until: "tomorrow".into() }.check().is_err());
        assert!(Lock::UntilRestart.check().is_ok());
    }

    #[test]
    fn a_lock_travels_as_json() {
        assert_eq!(serde_json::to_string(&Lock::None).unwrap(), r#"{"method":"none"}"#);
        let timed = Lock::Duration { duration: "10m".into() };
        assert_eq!(serde_json::to_string(&timed).unwrap(), r#"{"method":"duration","duration":"10m"}"#);
        assert_eq!(serde_json::from_str::<Lock>(r#"{"method":"duration","duration":"10m"}"#).unwrap(), timed);
        assert_eq!(serde_json::to_string(&Lock::Until { until: "2026-10-06 18:00".into() }).unwrap(), r#"{"method":"until","until":"2026-10-06 18:00"}"#);
        assert_eq!(serde_json::to_string(&Lock::UntilRestart).unwrap(), r#"{"method":"until_restart"}"#);
        assert!(serde_json::from_str::<Lock>(r#"{"method":"forever"}"#).is_err());
        assert_eq!(serde_json::to_string(&Lock::Scheduled { schedule: "s".into() }).unwrap(), r#"{"method":"scheduled","schedule":"s"}"#);
        // A client sends the password; what it gets back has neither the password nor the hash.
        let sent: Lock = serde_json::from_str(r#"{"method":"password","password":"secret"}"#).unwrap();
        assert_eq!(sent, Lock::Password { password: Some("secret".into()), hash: None });
        let kept = Lock::Password { password: None, hash: Some("ab$cd".into()) };
        assert_eq!(serde_json::to_string(&kept.redacted()).unwrap(), r#"{"method":"password"}"#);
    }
}
