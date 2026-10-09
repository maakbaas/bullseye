//! Allowances: what lets a session go quiet for a while. A session has at most one, set on the session
//! itself like its lock (`definitions::Definition::allowance`), never a list of its own. Four kinds
//! (`Terms`):
//! - a **budget**: an allowed time of use, for the session as a whole or for each item of it (each
//!   site, program, Store app or window title) on its own, that is used up while the item is in
//!   use and given back by a reset (never, every so long, any duration, or each time the session's
//!   condition becomes active);
//! - a **Pomodoro**: a fixed cycle that blocks for a while and then allows everything for a while,
//!   counted from the start of the session or from the clock (cycles from midnight, so a cycle that fits
//!   a day evenly starts at the same times every day);
//! - a **delayed start**: nothing is enforced until a while after the session starts, or from a
//!   date and time, and everything is from then on. The session is running, and locked, all along;
//! - **breaks**: a number of breaks of a set length, taken by hand (never by themselves) while the
//!   condition is active; during one nothing is enforced. They are given back by a reset like a
//!   budget's; ending one early still uses it up.
//!
//! What is allowed is decided by the service (`session.rs`), never by the app: an item with budget left
//! is taken out of what the app closes and handed to it as something to watch, and the app says what
//! it sees in use (`proto::Used`), which the service counts. Use is whatever the app sees: a page
//! shown in a tab, a window or app that is open, a program that runs. Time is internet time.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::datetime::DateTime;
use crate::duration;
#[cfg(test)]
use crate::lists::Entries;
use crate::localtime::Zone;
use crate::state;

const DAY: u64 = 86_400;

/// The key of a budget for the session as a whole.
pub const TOTAL: &str = "";

/// Whether a budget is for the session as a whole or for each of its items.
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
    /// Not while the session runs.
    Never,
    /// Every so long (the budget's `every`), counted from its `anchor`: the session's start, or local
    /// midnight (then it fits a day evenly, so `1d` is at midnight and `1h` on the hour).
    Every,
    /// Each time the session's condition turns it on (a time block in a schedule, a drive being
    /// unplugged). Without a condition a session is on all the time, so this never resets.
    Condition,
}

/// What a Pomodoro cycle, or a budget's reset every so long, is counted from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The session's start (a cycle is blocked first).
    #[default]
    Start,
    /// Local midnight, so a cycle or a reset of an hour comes at every xx:00.
    Clock,
}

/// Whether a delayed start counts a while from the start of the session, or is a date and time.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelayKind {
    /// `value` is a duration (`30m`), counted from the session's start.
    After,
    /// `value` is a local date and time (`2026-10-06 18:00`, `datetime.rs`), read in the session's time zone.
    From,
}

/// What an allowance is, tagged by `type`: a session's `allowance` in the file.
///
/// **A new kind of allowance** is a variant here with what it means in `Run`, fields in the session
/// editor's allowance section (`allowanceSection` in `src/js/forms.js`), and an option in the CLI's
/// `new|edit --allowance`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Terms {
    /// `allowed` (`30m`) of use, for the session or for each item of it, given back by `reset`: with
    /// `Reset::Every`, every `every` (`1d`) counted from `anchor` (empty and the default otherwise).
    Budget {
        allowed: String,
        scope: Scope,
        reset: Reset,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        every: String,
        #[serde(default)]
        anchor: Anchor,
    },
    /// Blocked for `blocked`, then everything allowed for `allowed`, over and over.
    Pomodoro { blocked: String, allowed: String, anchor: Anchor },
    /// Nothing is enforced until `value` says (`kind`); from then on everything is, for good.
    Delayed { kind: DelayKind, value: String },
    /// `count` breaks of `length` (`10m`), taken by hand, given back by `reset` as a budget is.
    Breaks {
        count: u32,
        length: String,
        reset: Reset,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        every: String,
        #[serde(default)]
        anchor: Anchor,
    },
}

/// The most breaks an allowance can have.
pub const MAX_BREAKS: u32 = 100;

/// A reset every so long, checked: its `every` read and, on the clock, fitting a day evenly; any other
/// reset has no `every` and counts from the start.
fn checked_reset(reset: Reset, every: &str, anchor: Anchor, what: &str) -> Result<(String, Anchor), String> {
    if reset != Reset::Every {
        return Ok((String::new(), Anchor::Start));
    }
    let period = seconds(every, &format!("how often {what}"))?;
    if anchor == Anchor::Clock && !DAY.is_multiple_of(period) {
        return Err(format!(
            "On the clock {what} in a way that fits a day evenly, such as every hour or every day; every {} does not",
            duration::period(period)
        ));
    }
    Ok((every.to_string(), anchor))
}

/// Whether a reset comes no more often than before: never; or every so long, longer, counted from the
/// same point (another one would bring the next reset nearer); a reset by condition only stays.
fn rarer(old: (Reset, &str, Anchor), new: (Reset, &str, Anchor)) -> bool {
    match (old.0, new.0) {
        (Reset::Condition, _) | (_, Reset::Condition) => old.0 == new.0,
        (_, Reset::Never) => true,
        (Reset::Never, Reset::Every) => false,
        (Reset::Every, Reset::Every) => old.2 == new.2 && secs(new.1) >= secs(old.1),
    }
}

/// A reset in words: `never reset`, `reset every day, on the clock, from midnight`.
fn reset_words(reset: Reset, every: &str, anchor: Anchor) -> String {
    match (reset, anchor) {
        (Reset::Never, _) => "never reset".to_string(),
        (Reset::Condition, _) => "reset each time its condition turns on".to_string(),
        (Reset::Every, Anchor::Start) => format!("reset every {}, from the start of the session", duration::period(secs(every))),
        (Reset::Every, Anchor::Clock) => format!("reset every {}, on the clock, from midnight", duration::period(secs(every))),
    }
}

/// A date and time already checked, as it is worded (`Tuesday 6 October 2026, 18:00`).
fn moment(text: &str) -> String {
    DateTime::parse(text).map_or_else(|_| text.to_string(), |at| at.to_string())
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
    /// The terms, checked: durations that can be read and are not zero, and a cycle or a reset on the
    /// clock that fits a day evenly. A budget that does not reset every so long has no `every`.
    pub fn checked(self) -> Result<Terms, String> {
        match &self {
            Terms::Delayed { kind: DelayKind::After, value } => {
                seconds(value, "how long to wait before enforcing")?;
            }
            Terms::Delayed { kind: DelayKind::From, value } => {
                DateTime::parse(value)?;
            }
            Terms::Budget { allowed, scope, reset, every, anchor } => {
                seconds(allowed, "how long is allowed")?;
                let (every, anchor) = checked_reset(*reset, every, *anchor, "the time has to be given back")?;
                return Ok(Terms::Budget { allowed: allowed.clone(), scope: *scope, reset: *reset, every, anchor });
            }
            Terms::Breaks { count, length, reset, every, anchor } => {
                if !(1..=MAX_BREAKS).contains(count) {
                    return Err(format!("Enter how many breaks, 1 to {MAX_BREAKS}"));
                }
                if seconds(length, "how long a break is")? > DAY {
                    return Err("A break can be at most 24 hours".into());
                }
                let (every, anchor) = checked_reset(*reset, every, *anchor, "the breaks have to be given back")?;
                return Ok(Terms::Breaks { count: *count, length: length.clone(), reset: *reset, every, anchor });
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

    /// Whether `new` allows nothing this does not, which is all the allowance of a locked session may be
    /// changed by: a budget may get smaller or reset less often, never the other way, and its
    /// scope and a reset by condition stay; a cycle stays exactly as it is (any change moves where it is
    /// in its cycle). The error says what is wrong, for a person.
    pub fn tightened_by(&self, new: &Terms) -> Result<(), String> {
        // The kind first: another kind is refused whatever its terms are.
        if self.kind() != new.kind() {
            return Err("This session is locked, so its allowance's type cannot change".into());
        }
        let new = new.clone().checked()?;
        match (self, &new) {
            (
                Terms::Budget { allowed: old_allowed, scope: old_scope, reset: old_reset, every: old_every, anchor: old_anchor },
                Terms::Budget { allowed: new_allowed, scope: new_scope, reset: new_reset, every: new_every, anchor: new_anchor },
            ) => {
                if old_scope != new_scope {
                    return Err("This session is locked, so its allowance cannot change between a total and each item".into());
                }
                if secs(new_allowed) > secs(old_allowed) {
                    return Err("This session is locked, so its allowed time can get shorter but not longer".into());
                }
                if rarer((*old_reset, old_every, *old_anchor), (*new_reset, new_every, *new_anchor)) {
                    Ok(())
                } else {
                    Err("This session is locked, so its allowance can be reset less often but not more often".into())
                }
            }
            (
                Terms::Breaks { count: old_count, length: old_length, reset: old_reset, every: old_every, anchor: old_anchor },
                Terms::Breaks { count: new_count, length: new_length, reset: new_reset, every: new_every, anchor: new_anchor },
            ) => {
                if new_count > old_count {
                    return Err("This session is locked, so it can have fewer breaks but not more".into());
                }
                if secs(new_length) > secs(old_length) {
                    return Err("This session is locked, so its breaks can get shorter but not longer".into());
                }
                if rarer((*old_reset, old_every, *old_anchor), (*new_reset, new_every, *new_anchor)) {
                    Ok(())
                } else {
                    Err("This session is locked, so its allowance can be reset less often but not more often".into())
                }
            }
            (
                Terms::Pomodoro { blocked: old_blocked, allowed: old_allowed, anchor: old_anchor },
                Terms::Pomodoro { blocked: new_blocked, allowed: new_allowed, anchor: new_anchor },
            ) => {
                if secs(old_blocked) == secs(new_blocked) && secs(old_allowed) == secs(new_allowed) && old_anchor == new_anchor {
                    Ok(())
                } else {
                    Err("This session is locked, so its allowance's cycle cannot change".into())
                }
            }
            (Terms::Delayed { kind: old_kind, value: old }, Terms::Delayed { kind: new_kind, value: new }) => {
                if old_kind != new_kind {
                    return Err("This session is locked, so its start cannot change between after a while and from a time".into());
                }
                // Earlier is stricter: enforcing starts sooner.
                let sooner = match new_kind {
                    DelayKind::After => secs(new) <= secs(old),
                    DelayKind::From => matches!((DateTime::parse(new), DateTime::parse(old)), (Ok(new), Ok(old)) if new <= old),
                };
                if sooner {
                    Ok(())
                } else {
                    Err("This session is locked, so its start can come sooner but not later".into())
                }
            }
            _ => Err("This session is locked, so its allowance's type cannot change".into()),
        }
    }

    /// The kind, as its `type` is written in the file: `budget`, `pomodoro`, `delayed` or `breaks`.
    pub fn kind(&self) -> &'static str {
        match self {
            Terms::Budget { .. } => "budget",
            Terms::Pomodoro { .. } => "pomodoro",
            Terms::Delayed { .. } => "delayed",
            Terms::Breaks { .. } => "breaks",
        }
    }

    /// A budget's or the breaks' reset, its `every` and what that counts from; `None` for the others.
    fn reset(&self) -> Option<(Reset, &str, Anchor)> {
        match self {
            Terms::Budget { reset, every, anchor, .. } | Terms::Breaks { reset, every, anchor, .. } => Some((*reset, every, *anchor)),
            Terms::Pomodoro { .. } | Terms::Delayed { .. } => None,
        }
    }

    /// Whether a budget is for the session or for each item; `None` for a cycle.
    pub fn scope(&self) -> Option<Scope> {
        match self {
            Terms::Budget { scope, .. } => Some(*scope),
            Terms::Pomodoro { .. } | Terms::Delayed { .. } | Terms::Breaks { .. } => None,
        }
    }

    /// In words, for the window and the CLI: `30 minutes in total, reset every day, on the clock`,
    /// `Blocked for 50 minutes, then allowed for 10 minutes, from the start of the session`.
    pub fn describe(&self) -> String {
        match self {
            Terms::Budget { allowed, scope, reset, every, anchor } => {
                let scope = match scope {
                    Scope::Total => "in total",
                    Scope::Each => "for each item",
                };
                format!("{} {scope}, {}", duration::countdown(secs(allowed)), reset_words(*reset, every, *anchor))
            }
            Terms::Breaks { count, length, reset, every, anchor } => {
                let breaks = if *count == 1 { "break" } else { "breaks" };
                format!("{count} {breaks} of {}, {}", duration::countdown(secs(length)), reset_words(*reset, every, *anchor))
            }
            Terms::Delayed { kind, value } => match kind {
                DelayKind::After => format!("Nothing is enforced for the first {}", duration::countdown(secs(value))),
                DelayKind::From => format!("Nothing is enforced until {}", moment(value)),
            },
            Terms::Pomodoro { blocked, allowed, anchor } => {
                let from = match anchor {
                    Anchor::Start => "from the start of the session",
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

/// An allowance in words, for the window (`describe_allowance`): in full, as the session editor and
/// the CLI say it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Described {
    pub words: String,
}

impl Terms {
    /// Checked, in words.
    pub fn described(self) -> Result<Described, String> {
        let terms = self.checked()?;
        Ok(Described { words: terms.describe() })
    }
}

/// Whether a locked session whose allowance is `old` may be set to the kind `to` (`none`, `budget`,
/// `pomodoro`, `delayed` or `breaks`): taking it away, or the kind it has; asked before its terms are, so a
/// change that cannot be made is not asked about (`tightened` then judges the terms).
pub fn kind_allowed(old: Option<&Terms>, to: &str) -> Result<(), String> {
    match (old, to) {
        (_, "none") => Ok(()),
        (None, _) => Err("This session is locked, so it cannot be given an allowance".into()),
        (Some(old), to) if old.kind() == to => Ok(()),
        _ => Err("This session is locked, so its allowance's type cannot change".into()),
    }
}

/// Whether a locked session's allowance may go from `old` to `new`: taking it away is stricter, giving
/// one is not, and terms may only get stricter (`Terms::tightened_by`).
pub fn tightened(old: Option<&Terms>, new: Option<&Terms>) -> Result<(), String> {
    match (old, new) {
        (_, None) => Ok(()),
        (None, Some(_)) => Err("This session is locked, so it cannot be given an allowance".into()),
        (Some(old), Some(new)) => old.tightened_by(new),
    }
}

/// The keys of the items of a target's lists, which is what a budget for each item counts by:
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

/// Seconds into the period of a budget's (or the breaks') reset every so long, and how long it is: from the session's
/// start (`started`, milliseconds since the Unix epoch) or from local midnight. `None` for the other
/// resets, or if the time cannot be told.
fn into_period(terms: &Terms, started: u64, zone: &Zone, now: SystemTime) -> Option<(u64, u64)> {
    let Some((Reset::Every, every, anchor)) = terms.reset() else { return None };
    let length = secs(every);
    if length == 0 {
        return None;
    }
    Some((offset(anchor, started, zone, now, length)?, length))
}

/// Seconds into a stretch of `length` that repeats from the session's start (`started`, milliseconds
/// since the Unix epoch) or from local midnight; `None` if the time cannot be told.
fn offset(anchor: Anchor, started: u64, zone: &Zone, now: SystemTime, length: u64) -> Option<u64> {
    match anchor {
        Anchor::Start => Some(state::to_unix(now)?.saturating_sub(started) / 1000 % length),
        Anchor::Clock => {
            let local = zone.local(now)?;
            Some((u64::from(local.minute) * 60 + u64::from(local.second)) % length)
        }
    }
}

/// Where a Pomodoro cycle stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    /// Whether everything is allowed now.
    pub allowed: bool,
    /// Seconds until it turns the other way.
    pub change: u64,
}

/// An allowance as a running session holds it, and as it is kept on disk (`state::Saved`): its terms
/// as they were at the start (or as the session was last given them, `Sessions::refresh`) and what is
/// used up.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Run {
    pub terms: Terms,
    /// Milliseconds used in the current period, by key (`TOTAL`, or an item's `item_keys`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub used: BTreeMap<String, u64>,
    /// The start of the period of a reset every so long that `used` belongs to (seconds since the
    /// Unix epoch); `None` until it is known (a new run, or new terms: `retermed`).
    #[serde(default)]
    pub period: Option<u64>,
    /// For a reset by condition: whether the session's condition had it on when last looked at.
    #[serde(default)]
    pub was_active: bool,
    /// When the session started, milliseconds since the Unix epoch: what a cycle from the start counts from.
    pub started: u64,
    /// Breaks: how many are taken in the current period, also the one going on.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub taken: u32,
    /// Breaks: when the one going on ends (milliseconds since the Unix epoch); `None` when none is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_until: Option<u64>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Run {
    pub fn new(terms: Terms, now: SystemTime) -> Run {
        Run {
            terms,
            used: BTreeMap::new(),
            period: None,
            was_active: false,
            started: state::to_unix(now).unwrap_or(0),
            taken: 0,
            break_until: None,
        }
    }

    /// Gives it other terms, as its allowance was edited: what is used stays, unless it was counted
    /// another way (another scope). The period it is in is taken as it is (`advance`), so a reset
    /// that now falls elsewhere does not give the time back early.
    pub fn retermed(mut self, terms: Terms) -> Run {
        if self.terms.scope() != terms.scope() {
            self.used.clear();
        }
        if self.terms != terms {
            self.period = None;
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
            Terms::Pomodoro { .. } | Terms::Delayed { .. } | Terms::Breaks { .. } => 0,
        }
    }

    /// Gives back what is used when its reset has come. `active`: whether the session's condition has
    /// it on now. To be called whenever time has passed, at least about once a second while the session runs.
    pub fn advance(&mut self, zone: &Zone, now: SystemTime, active: bool) {
        // A break that is over is over (it stays taken).
        if self.break_until.is_some_and(|until| state::to_unix(now).is_some_and(|now| now >= until)) {
            self.break_until = None;
        }
        let Some((reset, ..)) = self.terms.reset() else { return };
        match reset {
            Reset::Never => {}
            Reset::Condition => {
                if active && !self.was_active {
                    self.give_back();
                }
                self.was_active = active;
            }
            Reset::Every => {
                match self.period_start(zone, now) {
                    // Not known yet (a new run, or new terms): this period is taken as it is.
                    Some(start) if self.period.is_none() => self.period = Some(start),
                    Some(start) if Some(start) != self.period => {
                        self.give_back();
                        self.period = Some(start);
                    }
                    _ => {}
                }
            }
        }
    }

    /// A reset: the budget's time and the breaks given back (a break going on goes on).
    fn give_back(&mut self) {
        self.used.clear();
        self.taken = 0;
    }

    /// Breaks: how many are left to take.
    pub fn breaks_left(&self) -> Option<u32> {
        match &self.terms {
            Terms::Breaks { count, .. } => Some(count.saturating_sub(self.taken)),
            _ => None,
        }
    }

    /// Takes a break now: one of those left, for its length. Not while one is going on.
    pub fn take_break(&mut self, now: SystemTime) -> Result<(), String> {
        let Terms::Breaks { length, .. } = &self.terms else { return Err("This session has no breaks".into()) };
        let now = state::to_unix(now).ok_or("Cannot tell the time")?;
        if self.break_until.is_some_and(|until| now < until) {
            return Err("A break is going on already".into());
        }
        if self.breaks_left() == Some(0) {
            return Err("No breaks left".into());
        }
        self.taken += 1;
        self.break_until = Some(now.saturating_add(secs(length).saturating_mul(1000)));
        Ok(())
    }

    /// Ends the break going on, early: it stays used up.
    pub fn end_break(&mut self, now: SystemTime) -> Result<(), String> {
        if self.breaks_left().is_none() {
            return Err("This session has no breaks".into());
        }
        let now = state::to_unix(now).ok_or("Cannot tell the time")?;
        match self.break_until {
            Some(until) if now < until => {
                self.break_until = None;
                Ok(())
            }
            _ => Err("No break is going on".into()),
        }
    }

    /// When the period of a reset every so long that `now` is in began, in seconds since the Unix
    /// epoch: worked out from the period's number, so it is the same at every moment of it.
    fn period_start(&self, zone: &Zone, now: SystemTime) -> Option<u64> {
        let Some((Reset::Every, every, anchor)) = self.terms.reset() else { return None };
        let length = secs(every);
        if length == 0 {
            return None;
        }
        match anchor {
            Anchor::Start => {
                let since = state::to_unix(now)?.saturating_sub(self.started) / 1000;
                Some(self.started / 1000 + since / length * length)
            }
            Anchor::Clock => {
                let (into, _) = into_period(&self.terms, self.started, zone, now)?;
                Some(now.duration_since(UNIX_EPOCH).ok()?.as_secs().saturating_sub(into))
            }
        }
    }

    /// Whether the item (or the session, with `TOTAL`) of a budget still has time to use.
    pub fn is_free(&self, key: &str) -> bool {
        self.used.get(key).copied().unwrap_or(0) < self.allowed_ms()
    }

    /// Counts `millis` of use against `key`, never beyond what is allowed.
    pub fn charge(&mut self, key: &str, millis: u64) {
        let cap = self.allowed_ms();
        let used = self.used.entry(key.to_string()).or_default();
        *used = used.saturating_add(millis).min(cap);
    }

    /// Where a Pomodoro cycle stands, how long a delayed start has to go (`None` once it has begun) or
    /// what is left of a break going on (`None` without one); `None` for a budget, or if the local
    /// time cannot be told (a cycle on the clock), when it is blocked.
    pub fn phase(&self, zone: &Zone, now: SystemTime) -> Option<Phase> {
        // A break going on: allowed for what is left of it.
        if let Terms::Breaks { .. } = &self.terms {
            let left = self.break_until?.saturating_sub(state::to_unix(now)?).div_ceil(1000);
            return (left > 0).then_some(Phase { allowed: true, change: left });
        }
        if let Terms::Delayed { kind, value } = &self.terms {
            // Allowed until the start, for the time that is left; enforced after, or if the start
            // cannot be told.
            let left = self.delay_left(*kind, value, zone, now)?;
            return (left > 0).then_some(Phase { allowed: true, change: left });
        }
        let Terms::Pomodoro { blocked, allowed, anchor } = &self.terms else { return None };
        let (blocked, cycle) = (secs(blocked), secs(blocked) + secs(allowed));
        if cycle == 0 {
            return None;
        }
        let at = offset(*anchor, self.started, zone, now, cycle)?;
        Some(if at >= blocked { Phase { allowed: true, change: cycle - at } } else { Phase { allowed: false, change: blocked - at } })
    }

    /// Seconds until a delayed start begins to enforce (0 once it has); `None` if that cannot be told
    /// (the time, or a date the time zone cannot place).
    fn delay_left(&self, kind: DelayKind, value: &str, zone: &Zone, now: SystemTime) -> Option<u64> {
        let begins = match kind {
            DelayKind::After => self.started.saturating_add(secs(value).saturating_mul(1000)),
            DelayKind::From => state::to_unix(zone.instant(&DateTime::parse(value).ok()?)?)?,
        };
        Some(begins.saturating_sub(state::to_unix(now)?).div_ceil(1000))
    }

    /// Whether everything is allowed now, by a Pomodoro cycle, before a delayed start or during a break.
    pub fn everything_allowed(&self, zone: &Zone, now: SystemTime) -> bool {
        self.phase(zone, now).is_some_and(|phase| phase.allowed)
    }

    /// What is left, for the window and the CLI.
    pub fn status(&self, zone: &Zone, now: SystemTime) -> Status {
        let left = |key: &str| self.allowed_ms().saturating_sub(self.used.get(key).copied().unwrap_or(0)).div_ceil(1000);
        let words = duration::left;
        let phase = self.phase(zone, now);
        let total = (self.scope() == Some(Scope::Total)).then(|| left(TOTAL));
        let reset_in = into_period(&self.terms, self.started, zone, now).map(|(into, length)| length - into);
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
                Terms::Pomodoro { .. } | Terms::Delayed { .. } | Terms::Breaks { .. } => phase.is_some_and(|phase| phase.allowed),
                Terms::Budget { .. } => total.is_none_or(|left| left > 0),
            },
            change: phase.map(|phase| phase.change),
            change_words: phase.map(|phase| words(phase.change)),
            left: total,
            left_words: total.map(words),
            reset_in,
            reset_words: reset_in.map(words),
            items,
            breaks_left: self.breaks_left(),
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

/// What a running session's allowance is doing, in numbers and in words (`duration::left`).
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Status {
    pub terms: Terms,
    /// Whether something is allowed now: a cycle's allowed time, or a budget with time left (a
    /// budget for each item is open, each item has its own).
    pub open: bool,
    /// A cycle: seconds until it turns from blocked to allowed or the other way.
    pub change: Option<u64>,
    pub change_words: Option<String>,
    /// A budget for the whole session: seconds left.
    pub left: Option<u64>,
    pub left_words: Option<String>,
    /// Seconds until a reset by the clock.
    pub reset_in: Option<u64>,
    pub reset_words: Option<String>,
    /// A budget for each item: the items that have used some, and what they have left.
    pub items: Vec<ItemLeft>,
    /// Breaks: how many are left to take (a break going on is `open`, for `change` seconds).
    #[serde(default)]
    pub breaks_left: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(allowed: &str, scope: Scope, reset: Reset) -> Terms {
        Terms::Budget { allowed: allowed.into(), scope, reset, every: String::new(), anchor: Anchor::Start }
    }

    /// A budget given back every `period`, counted from `anchor`.
    fn every(allowed: &str, scope: Scope, period: &str, anchor: Anchor) -> Terms {
        Terms::Budget { allowed: allowed.into(), scope, reset: Reset::Every, every: period.into(), anchor }
    }

    fn pomodoro(blocked: &str, allowed: &str, anchor: Anchor) -> Terms {
        Terms::Pomodoro { blocked: blocked.into(), allowed: allowed.into(), anchor }
    }

    fn at(seconds: u64) -> SystemTime {
        state::from_unix(seconds * 1000)
    }

    #[test]
    fn terms_are_checked() {
        assert!(every("30m", Scope::Total, "1d", Anchor::Clock).checked().is_ok());
        assert!(every("", Scope::Total, "1d", Anchor::Clock).checked().is_err());
        assert!(every("soon", Scope::Total, "1d", Anchor::Clock).checked().is_err());
        // 50 minutes does not fit a day evenly; from the session's start it does not need to.
        // 45 minutes does not fit a day evenly; from the session's start it does not need to.
        assert!(pomodoro("30m", "20m", Anchor::Clock).checked().is_err());
        assert!(pomodoro("30m", "15m", Anchor::Start).checked().is_ok());
        assert!(pomodoro("23h", "2h", Anchor::Start).checked().is_err());
        assert!(pomodoro("", "10m", Anchor::Start).checked().is_err());
    }

    #[test]
    fn a_locked_session_only_gets_a_stricter_allowance() {
        let old = every("30m", Scope::Total, "1d", Anchor::Clock);
        assert!(old.tightened_by(&every("20m", Scope::Total, "1d", Anchor::Clock)).is_ok());
        assert!(old.tightened_by(&budget("30m", Scope::Total, Reset::Never)).is_ok(), "never is rarest");
        assert!(old.tightened_by(&every("20m", Scope::Total, "7d", Anchor::Start)).is_err(), "counted from elsewhere");
        assert!(old.tightened_by(&every("40m", Scope::Total, "1d", Anchor::Clock)).is_err());
        assert!(old.tightened_by(&every("30m", Scope::Total, "1h", Anchor::Clock)).is_err());
        let from_start = every("30m", Scope::Total, "1d", Anchor::Start);
        assert!(from_start.tightened_by(&every("30m", Scope::Total, "2d", Anchor::Start)).is_ok(), "less often");
        assert!(from_start.tightened_by(&every("30m", Scope::Total, "12h", Anchor::Start)).is_err());
        assert!(budget("30m", Scope::Total, Reset::Never).tightened_by(&from_start).is_err());
        assert!(old.tightened_by(&every("30m", Scope::Each, "1d", Anchor::Clock)).is_err());
        assert!(old.tightened_by(&pomodoro("50m", "10m", Anchor::Start)).is_err());
        let by_condition = budget("30m", Scope::Total, Reset::Condition);
        assert!(by_condition.tightened_by(&budget("30m", Scope::Total, Reset::Condition)).is_ok());
        assert!(by_condition.tightened_by(&budget("30m", Scope::Total, Reset::Never)).is_err());
        let cycle = pomodoro("50m", "10m", Anchor::Start);
        assert!(cycle.tightened_by(&pomodoro("50m", "10m", Anchor::Start)).is_ok());
        assert!(cycle.tightened_by(&pomodoro("50m", "5m", Anchor::Start)).is_err());
        assert!(cycle.tightened_by(&pomodoro("50m", "10m", Anchor::Clock)).is_err());
    }

    fn breaks(count: u32, length: &str, reset: Reset, every: &str, anchor: Anchor) -> Terms {
        Terms::Breaks { count, length: length.into(), reset, every: every.into(), anchor }
    }

    #[test]
    fn breaks_are_checked_described_and_only_get_stricter_while_locked() {
        let daily = breaks(3, "10m", Reset::Every, "1d", Anchor::Clock);
        assert_eq!(daily.clone().checked(), Ok(daily.clone()));
        assert_eq!(daily.describe(), "3 breaks of 10 minutes, reset every day, on the clock, from midnight");
        assert_eq!(breaks(1, "1h", Reset::Never, "", Anchor::Start).describe(), "1 break of 1 hour, never reset");
        assert_eq!(daily.kind(), "breaks");
        assert_eq!(daily.scope(), None);
        // Another reset than every so long keeps no `every`.
        assert_eq!(
            breaks(3, "10m", Reset::Condition, "1d", Anchor::Clock).checked(),
            Ok(breaks(3, "10m", Reset::Condition, "", Anchor::Start))
        );
        for (bad, why) in [
            (breaks(0, "10m", Reset::Never, "", Anchor::Start), "how many breaks"),
            (breaks(MAX_BREAKS + 1, "10m", Reset::Never, "", Anchor::Start), "how many breaks"),
            (breaks(3, "", Reset::Never, "", Anchor::Start), "how long a break is"),
            (breaks(3, "25h", Reset::Never, "", Anchor::Start), "at most 24 hours"),
            (breaks(3, "10m", Reset::Every, "", Anchor::Start), "how often"),
            (breaks(3, "10m", Reset::Every, "7h", Anchor::Clock), "fits a day evenly"),
        ] {
            let error = bad.clone().checked().unwrap_err();
            assert!(error.contains(why), "{bad:?}: {error}");
        }
        // Locked: fewer, shorter, rarer; never more, longer or more often.
        assert!(daily.tightened_by(&breaks(2, "5m", Reset::Every, "1d", Anchor::Clock)).is_ok());
        let from_start = breaks(3, "10m", Reset::Every, "1d", Anchor::Start);
        assert!(from_start.tightened_by(&breaks(3, "10m", Reset::Every, "2d", Anchor::Start)).is_ok());
        assert!(daily.tightened_by(&breaks(3, "10m", Reset::Never, "", Anchor::Start)).is_ok());
        assert!(daily.tightened_by(&breaks(4, "10m", Reset::Every, "1d", Anchor::Clock)).unwrap_err().contains("fewer breaks"));
        assert!(daily.tightened_by(&breaks(3, "11m", Reset::Every, "1d", Anchor::Clock)).unwrap_err().contains("shorter"));
        assert!(daily.tightened_by(&breaks(3, "10m", Reset::Every, "12h", Anchor::Clock)).unwrap_err().contains("less often"));
        assert!(daily.tightened_by(&budget("10m", Scope::Total, Reset::Never)).unwrap_err().contains("type"));
        assert!(kind_allowed(Some(&daily), "breaks").is_ok() && kind_allowed(Some(&daily), "budget").is_err());
    }

    #[test]
    fn a_break_is_taken_ended_early_and_given_back() {
        let zone = Zone::fixed(0);
        let mut run = Run::new(breaks(2, "10m", Reset::Every, "1h", Anchor::Clock), at(0));
        run.advance(&zone, at(1), true);
        assert_eq!((run.breaks_left(), run.everything_allowed(&zone, at(1))), (Some(2), false));
        assert_eq!(run.status(&zone, at(1)).breaks_left, Some(2));
        // Taken: everything allowed for its length, one fewer left; not twice at once.
        run.take_break(at(60)).unwrap();
        assert_eq!(run.phase(&zone, at(120)), Some(Phase { allowed: true, change: 540 }));
        let status = run.status(&zone, at(120));
        assert!(status.open && status.change == Some(540) && status.breaks_left == Some(1));
        assert_eq!(run.take_break(at(120)), Err("A break is going on already".into()));
        // Ended early: over, and still used.
        run.end_break(at(180)).unwrap();
        assert!(!run.everything_allowed(&zone, at(181)) && run.breaks_left() == Some(1));
        assert_eq!(run.end_break(at(181)), Err("No break is going on".into()));
        // The last one runs out by itself.
        run.take_break(at(200)).unwrap();
        run.advance(&zone, at(200 + 600), true);
        assert_eq!((run.break_until, run.breaks_left()), (None, Some(0)));
        assert_eq!(run.take_break(at(900)), Err("No breaks left".into()));
        // Kept as it is on disk: what is taken, and a break going on.
        let kept: Run = serde_json::from_value(serde_json::to_value(&run).unwrap()).unwrap();
        assert_eq!(kept, run);
        // Given back on the hour.
        run.advance(&zone, at(3600), true);
        assert_eq!(run.breaks_left(), Some(2));
        // By condition: each time it turns on.
        let mut by_condition = Run::new(breaks(1, "5m", Reset::Condition, "", Anchor::Start), at(0));
        by_condition.take_break(at(0)).unwrap();
        by_condition.advance(&zone, at(400), false);
        assert_eq!(by_condition.breaks_left(), Some(0));
        by_condition.advance(&zone, at(500), true);
        assert_eq!(by_condition.breaks_left(), Some(1));
        // Other allowances have no breaks.
        let mut budget_run = Run::new(budget("1m", Scope::Total, Reset::Never), at(0));
        assert!(budget_run.take_break(at(0)).is_err() && budget_run.breaks_left().is_none());
    }

    #[test]
    fn a_budget_is_used_up_and_given_back() {
        let zone = Zone::fixed(0);
        let mut run = Run::new(every("1m", Scope::Total, "1h", Anchor::Clock), at(0));
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
        // From the start of the session, which was at 1000 seconds.
        let run = Run::new(pomodoro("50m", "10m", Anchor::Start), at(1000));
        assert_eq!(run.phase(&zone, at(1000)), Some(Phase { allowed: false, change: 3000 }));
        assert_eq!(run.phase(&zone, at(1000 + 3000)), Some(Phase { allowed: true, change: 600 }));
        assert_eq!(run.phase(&zone, at(1000 + 3600)), Some(Phase { allowed: false, change: 3000 }));
        // On the clock, an hour cycle is allowed from xx:50 to xx:00, whenever the session started.
        let clock = Run::new(pomodoro("50m", "10m", Anchor::Clock), at(1000));
        assert!(!clock.everything_allowed(&zone, at(3 * 3600 + 49 * 60)));
        assert!(clock.everything_allowed(&zone, at(3 * 3600 + 50 * 60)));
        assert!(clock.everything_allowed(&zone, at(3 * 3600 + 59 * 60 + 59)));
        assert!(!clock.everything_allowed(&zone, at(4 * 3600)));
    }

    #[test]
    fn terms_are_written_out() {
        let cases = [
            (budget("30m", Scope::Total, Reset::Never), "30 minutes in total, never reset"),
            (every("1h", Scope::Total, "1h", Anchor::Clock), "1 hour in total, reset every hour, on the clock, from midnight"),
            (every("30m", Scope::Total, "1d", Anchor::Clock), "30 minutes in total, reset every day, on the clock, from midnight"),
            (
                every("2h", Scope::Each, "36h", Anchor::Start),
                "2 hours for each item, reset every 1 day and 12 hours, from the start of the session",
            ),
            (budget("10m", Scope::Each, Reset::Condition), "10 minutes for each item, reset each time its condition turns on"),
            (pomodoro("50m", "10m", Anchor::Start), "Blocked for 50 minutes, then allowed for 10 minutes, from the start of the session"),
            (pomodoro("1h", "30m", Anchor::Clock), "Blocked for 1 hour, then allowed for 30 minutes, on the clock, from midnight"),
        ];
        for (terms, words) in cases {
            assert_eq!(terms.describe(), words);
            assert_eq!(terms.described(), Ok(Described { words: words.into() }));
        }
        // What cannot be saved is not written out either.
        assert!(every("soon", Scope::Total, "1d", Anchor::Clock).described().is_err());
    }

    #[test]
    fn a_locked_session_can_lose_its_allowance_but_not_be_given_one() {
        let old = every("30m", Scope::Total, "1d", Anchor::Clock);
        assert_eq!(tightened(None, None), Ok(()));
        assert_eq!(tightened(Some(&old), None), Ok(()));
        assert!(tightened(None, Some(&old)).is_err());
        assert_eq!(tightened(Some(&old), Some(&every("10m", Scope::Total, "1d", Anchor::Clock))), Ok(()));
        assert!(tightened(Some(&old), Some(&every("1h", Scope::Total, "1d", Anchor::Clock))).is_err());
        // New terms that cannot be read are no stricter.
        assert!(tightened(Some(&old), Some(&every("", Scope::Total, "1d", Anchor::Clock))).is_err());
    }

    #[test]
    fn a_budget_from_the_start_is_given_back_every_so_long_after_it() {
        let zone = Zone::fixed(0);
        // Started at 10:00:00.5, given back every 7 days from then.
        let start = state::from_unix(10 * 3600 * 1000 + 500);
        let later = |seconds: u64| start + std::time::Duration::from_secs(seconds);
        let mut run = Run::new(every("1m", Scope::Total, "7d", Anchor::Start), start);
        run.advance(&zone, later(DAY), true);
        run.charge(TOTAL, 60_000);
        // Every moment of the period is the same period, whatever its milliseconds.
        for moment in [later(DAY) + std::time::Duration::from_millis(600), later(3 * DAY), later(7 * DAY - 1)] {
            run.advance(&zone, moment, true);
            assert!(!run.is_free(TOTAL), "still the first week at {moment:?}");
        }
        assert_eq!(run.status(&zone, later(7 * DAY - 1)).reset_in, Some(1));
        run.advance(&zone, later(7 * DAY), true);
        assert!(run.is_free(TOTAL));
        // Without a reset, what is used stays.
        let mut never = Run::new(budget("1m", Scope::Total, Reset::Never), at(0));
        never.charge(TOTAL, 60_000);
        never.advance(&zone, at(30 * DAY), true);
        assert!(!never.is_free(TOTAL) && never.status(&zone, at(30 * DAY)).reset_in.is_none());
    }

    #[test]
    fn a_budget_on_the_clock_is_given_back_at_the_times_that_fit_the_day() {
        let zone = Zone::fixed(0);
        // Every 12 hours on the clock: at 00:00 and 12:00, whenever the session started (09:00).
        let mut run = Run::new(every("1m", Scope::Total, "12h", Anchor::Clock), at(9 * 3600));
        run.advance(&zone, at(9 * 3600), true);
        run.charge(TOTAL, 60_000);
        run.advance(&zone, at(12 * 3600 - 1), true);
        assert!(!run.is_free(TOTAL));
        assert_eq!(run.status(&zone, at(12 * 3600 - 1)).reset_in, Some(1));
        run.advance(&zone, at(12 * 3600), true);
        assert!(run.is_free(TOTAL));
        // On the clock it has to fit a day; from the start any duration will do.
        assert!(every("1m", Scope::Total, "7h", Anchor::Clock).checked().unwrap_err().contains("fits a day evenly"));
        assert!(every("1m", Scope::Total, "7d", Anchor::Clock).checked().is_err());
        assert!(every("1m", Scope::Total, "7h", Anchor::Start).checked().is_ok());
        assert!(every("1m", Scope::Total, "", Anchor::Start).checked().is_err());
        assert!(every("1m", Scope::Total, "0m", Anchor::Start).checked().is_err());
        // Another reset keeps no period of its own: checked, it is stored without one.
        let mut stray = budget("1m", Scope::Total, Reset::Never);
        if let Terms::Budget { every, anchor, .. } = &mut stray {
            *every = "1d".into();
            *anchor = Anchor::Clock;
        }
        assert_eq!(stray.checked(), Ok(budget("1m", Scope::Total, Reset::Never)));
    }

    #[test]
    fn new_terms_do_not_give_the_time_back_early() {
        let zone = Zone::fixed(0);
        let mut run = Run::new(every("10m", Scope::Total, "1h", Anchor::Clock), at(0));
        run.advance(&zone, at(1800), true);
        run.charge(TOTAL, 600_000);
        // Edited to every 2 hours from the start, at 00:30: its periods now fall elsewhere, but what
        // is used is kept until the next reset.
        let mut run = run.retermed(every("10m", Scope::Total, "2h", Anchor::Start));
        run.advance(&zone, at(1801), true);
        assert!(!run.is_free(TOTAL), "kept");
        run.advance(&zone, at(2 * 3600 - 1), true);
        assert!(!run.is_free(TOTAL));
        run.advance(&zone, at(2 * 3600), true);
        assert!(run.is_free(TOTAL), "given back at the next reset of the new terms");
    }

    #[test]
    fn new_terms_keep_what_is_used_unless_it_is_counted_another_way() {
        let mut run = Run::new(budget("10m", Scope::Total, Reset::Never), at(0));
        run.charge(TOTAL, 60_000);
        let shorter = run.clone().retermed(every("5m", Scope::Total, "1d", Anchor::Clock));
        assert_eq!(shorter.used.get(TOTAL), Some(&60_000));
        assert_eq!(shorter.scope(), Some(Scope::Total));
        let each = run.retermed(budget("5m", Scope::Each, Reset::Never));
        assert!(each.used.is_empty());
        // A cycle has no scope and no time to use up.
        let cycle = Run::new(pomodoro("50m", "10m", Anchor::Start), at(0));
        assert_eq!(cycle.scope(), None);
        assert!(!cycle.is_free(TOTAL));
    }

    #[test]
    fn items_have_keys_a_person_can_read() {
        let entries = Entries { filters: vec!["a.com".into()], programs: vec!["game.exe".into()], ..Entries::default() };
        let keys = item_keys(&entries);
        assert_eq!(keys, vec!["site:a.com", "program:game.exe"]);
        assert_eq!(key_text(&keys[1]), "game.exe");
    }

    fn after(value: &str) -> Terms {
        Terms::Delayed { kind: DelayKind::After, value: value.into() }
    }

    fn from(value: &str) -> Terms {
        Terms::Delayed { kind: DelayKind::From, value: value.into() }
    }

    #[test]
    fn a_delayed_start_is_checked() {
        assert_eq!(after("30m").checked(), Ok(after("30m")));
        assert_eq!(from("2026-10-06 18:00").checked(), Ok(from("2026-10-06 18:00")));
        assert!(after("").checked().unwrap_err().contains("how long to wait before enforcing"));
        assert!(after("0m").checked().is_err());
        assert!(after("soon").checked().is_err());
        assert!(from("").checked().is_err());
        assert!(from("tomorrow-ish").checked().is_err());
    }

    #[test]
    fn a_delayed_start_is_written_out() {
        let cases = [
            (after("30m"), "Nothing is enforced for the first 30 minutes"),
            (after("1h"), "Nothing is enforced for the first 1 hour"),
            (from("1970-01-02 00:00"), "Nothing is enforced until Friday 2 January 1970, 00:00"),
        ];
        for (terms, words) in cases {
            assert_eq!(terms.describe(), words);
            assert_eq!(terms.clone().described(), Ok(Described { words: words.into() }));
            assert_eq!(terms.scope(), None);
        }
        // A date that cannot be read is worded as typed.
        assert_eq!(moment("later"), "later");
        assert!(after("0m").described().is_err());
    }

    #[test]
    fn a_delayed_start_is_written_as_json_tagged_by_type_and_kind() {
        let text = serde_json::to_string(&after("30m")).unwrap();
        assert_eq!(text, r#"{"type":"delayed","kind":"after","value":"30m"}"#);
        assert_eq!(serde_json::from_str::<Terms>(&text).unwrap(), after("30m"));
        let text = serde_json::to_string(&from("2026-10-06 18:00")).unwrap();
        assert_eq!(text, r#"{"type":"delayed","kind":"from","value":"2026-10-06 18:00"}"#);
        assert_eq!(serde_json::from_str::<Terms>(&text).unwrap(), from("2026-10-06 18:00"));
    }

    #[test]
    fn a_delayed_start_allows_everything_until_it_begins() {
        let zone = Zone::fixed(0);
        // After: counted from the start of the session, at 1000 seconds.
        let run = Run::new(after("30m"), at(1000));
        assert_eq!(run.phase(&zone, at(1000)), Some(Phase { allowed: true, change: 1800 }));
        assert_eq!(run.phase(&zone, at(1000 + 1799)), Some(Phase { allowed: true, change: 1 }));
        assert!(run.everything_allowed(&zone, at(1000 + 1799)));
        // Once it has begun it enforces for good.
        assert_eq!(run.phase(&zone, at(1000 + 1800)), None);
        assert!(!run.everything_allowed(&zone, at(1000 + 1800)));
        assert!(!run.everything_allowed(&zone, at(1000 + 99_999)));
        let before = run.status(&zone, at(1000 + 600));
        assert!(before.open);
        assert_eq!((before.change, before.change_words.as_deref()), (Some(1200), Some("20 minutes")));
        assert_eq!((before.left, before.reset_in), (None, None));
        assert!(before.items.is_empty());
        let begun = run.status(&zone, at(1000 + 1800));
        assert!(!begun.open);
        assert_eq!((begun.change, begun.change_words), (None, None));
        // From: a local date and time, read in the session's zone.
        let run = Run::new(from("1970-01-02 00:00"), at(1000));
        assert_eq!(run.phase(&zone, at(1000)), Some(Phase { allowed: true, change: 86_400 - 1000 }));
        assert!(!run.everything_allowed(&zone, at(86_400)));
        // Two hours east of UTC, local midnight came two hours earlier.
        let east = Zone::fixed(120);
        assert_eq!(run.phase(&east, at(1000)), Some(Phase { allowed: true, change: 86_400 - 7200 - 1000 }));
        assert_eq!(run.phase(&east, at(86_400 - 7200)), None);
        // A start that cannot be told (a date nobody can read) is enforced.
        let broken = Run::new(from("never"), at(0));
        assert_eq!(broken.phase(&zone, at(0)), None);
        assert!(!broken.everything_allowed(&zone, at(0)));
        // Nothing to use up, and not given back.
        assert!(!run.is_free(TOTAL));
        let mut run = run;
        run.advance(&zone, at(10), true);
        run.charge(TOTAL, 1000);
        assert!(!run.is_free(TOTAL));
        assert_eq!(run.status(&zone, at(10)).reset_in, None);
    }

    #[test]
    fn a_locked_session_can_only_start_enforcing_sooner() {
        let old = after("1h");
        assert_eq!(old.tightened_by(&after("1h")), Ok(()));
        assert_eq!(old.tightened_by(&after("30m")), Ok(()));
        assert_eq!(old.tightened_by(&after("2h")), Err("This session is locked, so its start can come sooner but not later".into()));
        assert!(old.tightened_by(&after("")).is_err(), "unreadable terms are no stricter");
        let kinds = old.tightened_by(&from("2026-10-06 18:00")).unwrap_err();
        assert!(kinds.contains("between after a while and from a time"), "{kinds}");
        let old = from("2026-10-06 18:00");
        assert_eq!(old.tightened_by(&from("2026-10-06 18:00")), Ok(()));
        assert_eq!(old.tightened_by(&from("2026-10-06 17:59")), Ok(()));
        assert!(old.tightened_by(&from("2026-10-06 18:01")).unwrap_err().contains("sooner but not later"));
        assert!(old.tightened_by(&after("1h")).is_err());
        // An old date nobody can read is never beaten.
        assert!(from("never").tightened_by(&from("2026-10-06 18:00")).unwrap_err().contains("sooner but not later"));
        // Another type altogether.
        assert!(old.tightened_by(&pomodoro("50m", "10m", Anchor::Start)).unwrap_err().contains("type cannot change"));
        assert!(pomodoro("50m", "10m", Anchor::Start).tightened_by(&old).unwrap_err().contains("type cannot change"));
        assert_eq!(tightened(Some(&after("1h")), Some(&after("10m"))), Ok(()));
        assert!(tightened(Some(&after("1h")), Some(&after("2h"))).is_err());
        assert!(tightened(None, Some(&after("1h"))).is_err());
    }

    #[test]
    fn new_delayed_terms_keep_the_run() {
        let run = Run::new(after("1h"), at(5));
        let run = run.retermed(from("2026-10-06 18:00"));
        assert_eq!(run.scope(), None);
        assert_eq!(run.started, 5000);
    }
}
