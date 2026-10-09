//! What a session's Condition, Allowance and Lock rows say, in a few words, and the state its tag
//! shows: while it runs what is happening (`Active for 4 hours`, `12 minutes left`), while it is
//! stopped what is set (`By schedule`, `Allowed for 30 minutes`). Worked out once here, in the
//! window's words (`strings.rs`, `sessions.now`), for the tile and the CLI. Times are one unit,
//! rounded (`duration::left`); a date is the time today and the day otherwise (`DateTime::short`).

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::allowance::{DelayKind, Scope, Terms};
use crate::condition::Rule;
use crate::datetime::DateTime;
use crate::definitions::{Condition, Definition};
use crate::duration;
use crate::localtime::Zone;
use crate::lock::{Lock, TimedKind};
use crate::session::Running;
use crate::strings::t;

/// One session (`Definition::id`): its state and its three rows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Status {
    pub id: String,
    pub state: State,
    pub condition: Line,
    pub allowance: Line,
    pub lock: Line,
}

/// What a session's tag says (`sessions.state`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    NotRunning,
    Running,
    /// Running and locked.
    Locked,
    /// Running, its condition off: nothing is blocked.
    Inactive,
}

impl State {
    /// The tag's words, as on the tile.
    pub fn words(self) -> String {
        let key = match self {
            State::NotRunning => "notRunning",
            State::Running => "running",
            State::Locked => "runningLocked",
            State::Inactive => "inactive",
        };
        t(&format!("sessions.state.{key}"), &[])
    }
}

/// One row's words. A condition named in them stands as `{condition}` in `text`, with its id and
/// name, so the window can make it a link (`plain` is the text with the name in it).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Named>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Named {
    pub id: String,
    pub name: String,
}

impl Line {
    fn new(text: String) -> Line {
        Line { text, condition: None }
    }

    /// The text with the condition's name in it, quoted, as the CLI shows it.
    pub fn plain(&self) -> String {
        match &self.condition {
            Some(named) => self.text.replace("{condition}", &crate::strings::quoted(&named.name)),
            None => self.text.clone(),
        }
    }
}

/// A text of `sessions.now`.
fn now(key: &str, values: &[(&str, &str)]) -> String {
    t(&format!("sessions.now.{key}"), values)
}

/// The rows of `session`, running as `running` or stopped, with `conditions` to name its condition and
/// its lock's by. `zone` and `at` tell which day it is, for a date.
pub fn of(session: &Definition, running: Option<&Running>, conditions: &[Condition], zone: &Zone, at: SystemTime) -> Status {
    let (state, (condition, allowance, lock)) = match running {
        Some(running) => {
            let state = if !running.enforcing {
                State::Inactive
            } else if running.locked {
                State::Locked
            } else {
                State::Running
            };
            (state, running_lines(running))
        }
        None => (State::NotRunning, stopped_lines(session, conditions, zone, at)),
    };
    Status { id: session.id.clone(), state, condition, allowance, lock }
}

/// A time set as typed (`90m`) in one unit (`2 hours`); as typed if it cannot be read.
fn short(text: &str) -> String {
    match duration::parse(text) {
        Ok(Some(length)) => duration::left(length.as_secs()),
        _ => text.to_string(),
    }
}

/// A date and time as typed in a few words: the time if it is today, the day otherwise.
fn short_date(text: &str, zone: &Zone, at: SystemTime) -> String {
    let Ok(date) = DateTime::parse(text) else { return text.to_string() };
    // Today when the day's midnight has passed and the next has not.
    let midnight = zone.instant(&DateTime { hour: 0, minute: 0, ..date });
    let today = midnight.is_some_and(|midnight| at.duration_since(midnight).is_ok_and(|since| since.as_secs() < 86400));
    date.short(today)
}

/// What has a rule on or off: a schedule, a device, a webhook, a combination.
fn by(rule: &Rule) -> &'static str {
    match rule {
        Rule::Schedule { .. } => "Schedule",
        Rule::FlashDrive { .. } => "Device",
        Rule::Webhook(_) => "Webhook",
        Rule::Combined { .. } => "Combination",
    }
}

/// A rule's state now (`state`: active / inactive, locked / unlocked): for how long when that is
/// known, otherwise what has it so; a schedule always knows.
fn rule_now(rule: &Rule, state: &str, change: Option<&String>) -> String {
    match (change, rule) {
        (Some(change), _) => now(&format!("{state}For"), &[("change", change)]),
        (None, Rule::Schedule { .. }) => now(state, &[]),
        (None, rule) => now(&format!("{state}By{}", by(rule)), &[]),
    }
}

fn running_lines(running: &Running) -> (Line, Line, Line) {
    let condition = match &running.condition {
        None => now("active", &[]),
        Some(rule) => rule_now(rule, if running.enforcing { "active" } else { "inactive" }, running.condition_change_words.as_ref()),
    };
    // The same while the condition is off: a cycle and a delayed start go by the clock, a budget keeps
    // what is left (it is only used while the condition is on) and its reset.
    let allowance = match &running.allowance {
        None => now("neverPaused", &[]),
        Some(state) => {
            let change = state.change_words.clone().unwrap_or_default();
            match &state.terms {
                // A cycle or a start whose time cannot be told is enforced: blocked, begun.
                Terms::Pomodoro { .. } if state.open => now("allowedFor", &[("change", &change)]),
                Terms::Pomodoro { .. } if state.change_words.is_some() => now("allowedIn", &[("change", &change)]),
                Terms::Pomodoro { .. } => now("blocked", &[]),
                Terms::Delayed { .. } if state.open => now("startsIn", &[("change", &change)]),
                Terms::Delayed { .. } => now("started", &[]),
                // A break going on: what is left of it; otherwise how many are left.
                Terms::Breaks { .. } if state.open => now("left", &[("left", &change)]),
                // None left: back when the reset gives them back, as a budget's time.
                Terms::Breaks { .. } => match (state.breaks_left.unwrap_or(0), &state.reset_words) {
                    (0, Some(reset)) => now("back", &[("reset", reset)]),
                    (0, None) => now("noneLeft", &[]),
                    (n, _) => now("breaksLeft", &[("n", &n.to_string())]),
                },
                Terms::Budget { .. } => match (state.left, &state.left_words, &state.reset_words) {
                    (Some(left), Some(words), _) if left > 0 => now("left", &[("left", words)]),
                    (Some(_), _, Some(reset)) => now("back", &[("reset", reset)]),
                    (Some(_), _, None) => now("usedUp", &[]),
                    // A budget for each item: how many have used theirs up, not each by name.
                    (None, _, _) => match state.items.iter().filter(|item| item.left == 0).count() {
                        0 => now("noneUsedUp", &[]),
                        n => now("itemsUsedUp", &[("n", &n.to_string())]),
                    },
                },
            }
        }
    };
    let lock = if let Some(delay) = running.stop_delay {
        match (running.stop_remaining, &running.stop_remaining_words) {
            (None, _) => now("delay", &[("delay", &duration::left(delay))]),
            (Some(0), _) => now("unlocked", &[]),
            (Some(_), words) => now("in", &[("left", words.as_deref().unwrap_or_default())]),
        }
    } else if let Some(left) = &running.remaining_words {
        now("timeLeft", &[("left", left)])
    } else if let Some(rule) = &running.lock_condition {
        rule_now(rule, if running.locked { "locked" } else { "unlocked" }, running.lock_change_words.as_ref())
    } else if running.until_restart {
        now("restart", &[])
    } else if running.lock_text.is_some() {
        now("text", &[])
    } else if running.password {
        now("password", &[])
    } else {
        now("notLocked", &[])
    };
    (Line::new(condition), Line::new(allowance), Line::new(lock))
}

fn stopped_lines(session: &Definition, conditions: &[Condition], zone: &Zone, at: SystemTime) -> (Line, Line, Line) {
    let find = |id: &str| conditions.iter().find(|c| c.id == id);
    // What switches it on and off, not whether it is on: it does not run.
    let condition = match session.condition.as_deref().and_then(find) {
        None => now("always", &[]),
        Some(condition) => now(&format!("by{}", by(&condition.rule)), &[]),
    };
    let allowance = match &session.allowance {
        None => now("neverPaused", &[]),
        Some(Terms::Pomodoro { blocked, allowed, .. }) => now("cycle", &[("blocked", &short(blocked)), ("allowed", &short(allowed))]),
        Some(Terms::Delayed { kind: DelayKind::After, value }) => now("after", &[("time", &short(value))]),
        Some(Terms::Breaks { count, length, .. }) => now("breaks", &[("n", &count.to_string()), ("time", &short(length))]),
        Some(Terms::Delayed { kind: DelayKind::From, value }) => now("from", &[("date", &short_date(value, zone, at))]),
        Some(Terms::Budget { allowed, scope: Scope::Each, .. }) => now("perItem", &[("time", &short(allowed))]),
        Some(Terms::Budget { allowed, .. }) => now("budget", &[("time", &short(allowed))]),
    };
    let lock = match &session.lock {
        Lock::None => Line::new(now("notLocked", &[])),
        Lock::Timed { kind: TimedKind::For, value } => Line::new(now("for", &[("time", &short(value))])),
        Lock::Timed { kind: TimedKind::Until, value } => Line::new(now("until", &[("date", &short_date(value, zone, at))])),
        Lock::UntilRestart => Line::new(now("restart", &[])),
        Lock::Delay { delay } => Line::new(now("delay", &[("delay", &short(delay))])),
        Lock::Text { .. } | Lock::RandomText { .. } => Line::new(now("text", &[])),
        Lock::Password { .. } => Line::new(now("password", &[])),
        // "During" a schedule's times; "by" what has the others on.
        Lock::Condition { condition } => match find(condition) {
            None => Line::new(t("locks.condition.label", &[])),
            Some(c) => Line {
                text: now(if matches!(c.rule, Rule::Schedule { .. }) { "during" } else { "by" }, &[]),
                condition: Some(Named { id: c.id.clone(), name: c.name.clone() }),
            },
        },
    };
    (Line::new(condition), Line::new(allowance), lock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowance::{Anchor, ItemLeft, Reset, Status as AllowanceStatus};
    use crate::condition::Mode;
    use crate::schedule::Week;
    use crate::webhook::{Hook, Match, Matcher, Method};
    use std::time::{Duration, UNIX_EPOCH};

    fn hook() -> Hook {
        Hook {
            url: "https://example.com".into(),
            method: Method::Get,
            interval: 60,
            fallback: false,
            matcher: Matcher { on: Match::Status, status: Some(200), body: None },
        }
    }

    fn week(text: &str) -> Week {
        Week::parse(text).unwrap().unwrap()
    }

    fn session(lock: Lock, condition: Option<&str>, allowance: Option<Terms>) -> Definition {
        Definition {
            id: "s".into(),
            name: "S".into(),
            targets: vec![],
            condition: condition.map(Into::into),
            allowance,
            lock,
            kept_locks: vec![],
        }
    }

    fn conditions() -> Vec<Condition> {
        let week = week("mon-fri 09:00-17:00");
        vec![
            Condition { id: "hours".into(), name: "Work hours".into(), rule: Rule::Schedule { week } },
            Condition { id: "key".into(), name: "Key".into(), rule: Rule::FlashDrive { serial: "1".into(), drive: "Stick".into() } },
            Condition { id: "hook".into(), name: "Home".into(), rule: Rule::Webhook(hook()) },
            Condition { id: "both".into(), name: "Both".into(), rule: Rule::Combined { mode: Mode::Any, members: vec![], rules: vec![] } },
        ]
    }

    // 1970-01-02 12:00 UTC, a zone at UTC: "today" is 2 January 1970.
    fn noon() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(86400 + 12 * 3600)
    }

    fn stopped(session: &Definition) -> (String, String, String) {
        let status = of(session, None, &conditions(), &Zone::fixed(0), noon());
        (status.condition.plain(), status.allowance.plain(), status.lock.plain())
    }

    fn running(run: Running) -> (String, String, String) {
        let status = of(&session(Lock::None, None, None), Some(&run), &conditions(), &Zone::fixed(0), noon());
        (status.condition.plain(), status.allowance.plain(), status.lock.plain())
    }

    fn run() -> Running {
        Running {
            id: "s".into(),
            name: "S".into(),
            targets: vec![],
            remaining: None,
            lock_condition: None,
            until_restart: false,
            lock_text: None,
            stop_delay: None,
            stop_delay_words: None,
            stop_remaining: None,
            stop_remaining_words: None,
            password: false,
            locked: false,
            condition: None,
            enforcing: true,
            condition_change: None,
            lock_change: None,
            remaining_words: None,
            condition_change_words: None,
            lock_change_words: None,
            allowance: None,
        }
    }

    fn allowance(terms: Terms, open: bool, change: Option<&str>) -> AllowanceStatus {
        AllowanceStatus {
            terms,
            open,
            change: change.map(|_| 60),
            change_words: change.map(Into::into),
            left: None,
            left_words: None,
            reset_in: None,
            reset_words: None,
            items: vec![],
            breaks_left: None,
        }
    }

    fn breaks(count: u32) -> Terms {
        Terms::Breaks { count, length: "10m".into(), reset: Reset::Never, every: String::new(), anchor: Anchor::Start }
    }

    fn budget(scope: Scope) -> Terms {
        Terms::Budget { allowed: "30m".into(), scope, reset: Reset::Never, every: String::new(), anchor: Anchor::Start }
    }

    #[test]
    fn a_stopped_session_says_what_is_set() {
        let pomodoro = Terms::Pomodoro { blocked: "25m".into(), allowed: "5m".into(), anchor: Anchor::Start };
        assert_eq!(stopped(&session(Lock::None, None, None)), ("At all times".into(), "Never paused".into(), "Not locked".into()));
        assert_eq!(stopped(&session(Lock::None, Some("hours"), Some(pomodoro))).0, "By schedule");
        for (id, words) in [("key", "By device"), ("hook", "By webhook"), ("both", "By combination"), ("gone", "At all times")] {
            assert_eq!(stopped(&session(Lock::None, Some(id), None)).0, words);
        }
        let allowances = [
            (Terms::Pomodoro { blocked: "25m".into(), allowed: "5m".into(), anchor: Anchor::Start }, "25 minutes / 5 minutes"),
            (Terms::Delayed { kind: DelayKind::After, value: "90m".into() }, "After 2 hours"),
            (Terms::Delayed { kind: DelayKind::From, value: "1970-01-02 18:00".into() }, "From 18:00"),
            (Terms::Delayed { kind: DelayKind::From, value: "1970-01-03 18:00".into() }, "From January 3"),
            (budget(Scope::Total), "Allowed for 30 minutes"),
            (budget(Scope::Each), "30 minutes per item"),
            (breaks(3), "3 breaks of 10 minutes"),
            (breaks(1), "1 break of 10 minutes"),
        ];
        for (terms, words) in allowances {
            assert_eq!(stopped(&session(Lock::None, None, Some(terms))).1, words);
        }
        let locks = [
            (Lock::timed_for("90m"), "For 2 hours"),
            (Lock::timed_for("soon"), "For soon"),
            (Lock::Timed { kind: TimedKind::Until, value: "1970-01-02 18:00".into() }, "Until 18:00"),
            (Lock::Timed { kind: TimedKind::Until, value: "soon".into() }, "Until soon"),
            (Lock::UntilRestart, "Until restart"),
            (Lock::Delay { delay: "5m".into() }, "Delay 5 minutes"),
            (Lock::Text { text: "x".into() }, "Type text to stop"),
            (Lock::Password { password: None, hash: None }, "Password to stop"),
            (Lock::Condition { condition: "hours".into() }, "During “Work hours”"),
            (Lock::Condition { condition: "key".into() }, "By “Key”"),
            (Lock::Condition { condition: "gone".into() }, "Condition"),
        ];
        for (lock, words) in locks {
            assert_eq!(stopped(&session(lock, None, None)).2, words);
        }
        // The window links the condition: its id and name, apart from the text.
        let status = of(&session(Lock::Condition { condition: "key".into() }, None, None), None, &conditions(), &Zone::fixed(0), noon());
        assert_eq!(status.lock, Line { text: "By {condition}".into(), condition: Some(Named { id: "key".into(), name: "Key".into() }) });
    }

    #[test]
    fn the_tag_says_whether_it_runs_is_locked_or_has_nothing_to_block() {
        let state = |run: Option<Running>| of(&session(Lock::None, None, None), run.as_ref(), &[], &Zone::fixed(0), noon()).state;
        assert_eq!(state(None), State::NotRunning);
        assert_eq!(state(Some(run())), State::Running);
        assert_eq!(state(Some(Running { locked: true, ..run() })), State::Locked);
        assert_eq!(state(Some(Running { locked: true, enforcing: false, ..run() })), State::Inactive);
        let words = [State::NotRunning, State::Running, State::Locked, State::Inactive].map(State::words);
        assert_eq!(words, ["Not running", "Running", "Running · locked", "Inactive"].map(String::from));
        // Sent to the window by these names.
        assert_eq!(serde_json::to_string(&State::NotRunning).unwrap(), "\"not_running\"");
    }

    #[test]
    fn a_running_session_says_what_is_happening() {
        assert_eq!(running(run()), ("Active".into(), "Never paused".into(), "Not locked".into()));
        let schedule = Rule::Schedule { week: week("daily") };
        let ruled = |rule: Rule, enforcing: bool, change: Option<&str>| {
            running(Running { condition: Some(rule), enforcing, condition_change_words: change.map(Into::into), ..run() }).0
        };
        assert_eq!(ruled(schedule.clone(), true, Some("4 hours")), "Active for 4 hours");
        assert_eq!(ruled(schedule.clone(), false, Some("3 hours")), "Inactive for 3 hours");
        assert_eq!(ruled(schedule, true, None), "Active");
        assert_eq!(ruled(Rule::FlashDrive { serial: "1".into(), drive: "Stick".into() }, false, None), "Inactive by device");
        assert_eq!(ruled(Rule::Webhook(hook()), true, None), "Active by webhook");
        assert_eq!(ruled(Rule::Combined { mode: Mode::All, members: vec![], rules: vec![] }, true, None), "Active by combination");
    }

    #[test]
    fn a_running_allowance_says_what_it_allows() {
        let with = |state: AllowanceStatus| running(Running { allowance: Some(state), ..run() }).1;
        let pomodoro = Terms::Pomodoro { blocked: "25m".into(), allowed: "5m".into(), anchor: Anchor::Start };
        assert_eq!(with(allowance(pomodoro.clone(), true, Some("4 minutes"))), "Allowed for 4 minutes");
        assert_eq!(with(allowance(pomodoro.clone(), false, Some("20 minutes"))), "Allowed in 20 minutes");
        assert_eq!(with(allowance(pomodoro, false, None)), "Blocked");
        let delayed = Terms::Delayed { kind: DelayKind::After, value: "1h".into() };
        assert_eq!(with(allowance(delayed.clone(), true, Some("10 minutes"))), "Starts in 10 minutes");
        assert_eq!(with(allowance(delayed, false, None)), "Started");
        let total = |left: u64, reset: Option<&str>| AllowanceStatus {
            left: Some(left),
            left_words: Some(duration::left(left)),
            reset_words: reset.map(Into::into),
            ..allowance(budget(Scope::Total), left > 0, None)
        };
        assert_eq!(with(total(720, Some("8 hours"))), "12 minutes left");
        assert_eq!(with(total(0, Some("8 hours"))), "Back in 8 hours");
        assert_eq!(with(total(0, None)), "Used up");
        let each = |used_up: usize| AllowanceStatus {
            items: (0..3)
                .map(|i| ItemLeft { item: format!("site:{i}"), left: if i < used_up { 0 } else { 60 }, left_words: String::new() })
                .collect(),
            ..allowance(budget(Scope::Each), true, None)
        };
        assert_eq!(with(each(0)), "None used up");
        assert_eq!(with(each(1)), "1 item used up");
        assert_eq!(with(each(2)), "2 items used up");
        // Breaks: one going on, then how many are left.
        assert_eq!(with(allowance(breaks(3), true, Some("7 minutes"))), "7 minutes left");
        let left = |n| AllowanceStatus { breaks_left: Some(n), ..allowance(breaks(3), false, None) };
        assert_eq!(with(left(3)), "3 breaks left");
        assert_eq!(with(left(1)), "1 break left");
        assert_eq!(with(left(0)), "None left");
        let back = AllowanceStatus { reset_words: Some("3 hours".into()), ..left(0) };
        assert_eq!(with(back), "Back in 3 hours");
        // While the condition is off, the same.
        let off = Running { enforcing: false, condition: Some(Rule::FlashDrive { serial: "1".into(), drive: "Stick".into() }), ..run() };
        assert_eq!(running(off).1, "Never paused");
    }

    #[test]
    fn a_running_lock_says_how_it_stands() {
        let lock = |run: Running| running(run).2;
        assert_eq!(lock(Running { remaining_words: Some("83 minutes".into()), ..run() }), "83 minutes left");
        assert_eq!(lock(Running { stop_delay: Some(300), ..run() }), "Delay 5 minutes");
        assert_eq!(
            lock(Running { stop_delay: Some(300), stop_remaining: Some(180), stop_remaining_words: Some("3 minutes".into()), ..run() }),
            "In 3 minutes"
        );
        assert_eq!(lock(Running { stop_delay: Some(300), stop_remaining: Some(0), ..run() }), "Unlocked");
        let drive = Rule::FlashDrive { serial: "1".into(), drive: "Stick".into() };
        assert_eq!(lock(Running { lock_condition: Some(drive.clone()), locked: true, ..run() }), "Locked by device");
        assert_eq!(lock(Running { lock_condition: Some(drive), ..run() }), "Unlocked by device");
        let schedule = Rule::Schedule { week: week("daily") };
        assert_eq!(
            lock(Running { lock_condition: Some(schedule), locked: true, lock_change_words: Some("2 hours".into()), ..run() }),
            "Locked for 2 hours"
        );
        assert_eq!(lock(Running { until_restart: true, ..run() }), "Until restart");
        assert_eq!(lock(Running { lock_text: Some("x".into()), ..run() }), "Type text to stop");
        assert_eq!(lock(Running { password: true, ..run() }), "Password to stop");
    }
}
