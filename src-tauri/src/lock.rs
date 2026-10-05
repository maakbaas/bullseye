//! Lock methods: what keeps a running block from being stopped. A block is three layers: what it
//! enforces (lists and enforcers, `block::Lists`, `state::Enforcers`), when it is active (its
//! schedule, `definitions::Schedule`), and how it is locked (this).
//!
//! **A new lock method goes here**, as a variant, together with what it means in `Blocklist`
//! (`block.rs`), an entry in the UI's `LOCKS` table and an option in the CLI (`cli.rs`, `CLI.md`).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::duration;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Lock {
    /// Not locked: the block can be stopped at any time.
    #[default]
    None,
    /// Locked for a while (`2h 30m`, `duration.rs`): the block cannot be stopped, and ends by
    /// itself when the time is up. Measured on internet time.
    Timer { duration: String },
}

impl Lock {
    /// How long the block is locked for; `None` for a block that is not locked.
    pub fn duration(&self) -> Result<Option<Duration>, String> {
        match self {
            Lock::None => Ok(None),
            Lock::Timer { duration } => duration::parse(duration)?.map(Some).ok_or_else(|| "Enter how long to lock the block for, e.g. 2h 30m".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_says_how_long() {
        assert_eq!(Lock::None.duration(), Ok(None));
        assert_eq!(Lock::Timer { duration: "2h 5m".into() }.duration(), Ok(Some(Duration::from_secs(7500))));
        assert!(Lock::Timer { duration: "".into() }.duration().is_err());
        assert!(Lock::Timer { duration: "soon".into() }.duration().is_err());
    }

    #[test]
    fn a_lock_travels_as_json() {
        assert_eq!(serde_json::to_string(&Lock::None).unwrap(), r#"{"method":"none"}"#);
        let timer = Lock::Timer { duration: "10m".into() };
        assert_eq!(serde_json::to_string(&timer).unwrap(), r#"{"method":"timer","duration":"10m"}"#);
        assert_eq!(serde_json::from_str::<Lock>(r#"{"method":"timer","duration":"10m"}"#).unwrap(), timer);
        assert!(serde_json::from_str::<Lock>(r#"{"method":"forever"}"#).is_err());
    }
}
