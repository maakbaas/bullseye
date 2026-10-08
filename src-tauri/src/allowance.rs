//! Allowances: what lets a block go quiet for a while. A block has at most one, set on the block
//! itself like its lock (`definitions::Definition::allowance`), never a list of its own. Two kinds
//! (`Terms`):
//! - a **budget**: an allowed time of use, for the block as a whole or for each item of it (each
//!   site, program, Store app or window title) on its own, that is used up while the item is in
//!   use and given back by a reset (never, every hour / day / week, or each time the block's
//!   condition becomes active);
//! - a **Pomodoro**: a fixed cycle that blocks for a while and then allows everything for a while,
//!   counted from the start of the block or from the clock (cycles from midnight, so a cycle that fits
//!   a day evenly starts at the same times every day).
//!
//! What is allowed is decided by the service (`block.rs`), never by the app: an item with budget left
//! is taken out of what the app closes and handed to it as something to watch, and the app says what
//! it sees in use (`proto::Used`), which the service counts. Use is whatever the app sees: a page
//! shown in a tab, a window or app that is open, a program that runs. Time is internet time.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::duration;
#[cfg(test)]
use crate::lists::Entries;
use crate::localtime::Zone;
use crate::state;

const DAY: u64 = 86_400;

/// The key of a budget for the block as a whole.
pub const TOTAL: &str = "";

/// Whether a budget is for the block as a whole or for each of its items.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Total,
    Each,
}

/// When what was used is given back.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reset {
    /// Not while the block runs.
    Never,
    /// At the start of every hour, day (midnight) or week (Monday), in the block's time zone.
    Hour,
    Day,
    Week,
    /// Each time the block's condition turns it on (a time block in a schedule, a drive being
    /// unplugged). Without a condition a block is on all the time, so this never resets.
    Condition,
}

impl Reset {
    /// How rare it is, for telling a stricter reset from a looser one: `None` for the one that
    /// cannot be compared with the others.
    fn rank(self) -> Option<u8> {
        match self {
            Reset::Hour => Some(1),
            Reset::Day => Some(2),
            Reset::Week => Some(3),
            Reset::Never => Some(4),
            Reset::Condition => None,
        }
    }

    /// Seconds in the period of a clock reset.
    fn length(self) -> Option<u64> {
        match self {
            Reset::Hour => Some(3600),
            Reset::Day => Some(DAY),
            Reset::Week => Some(7 * DAY),
            Reset::Never | Reset::Condition => None,
        }
    }
}

/// What a Pomodoro cycle is counted from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The block's start: it is blocked first.
    Start,
    /// Local midnight, so a cycle of an hour starts at every xx:00.
    Clock,
}

/// What an allowance is, tagged by `type`: a block's `allowance` in the file.
///
/// **A new kind of allowance** is a variant here with what it means in `Run`, fields in the block
/// editor's allowance section (`allowanceSection` in `src/js/forms.js`), and an option in the CLI's
/// `new|edit --allowance`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Terms {
    /// `allowed` (`30m`) of use, for the block or for each item of it, given back by `reset`.
    Budget { allowed: String, scope: Scope, reset: Reset },
    /// Blocked for `blocked`, then everything allowed for `allowed`, over and over.
    Pomodoro { blocked: String, allowed: String, anchor: Anchor },
}

/// A duration typed as `30m`, in seconds; never none or zero.
fn seconds(text: &str, what: &str) -> Result<u64, String> {
    duration::parse(text)?.map(|d| d.as_secs()).filter(|&s| s > 0).ok_or_else(|| format!("Enter {what}, e.g. 30m"))
}

/// A duration already checked, in seconds.
fn secs(text: &str) -> u64 {
    duration::parse(text).ok().flatten().map_or(0, |d| d.as_secs())
}

impl Terms {
    /// The terms, checked: durations that can be read and are not zero, and a cycle on the clock that
    /// fits a day evenly.
    pub fn checked(self) -> Result<Terms, String> {
        match &self {
            Terms::Budget { allowed, .. } => {
                seconds(allowed, "how long is allowed")?;
            }
            Terms::Pomodoro { blocked, allowed, anchor } => {
                let cycle = seconds(blocked, "how long to block for")?.saturating_add(seconds(allowed, "how long to allow for")?);
                if cycle > DAY {
                    return Err("A cycle, the blocked time and the allowed time together, can be at most 24 hours".into());
                }
                if *anchor == Anchor::Clock && !DAY.is_multiple_of(cycle) {
                    return Err(format!(
                        "On the clock a cycle has to fit a day evenly, such as 30 minutes or 1 hour; {} does not",
                        duration::countdown(cycle)
                    ));
                }
            }
        }
        Ok(self)
    }

    /// Whether `new` allows nothing this does not, which is all the allowance of a locked block may be
    /// changed by: a budget may get smaller or reset less often, never the other way, and its
    /// scope and a reset by condition stay; a cycle stays exactly as it is (any change moves where it is
    /// in its cycle). The error says what is wrong, for a person.
    pub fn tightened_by(&self, new: &Terms) -> Result<(), String> {
        let new = new.clone().checked()?;
        match (self, &new) {
            (
                Terms::Budget { allowed: old_allowed, scope: old_scope, reset: old_reset },
                Terms::Budget { allowed: new_allowed, scope: new_scope, reset: new_reset },
            ) => {
                if old_scope != new_scope {
                    return Err("The block is locked, so its allowance cannot change between a total and each item".into());
                }
                if secs(new_allowed) > secs(old_allowed) {
                    return Err("The block is locked, so its allowed time can get shorter but not longer".into());
                }
                let rarer = match (old_reset.rank(), new_reset.rank()) {
                    (Some(old), Some(new)) => new >= old,
                    _ => old_reset == new_reset,
                };
                if rarer {
                    Ok(())
                } else {
                    Err("The block is locked, so its allowance can be reset less often but not more often".into())
                }
            }
            (
                Terms::Pomodoro { blocked: old_blocked, allowed: old_allowed, anchor: old_anchor },
                Terms::Pomodoro { blocked: new_blocked, allowed: new_allowed, anchor: new_anchor },
            ) => {
                if secs(old_blocked) == secs(new_blocked) && secs(old_allowed) == secs(new_allowed) && old_anchor == new_anchor {
                    Ok(())
                } else {
                    Err("The block is locked, so its allowance's cycle cannot change".into())
                }
            }
            _ => Err("The block is locked, so its allowance's type cannot change".into()),
        }
    }

    /// Whether a budget is for the block or for each item; `None` for a cycle.
    pub fn scope(&self) -> Option<Scope> {
        match self {
            Terms::Budget { scope, .. } => Some(*scope),
            Terms::Pomodoro { .. } => None,
        }
    }

    /// In words, for the window and the CLI: `30 minutes in total, reset every day`,
    /// `Blocked for 50 minutes, then allowed for 10 minutes, from the start of the block`.
    pub fn describe(&self) -> String {
        match self {
            Terms::Budget { allowed, scope, reset } => {
                let scope = match scope {
                    Scope::Total => "in total",
                    Scope::Each => "for each item",
                };
                let reset = match reset {
                    Reset::Never => "never reset",
                    Reset::Hour => "reset every hour",
                    Reset::Day => "reset every day",
                    Reset::Week => "reset every week",
                    Reset::Condition => "reset each time its condition turns on",
                };
                format!("{} {scope}, {reset}", duration::countdown(secs(allowed)))
            }
            Terms::Pomodoro { blocked, allowed, anchor } => {
                let from = match anchor {
                    Anchor::Start => "from the start of the block",
                    Anchor::Clock => "on the clock, from midnight",
                };
                format!(
                    "Blocked for {}, then allowed for {}, {from}",
                    duration::countdown(secs(blocked)),
                    duration::countdown(secs(allowed))
                )
            }
        }
    }
}

/// Whether a locked block's allowance may go from `old` to `new`: taking it away is stricter, giving
/// one is not, and terms may only get stricter (`Terms::tightened_by`).
pub fn tightened(old: Option<&Terms>, new: Option<&Terms>) -> Result<(), String> {
    match (old, new) {
        (_, None) => Ok(()),
        (None, Some(_)) => Err("The block is locked, so it cannot be given an allowance".into()),
        (Some(old), Some(new)) => old.tightened_by(new),
    }
}

/// The keys of the items of a block list's lists, which is what a budget for each item counts by:
/// `site:youtube.com`, `program:steam.exe`, `app:Microsoft.Xbox_x`, `title:*Chat*`. Read from
/// `Lists::entries`, so the same entry always has the same key.
#[cfg(test)]
pub fn item_keys(entries: &Entries) -> Vec<String> {
    let each = |prefix: &str, list: &[String]| list.iter().map(|entry| format!("{prefix}:{entry}")).collect::<Vec<_>>();
    let mut keys = each("site", &entries.filters);
    keys.extend(each("program", &entries.programs));
    keys.extend(each("app", &entries.store_apps));
    keys.extend(each("title", &entries.titles));
    keys
}

/// An item's key without its kind, as a person reads it.
pub fn key_text(key: &str) -> &str {
    key.split_once(':').map_or(key, |(_, text)| text)
}

/// Seconds into the period of a clock reset, and how long it is; `None` for the other resets or if
/// the local time cannot be told.
fn into_period(reset: Reset, zone: &Zone, now: SystemTime) -> Option<(u64, u64)> {
    let length = reset.length()?;
    let local = zone.local(now)?;
    let into_day = u64::from(local.minute) * 60 + u64::from(local.second);
    let into = match reset {
        Reset::Hour => into_day % 3600,
        Reset::Day => into_day,
        _ => local.weekday as u64 * DAY + into_day,
    };
    Some((into, length))
}

/// Where a Pomodoro cycle stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    /// Whether everything is allowed now.
    pub allowed: bool,
    /// Seconds until it turns the other way.
    pub change: u64,
}

/// An allowance as a running block holds it, and as it is kept on disk (`state::Saved`): its terms
/// as they were at the start (or as the block was last given them, `Blocklist::refresh`) and what is
/// used up.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Run {
    pub terms: Terms,
    /// Milliseconds used in the current period, by key (`TOTAL`, or an item's `item_keys`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub used: BTreeMap<String, u64>,
    /// The start of the clock period `used` belongs to (seconds since the Unix epoch).
    #[serde(default)]
    pub period: u64,
    /// For a reset by condition: whether the block's condition had it on when last looked at.
    #[serde(default)]
    pub was_active: bool,
    /// When the block started, milliseconds since the Unix epoch: what a cycle from the start counts from.
    pub started: u64,
}

impl Run {
    pub fn new(terms: Terms, now: SystemTime) -> Run {
        Run { terms, used: BTreeMap::new(), period: 0, was_active: false, started: state::to_unix(now).unwrap_or(0) }
    }

    /// Gives it other terms, as its allowance was edited: what is used stays, unless it was counted
    /// another way (another scope).
    pub fn retermed(mut self, terms: Terms) -> Run {
        if self.terms.scope() != terms.scope() {
            self.used.clear();
        }
        self.terms = terms;
        self
    }

    pub fn scope(&self) -> Option<Scope> {
        self.terms.scope()
    }

    /// The allowed time of a budget, in milliseconds.
    fn allowed_ms(&self) -> u64 {
        match &self.terms {
            Terms::Budget { allowed, .. } => secs(allowed).saturating_mul(1000),
            Terms::Pomodoro { .. } => 0,
        }
    }

    /// Gives back what is used when its reset has come. `active`: whether the block's condition has
    /// it on now. To be called whenever time has passed, at least about once a second while the block runs.
    pub fn advance(&mut self, zone: &Zone, now: SystemTime, active: bool) {
        let Terms::Budget { reset, .. } = &self.terms else { return };
        let reset = *reset;
        match reset {
            Reset::Never => {}
            Reset::Condition => {
                if active && !self.was_active {
                    self.used.clear();
                }
                self.was_active = active;
            }
            clock => {
                let start = into_period(clock, zone, now)
                    .and_then(|(into, _)| now.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs().saturating_sub(into)));
                if let Some(start) = start.filter(|&start| start != self.period) {
                    self.used.clear();
                    self.period = start;
                }
            }
        }
    }

    /// Whether the item (or the block, with `TOTAL`) of a budget still has time to use.
    pub fn is_free(&self, key: &str) -> bool {
        self.used.get(key).copied().unwrap_or(0) < self.allowed_ms()
    }

    /// Counts `millis` of use against `key`, never beyond what is allowed.
    pub fn charge(&mut self, key: &str, millis: u64) {
        let cap = self.allowed_ms();
        let used = self.used.entry(key.to_string()).or_default();
        *used = used.saturating_add(millis).min(cap);
    }

    /// Where a Pomodoro cycle stands; `None` for a budget, or if the local time cannot be told (a
    /// cycle on the clock), when it is blocked.
    pub fn phase(&self, zone: &Zone, now: SystemTime) -> Option<Phase> {
        let Terms::Pomodoro { blocked, allowed, anchor } = &self.terms else { return None };
        let (blocked, cycle) = (secs(blocked), secs(blocked) + secs(allowed));
        if cycle == 0 {
            return None;
        }
        let at = match anchor {
            Anchor::Start => state::to_unix(now)?.saturating_sub(self.started) / 1000 % cycle,
            Anchor::Clock => {
                let local = zone.local(now)?;
                (u64::from(local.minute) * 60 + u64::from(local.second)) % cycle
            }
        };
        Some(if at >= blocked { Phase { allowed: true, change: cycle - at } } else { Phase { allowed: false, change: blocked - at } })
    }

    /// Whether everything is allowed now, by a Pomodoro cycle.
    pub fn everything_allowed(&self, zone: &Zone, now: SystemTime) -> bool {
        self.phase(zone, now).is_some_and(|phase| phase.allowed)
    }

    /// What is left, for the window and the CLI.
    pub fn status(&self, zone: &Zone, now: SystemTime) -> Status {
        let left = |key: &str| self.allowed_ms().saturating_sub(self.used.get(key).copied().unwrap_or(0)).div_ceil(1000);
        let words = |seconds: u64| duration::countdown(seconds);
        let phase = self.phase(zone, now);
        let total = (self.scope() == Some(Scope::Total)).then(|| left(TOTAL));
        let reset_in = match &self.terms {
            Terms::Budget { reset, .. } => into_period(*reset, zone, now).map(|(into, length)| length - into),
            Terms::Pomodoro { .. } => None,
        };
        let items = if self.scope() == Some(Scope::Each) {
            self.used
                .keys()
                .filter(|key| *key != TOTAL)
                .map(|key| ItemLeft { item: key_text(key).to_string(), left: left(key), left_words: words(left(key)) })
                .collect()
        } else {
            Vec::new()
        };
        Status {
            terms: self.terms.clone(),
            open: match &self.terms {
                Terms::Pomodoro { .. } => phase.is_some_and(|phase| phase.allowed),
                Terms::Budget { .. } => total.is_none_or(|left| left > 0),
            },
            change: phase.map(|phase| phase.change),
            change_words: phase.map(|phase| words(phase.change)),
            left: total,
            left_words: total.map(words),
            reset_in,
            reset_words: reset_in.map(words),
            items,
        }
    }
}

/// How much time an item has left, as the window and the CLI show it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct ItemLeft {
    pub item: String,
    pub left: u64,
    pub left_words: String,
}

/// What a running block's allowance is doing, in numbers and in words (`duration::countdown`).
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Status {
    pub terms: Terms,
    /// Whether something is allowed now: a cycle's allowed time, or a budget with time left (a
    /// budget for each item is open, each item has its own).
    pub open: bool,
    /// A cycle: seconds until it turns from blocked to allowed or the other way.
    pub change: Option<u64>,
    pub change_words: Option<String>,
    /// A budget for the whole block: seconds left.
    pub left: Option<u64>,
    pub left_words: Option<String>,
    /// Seconds until a reset by the clock.
    pub reset_in: Option<u64>,
    pub reset_words: Option<String>,
    /// A budget for each item: the items that have used some, and what they have left.
    pub items: Vec<ItemLeft>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(allowed: &str, scope: Scope, reset: Reset) -> Terms {
        Terms::Budget { allowed: allowed.into(), scope, reset }
    }

    fn pomodoro(blocked: &str, allowed: &str, anchor: Anchor) -> Terms {
        Terms::Pomodoro { blocked: blocked.into(), allowed: allowed.into(), anchor }
    }

    fn at(seconds: u64) -> SystemTime {
        state::from_unix(seconds * 1000)
    }

    #[test]
    fn terms_are_checked() {
        assert!(budget("30m", Scope::Total, Reset::Day).checked().is_ok());
        assert!(budget("", Scope::Total, Reset::Day).checked().is_err());
        assert!(budget("soon", Scope::Total, Reset::Day).checked().is_err());
        // 50 minutes does not fit a day evenly; from the block's start it does not need to.
        // 45 minutes does not fit a day evenly; from the block's start it does not need to.
        assert!(pomodoro("30m", "20m", Anchor::Clock).checked().is_err());
        assert!(pomodoro("30m", "15m", Anchor::Start).checked().is_ok());
        assert!(pomodoro("23h", "2h", Anchor::Start).checked().is_err());
        assert!(pomodoro("", "10m", Anchor::Start).checked().is_err());
    }

    #[test]
    fn a_locked_block_only_gets_a_stricter_allowance() {
        let old = budget("30m", Scope::Total, Reset::Day);
        assert!(old.tightened_by(&budget("20m", Scope::Total, Reset::Day)).is_ok());
        assert!(old.tightened_by(&budget("20m", Scope::Total, Reset::Week)).is_ok());
        assert!(old.tightened_by(&budget("40m", Scope::Total, Reset::Day)).is_err());
        assert!(old.tightened_by(&budget("30m", Scope::Total, Reset::Hour)).is_err());
        assert!(old.tightened_by(&budget("30m", Scope::Each, Reset::Day)).is_err());
        assert!(old.tightened_by(&pomodoro("50m", "10m", Anchor::Start)).is_err());
        let by_condition = budget("30m", Scope::Total, Reset::Condition);
        assert!(by_condition.tightened_by(&budget("30m", Scope::Total, Reset::Condition)).is_ok());
        assert!(by_condition.tightened_by(&budget("30m", Scope::Total, Reset::Never)).is_err());
        let cycle = pomodoro("50m", "10m", Anchor::Start);
        assert!(cycle.tightened_by(&pomodoro("50m", "10m", Anchor::Start)).is_ok());
        assert!(cycle.tightened_by(&pomodoro("50m", "5m", Anchor::Start)).is_err());
        assert!(cycle.tightened_by(&pomodoro("50m", "10m", Anchor::Clock)).is_err());
    }

    #[test]
    fn a_budget_is_used_up_and_given_back() {
        let zone = Zone::fixed(0);
        let mut run = Run::new(budget("1m", Scope::Total, Reset::Hour), at(0));
        run.advance(&zone, at(10), true);
        assert!(run.is_free(TOTAL));
        run.charge(TOTAL, 59_000);
        assert!(run.is_free(TOTAL));
        run.charge(TOTAL, 5_000);
        assert!(!run.is_free(TOTAL));
        assert_eq!(run.status(&zone, at(10)).left, Some(0));
        // Still the same hour: still used up; the next hour: given back.
        run.advance(&zone, at(3599), true);
        assert!(!run.is_free(TOTAL));
        run.advance(&zone, at(3600), true);
        assert!(run.is_free(TOTAL));
    }

    #[test]
    fn each_item_has_its_own_time() {
        let mut run = Run::new(budget("1m", Scope::Each, Reset::Never), at(0));
        run.charge("site:a.com", 60_000);
        assert!(!run.is_free("site:a.com") && run.is_free("site:b.com"));
        run.advance(&Zone::fixed(0), at(100_000), true);
        assert!(!run.is_free("site:a.com"));
        assert_eq!(run.status(&Zone::fixed(0), at(0)).items[0].item, "a.com");
    }

    #[test]
    fn a_budget_by_condition_is_given_back_when_the_condition_turns_on() {
        let zone = Zone::fixed(0);
        let mut run = Run::new(budget("1m", Scope::Total, Reset::Condition), at(0));
        run.advance(&zone, at(1), true);
        run.charge(TOTAL, 60_000);
        run.advance(&zone, at(2), true);
        assert!(!run.is_free(TOTAL));
        run.advance(&zone, at(3), false);
        assert!(!run.is_free(TOTAL));
        run.advance(&zone, at(4), true);
        assert!(run.is_free(TOTAL));
    }

    #[test]
    fn a_cycle_blocks_then_allows() {
        let zone = Zone::fixed(0);
        // From the start of the block, which was at 1000 seconds.
        let run = Run::new(pomodoro("50m", "10m", Anchor::Start), at(1000));
        assert_eq!(run.phase(&zone, at(1000)), Some(Phase { allowed: false, change: 3000 }));
        assert_eq!(run.phase(&zone, at(1000 + 3000)), Some(Phase { allowed: true, change: 600 }));
        assert_eq!(run.phase(&zone, at(1000 + 3600)), Some(Phase { allowed: false, change: 3000 }));
        // On the clock, an hour cycle is allowed from xx:50 to xx:00, whenever the block started.
        let clock = Run::new(pomodoro("50m", "10m", Anchor::Clock), at(1000));
        assert!(!clock.everything_allowed(&zone, at(3 * 3600 + 49 * 60)));
        assert!(clock.everything_allowed(&zone, at(3 * 3600 + 50 * 60)));
        assert!(clock.everything_allowed(&zone, at(3 * 3600 + 59 * 60 + 59)));
        assert!(!clock.everything_allowed(&zone, at(4 * 3600)));
    }

    #[test]
    fn items_have_keys_a_person_can_read() {
        let entries = Entries { filters: vec!["a.com".into()], programs: vec!["game.exe".into()], ..Entries::default() };
        let keys = item_keys(&entries);
        assert_eq!(keys, vec!["site:a.com", "program:game.exe"]);
        assert_eq!(key_text(&keys[1]), "game.exe");
    }
}
