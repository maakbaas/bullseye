//! Conditions: when a session is active, and what a condition lock locks by. A condition has a name
//! (`definitions::Condition`) and a rule, of one of three kinds:
//! - a **weekly schedule** (`schedule.rs`): active at the times it has;
//! - a **flash drive** (`drives.rs`): active while that drive is *not* connected, so the drive is
//!   the key that switches a session off (as a lock, the session can only be stopped while it is
//!   plugged in);
//! - a **combination** of two or more of the others: active when any of them is, or when all are
//!   (`Mode`). Never a combination of combinations. A session can use one anywhere it can use a
//!   condition, as its condition or its lock's.
//!
//! A session that is not active keeps running, and stays locked, but enforces nothing. A running
//! session holds a copy of the rule it started with (`session::Session`).
//!
//! **A new kind of condition** is a variant of `Rule`, with what it means in `active` and
//! `next_change`, an editor in the window (`src/js/forms.js`) and an option in the CLI (`cli.rs`).

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::drives;
use crate::localtime::Zone;
use crate::schedule::Week;
use crate::webhook::{self, Hook};

/// How a combined condition puts its conditions together.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Active when any of them is (OR).
    Any,
    /// Active when all of them are (AND).
    All,
}

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
    /// Active or not by what a web address answers, asked every so often (`webhook.rs`).
    Webhook(Hook),
    /// Two or more of the other conditions, put together by `mode`; never one of its own kind, so
    /// no nesting. As defined it holds their ids (`members`); a running session holds copies of what
    /// they were when it started (`rules`, in the order of `members`, which a definition has empty:
    /// `definitions::Library::rule_of` fills them in).
    Combined {
        mode: Mode,
        members: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rules: Vec<Rule>,
    },
}

impl Rule {
    /// The rule, checked and in order: a week sorted, merged and not empty, a drive chosen.
    pub fn checked(self) -> Result<Rule, String> {
        match self {
            Rule::Schedule { week } => Ok(Rule::Schedule { week: week.normalized()? }),
            Rule::FlashDrive { serial, .. } if serial.trim().is_empty() => Err("Choose the flash drive".into()),
            Rule::Combined { mode, members, rules } => {
                if members.len() < 2 {
                    return Err("Choose at least two conditions to combine".into());
                }
                let mut once = members.clone();
                once.sort();
                once.dedup();
                if once.len() != members.len() {
                    return Err("A condition can be in a combination only once".into());
                }
                let rules = rules
                    .into_iter()
                    .map(|rule| match rule {
                        Rule::Combined { .. } => Err("A combination cannot hold another combination".to_string()),
                        rule => rule.checked(),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if !rules.is_empty() && rules.len() != members.len() {
                    return Err("A combination holds one rule for each of its conditions".into());
                }
                Ok(Rule::Combined { mode, members, rules })
            }
            Rule::Webhook(hook) => {
                hook.check()?;
                Ok(Rule::Webhook(Hook { url: hook.url.trim().to_string(), ..hook }))
            }
            other => Ok(other),
        }
    }

    /// Whether it is active at `now`, in the time zone a schedule is read in. If the local time
    /// cannot be known, or the connected drives cannot be asked, it is: a session errs on the side
    /// of blocking, and a lock on the side of locking.
    pub fn active(&self, zone: &Zone, now: SystemTime) -> bool {
        match self {
            Rule::Schedule { week } => zone.local(now).is_none_or(|local| week.is_on(local)),
            Rule::FlashDrive { serial, .. } => !drives::is_present(serial),
            // What the poller last found out, or the fallback (`webhook::answer`).
            Rule::Webhook(hook) => webhook::answer(hook),
            // Unresolved (no rules): not known, so active.
            Rule::Combined { mode, rules, .. } => {
                rules.is_empty()
                    || match mode {
                        Mode::Any => rules.iter().any(|rule| rule.active(zone, now)),
                        Mode::All => rules.iter().all(|rule| rule.active(zone, now)),
                    }
            }
        }
    }

    /// Seconds until it turns on or off, if it has a time for that: a drive is plugged in or out
    /// when someone does it.
    pub fn next_change(&self, zone: &Zone, now: SystemTime) -> Option<u64> {
        match self {
            Rule::Schedule { week } => week.next_change(zone.local(now)?),
            Rule::FlashDrive { .. } | Rule::Webhook(_) => None,
            // When the combination itself turns on or off, which is not every time one of its
            // conditions does: look at each change of any of them in turn, a day-count at most.
            // With a drive in it there is no time: it is plugged in or out by hand, so a time
            // worked out from the schedules alone would be a guess that the drive can undo.
            Rule::Combined { rules, .. } => {
                if rules.iter().any(|rule| matches!(rule, Rule::FlashDrive { .. } | Rule::Webhook(_))) {
                    return None;
                }
                let now_active = self.active(zone, now);
                let mut offset = 0u64;
                for _ in 0..64 {
                    let step = rules.iter().filter_map(|rule| rule.next_change(zone, now + Duration::from_secs(offset))).min()?;
                    if step == 0 {
                        return None;
                    }
                    offset += step;
                    if self.active(zone, now + Duration::from_secs(offset + 1)) != now_active {
                        return Some(offset);
                    }
                }
                None
            }
        }
    }

    /// The ids of the conditions a combination is of; none for any other.
    pub fn members(&self) -> &[String] {
        match self {
            Rule::Combined { members, .. } => members,
            _ => &[],
        }
    }

    /// Whether `new` is active whenever this is, which is all a locked session may be changed by: a
    /// schedule that keeps every time it has (and may add more), a drive that is the same one
    /// (named differently, maybe). A condition of another type, or another drive, is not stricter
    /// or looser but something else. The error says what is wrong, for a person.
    pub fn tightened_by(&self, new: &Rule) -> Result<(), String> {
        match (self, new.clone().checked()?) {
            (Rule::Schedule { week: old }, Rule::Schedule { week: new }) => {
                let covered =
                    old.days.iter().zip(&new.days).all(|(old, new)| old.iter().all(|s| new.iter().any(|n| n.0 <= s.0 && s.1 <= n.1)));
                if covered {
                    Ok(())
                } else {
                    Err("A locked session uses this schedule, so it can only get more times, never lose any".into())
                }
            }
            (Rule::FlashDrive { serial: old, .. }, Rule::FlashDrive { serial: new, .. }) if old.trim().eq_ignore_ascii_case(new.trim()) => {
                Ok(())
            }
            // More active is stricter: an "any" with more conditions, an "all" with fewer, or an
            // "all" made an "any". (The conditions in it have their own say, `Library::usages`.)
            (Rule::Combined { mode: old_mode, members: old, .. }, Rule::Combined { mode: new_mode, members: new, .. }) => {
                let more = match (old_mode, new_mode) {
                    (Mode::Any, Mode::Any) => old.iter().all(|m| new.contains(m)),
                    (Mode::All, Mode::All) => new.iter().all(|m| old.contains(m)),
                    (Mode::All, Mode::Any) => new.iter().any(|m| old.contains(m)),
                    (Mode::Any, Mode::All) => false,
                };
                if more {
                    Ok(())
                } else {
                    Err("A locked session uses this combination, so it can only get more active: conditions added to an \"any\", taken out of an \"all\", or an \"all\" made an \"any\"".into())
                }
            }
            // What a web address will answer cannot be compared: only the same one is allowed.
            (Rule::Webhook(old), Rule::Webhook(new)) if *old == new => Ok(()),
            (Rule::Webhook(_), _) => Err("A locked session uses this webhook condition, so it cannot change".into()),
            _ => Err("A locked session uses this condition, so its type and its drive cannot change".into()),
        }
    }

    /// The week of a weekly schedule (what the tests read).
    #[cfg(test)]
    pub fn week(&self) -> Option<&Week> {
        match self {
            Rule::Schedule { week } => Some(week),
            Rule::FlashDrive { .. } | Rule::Webhook(_) | Rule::Combined { .. } => None,
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
    fn a_rule_can_only_be_tightened_by_more_of_the_same() {
        let rule = |text: &str| Rule::Schedule { week: Week::parse(text).unwrap().unwrap() };
        let drive = |serial: &str, name: &str| Rule::FlashDrive { serial: serial.into(), drive: name.into() };
        let office = rule("mon-fri 8-17");
        // The same, with more days, longer hours, or spans that join up.
        assert!(office.tightened_by(&rule("mon-fri 8-17")).is_ok());
        assert!(office.tightened_by(&rule("mon-fri 8-17; sat 10-12")).is_ok());
        assert!(office.tightened_by(&rule("mon-fri 7-18")).is_ok());
        assert!(rule("mon 8-10; mon 10-12").tightened_by(&rule("mon 8-12")).is_ok());
        // Less of it, or something else: no.
        assert!(office.tightened_by(&rule("mon-fri 9-17")).is_err());
        assert!(office.tightened_by(&rule("mon-thu 8-17")).is_err());
        assert!(office.tightened_by(&rule("sat")).is_err());
        assert!(office.tightened_by(&drive("x", "Stick")).is_err());
        // A drive stays the same drive; its name may change.
        assert!(drive("x", "Stick").tightened_by(&drive("X ", "Renamed")).is_ok());
        assert!(drive("x", "Stick").tightened_by(&drive("y", "Stick")).is_err());
        assert!(drive("x", "Stick").tightened_by(&office).is_err());
    }

    fn combined(mode: Mode, members: &[&str], rules: &[Rule]) -> Rule {
        Rule::Combined { mode, members: members.iter().map(|m| m.to_string()).collect(), rules: rules.to_vec() }
    }

    #[test]
    fn a_combination_is_active_when_any_or_all_of_its_rules_are() {
        let zone = Zone::fixed(0);
        let day = |text: &str| Rule::Schedule { week: Week::parse(text).unwrap().unwrap() };
        // 1 January 1970 was a Thursday.
        let thursday = SystemTime::UNIX_EPOCH;
        let any = |rules: &[Rule]| combined(Mode::Any, &["a", "b"], rules);
        let all = |rules: &[Rule]| combined(Mode::All, &["a", "b"], rules);
        assert!(any(&[day("thu"), day("fri")]).active(&zone, thursday));
        assert!(!any(&[day("fri"), day("sat")]).active(&zone, thursday));
        assert!(all(&[day("thu"), day("daily")]).active(&zone, thursday));
        assert!(!all(&[day("thu"), day("fri")]).active(&zone, thursday));
    }

    #[test]
    fn a_combination_changes_when_the_combination_does_not_when_one_of_its_rules_does() {
        let zone = Zone::fixed(0);
        let day = |text: &str| Rule::Schedule { week: Week::parse(text).unwrap().unwrap() };
        let at = |hours: f64| SystemTime::UNIX_EPOCH + Duration::from_secs((hours * 3600.0) as u64);
        let any = combined(Mode::Any, &["a", "b"], &[day("thu 8-10"), day("thu 9-12")]);
        // Off: on when the first one is, at 8:00.
        assert!(matches!(any.next_change(&zone, at(0.0)), Some(s) if (28799..=28801).contains(&s)));
        // On at 8:30: the first ending at 10:00 changes nothing, the second ending at 12:00 does.
        assert!(matches!(any.next_change(&zone, at(8.5)), Some(s) if (12599..=12601).contains(&s)));
        let all = combined(Mode::All, &["a", "b"], &[day("thu 8-10"), day("thu 9-12")]);
        // Both on only from 9:00 to 10:00.
        assert!(matches!(all.next_change(&zone, at(8.5)), Some(s) if (1799..=1801).contains(&s)));
        // Drives are plugged in and out by hand: nothing to count down to.
        let drives = combined(
            Mode::Any,
            &["a", "b"],
            &[Rule::FlashDrive { serial: "x".into(), drive: String::new() }, Rule::FlashDrive { serial: "y".into(), drive: String::new() }],
        );
        assert_eq!(drives.next_change(&zone, at(0.0)), None);
        // A drive with a schedule: the schedule's times say nothing while the drive can change it.
        for mode in [Mode::Any, Mode::All] {
            let mixed = combined(mode, &["a", "b"], &[day("thu 8-10"), Rule::FlashDrive { serial: "x".into(), drive: String::new() }]);
            assert_eq!(mixed.next_change(&zone, at(0.0)), None);
            assert_eq!(mixed.next_change(&zone, at(8.5)), None);
        }
    }

    #[test]
    fn a_combination_needs_two_different_conditions_and_no_nesting() {
        let day = Rule::Schedule { week: Week::parse("sat").unwrap().unwrap() };
        assert!(combined(Mode::Any, &["a", "b"], &[]).checked().is_ok());
        assert!(combined(Mode::Any, &["a"], &[]).checked().is_err());
        assert!(combined(Mode::All, &["a", "a"], &[]).checked().is_err());
        assert!(combined(Mode::All, &["a", "b"], std::slice::from_ref(&day)).checked().is_err());
        let nested = combined(Mode::Any, &["a", "b"], &[day.clone(), combined(Mode::All, &["c", "d"], &[])]);
        assert!(nested.checked().is_err());
        // As JSON: a definition has no rules, a running session's has them.
        let json = serde_json::to_string(&combined(Mode::All, &["a", "b"], &[])).unwrap();
        assert_eq!(json, r#"{"type":"combined","mode":"all","members":["a","b"]}"#);
        assert_eq!(serde_json::from_str::<Rule>(&json).unwrap(), combined(Mode::All, &["a", "b"], &[]));
    }

    #[test]
    fn a_combination_can_only_get_more_active() {
        let any = |members: &[&str]| combined(Mode::Any, members, &[]);
        let all = |members: &[&str]| combined(Mode::All, members, &[]);
        assert!(any(&["a", "b"]).tightened_by(&any(&["a", "b", "c"])).is_ok());
        assert!(any(&["a", "b"]).tightened_by(&any(&["a", "c"])).is_err());
        assert!(all(&["a", "b", "c"]).tightened_by(&all(&["a", "b"])).is_ok());
        assert!(all(&["a", "b"]).tightened_by(&all(&["a", "b", "c"])).is_err());
        assert!(all(&["a", "b"]).tightened_by(&any(&["b", "c"])).is_ok());
        assert!(all(&["a", "b"]).tightened_by(&any(&["c", "d"])).is_err());
        assert!(any(&["a", "b"]).tightened_by(&all(&["a", "b"])).is_err());
        assert!(any(&["a", "b"]).tightened_by(&weekdays()).is_err());
    }

    #[test]
    fn a_schedule_has_a_week_and_a_drive_does_not() {
        assert!(weekdays().week().is_some());
        let drive = Rule::FlashDrive { serial: "x".into(), drive: String::new() };
        assert!(drive.week().is_none());
        assert_eq!(drive.next_change(&Zone::fixed(0), SystemTime::now()), None);
    }

    fn hook(url: &str) -> Hook {
        Hook {
            url: url.into(),
            method: webhook::Method::Get,
            interval: 60,
            fallback: false,
            matcher: webhook::Matcher { on: webhook::Match::Status, status: Some(200), body: None },
        }
    }

    #[test]
    fn a_webhook_condition_is_checked_and_only_ever_the_same_while_locked() {
        // Checked as the hook is, and kept without the white space around its address.
        assert_eq!(Rule::Webhook(hook(" https://example.com/on ")).checked(), Ok(Rule::Webhook(hook("https://example.com/on"))));
        assert!(Rule::Webhook(hook("example.com")).checked().is_err());
        let json = serde_json::to_string(&Rule::Webhook(hook("https://example.com/on"))).unwrap();
        assert!(json.starts_with(r#"{"type":"webhook","url":"https://example.com/on""#), "{json}");
        // No time to say when it changes, alone or in a combination.
        let now = SystemTime::now();
        let zone = Zone::fixed(0);
        assert_eq!(Rule::Webhook(hook("https://example.com/on")).next_change(&zone, now), None);
        let with = Rule::Combined {
            mode: Mode::Any,
            members: vec!["a".into(), "b".into()],
            rules: vec![weekdays(), Rule::Webhook(hook("https://example.com/on"))],
        };
        assert_eq!(with.next_change(&zone, now), None);
        // While locked, the same hook only.
        let old = Rule::Webhook(hook("https://example.com/on"));
        assert!(old.tightened_by(&Rule::Webhook(hook(" https://example.com/on"))).is_ok());
        assert!(old
            .tightened_by(&Rule::Webhook(Hook { fallback: true, ..hook("https://example.com/on") }))
            .unwrap_err()
            .contains("webhook"));
        assert!(old.tightened_by(&weekdays()).unwrap_err().contains("webhook"));
        assert!(weekdays().tightened_by(&old).unwrap_err().contains("type"));
    }

    #[test]
    fn a_webhook_that_has_not_answered_goes_by_its_fallback() {
        // Nothing listens on port 9 of this machine: no answer comes, so the fallback holds.
        let zone = Zone::fixed(0);
        assert!(!Rule::Webhook(hook("http://127.0.0.1:9/")).active(&zone, SystemTime::now()));
        assert!(Rule::Webhook(Hook { fallback: true, ..hook("http://127.0.0.1:9/") }).active(&zone, SystemTime::now()));
    }

    #[test]
    fn a_combination_is_checked_and_needs_its_conditions_resolved() {
        let combined = |members: &[&str], rules: Vec<Rule>| Rule::Combined {
            mode: Mode::All,
            members: members.iter().map(|m| m.to_string()).collect(),
            rules,
        };
        assert!(combined(&["a"], vec![]).checked().unwrap_err().contains("at least two"));
        assert!(combined(&["a", "a"], vec![]).checked().unwrap_err().contains("only once"));
        assert!(combined(&["a", "b"], vec![weekdays()]).checked().unwrap_err().contains("one rule for each"));
        let nested = combined(&["a", "b"], vec![weekdays(), combined(&["c", "d"], vec![])]);
        assert!(nested.checked().unwrap_err().contains("another combination"));
        assert!(combined(&["a", "b"], vec![]).checked().is_ok());
        // Unresolved (no rules), it cannot be known, so it is active; resolved, all must be.
        let zone = Zone::fixed(0);
        // 1970-01-05 was a Monday: 10:00 then is in office hours, Saturday is not.
        let monday = SystemTime::UNIX_EPOCH + Duration::from_secs(4 * 86400 + 10 * 3600);
        assert!(combined(&["a", "b"], vec![]).active(&zone, monday));
        let saturday = Rule::Schedule { week: Week::parse("sat").unwrap().unwrap() };
        assert!(!combined(&["a", "b"], vec![weekdays(), saturday.clone()]).active(&zone, monday));
        assert!(Rule::Combined { mode: Mode::Any, members: vec!["a".into(), "b".into()], rules: vec![weekdays(), saturday] }
            .active(&zone, monday));
    }
}
