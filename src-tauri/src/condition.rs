//! Conditions: when a block is active, and what a condition lock locks by. A condition has a name
//! (`definitions::Condition`) and a rule, of one of two kinds:
//! - a **weekly schedule** (`schedule.rs`): active at the times it has;
//! - a **flash drive** (`drives.rs`): active while that drive is *not* connected, so the drive is
//!   the key that switches a block off (as a lock, the block can only be stopped while it is
//!   plugged in).
//!
//! A block that is not active keeps running, and stays locked, but enforces nothing. A running
//! block holds a copy of the rule it started with (`block::Block`).
//!
//! **A new kind of condition** is a variant of `Rule`, with what it means in `active` and
//! `next_change`, an editor in the window (`src/js/forms.js`) and an option in the CLI (`cli.rs`).

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::drives;
use crate::localtime::Zone;
use crate::schedule::Week;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Rule {
    /// Active when the weekly schedule has it on (`days`, Monday first).
    Schedule {
        #[serde(flatten)]
        week: Week,
    },
    /// Active while the flash drive with this serial number is not connected. `drive` is what
    /// Windows calls it, for words.
    FlashDrive { serial: String, drive: String },
}

impl Rule {
    /// The rule, checked and in order: a week sorted, merged and not empty, a drive chosen.
    pub fn checked(self) -> Result<Rule, String> {
        match self {
            Rule::Schedule { week } => Ok(Rule::Schedule { week: week.normalized()? }),
            Rule::FlashDrive { serial, .. } if serial.trim().is_empty() => Err("Choose the flash drive".into()),
            other => Ok(other),
        }
    }

    /// Whether it is active at `now`, in the time zone a schedule is read in. If the local time
    /// cannot be known, or the connected drives cannot be asked, it is: a block errs on the side
    /// of blocking, and a lock on the side of locking.
    pub fn active(&self, zone: &Zone, now: SystemTime) -> bool {
        match self {
            Rule::Schedule { week } => zone.local(now).is_none_or(|local| week.is_on(local)),
            Rule::FlashDrive { serial, .. } => !drives::is_present(serial),
        }
    }

    /// Seconds until it turns on or off, if it has a time for that: a drive is plugged in or out
    /// when someone does it.
    pub fn next_change(&self, zone: &Zone, now: SystemTime) -> Option<u64> {
        match self {
            Rule::Schedule { week } => week.next_change(zone.local(now)?),
            Rule::FlashDrive { .. } => None,
        }
    }

    /// The week of a weekly schedule (what the tests read).
    #[cfg(test)]
    pub fn week(&self) -> Option<&Week> {
        match self {
            Rule::Schedule { week } => Some(week),
            Rule::FlashDrive { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weekdays() -> Rule {
        Rule::Schedule { week: Week::parse("mon-fri 8-17").unwrap().unwrap() }
    }

    #[test]
    fn a_rule_travels_as_json_with_its_type() {
        let json = serde_json::to_string(&Rule::Schedule { week: Week::parse("sat").unwrap().unwrap() }).unwrap();
        assert_eq!(json, r#"{"type":"schedule","days":[[],[],[],[],[],[[0,1440]],[]]}"#);
        assert_eq!(serde_json::from_str::<Rule>(&json).unwrap(), Rule::Schedule { week: Week::parse("sat").unwrap().unwrap() });
        let drive = Rule::FlashDrive { serial: "4E0985D05EC9".into(), drive: "Kingston DataTraveler 3.0".into() };
        let json = serde_json::to_string(&drive).unwrap();
        assert_eq!(json, r#"{"type":"flash_drive","serial":"4E0985D05EC9","drive":"Kingston DataTraveler 3.0"}"#);
        assert_eq!(serde_json::from_str::<Rule>(&json).unwrap(), drive);
    }

    #[test]
    fn a_rule_is_checked_when_it_is_saved() {
        assert!(weekdays().checked().is_ok());
        assert!(Rule::Schedule { week: Week::default() }.checked().is_err());
        assert!(Rule::FlashDrive { serial: "  ".into(), drive: String::new() }.checked().is_err());
        assert!(Rule::FlashDrive { serial: "x".into(), drive: String::new() }.checked().is_ok());
    }

    #[test]
    fn a_schedule_has_a_week_and_a_drive_does_not() {
        assert!(weekdays().week().is_some());
        let drive = Rule::FlashDrive { serial: "x".into(), drive: String::new() };
        assert!(drive.week().is_none());
        assert_eq!(drive.next_change(&Zone::fixed(0), SystemTime::now()), None);
    }
}
