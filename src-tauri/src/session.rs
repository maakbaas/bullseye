//! The running sessions: for each, its targets (each with its own actions), its condition
//! if it has one, and its lock (`lock.rs`): when a duration or until lock ends, the lock condition,
//! the start of Windows a restart lock waits out, a password's hash. Each session is started from a
//! definition (`definitions.rs`) and runs on its own: several can run at once, and what is enforced
//! is all of them together. A session with a duration or until lock cannot be stopped early and stops
//! by itself once its time is up; one locked until Windows restarts ends with the restart. Outside
//! its condition a session keeps running, and stays locked, but enforces nothing.
//!
//! The running sessions are also kept on disk (`state.rs`), so ending the app, or restarting
//! Windows, does not end them: the next run picks them up again while their time is not up.
//!
//! Time is internet time (`clock.rs`), not the Windows clock, so setting that clock forward
//! does not end a timed session early.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::allowance::{self, Run, Scope, Terms, TOTAL};
use crate::clock::{self, Clock};
use crate::condition::Rule;
use crate::datetime::DateTime;
use crate::duration;
use crate::lists::{Entries, Lists};
use crate::localtime::Zone;
use crate::state::{self, Actions, Store, Uses};
use crate::sync::LockExt;
use crate::unlock;

/// How often running timed sessions write down the time they have reached (`Saved::seen`).
const CHECKPOINT: Duration = Duration::from_secs(60);

/// The longest time that one report of use is counted for, whatever the time since the last look:
/// the app asks every second, so more means it was not asking.
const MAX_GAP: Duration = Duration::from_secs(5);

/// Shared between the request handling and the ticker thread.
#[derive(Clone)]
pub struct Sessions {
    inner: Arc<Mutex<Inner>>,
    /// Where the sessions are kept between runs; `None` disables surviving a restart.
    store: Option<Arc<Store>>,
    /// The app's internet clock; tests have their own.
    clock: &'static Clock,
    /// The time zone a new session's schedule is read in; `None` is Windows' own (`localtime.rs`).
    /// Tests use a fixed one.
    zone: Option<Zone>,
    /// This start of Windows, for locks until it restarts; `None` is Windows' own (`boot.rs`).
    /// Tests use their own.
    boot: Option<String>,
}

struct Inner {
    sessions: Vec<Session>,
    /// When the sessions were last written to disk.
    saved_at: Instant,
    /// When the app last asked what to enforce: the time since is what a report of use counts for.
    polled: Instant,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner { sessions: Vec::new(), saved_at: Instant::now(), polled: Instant::now() }
    }
}

/// Something an allowance lets through for now, which the app is to watch rather than close: when
/// it is in use, that is counted against `key` of the session (`allowance.rs`). Its lists include
/// the always-allowed sites and titles of its target, which are not in use as they are not
/// blocked.
#[derive(Clone)]
pub struct Free {
    /// The running session, by its definition's id.
    pub session: String,
    /// What it is counted against: `allowance::TOTAL` or an item's key (`allowance::item_keys`).
    pub key: String,
    pub lists: Arc<Lists>,
}

/// What one running session enforces.
#[derive(Clone)]
pub struct Enforced {
    pub lists: Arc<Lists>,
    pub actions: Actions,
    /// What its allowance lets through now, to be watched (usually nothing).
    pub free: Vec<Free>,
}

impl Enforced {
    pub fn new(lists: Arc<Lists>, actions: Actions) -> Enforced {
        Enforced { lists, actions, free: Vec::new() }
    }

    /// Whether a page is to be closed: on the target, and not on the list of allowed sites.
    /// Allowed wins, whatever the target says, so `*` on the target with `google.com`
    /// allowed blocks every site but that one. Each session's allowed sites are exceptions to its
    /// own target only.
    pub fn blocks_site(&self, url: &str) -> bool {
        self.lists.sites.is_blocked(url) && !self.lists.allowed.is_blocked(url)
    }
}

/// What the app has seen in use that an allowance counts (`Free`), as (session, key), until it is
/// reported to the service (`client::Remote::active`). Shared by every copy of the same `Active`.
#[derive(Clone, Default)]
pub struct Usage(Arc<Mutex<BTreeSet<(String, String)>>>);

impl Usage {
    fn note(&self, session: &str, key: &str) {
        self.0.locked().insert((session.to_string(), key.to_string()));
    }

    /// What was seen since the last time, which is then forgotten.
    pub fn take(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.0.locked()).into_iter().collect()
    }

    /// Takes back what could not be reported.
    pub fn put_back(&self, used: Vec<(String, String)>) {
        self.0.locked().extend(used);
    }
}

/// What is done with a window by its title (`Active::title_action`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TitleAction {
    /// Asked to close, as its X does.
    Close,
    /// Minimized to the taskbar: nothing is lost.
    Minimize,
}

/// What is to be enforced right now: every running session's part, each with its own actions.
#[derive(Clone)]
pub struct Active {
    pub sessions: Vec<Enforced>,
    /// What an allowance's items were seen in use (written by the `blocks_*` questions below).
    pub used: Usage,
}

impl Active {
    /// What the closing asks of every page, program and window: whether any session closes it
    /// (`closes`, among those that are `on`: have the action for it). If none does, what an
    /// allowance lets through and that matches (`in_use`) is noted as in use, to be counted.
    fn judge(&self, on: impl Fn(&Enforced) -> bool, closes: impl Fn(&Enforced) -> bool, in_use: impl Fn(&Lists) -> bool) -> bool {
        if self.sessions.iter().any(|b| on(b) && closes(b)) {
            return true;
        }
        for session in self.sessions.iter().filter(|b| on(b)) {
            for free in session.free.iter().filter(|f| in_use(&f.lists)) {
                self.used.note(&free.session, &free.key);
            }
        }
        false
    }

    /// Whether a page is on a target and not allowed (`Enforced::blocks_site`), for the free items.
    fn site_in_use(lists: &Lists, url: &str) -> bool {
        lists.sites.is_blocked(url) && !lists.allowed.is_blocked(url)
    }

    /// Whether a page shown in a browser is to be closed, however it shows (a tab, a split view, an
    /// app window): by any session whose target has its site.
    pub fn blocks_page(&self, url: &str) -> bool {
        self.judge(|b| b.actions.uses(Uses::Sites), |b| b.blocks_site(url), |lists| Active::site_in_use(lists, url))
    }

    /// Whether any session has sites to look for, so the browsers are worth looking at: to close, or
    /// to see in use for an allowance.
    pub fn watches_sites(&self) -> bool {
        self.sessions
            .iter()
            .any(|b| b.actions.uses(Uses::Sites) && (!b.lists.sites.is_empty() || b.free.iter().any(|f| !f.lists.sites.is_empty())))
    }

    /// Whether any session closes programs and has some to close (or to see in use).
    pub fn closes_programs(&self) -> bool {
        self.sessions.iter().any(|b| {
            b.actions.uses(Uses::Programs) && (!b.lists.programs.is_empty() || b.free.iter().any(|f| !f.lists.programs.is_empty()))
        })
    }

    /// Whether a program (by file name) is closed by any session that closes programs.
    pub fn blocks_program(&self, file_name: &str) -> bool {
        self.judge(
            |b| b.actions.uses(Uses::Programs),
            |b| b.lists.programs.is_blocked(file_name),
            |lists| lists.programs.is_blocked(file_name),
        )
    }

    /// Whether any session closes Store apps and has some to close (or to see in use).
    pub fn closes_store_apps(&self) -> bool {
        self.sessions.iter().any(|b| {
            b.actions.uses(Uses::StoreApps) && (!b.lists.store_apps.is_empty() || b.free.iter().any(|f| !f.lists.store_apps.is_empty()))
        })
    }

    /// Whether a Store app (by package family name) is closed by any session that closes them. Never
    /// one that is part of Windows (`store_apps::is_protected`), whatever the lists say.
    pub fn blocks_store_app(&self, family: &str) -> bool {
        !crate::store_apps::is_protected(family)
            && self.judge(
                |b| b.actions.uses(Uses::StoreApps),
                |b| b.lists.store_apps.is_match(family),
                |lists| lists.store_apps.is_match(family),
            )
    }

    /// Whether any session closes or minimizes windows by their title and has some titles (or to see
    /// in use).
    pub fn watches_titles(&self) -> bool {
        self.sessions
            .iter()
            .any(|b| b.actions.uses(Uses::Titles) && (!b.lists.titles.is_empty() || b.free.iter().any(|f| !f.lists.titles.is_empty())))
    }

    /// Whether any session minimizes windows by their title, so any window coming up is worth a look
    /// at once (`wake::watch_every_window`).
    pub fn minimizes_windows(&self) -> bool {
        self.sessions.iter().any(|b| b.actions.window_titles_minimize && !b.lists.titles.is_empty())
    }

    /// What is done with a window of this title (`minimized`: whether it is down already), by the
    /// window titles of each target less its always-allowed ones (`Lists::blocks_title`): closed
    /// when any session that closes windows by title has it, the stricter, else minimized when one
    /// that minimizes them has it and it is up. If neither, what an allowance lets through and
    /// that matches is noted as in use: an open window for a session that closes, a window that is
    /// up for one that minimizes (a window down is not in use).
    pub fn title_action(&self, title: &str, minimized: bool) -> Option<TitleAction> {
        let closes = |b: &&Enforced| !b.actions.window_titles_minimize;
        let minimizes = |b: &&Enforced| b.actions.window_titles_minimize && !minimized;
        if self.sessions.iter().filter(closes).any(|b| b.lists.blocks_title(title)) {
            return Some(TitleAction::Close);
        }
        if self.sessions.iter().filter(minimizes).any(|b| b.lists.blocks_title(title)) {
            return Some(TitleAction::Minimize);
        }
        for session in self.sessions.iter().filter(|b| closes(b) || minimizes(b)) {
            for free in session.free.iter().filter(|f| f.lists.blocks_title(title)) {
                self.used.note(&free.session, &free.key);
            }
        }
        None
    }

    /// Whether any session closes Obsidian when it shows web pages.
    pub fn closes_obsidian(&self) -> bool {
        self.sessions.iter().any(|b| b.actions.obsidian_web_viewer)
    }
}

/// One target of a session, as it is started: its name, its lists and its actions.
pub struct Part {
    pub name: String,
    pub lists: Lists,
    pub actions: Actions,
}

impl Part {
    /// Whether it has anything to do: an action that is on and has its list (`Actions::can_run`).
    pub fn can_run(&self) -> bool {
        self.actions.can_run(|uses| self.lists.has(uses))
    }
}

/// How a session is to be locked when it starts: its definition's lock (`lock::Lock`) with what it
/// refers to looked up (a lock condition's rule, a password's hash).
pub enum StartLock {
    None,
    /// For this long from now.
    For(Duration),
    /// Until this local date and time, read in the session's time zone.
    Until(DateTime),
    /// While this condition is active (`condition.rs`).
    Condition(Rule),
    UntilRestart,
    /// Until stopping has been asked for and waited out, this long (`Lock::Delay`).
    Delay(Duration),
    /// Until this text is typed in (`Lock::Text`).
    Text(String),
    /// Until stopped with the password of this hash (`unlock::new_hash`).
    Password(String),
}

/// How a running session is locked, as it runs and as it is saved (`state::Saved::lock`). A
/// duration or until lock is the moment it ends by then.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RunLock {
    None,
    /// Cannot be stopped, and ends at `at` (milliseconds since the Unix epoch, internet time), the session
    /// with it.
    Ends {
        at: u64,
    },
    /// Cannot be stopped while `condition` is active (a schedule is read in the session's time zone).
    Condition {
        condition: Rule,
    },
    /// Cannot be stopped, and ends when Windows restarts: `boot` is the start of Windows it was
    /// started in (`boot::id`).
    UntilRestart {
        boot: String,
    },
    /// Stopped by asking to and waiting `delay` seconds, on internet time: `asked` is when it was
    /// asked for (milliseconds since the Unix epoch), `None` while it is not.
    Delay {
        delay: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asked: Option<u64>,
    },
    /// Stopped only by typing `text` in exactly.
    Text {
        text: String,
    },
    /// Stopped only with the password of `hash`.
    Password {
        hash: String,
    },
}

/// What the lists of an entry are called in an item's key and as a field of `Entries`
/// (`allowance::item_keys`).
const ITEMS: [(&str, &str); 4] = [("site", "filters"), ("program", "programs"), ("app", "store_apps"), ("title", "titles")];

/// A part of a running session: its target's name and what it enforces.
struct RunPart {
    name: String,
    enforced: Enforced,
    /// Each entry of its lists on its own, with the always-allowed sites and titles, by its key:
    /// what a budget for each item watches.
    items: Vec<(String, Arc<Lists>)>,
    /// Only its always-allowed sites and titles: what is left to enforce when everything of it is let through.
    exceptions: Arc<Lists>,
    /// Its lists without the items that are let through, for the ones that were (`remainder`).
    remaining: Option<(Vec<String>, Arc<Lists>)>,
}

impl RunPart {
    fn new(name: String, lists: Lists, actions: Actions) -> RunPart {
        let entries = lists.entries();
        let mut source = entries.clone();
        let mut items = Vec::new();
        for (prefix, field) in ITEMS {
            for entry in source.field(field).cloned().unwrap_or_default() {
                let mut one = Lists::exceptions(&entries);
                if let Some(list) = one.field(field) {
                    list.push(entry.clone());
                }
                if let Ok(parsed) = Lists::parse(&one) {
                    items.push((format!("{prefix}:{entry}"), Arc::new(parsed)));
                }
            }
        }
        let exceptions = Arc::new(Lists::parse(&Lists::exceptions(&entries)).unwrap_or_default());
        RunPart { name, enforced: Enforced::new(Arc::new(lists), actions), items, exceptions, remaining: None }
    }

    /// Its lists without the items `free`, as long as it is the same ones (parsed once). If they cannot
    /// be read, all of them: the session errs on the side of blocking.
    fn remainder(&mut self, free: &[String]) -> Arc<Lists> {
        if free.is_empty() {
            return self.enforced.lists.clone();
        }
        if let Some((keys, lists)) = &self.remaining {
            if keys == free {
                return lists.clone();
            }
        }
        let mut entries = self.enforced.lists.entries();
        for (prefix, field) in ITEMS {
            if let Some(list) = entries.field(field) {
                list.retain(|entry| !free.contains(&format!("{prefix}:{entry}")));
            }
        }
        let lists = Lists::parse(&entries).map(Arc::new).unwrap_or_else(|_| self.enforced.lists.clone());
        self.remaining = Some((free.to_vec(), lists.clone()));
        lists
    }
}

struct Session {
    /// The definition it was started from.
    id: String,
    name: String,
    /// One per target, each by its own actions and allowed sites.
    parts: Vec<RunPart>,
    lock: RunLock,
    /// When the session is active; `None` is always.
    condition: Option<Rule>,
    /// The time zone a schedule is read in: Windows' when the session started, kept until it ends,
    /// so changing the time zone does not move the schedule.
    zone: Zone,
    /// What lets it go quiet for a while, and what of that is used (`allowance.rs`).
    allowance: Option<Run>,
}

impl Session {
    /// Gives an allowance what is due with the time: a budget's reset.
    fn advance(&mut self, now: SystemTime) {
        let active = self.in_condition(now);
        if let Some(run) = &mut self.allowance {
            run.advance(&self.zone, now, active);
        }
    }

    /// Whether it has something to keep on disk besides its lock: a budget that is being used up.
    fn keeps_count(&self) -> bool {
        self.allowance.as_ref().is_some_and(|run| run.scope().is_some())
    }

    /// What of it the app is to enforce at `now`, by its allowance: all of it with none; nothing while a
    /// cycle allows everything; with a budget, what has time left is not enforced but watched
    /// (`Free`), so its use can be counted. Obsidian's action has no list to watch, so it stays.
    fn enforced(&mut self, now: SystemTime) -> Vec<Enforced> {
        let Session { id, zone, allowance, parts, .. } = self;
        let Some(run) = allowance.as_ref() else { return parts.iter().map(|p| p.enforced.clone()).collect() };
        match run.scope() {
            None if run.everything_allowed(zone, now) => Vec::new(),
            None => parts.iter().map(|p| p.enforced.clone()).collect(),
            Some(Scope::Total) if !run.is_free(TOTAL) => parts.iter().map(|p| p.enforced.clone()).collect(),
            Some(Scope::Total) => parts
                .iter()
                .map(|p| Enforced {
                    lists: p.exceptions.clone(),
                    actions: p.enforced.actions,
                    free: vec![Free { session: id.clone(), key: TOTAL.to_string(), lists: p.enforced.lists.clone() }],
                })
                .collect(),
            Some(Scope::Each) => parts
                .iter_mut()
                .map(|p| {
                    let free: Vec<String> = p.items.iter().filter(|(key, _)| run.is_free(key)).map(|(key, _)| key.clone()).collect();
                    let lists = p.remainder(&free);
                    let watched = p.items.iter().filter(|(key, _)| free.contains(key)).map(|(key, lists)| Free {
                        session: id.clone(),
                        key: key.clone(),
                        lists: lists.clone(),
                    });
                    Enforced { lists, actions: p.enforced.actions, free: watched.collect() }
                })
                .collect(),
        }
    }

    /// Whether `key` is something this session's allowance counts: the session as a whole, or one of its items.
    fn counts(&self, key: &str) -> bool {
        match self.allowance.as_ref().and_then(Run::scope) {
            Some(Scope::Total) => key == TOTAL,
            Some(Scope::Each) => self.parts.iter().any(|p| p.items.iter().any(|(item, _)| item == key)),
            None => false,
        }
    }

    /// Whether its condition has the session active at `now`. If it cannot be known (the local time,
    /// the connected drives), it is: a session errs on the side of blocking.
    fn in_condition(&self, now: SystemTime) -> bool {
        self.condition.as_ref().is_none_or(|rule| rule.active(&self.zone, now))
    }

    /// Seconds until the condition turns the session on or off, if it has a time for that.
    fn condition_change(&self, now: SystemTime) -> Option<u64> {
        self.condition.as_ref()?.next_change(&self.zone, now)
    }

    /// When a duration or until lock ends, and the session with it.
    fn ends(&self) -> Option<SystemTime> {
        match self.lock {
            RunLock::Ends { at } => Some(state::from_unix(at)),
            _ => None,
        }
    }

    /// Seconds left of a duration or until lock, rounded up.
    fn remaining(&self, now: SystemTime) -> Option<u64> {
        let left = self.ends()?.duration_since(now).ok()?;
        Some(left.as_secs_f64().ceil() as u64)
    }

    /// The rule of its condition lock.
    fn lock_rule(&self) -> Option<&Rule> {
        match &self.lock {
            RunLock::Condition { condition } => Some(condition),
            _ => None,
        }
    }

    /// Seconds until its lock condition locks or frees it, if it has a time for that.
    fn lock_change(&self, now: SystemTime) -> Option<u64> {
        self.lock_rule()?.next_change(&self.zone, now)
    }

    /// How long a delay lock makes stopping take, in seconds.
    fn stop_delay(&self) -> Option<u64> {
        match self.lock {
            RunLock::Delay { delay, .. } => Some(delay),
            _ => None,
        }
    }

    /// Seconds until a delay lock can be stopped: `None` when it was not asked for, `Some(0)` when it
    /// can be stopped now.
    fn stop_remaining(&self, now: SystemTime) -> Option<u64> {
        let RunLock::Delay { delay, asked: Some(asked) } = self.lock else { return None };
        let ready = asked.saturating_add(delay.saturating_mul(1000));
        Some(ready.saturating_sub(state::to_unix(now)?).div_ceil(1000))
    }

    /// The text a text lock asks to be typed.
    fn lock_text(&self) -> Option<&str> {
        match &self.lock {
            RunLock::Text { text } => Some(text),
            _ => None,
        }
    }

    fn password_hash(&self) -> Option<&str> {
        match &self.lock {
            RunLock::Password { hash } => Some(hash),
            _ => None,
        }
    }

    /// Why it cannot be stopped at `now`, if it cannot (without its password, for a password lock).
    fn locked(&self, now: SystemTime) -> Option<String> {
        let name = &self.name;
        match &self.lock {
            RunLock::None => None,
            RunLock::Ends { .. } => self.remaining(now).map(|left| format!("“{name}” is locked for the next {}", duration::left(left))),
            // Like `in_condition`, locked if it cannot be known.
            RunLock::Condition { condition } => condition.active(&self.zone, now).then(|| {
                let until = self.lock_change(now).map(|s| format!(" for the next {}", duration::left(s))).unwrap_or_default();
                match condition {
                    Rule::Schedule { .. } => format!("“{name}” is locked by its lock schedule{until}"),
                    Rule::FlashDrive { drive, .. } => format!("“{name}” is locked until the flash drive “{drive}” is connected"),
                    Rule::Webhook(_) => format!("“{name}” is locked by its webhook condition"),
                    Rule::Combined { .. } => format!("“{name}” is locked by its lock condition{until}"),
                }
            }),
            RunLock::UntilRestart { .. } => Some(format!("“{name}” is locked until Windows restarts")),
            RunLock::Delay { delay, .. } => match self.stop_remaining(now) {
                Some(0) => None,
                Some(left) => Some(format!("“{name}” can be stopped in {}", duration::left(left))),
                None => Some(format!("“{name}” is locked: asking to stop it starts a wait of {}", duration::countdown(*delay))),
            },
            RunLock::Text { .. } => Some(format!("“{name}” is locked until its text is typed in")),
            RunLock::Password { .. } => Some(format!("“{name}” is locked with a password")),
        }
    }
}

/// One target of a running session, as the UI shows it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct RunningTarget {
    pub name: String,
    #[serde(flatten)]
    pub entries: Entries,
    pub actions: Actions,
}

/// One running session, as the UI shows it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Running {
    pub id: String,
    pub name: String,
    /// What it blocks and how, as it was started, so the UI can show what a running session does.
    pub targets: Vec<RunningTarget>,
    /// Seconds left of a duration or until lock; `None`: no such lock.
    pub remaining: Option<u64>,
    /// Locked while this condition is active (`Lock::Condition`).
    pub lock_condition: Option<Rule>,
    /// Locked until Windows restarts, and ends then (`Lock::UntilRestart`).
    pub until_restart: bool,
    /// Locked until this text is typed in (`Lock::Text`): shown by the window when it asks.
    pub lock_text: Option<String>,
    /// Locked behind a wait before it can be stopped (`Lock::Delay`): how long it takes, in seconds
    /// and in words.
    pub stop_delay: Option<u64>,
    pub stop_delay_words: Option<String>,
    /// Seconds until it can be stopped, once asked for: `None` while not asked for, `Some(0)` when
    /// it can be stopped now; and in words.
    pub stop_remaining: Option<u64>,
    pub stop_remaining_words: Option<String>,
    /// Locked with a password: stopped only with it (`Lock::Password`).
    pub password: bool,
    /// Whether it cannot be stopped right now (without its password, for a password lock).
    pub locked: bool,
    /// Its condition; `None` is always active.
    pub condition: Option<Rule>,
    /// Whether it enforces anything right now: always, unless its condition has it inactive.
    pub enforcing: bool,
    /// Seconds until its condition turns it on or off, if it has one that ever does.
    pub condition_change: Option<u64>,
    /// Seconds until its lock condition locks or frees it, if it has one that ever does.
    pub lock_change: Option<u64>,
    /// The three times above in words (`duration::countdown`), so the window and the CLI show them the
    /// same way without a formatter of their own.
    pub remaining_words: Option<String>,
    pub condition_change_words: Option<String>,
    pub lock_change_words: Option<String>,
    /// What its allowance is doing, if it has one.
    pub allowance: Option<allowance::Status>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Status {
    pub running: Vec<Running>,
    /// Whether the time comes from the internet yet; until then, time with the service not running
    /// does not count.
    pub clock_synced: bool,
}

/// The lock a locked session may take instead of `old`: only a stricter one of the same kind. A
/// lock that ends at a time may end later (a duration or until, whichever it was given as), and a
/// delay may be longer, its wait if it has been asked for going on from when it was asked.
fn stricter(old: &RunLock, new: RunLock) -> Result<RunLock, String> {
    match (old, new) {
        (RunLock::Ends { at: old }, RunLock::Ends { at }) if at > *old => Ok(RunLock::Ends { at }),
        (RunLock::Ends { .. }, RunLock::Ends { .. }) => Err("This session is locked, so its lock can only end later".into()),
        (RunLock::Delay { delay: old, asked }, RunLock::Delay { delay, .. }) if delay > *old => Ok(RunLock::Delay { delay, asked: *asked }),
        (RunLock::Delay { .. }, RunLock::Delay { .. }) => Err("This session is locked, so its delay can only get longer".into()),
        _ => {
            Err("This session is locked, so its lock cannot be changed until it can be stopped, except to end later or to wait longer"
                .into())
        }
    }
}

impl Sessions {
    /// The service's target, over the real store in its data folder.
    pub fn new() -> Sessions {
        Sessions { inner: Arc::default(), store: state::store().map(Arc::new), clock: clock::global(), zone: None, boot: None }
    }

    /// A target over a store of its own and a clock of its own, for other modules' tests.
    #[cfg(test)]
    pub fn with_store(store: Store) -> Sessions {
        Sessions {
            inner: Arc::default(),
            store: Some(Arc::new(store)),
            clock: Box::leak(Box::default()),
            zone: Some(Zone::fixed(0)),
            boot: Some("boot".into()),
        }
    }

    /// Starts a session from the definition `id`, unless that one is running: checked here rather
    /// than only in the UI, so a locked session cannot be swapped for a shorter or unlocked one.
    /// `condition` says when it is active (`None`: always), `lock` how it is locked (`lock.rs`).
    /// Other sessions are not affected.
    #[cfg(test)]
    pub fn start(&self, id: &str, name: &str, parts: Vec<Part>, condition: Option<Rule>, lock: StartLock) -> Result<(), String> {
        self.start_with(id, name, parts, condition, None, lock)
    }

    /// As `start`, with an allowance (`allowance.rs`): `None` is none.
    pub fn start_with(
        &self,
        id: &str,
        name: &str,
        parts: Vec<Part>,
        condition: Option<Rule>,
        allowance: Option<Terms>,
        lock: StartLock,
    ) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        if inner.sessions.iter().any(|b| b.id == id) {
            return Err("This session is already running".into());
        }
        let zone = self.zone.clone().unwrap_or_else(Zone::current);
        let now = self.clock.now();
        let lock = self.run_lock(lock, &zone, now)?;
        let parts = parts.into_iter().map(|p| RunPart::new(p.name, p.lists, p.actions)).collect();
        let allowance = allowance.map(|terms| Run::new(terms, now));
        let new = Session { id: id.to_string(), name: name.to_string(), parts, lock, condition, zone, allowance };
        // Written before the session goes live, so a session cannot be missed after a restart. A
        // session that cannot be written would not survive the service ending, so it does not start.
        let mut all: Vec<&Session> = inner.sessions.iter().collect();
        all.push(&new);
        self.save(&all).map_err(|e| format!("Cannot save the session, so it would not survive a restart: {e}"))?;
        inner.sessions.push(new);
        inner.saved_at = Instant::now();
        Ok(())
    }

    /// What a lock asked for runs as, from `now` in `zone`: a duration or a date becomes the moment
    /// it ends, a restart this start of Windows.
    fn run_lock(&self, lock: StartLock, zone: &Zone, now: SystemTime) -> Result<RunLock, String> {
        let ends = |end: SystemTime| state::to_unix(end).map(|at| RunLock::Ends { at }).ok_or("Cannot tell the time");
        Ok(match lock {
            StartLock::None => RunLock::None,
            StartLock::For(length) => ends(now + length)?,
            StartLock::Until(at) => {
                let end = zone.instant(&at).ok_or_else(|| format!("Cannot tell when {at} is in this time zone"))?;
                if end <= now {
                    return Err(format!("{at} has passed"));
                }
                ends(end)?
            }
            StartLock::Condition(condition) => RunLock::Condition { condition },
            StartLock::UntilRestart => {
                RunLock::UntilRestart { boot: self.boot().ok_or("Cannot tell when Windows restarts, so this lock cannot be used")? }
            }
            StartLock::Delay(wait) => RunLock::Delay { delay: wait.as_secs(), asked: None },
            StartLock::Text(text) => RunLock::Text { text },
            StartLock::Password(hash) => RunLock::Password { hash },
        })
    }

    /// Gives the running session `id` another lock, as if it started with it now (in the time zone
    /// it runs in). Only while it is not locked: it could be stopped and started again with that
    /// lock anyway, so this loosens nothing. A locked one keeps its lock, except that a lock that
    /// ends at a time (a duration or until) may end later, and a delay may be longer (`stricter`).
    pub fn relock(&self, id: &str, lock: StartLock) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        let session = inner.sessions.iter().find(|b| b.id == id).ok_or("This session is not running")?;
        let lock = self.lock_for(session, lock, now)?;
        let at = inner.sessions.iter().position(|b| b.id == id).expect("found above");
        let old = std::mem::replace(&mut inner.sessions[at].lock, lock);
        // Written at once, as starting is: a lock that cannot be kept over a restart is not given.
        if let Err(e) = self.save(&inner.sessions.iter().collect::<Vec<_>>()) {
            inner.sessions[at].lock = old;
            return Err(format!("Cannot save the session, so it would not survive a restart: {e}"));
        }
        inner.saved_at = Instant::now();
        Ok(())
    }

    /// What `relock` would do, without doing it: whether the running session `id` may take this lock.
    pub fn check_relock(&self, id: &str, lock: StartLock) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let session = inner.sessions.iter().find(|b| b.id == id).ok_or("This session is not running")?;
        self.lock_for(session, lock, self.clock.now()).map(|_| ())
    }

    /// The lock a running session takes when it is given `lock`: as it starts with it now, and for a
    /// session that is locked only a stricter one (`stricter`).
    fn lock_for(&self, session: &Session, lock: StartLock, now: SystemTime) -> Result<RunLock, String> {
        let lock = self.run_lock(lock, &session.zone, now)?;
        if session.locked(now).is_some() {
            return stricter(&session.lock, lock);
        }
        Ok(lock)
    }

    /// Stops the session `id` unless it is locked right now. `answered`: the right password of its
    /// password lock, or the right text of its text lock, was given (`password_matches`,
    /// `text_matches`), which is what lets those stop.
    pub fn stop(&self, id: &str, answered: bool) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let Some(at) = inner.sessions.iter().position(|b| b.id == id) else {
            return Err("This session is not running".into());
        };
        let session = &inner.sessions[at];
        let password_locked = session.password_hash().is_some();
        let text_locked = session.lock_text().is_some();
        if !(answered && (password_locked || text_locked)) {
            if let Some(why) = session.locked(self.clock.now()) {
                let how = if password_locked {
                    "; give its password to stop it"
                } else if text_locked {
                    "; type its text in to stop it"
                } else {
                    ", so it cannot be stopped now"
                };
                return Err(format!("{why}{how}"));
            }
        }
        inner.sessions.remove(at);
        self.save_all(&mut inner);
        Ok(())
    }

    /// Starts the wait of a session locked with a delay; asking again keeps the wait that runs.
    pub fn ask_to_stop(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = state::to_unix(self.clock.now()).ok_or("Cannot tell the time")?;
        let session = inner.sessions.iter_mut().find(|b| b.id == id).ok_or("This session is not running")?;
        match &mut session.lock {
            RunLock::Delay { asked, .. } => {
                asked.get_or_insert(now);
            }
            _ => return Err("This session is not locked with a delay".into()),
        }
        self.save_all(&mut inner);
        Ok(())
    }

    /// Ends that wait, also when it is over: locked again.
    pub fn abort_stop(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let session = inner.sessions.iter_mut().find(|b| b.id == id).ok_or("This session is not running")?;
        match &mut session.lock {
            RunLock::Delay { asked, .. } => *asked = None,
            _ => return Err("This session is not locked with a delay".into()),
        }
        self.save_all(&mut inner);
        Ok(())
    }

    /// Takes one of the session's breaks now (`allowance::Run::take_break`): only while its condition
    /// has it on, as nothing is enforced otherwise. Kept on disk, so a restart does not give it back.
    pub fn take_break(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        let session = inner.sessions.iter_mut().find(|b| b.id == id).ok_or("This session is not running")?;
        if !session.in_condition(now) {
            return Err("A break can only be taken while the session's condition is active".into());
        }
        session.advance(now);
        session.allowance.as_mut().ok_or("This session has no breaks")?.take_break(now)?;
        self.save_all(&mut inner);
        Ok(())
    }

    /// Ends the break going on, early: it stays used up.
    pub fn end_break(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        let session = inner.sessions.iter_mut().find(|b| b.id == id).ok_or("This session is not running")?;
        session.allowance.as_mut().ok_or("This session has no breaks")?.end_break(now)?;
        self.save_all(&mut inner);
        Ok(())
    }

    /// Picks up the sessions left behind by an earlier run, so restarting Windows does not end them.
    /// A session whose time is up while the service was not running is not resumed.
    pub fn restore(&self) {
        let Some(store) = &self.store else { return };
        let saved = store.load();
        let mut sessions = Vec::new();
        for saved in &saved {
            // Until internet time is in, the clock carries on from where the sessions last were,
            // rather than from a Windows clock that may have been set forward meanwhile.
            if let Some(seen) = saved.seen {
                self.clock.at_least(state::from_unix(seen));
            }
        }
        for saved in saved {
            // A target we cannot read is dropped rather than resumed as something unknown.
            let parts: Result<Vec<RunPart>, String> =
                saved.parts.into_iter().map(|p| Ok(RunPart::new(p.name, Lists::parse(&p.entries)?, p.actions))).collect();
            let condition = saved.condition.clone().map(Rule::checked).transpose();
            let (Ok(parts), Ok(condition)) = (parts, condition) else {
                eprintln!("session: dropping an unreadable saved session");
                continue;
            };
            let ended = match &saved.lock {
                RunLock::Ends { at } => self.clock.now() >= state::from_unix(*at),
                // Locked until Windows restarts, and it has: the session ended with it. If this
                // start of Windows cannot be told, it errs on the side of blocking.
                RunLock::UntilRestart { boot } => self.boot().is_some_and(|now| now != *boot),
                _ => false,
            };
            if ended {
                continue;
            }
            sessions.push(Session {
                id: saved.id,
                name: saved.name,
                parts,
                lock: saved.lock,
                condition,
                zone: Zone::named(&saved.zone),
                allowance: saved.allowance,
            });
        }
        let mut inner = self.inner.locked();
        inner.sessions = sessions;
        // What was dropped is cleaned up, rather than being re-checked on every run.
        self.save_all(&mut inner);
    }

    /// What is to be enforced right now: every target of every running session that its condition
    /// has active. Ends timed sessions whose time is up. `None` when nothing is to be enforced.
    pub fn active(&self) -> Option<Active> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        // Keeps the time on disk moving, for a restart while offline (`Saved::seen`), and what a
        // budget has used.
        if inner.sessions.iter().any(|b| b.ends().is_some() || b.keeps_count()) && inner.saved_at.elapsed() >= CHECKPOINT {
            self.save_all(&mut inner);
        }
        let now = self.clock.now();
        for session in inner.sessions.iter_mut() {
            session.advance(now);
        }
        let sessions: Vec<Enforced> = inner.sessions.iter_mut().filter(|b| b.in_condition(now)).flat_map(|b| b.enforced(now)).collect();
        (!sessions.is_empty()).then_some(Active { sessions, used: Usage::default() })
    }

    /// The app has been told what to enforce: what use it reports from now on counts from here
    /// (`charge`). Only its own question does this, not the service looking for itself.
    pub fn polled(&self) {
        self.inner.locked().polled = Instant::now();
    }

    /// Counts what the app has seen in use (session id and key, `Free`) against the allowances of the
    /// sessions, for the time since the app last asked what to enforce (`MAX_GAP` at most). Only what
    /// a session's allowance counts and its condition has active now; anything else is ignored.
    pub fn charge(&self, used: &[(String, String)]) {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let millis = u64::try_from(inner.polled.elapsed().min(MAX_GAP).as_millis()).unwrap_or(0);
        let now = self.clock.now();
        for session in inner.sessions.iter_mut() {
            session.advance(now);
            let active = session.in_condition(now);
            let keys: Vec<&String> = used.iter().filter(|(id, key)| *id == session.id && session.counts(key)).map(|(_, key)| key).collect();
            if !active || keys.is_empty() {
                continue;
            }
            let keys: Vec<String> = keys.into_iter().cloned().collect();
            if let Some(run) = &mut session.allowance {
                for key in keys {
                    run.charge(&key, millis);
                }
            }
        }
    }

    /// The time the service goes by (internet time, `clock.rs`).
    pub fn now(&self) -> SystemTime {
        self.clock.now()
    }

    /// Whether any session runs, also one that its condition has inactive right now: it is still
    /// there and maybe locked, so the app is kept running and the exe is not replaced.
    pub fn is_running(&self) -> bool {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        !inner.sessions.is_empty()
    }

    /// The ids of the running sessions.
    pub fn running_ids(&self) -> Vec<String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        inner.sessions.iter().map(|b| b.id.clone()).collect()
    }

    /// The ids of the running sessions that cannot be stopped right now (`Session::locked`): what a
    /// target or condition they use may only be made stricter by (`handler::edit_shared`).
    pub fn locked_ids(&self) -> Vec<String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        inner.sessions.iter().filter(|b| b.locked(now).is_some()).map(|b| b.id.clone()).collect()
    }

    /// Gives the running session `id` its name, targets and conditions as its definition has them
    /// now, after it or one of them was changed: all of them, its lock condition's rule too. Its
    /// lock (but for a condition lock, which follows its condition), its time zone and what else it started with stay. Whether the change may be made is
    /// the handler's to judge (`handler::edit_shared`, `edit_session`); nothing happens if the session
    /// does not run.
    pub fn refresh(
        &self,
        id: &str,
        name: &str,
        parts: Vec<Part>,
        condition: Option<Rule>,
        lock_rule: Option<Rule>,
        allowance: Option<Terms>,
    ) {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        let Some(session) = inner.sessions.iter_mut().find(|b| b.id == id) else { return };
        session.name = name.to_string();
        session.parts = parts.into_iter().map(|p| RunPart::new(p.name, p.lists, p.actions)).collect();
        session.condition = condition;
        // What is used of an allowance stays when it is changed; with another one added or taken
        // away it starts or ends.
        session.allowance = match (allowance, session.allowance.take()) {
            (Some(terms), Some(run)) => Some(run.retermed(terms)),
            (Some(terms), None) => Some(Run::new(terms, now)),
            (None, _) => None,
        };
        // A condition lock follows its condition: its rule, or no lock once the condition is deleted
        // (it was free, or the change would not have been made).
        if matches!(session.lock, RunLock::Condition { .. }) {
            session.lock = lock_rule.map_or(RunLock::None, |condition| RunLock::Condition { condition });
        }
        self.save_all(&mut inner);
    }

    /// Whether `password` is the password of the running session `id`'s password lock.
    /// Whether `text` is exactly the text of the running session `id`'s text lock (line breaks
    /// whichever way they were typed).
    pub fn text_matches(&self, id: &str, text: &str) -> bool {
        let inner = self.inner.locked();
        let same = |a: &str, b: &str| a.replace("\r\n", "\n") == b.replace("\r\n", "\n");
        inner.sessions.iter().find(|b| b.id == id).and_then(|b| b.lock_text()).is_some_and(|expected| same(expected, text))
    }

    pub fn password_matches(&self, id: &str, password: &str) -> bool {
        let inner = self.inner.locked();
        let hash = inner.sessions.iter().find(|b| b.id == id).and_then(|b| b.password_hash().map(str::to_string));
        // The hash is computed without holding the lock: it takes a moment.
        drop(inner);
        hash.is_some_and(|hash| unlock::check_hash(password, &hash))
    }

    /// Whether the session `id` runs.
    #[cfg(test)]
    pub fn runs(&self, id: &str) -> bool {
        self.running_ids().iter().any(|running| running == id)
    }

    /// Why some running session cannot be stopped right now, if one cannot: then neither may the
    /// service be uninstalled.
    pub fn locked(&self) -> Option<String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        inner.sessions.iter().find_map(|b| b.locked(now))
    }

    pub fn status(&self) -> Status {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        for session in inner.sessions.iter_mut() {
            session.advance(now);
        }
        let running = inner
            .sessions
            .iter()
            .map(|b| Running {
                id: b.id.clone(),
                name: b.name.clone(),
                targets: b
                    .parts
                    .iter()
                    .map(|p| RunningTarget { name: p.name.clone(), entries: p.enforced.lists.entries(), actions: p.enforced.actions })
                    .collect(),
                remaining: b.remaining(now),
                lock_condition: b.lock_rule().cloned(),
                until_restart: matches!(b.lock, RunLock::UntilRestart { .. }),
                lock_text: b.lock_text().map(str::to_string),
                stop_delay: b.stop_delay(),
                stop_delay_words: b.stop_delay().map(duration::countdown),
                stop_remaining: b.stop_remaining(now),
                stop_remaining_words: b.stop_remaining(now).map(duration::left),
                password: b.password_hash().is_some(),
                locked: b.locked(now).is_some(),
                condition: b.condition.clone(),
                enforcing: b.in_condition(now),
                condition_change: b.condition_change(now),
                remaining_words: b.remaining(now).map(duration::left),
                condition_change_words: b.condition_change(now).map(duration::left),
                lock_change: b.lock_change(now),
                lock_change_words: b.lock_change(now).map(duration::left),
                allowance: b.allowance.as_ref().map(|run| run.status(&b.zone, now)),
            })
            .collect();
        Status { running, clock_synced: self.clock.synced() }
    }

    /// Ends every session whatever its lock, and forgets them. Only for unlocking
    /// (`handler::unlock`): everything else goes through `stop`, which keeps a locked session.
    pub fn reset(&self) {
        let mut inner = self.inner.locked();
        inner.sessions.clear();
        self.save_all(&mut inner);
    }

    fn boot(&self) -> Option<String> {
        self.boot.clone().or_else(crate::boot::id)
    }

    /// Removes sessions whose duration or until lock is up; they end by themselves and leave nothing to resume.
    fn expire(&self, inner: &mut Inner) {
        let now = self.clock.now();
        let before = inner.sessions.len();
        inner.sessions.retain(|b| b.ends().is_none_or(|end| now < end));
        if inner.sessions.len() != before {
            self.save_all(inner);
        }
    }

    fn save_all(&self, inner: &mut Inner) {
        inner.saved_at = Instant::now();
        if let Err(e) = self.save(&inner.sessions.iter().collect::<Vec<_>>()) {
            eprintln!("session: {e}");
        }
    }

    fn save(&self, sessions: &[&Session]) -> Result<(), String> {
        let Some(store) = &self.store else { return Ok(()) };
        let seen = state::to_unix(self.clock.now());
        let saved: Vec<state::Saved> = sessions
            .iter()
            .map(|b| state::Saved {
                id: b.id.clone(),
                name: b.name.clone(),
                parts: b
                    .parts
                    .iter()
                    .map(|p| state::SavedPart { name: p.name.clone(), entries: p.enforced.lists.entries(), actions: p.enforced.actions })
                    .collect(),
                seen,
                lock: b.lock.clone(),
                condition: b.condition.clone(),
                zone: b.zone.name().to_string(),
                allowance: b.allowance.clone(),
            })
            .collect();
        store.save(&saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterList;
    use crate::programs::ProgramList;
    use crate::schedule::Week;
    use crate::state::Saved;

    /// A weekly schedule as a condition's rule; `None` for a text that has no time in it.
    fn weekly(text: &str) -> Option<Rule> {
        Week::parse(text).unwrap().map(|week| Rule::Schedule { week })
    }

    /// A clock of its own per target, so one test restoring a session does not move another's.
    fn test_clock() -> &'static Clock {
        Box::leak(Box::default())
    }

    /// A target over a store of its own, so the tests never touch the real one.
    /// `wiped` starts from an empty store, for a test that starts a session of its own.
    fn sessions_of(name: &str, wiped: bool) -> Sessions {
        let dir = std::env::temp_dir().join(format!("session-{}-{name}", std::process::id()));
        if wiped {
            let _ = std::fs::remove_dir_all(&dir);
        }
        Sessions {
            inner: Arc::default(),
            clock: test_clock(),
            zone: Some(Zone::fixed(0)),
            store: Some(Arc::new(Store::new(dir.join("state.json")))),
            boot: Some("boot".into()),
        }
    }

    fn sessions(name: &str) -> Sessions {
        sessions_of(name, true)
    }

    /// The same store, as a second run of the service would find it.
    fn reopened(name: &str) -> Sessions {
        sessions_of(name, false)
    }

    /// Only sites to block; the program list is empty.
    fn sites(entries: &[&str]) -> Lists {
        Lists { sites: FilterList::parse(entries).unwrap(), ..Lists::default() }
    }

    fn start(list: &Sessions, id: &str, lists: Lists, duration: Option<Duration>) -> Result<(), String> {
        list.start(
            id,
            &format!("Session {id}"),
            vec![part(lists, Actions::default())],
            None,
            duration.map_or(StartLock::None, StartLock::For),
        )
    }

    /// One target of a session.
    fn part(lists: Lists, actions: Actions) -> Part {
        Part { name: "List".into(), lists, actions }
    }

    fn running(list: &Sessions, id: &str) -> Running {
        list.status().running.into_iter().find(|r| r.id == id).expect("not running")
    }

    #[test]
    fn timed_session_cannot_stop_early_and_ends_by_itself() {
        let list = sessions("timed");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_millis(50))).unwrap();
        assert!(list.stop("a", false).is_err());
        assert!(list.is_running());
        std::thread::sleep(Duration::from_millis(60));
        assert!(!list.is_running());
        assert!(list.stop("a", false).is_err(), "nothing left to stop");

        start(&list, "a", sites(&["a.com"]), None).unwrap();
        assert!(list.stop("a", false).is_ok());
        assert!(!list.is_running());
    }

    #[test]
    fn a_delay_lock_is_stopped_by_asking_and_waiting() {
        let list = sessions("delay");
        let begin =
            || list.start("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, StartLock::Delay(Duration::from_secs(1)));
        begin().unwrap();
        assert!(list.stop("a", false).is_err(), "not asked for");
        assert_eq!(running(&list, "a").stop_remaining, None);
        assert!(list.ask_to_stop("a").is_ok());
        assert!(list.stop("a", false).is_err(), "still waiting");
        // Asking again does not restart the wait.
        std::thread::sleep(Duration::from_millis(400));
        list.ask_to_stop("a").unwrap();
        assert!(running(&list, "a").stop_remaining.is_some_and(|s| s <= 1));
        // Aborting locks it again.
        list.abort_stop("a").unwrap();
        assert!(list.stop("a", false).is_err());
        list.ask_to_stop("a").unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(running(&list, "a").stop_remaining, Some(0));
        assert!(!running(&list, "a").locked);
        assert!(list.locked().is_none());
        assert!(list.stop("a", false).is_ok());
        // Only a delay lock can be asked.
        start(&list, "b", sites(&["b.com"]), None).unwrap();
        assert!(list.ask_to_stop("b").is_err() && list.abort_stop("b").is_err());
        assert!(list.ask_to_stop("gone").is_err());
    }

    #[test]
    fn a_running_session_cannot_be_started_again() {
        let list = sessions("replace");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(3600))).unwrap();
        // Not as a shorter one, nor as an untimed one that could then be stopped.
        assert!(start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(1))).is_err());
        assert!(start(&list, "a", sites(&["a.com"]), None).is_err());
        assert!(running(&list, "a").remaining.is_some_and(|left| left > 3500));

        // An untimed session is stopped first, then it can start again, with new lists.
        let untimed = sessions("replace-untimed");
        start(&untimed, "a", sites(&["a.com"]), None).unwrap();
        assert!(start(&untimed, "a", sites(&["b.com"]), None).is_err());
        untimed.stop("a", false).unwrap();
        start(&untimed, "a", sites(&["b.com"]), None).unwrap();
        assert_eq!(running(&untimed, "a").targets[0].entries.filters, vec!["b.com"]);
    }

    #[test]
    fn sessions_run_side_by_side_and_end_on_their_own() {
        let list = sessions("several");
        start(&list, "timed", sites(&["a.com"]), Some(Duration::from_millis(50))).unwrap();
        start(&list, "untimed", sites(&["b.com"]), None).unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.sessions.len(), 2);
        assert!(active.blocks_page("https://a.com/") && active.blocks_page("https://b.com/"));
        assert!(list.locked().is_some_and(|why| why.contains("Session timed")));

        // The timed one ends by itself; the other goes on.
        std::thread::sleep(Duration::from_millis(60));
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/") && active.blocks_page("https://b.com/"));
        assert!(!list.runs("timed") && list.runs("untimed"));
        assert_eq!(list.locked(), None);
        // Stopping one leaves the others.
        start(&list, "other", sites(&["c.com"]), None).unwrap();
        list.stop("untimed", false).unwrap();
        assert_eq!(list.status().running.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["other"]);
    }

    #[test]
    fn each_session_goes_by_its_own_allowed_sites() {
        let list = sessions("own-rules");
        let no_obsidian = Actions { obsidian_web_viewer: false, ..Actions::default() };
        let everything_but_google =
            Lists { sites: FilterList::parse(&["*"]).unwrap(), allowed: FilterList::parse(&["google.com"]).unwrap(), ..Lists::default() };
        list.start("all", "All", vec![part(everything_but_google, no_obsidian)], None, StartLock::None).unwrap();
        assert!(!list.active().unwrap().blocks_page("https://google.com/") && list.active().unwrap().blocks_page("https://youtube.com/"));
        list.start("google", "Google", vec![part(sites(&["google.com"]), no_obsidian)], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        // One session's allowed site is not an exception to another session's list.
        assert!(active.blocks_page("https://google.com/") && active.blocks_page("https://youtube.com/"));
        assert!(active.watches_sites() && !active.closes_programs() && !active.closes_obsidian());
    }

    #[test]
    fn a_session_that_cannot_be_saved_does_not_start() {
        // A store whose folder is a file, so the state file can never be written.
        let dir = std::env::temp_dir().join(format!("session-{}-unsaved", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("not-a-folder"), "").unwrap();
        let list = Sessions {
            inner: Arc::default(),
            clock: test_clock(),
            zone: Some(Zone::fixed(0)),
            store: Some(Arc::new(Store::new(dir.join("not-a-folder").join("state.json")))),
            boot: Some("boot".into()),
        };
        assert!(start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).is_err());
        assert!(!list.is_running());
    }

    #[test]
    fn a_running_session_that_is_not_locked_takes_a_lock_as_if_it_started_now() {
        let list = sessions("relock");
        assert!(list.relock("a", StartLock::For(Duration::from_secs(60))).unwrap_err().contains("not running"));
        start(&list, "a", sites(&["a.com"]), None).unwrap();
        list.relock("a", StartLock::For(Duration::from_secs(3600))).unwrap();
        assert!(running(&list, "a").remaining.is_some_and(|left| (3595..=3600).contains(&left)));
        assert!(list.stop("a", false).is_err());
        // Locked now, so it keeps the lock it has, whatever else is asked.
        for other in [StartLock::None, StartLock::Delay(Duration::from_secs(60)), StartLock::Text("x".into())] {
            let refused = list.relock("a", other).unwrap_err();
            assert_eq!(
                refused,
                "This session is locked, so its lock cannot be changed until it can be stopped, except to end later or to wait longer"
            );
        }
        // The lock is kept over a restart, as a lock it started with is.
        let resumed = reopened("relock");
        resumed.restore();
        assert!(running(&resumed, "a").remaining.is_some_and(|left| (3595..=3600).contains(&left)));
        assert!(resumed.stop("a", false).is_err());
        // A lock that cannot be given (a date that has passed) leaves the session as it was.
        start(&list, "b", sites(&["b.com"]), None).unwrap();
        let passed = list.relock("b", StartLock::Until(DateTime::parse("2000-01-01 00:00").unwrap())).unwrap_err();
        assert!(passed.contains("has passed"), "{passed}");
        assert!(list.stop("b", false).is_ok());
    }

    #[test]
    fn a_locked_session_can_end_later_or_wait_longer_but_not_the_opposite() {
        let list = sessions("relock-stricter");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(3600))).unwrap();
        let hours = |n: u64| StartLock::For(Duration::from_secs(n * 3600));
        // Ending sooner or at the same time is refused; later is taken, and held over a restart.
        for sooner in [1, 0] {
            let refused = list.relock("a", StartLock::For(Duration::from_secs(sooner * 1800))).unwrap_err();
            assert_eq!(refused, "This session is locked, so its lock can only end later");
        }
        list.relock("a", hours(3)).unwrap();
        assert!(running(&list, "a").remaining.is_some_and(|left| (10795..=10800).contains(&left)));
        assert!(list.relock("a", hours(2)).is_err());
        let resumed = reopened("relock-stricter");
        resumed.restore();
        assert!(running(&resumed, "a").remaining.is_some_and(|left| (10795..=10800).contains(&left)));
        // A date does as well as a duration.
        let far = DateTime::parse("2099-01-01 00:00").unwrap();
        list.relock("a", StartLock::Until(far)).unwrap();
        assert!(running(&list, "a").remaining.is_some_and(|left| left > 10800));

        // A delay: only a longer one, and a wait that has begun goes on from when it was asked.
        start(&list, "d", sites(&["d.com"]), None).unwrap();
        list.relock("d", StartLock::Delay(Duration::from_secs(60))).unwrap();
        assert_eq!(running(&list, "d").stop_delay, Some(60));
        for shorter in [60, 30] {
            let refused = list.relock("d", StartLock::Delay(Duration::from_secs(shorter))).unwrap_err();
            assert_eq!(refused, "This session is locked, so its delay can only get longer");
        }
        list.ask_to_stop("d").unwrap();
        list.relock("d", StartLock::Delay(Duration::from_secs(600))).unwrap();
        assert_eq!(running(&list, "d").stop_delay, Some(600));
        assert!(running(&list, "d").stop_remaining.is_some_and(|left| left > 60));
    }

    #[test]
    fn a_lock_that_cannot_be_saved_is_not_given() {
        let dir = std::env::temp_dir().join(format!("session-{}-relock-unsaved", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("not-a-folder"), "").unwrap();
        let list =
            Sessions { inner: Arc::default(), clock: test_clock(), zone: Some(Zone::fixed(0)), store: None, boot: Some("boot".into()) };
        start(&list, "a", sites(&["a.com"]), None).unwrap();
        // The same sessions, now with a state file that can never be written.
        let unsaved =
            Sessions { store: Some(Arc::new(Store::new(dir.join("not-a-folder").join("state.json")))), inner: list.inner.clone(), ..list };
        assert!(unsaved.relock("a", StartLock::For(Duration::from_secs(60))).unwrap_err().contains("would not survive a restart"));
        assert!(unsaved.stop("a", false).is_ok());
    }

    #[test]
    fn sessions_outlive_the_service_that_started_them() {
        let list = sessions("resume");
        start(&list, "a", sites(&["https://tweakers.net/", "b.com"]), Some(Duration::from_secs(3600))).unwrap();
        start(&list, "b", sites(&["c.com"]), None).unwrap();
        assert_eq!(running(&list, "a").targets[0].entries.filters, vec!["tweakers.net", "b.com"]);

        // A new run over the same store: both are still going, with their names.
        let resumed = reopened("resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.blocks_page("https://tweakers.net/nieuws") && active.blocks_page("https://c.com/"));
        let a = running(&resumed, "a");
        assert_eq!(a.name, "Session a");
        assert!(a.remaining.is_some_and(|left| (3595..=3600).contains(&left)));
        assert!(resumed.stop("a", false).is_err());
        assert_eq!(running(&resumed, "b").remaining, None);
        assert!(resumed.stop("b", false).is_ok());
    }

    /// A session that blocks `blocked` and always allows `allow`.
    fn with_allowed(blocked: &[&str], allow: &[&str]) -> Active {
        let list = Sessions::with_store(Store::new(
            std::env::temp_dir()
                .join(format!("session-{}-allowed-{}", std::process::id(), blocked.join("_").len() + allow.len()))
                .join("state.json"),
        ));
        let lists = Lists { sites: FilterList::parse(blocked).unwrap(), allowed: FilterList::parse(allow).unwrap(), ..Lists::default() };
        // `Actions::default()` has an action that needs no list, so an empty target starts too.
        start(&list, "a", lists, None).unwrap();
        list.active().unwrap()
    }

    fn blocks_site(active: &Active, url: &str) -> bool {
        active.blocks_page(url)
    }

    #[test]
    fn allowed_sites_win_over_the_sites_to_block() {
        // Everything is blocked, except what is allowed.
        let active = with_allowed(&["*"], &["google.com", "*.wikipedia.org"]);
        assert!(blocks_site(&active, "https://youtube.com/watch?v=1"));
        assert!(blocks_site(&active, "https://tweakers.net/"));
        assert!(!blocks_site(&active, "https://google.com/search?q=a"));
        assert!(!blocks_site(&active, "https://www.google.com/"));
        assert!(!blocks_site(&active, "https://nl.wikipedia.org/wiki/Rust"));
        // A domain allows its subdomains too; an entry with a `*` is exactly as wide as written.
        assert!(!blocks_site(&active, "https://accounts.google.com/"));
        assert!(blocks_site(&active, "https://wikipedia.org/"));
        assert!(blocks_site(&active, "https://notgoogle.com/"));

        // A blocked site and an exception inside it: the exception wins.
        let active = with_allowed(&["youtube.com"], &["youtube.com/watch*"]);
        assert!(blocks_site(&active, "https://youtube.com/"));
        assert!(blocks_site(&active, "https://youtube.com/shorts/abc"));
        assert!(!blocks_site(&active, "https://youtube.com/watch?v=1"));

        // Allowed everything is allowed everything, whatever the target says.
        let active = with_allowed(&["youtube.com", "*.tweakers.net"], &["*"]);
        assert!(!blocks_site(&active, "https://youtube.com/") && !blocks_site(&active, "https://a.tweakers.net/"));
    }

    #[test]
    fn any_combination_of_the_two_lists_is_possible() {
        // Neither: nothing is blocked.
        let active = with_allowed(&[], &[]);
        assert!(!blocks_site(&active, "https://youtube.com/"));
        // Allowed alone: there is nothing to make exceptions to.
        let active = with_allowed(&[], &["google.com"]);
        assert!(!blocks_site(&active, "https://youtube.com/") && !blocks_site(&active, "https://google.com/"));
        // Blocked alone.
        let active = with_allowed(&["youtube.com"], &[]);
        assert!(blocks_site(&active, "https://youtube.com/") && !blocks_site(&active, "https://google.com/"));
        // The browser's own pages are neither blocked nor in need of an exception.
        let active = with_allowed(&["*"], &[]);
        assert!(!blocks_site(&active, "chrome://newtab/") && !blocks_site(&active, "about:blank"));
    }

    #[test]
    fn the_allowed_sites_survive_a_restart() {
        let list = sessions("allowed-resume");
        let lists = Lists {
            sites: FilterList::parse(&["*"]).unwrap(),
            allowed: FilterList::parse(&["Google.com", "*.wikipedia.org"]).unwrap(),
            ..Lists::default()
        };
        start(&list, "a", lists, Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(running(&list, "a").targets[0].entries.allowed, vec!["google.com", "*.wikipedia.org"]);

        let resumed = reopened("allowed-resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(blocks_site(&active, "https://youtube.com/") && !blocks_site(&active, "https://google.com/"));
        assert_eq!(running(&resumed, "a").targets[0].entries.allowed, vec!["google.com", "*.wikipedia.org"]);
    }

    #[test]
    fn a_resumed_session_keeps_its_programs() {
        let list = sessions("programs");
        let lists = Lists { programs: ProgramList::parse(&["Steam*", "game"]).unwrap(), ..Lists::default() };
        start(&list, "a", lists, Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(running(&list, "a").targets[0].entries.programs, vec!["steam*.exe", "game.exe"]);

        // A new run over the same store: the programs are still on the session.
        let resumed = reopened("programs");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.closes_programs());
        assert!(active.blocks_program("steamwebhelper.exe") && active.blocks_program("GAME.EXE"));
        assert!(!active.blocks_program("chrome.exe"));
        assert_eq!(running(&resumed, "a").targets[0].entries.programs, vec!["steam*.exe", "game.exe"]);
    }

    #[test]
    fn a_resumed_session_keeps_its_actions() {
        let only_obsidian = Actions { obsidian_web_viewer: true, window_titles_minimize: true };
        let list = sessions("actions");
        list.start("a", "A", vec![part(sites(&[]), only_obsidian)], None, StartLock::For(Duration::from_secs(3600))).unwrap();
        let resumed = reopened("actions");
        resumed.restore();
        assert_eq!(resumed.active().map(|active| active.sessions[0].actions), Some(only_obsidian));
        assert_eq!(running(&resumed, "a").targets[0].actions, only_obsidian);
    }

    /// 2030-01-07 00:00:00 UTC, a Monday; the tests' time zone is UTC.
    const MONDAY: u64 = 1_893_974_400;

    /// `MONDAY` plus `seconds`, as a time.
    fn monday(seconds: u64) -> SystemTime {
        state::from_unix((MONDAY + seconds) * 1000)
    }

    #[test]
    fn outside_its_schedule_a_session_runs_and_stays_locked_but_enforces_nothing() {
        let list = sessions("schedule");
        // Monday 16:00, office hours until 17:00, locked for two hours: the user's own example.
        list.clock.at_least(monday(16 * 3600));
        let office = weekly("mon-fri 08:00-17:00");
        list.start(
            "a",
            "A",
            vec![part(sites(&["a.com"]), Actions::default())],
            office.clone(),
            StartLock::For(Duration::from_secs(2 * 3600)),
        )
        .unwrap();
        // An always-active session next to it, to show the other one going quiet on its own.
        start(&list, "b", sites(&["b.com"]), None).unwrap();
        assert!(list.active().unwrap().blocks_page("https://a.com/"));
        let a = running(&list, "a");
        assert!(a.enforcing && a.condition == office);
        // The test runs on a real clock as well, so a few seconds may pass along the way.
        assert!(a.condition_change.is_some_and(|left| (3590..=3600).contains(&left)));
        assert!(running(&list, "b").enforcing && running(&list, "b").condition_change.is_none());

        // 17:00: the schedule has it inactive, so nothing of it is enforced; it still runs and is
        // still locked.
        list.clock.at_least(monday(17 * 3600));
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/") && active.blocks_page("https://b.com/"));
        assert!(list.runs("a") && list.stop("a", false).is_err());
        let a = running(&list, "a");
        assert!(!a.enforcing);
        // In words too, for the window and the CLI.
        assert!(
            a.remaining_words.as_deref().is_some_and(|words| words == "60 minutes" || words == "59 minutes"),
            "{:?}",
            a.remaining_words
        );
        assert!(a.condition_change_words.as_deref().is_some_and(|words| words.starts_with("14 hours") || words.starts_with("15 hours")));
        // Rounded up, and the start took a moment of the clock: a little over the hour.
        assert!(a.remaining.is_some_and(|left| (3590..=3610).contains(&left)));
        // Active again on Tuesday at 08:00.
        assert!(a.condition_change.is_some_and(|left| (15 * 3600 - 10..=15 * 3600).contains(&left)));
        // With only it running, nothing is enforced, but a session still runs (the app is kept alive).
        list.stop("b", false).unwrap();
        assert!(list.active().is_none() && list.is_running());

        // Just after 18:00 (the lock started a moment after 16:00): the lock is over, and with it
        // the session.
        list.clock.at_least(monday(18 * 3600 + 30));
        assert!(!list.runs("a") && !list.is_running());
    }

    #[test]
    fn a_lock_schedule_locks_only_while_it_is_active() {
        let list = sessions("lock-schedule");
        // Monday 16:00. Active all day, locked during office hours, until 17:00.
        list.clock.at_least(monday(16 * 3600));
        let office = weekly("mon-fri 08:00-17:00");
        let locks = || StartLock::Condition(office.clone().unwrap());
        list.start("a", "Work", vec![part(sites(&["a.com"]), Actions::default())], None, locks()).unwrap();
        let why = list.stop("a", false).unwrap_err();
        assert!(why.contains("locked by its lock schedule for the next 60 minutes") || why.contains("for the next 59 minutes"), "{why}");
        let a = running(&list, "a");
        assert!(a.locked && a.lock_condition == office && a.enforcing);
        assert!(a.lock_change.is_some_and(|left| (3590..=3600).contains(&left)), "{:?}", a.lock_change);
        assert!(list.locked().is_some(), "the service may not be uninstalled now");
        // 17:00: outside the lock schedule it can be stopped, and still blocks until it is.
        list.clock.at_least(monday(17 * 3600 + 5));
        let a = running(&list, "a");
        assert!(!a.locked && a.enforcing && list.locked().is_none());
        // Locked again on Tuesday at 08:00.
        assert!(a.lock_change.is_some_and(|left| (15 * 3600 - 10..=15 * 3600).contains(&left)));
        list.stop("a", false).unwrap();
        assert!(!list.is_running());

        // It survives a restart as it was.
        list.clock.at_least(monday(7 * 86400 + 9 * 3600));
        list.start("a", "Work", vec![part(sites(&["a.com"]), Actions::default())], None, locks()).unwrap();
        let resumed = reopened("lock-schedule");
        resumed.clock.at_least(monday(7 * 86400 + 9 * 3600));
        resumed.restore();
        assert!(resumed.stop("a", false).is_err() && running(&resumed, "a").lock_condition == office);
    }

    #[test]
    fn a_password_lock_stops_only_with_its_password() {
        let list = sessions("password");
        let locks = StartLock::Password(unlock::new_hash("partner").unwrap());
        list.start("a", "Games", vec![part(sites(&["a.com"]), Actions::default())], None, locks).unwrap();
        let why = list.stop("a", false).unwrap_err();
        assert!(why.contains("locked with a password"), "{why}");
        assert!(running(&list, "a").password && running(&list, "a").locked && list.locked().is_some());
        assert!(!list.password_matches("a", "wrong") && list.password_matches("a", "partner"));
        // It survives a restart, still with only its hash.
        let resumed = reopened("password");
        resumed.restore();
        assert!(resumed.stop("a", false).is_err());
        assert!(!std::fs::read_to_string(resumed.store.clone().unwrap().path()).unwrap().contains("partner"));
        assert!(resumed.password_matches("a", "partner"));
        resumed.stop("a", true).unwrap();
        assert!(!resumed.is_running());
    }

    #[test]
    fn a_text_lock_stops_only_with_exactly_its_text() {
        let list = sessions("text");
        let locks = StartLock::Text("Write this\nout by hand".into());
        list.start("a", "Games", vec![part(sites(&["a.com"]), Actions::default())], None, locks).unwrap();
        let why = list.stop("a", false).unwrap_err();
        assert!(why.contains("type its text in"), "{why}");
        let a = running(&list, "a");
        assert!(a.locked && !a.password && a.lock_text.as_deref() == Some("Write this\nout by hand"));
        // Exactly: every character, case and spacing; only the kind of line break is forgiven.
        assert!(list.text_matches("a", "Write this\nout by hand") && list.text_matches("a", "Write this\r\nout by hand"));
        for wrong in ["write this\nout by hand", "Write this\nout by hand ", "Write this out by hand", "", "Write this"] {
            assert!(!list.text_matches("a", wrong), "{wrong:?}");
        }
        assert!(!list.text_matches("nope", "Write this\nout by hand"));
        // It survives a restart, and then stops with the text.
        let resumed = reopened("text");
        resumed.restore();
        assert!(resumed.stop("a", false).is_err());
        assert!(resumed.text_matches("a", "Write this\nout by hand"));
        resumed.stop("a", true).unwrap();
        assert!(!resumed.is_running());
    }

    #[test]
    fn a_session_of_several_targets_enforces_each_by_its_own_rules() {
        let list = sessions("parts");
        let tabs_only = Actions { obsidian_web_viewer: false, window_titles_minimize: false };
        let programs_only = tabs_only;
        let games = Lists { programs: ProgramList::parse(&["steam"]).unwrap(), ..Lists::default() };
        let social = Part { name: "Social".into(), lists: sites(&["youtube.com"]), actions: tabs_only };
        list.start(
            "a",
            "Evening",
            vec![social, Part { name: "Games".into(), lists: games, actions: programs_only }],
            None,
            StartLock::None,
        )
        .unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.sessions.len(), 2);
        assert!(active.blocks_page("https://youtube.com/") && active.blocks_program("steam.exe"));
        let lists = running(&list, "a").targets;
        assert_eq!(lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Social", "Games"]);
        // And again after a restart.
        let resumed = reopened("parts");
        resumed.restore();
        assert_eq!(running(&resumed, "a").targets, lists);
    }

    #[test]
    fn an_until_lock_ends_at_its_local_time() {
        let list = sessions("until");
        // Monday 16:00 UTC; the test zone is UTC, so 18:00 is two hours away.
        list.clock.at_least(monday(16 * 3600));
        let at = |text: &str| StartLock::Until(DateTime::parse(text).unwrap());
        let why = list.start("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, at("2030-01-07 12:00")).unwrap_err();
        assert!(why.contains("has passed"), "{why}");
        list.start("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, at("2030-01-07 18:00")).unwrap();
        assert!(list.stop("a", false).is_err());
        assert!(running(&list, "a").remaining.is_some_and(|left| (7190..=7200).contains(&left)));
        list.clock.at_least(monday(18 * 3600 + 1));
        assert!(!list.runs("a"));
    }

    #[test]
    fn a_lock_until_restart_holds_until_windows_restarts() {
        let list = sessions("until-restart");
        list.start("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, StartLock::UntilRestart).unwrap();
        let why = list.stop("a", false).unwrap_err();
        assert!(why.contains("until Windows restarts"), "{why}");
        assert!(running(&list, "a").until_restart && list.locked().is_some());
        // The service started again without a restart of Windows: still there, still locked.
        let resumed = reopened("until-restart");
        resumed.restore();
        assert!(resumed.stop("a", false).is_err());
        // After a restart the session has ended.
        let mut restarted = reopened("until-restart");
        restarted.boot = Some("next boot".into());
        restarted.restore();
        assert!(!restarted.is_running());
    }

    #[test]
    fn a_resumed_session_keeps_its_schedule_and_time_zone() {
        let list = sessions("schedule-resume");
        list.clock.at_least(monday(12 * 3600));
        let weekend = weekly("weekends");
        list.start("a", "A", vec![part(sites(&["a.com"]), Actions::default())], weekend.clone(), StartLock::None).unwrap();
        assert!(list.is_running() && list.active().is_none(), "Monday is not in the weekend");
        let saved = list.store.clone().unwrap().load().remove(0);
        assert_eq!((saved.condition, saved.zone.as_str()), (weekend.clone(), "fixed:+0"));

        let resumed = reopened("schedule-resume");
        resumed.clock.at_least(monday(5 * 86400 + 3600));
        resumed.restore();
        assert_eq!(running(&resumed, "a").condition, weekend);
        assert!(resumed.active().is_some(), "Saturday is");
    }

    #[test]
    fn a_session_whose_time_is_up_is_not_resumed() {
        let list = sessions("expired");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(3600))).unwrap();
        // Rewritten as though it had been left behind by a run that ended an hour ago, next to one
        // that still runs.
        let store = list.store.clone().unwrap();
        let part = state::SavedPart {
            name: "List".into(),
            entries: Entries { filters: vec!["a.com".into()], ..Entries::default() },
            actions: Actions::default(),
        };
        let saved = |id: &str, lock| Saved {
            id: id.into(),
            name: id.into(),
            parts: vec![part.clone()],
            seen: None,
            lock,
            condition: None,
            zone: "fixed:+0".into(),
            allowance: None,
        };
        store
            .save(&[
                saved("old", RunLock::Ends { at: state::to_unix(SystemTime::now()).unwrap() - 3_600_000 }),
                saved("new", RunLock::None),
            ])
            .unwrap();

        let resumed = reopened("expired");
        resumed.restore();
        assert_eq!(resumed.status().running.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["new"]);
        // And it is cleaned up, rather than being re-checked on every run.
        assert_eq!(store.load().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["new"]);
    }

    #[test]
    fn ending_the_sessions_clears_them() {
        let list = sessions("cleared");
        let store = list.store.clone().unwrap();
        start(&list, "a", sites(&["a.com"]), None).unwrap();
        assert_eq!(store.load().len(), 1);
        list.stop("a", false).unwrap();
        assert!(store.load().is_empty());

        // A timed session that ends by itself is cleared too, so nothing is left to resume.
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_millis(50))).unwrap();
        assert_eq!(store.load().len(), 1);
        std::thread::sleep(Duration::from_millis(60));
        assert!(!list.is_running());
        assert!(store.load().is_empty());

        // The password's reset ends them all.
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).unwrap();
        start(&list, "b", sites(&["b.com"]), None).unwrap();
        list.reset();
        assert!(!list.is_running() && store.load().is_empty());
    }

    #[test]
    fn a_break_is_taken_by_hand_while_the_condition_is_active_and_survives_a_restart() {
        let list = sessions("breaks");
        list.clock.at_least(monday(16 * 3600));
        let terms = Terms::Breaks {
            count: 1,
            length: "10m".into(),
            reset: allowance::Reset::Never,
            every: String::new(),
            anchor: allowance::Anchor::Start,
        };
        let parts = vec![part(sites(&["a.com"]), Actions::default())];
        let office = weekly("mon-fri 08:00-17:00");
        list.start_with("a", "A", parts, office, Some(terms), StartLock::For(Duration::from_secs(2 * 3600))).unwrap();
        start(&list, "plain", sites(&["b.com"]), None).unwrap();
        assert!(list.active().unwrap().blocks_page("https://a.com/"));
        // Only a session with breaks takes one; one that does not run takes nothing.
        assert_eq!(list.take_break("plain"), Err("This session has no breaks".into()));
        assert_eq!(list.take_break("gone"), Err("This session is not running".into()));
        assert_eq!(list.end_break("plain"), Err("This session has no breaks".into()));
        assert_eq!(list.end_break("gone"), Err("This session is not running".into()));
        // Taken, locked as it is: nothing of it is enforced until it ends.
        list.take_break("a").unwrap();
        assert!(!list.active().unwrap().blocks_page("https://a.com/") && list.active().unwrap().blocks_page("https://b.com/"));
        assert_eq!(running(&list, "a").allowance.and_then(|status| status.breaks_left), Some(0));
        // A restart keeps the break going and the one taken.
        let resumed = reopened("breaks");
        resumed.clock.at_least(monday(16 * 3600 + 60));
        resumed.restore();
        assert!(!resumed.active().unwrap().blocks_page("https://a.com/"));
        // Ended early: enforced again, and still used up.
        resumed.end_break("a").unwrap();
        assert!(resumed.active().unwrap().blocks_page("https://a.com/"));
        assert_eq!(resumed.take_break("a"), Err("No breaks left".into()));
        // Outside its condition there is nothing to take a break from.
        resumed.clock.at_least(monday(17 * 3600));
        assert_eq!(resumed.take_break("a"), Err("A break can only be taken while the session's condition is active".into()));
    }

    fn budget(allowed: &str, scope: Scope) -> Terms {
        Terms::Budget {
            allowed: allowed.into(),
            scope,
            reset: allowance::Reset::Never,
            every: String::new(),
            anchor: allowance::Anchor::Start,
        }
    }

    #[test]
    fn a_budget_for_the_session_lets_it_through_until_it_is_used_up() {
        let list = sessions("budget-total");
        list.start_with(
            "a",
            "A",
            vec![part(sites(&["a.com"]), Actions::default())],
            None,
            Some(budget("1s", Scope::Total)),
            StartLock::None,
        )
        .unwrap();
        // With time left the site is let through and its use is noted, to be counted.
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/x"));
        assert_eq!(active.used.take(), vec![("a".to_string(), TOTAL.to_string())]);
        assert!(running(&list, "a").allowance.is_some_and(|status| status.open && status.left.is_some_and(|left| left > 0)));
        // What is reported counts from the last time the app asked; ignored: another session, another key.
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("other".to_string(), TOTAL.to_string()), ("a".to_string(), "site:a.com".to_string())]);
        assert!(!list.active().unwrap().blocks_page("https://a.com/x"));
        list.charge(&[("a".to_string(), TOTAL.to_string())]);
        // Used up: closed again.
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://a.com/x"));
        assert!(running(&list, "a").allowance.is_some_and(|status| !status.open && status.left == Some(0)));
    }

    #[test]
    fn a_budget_for_each_item_counts_each_on_its_own() {
        let list = sessions("budget-each");
        list.start_with(
            "a",
            "A",
            vec![part(sites(&["a.com", "b.com"]), Actions::default())],
            None,
            Some(budget("1s", Scope::Each)),
            StartLock::None,
        )
        .unwrap();
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/") && !active.blocks_page("https://b.com/"));
        assert_eq!(active.used.take().len(), 2);
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("a".to_string(), "site:a.com".to_string())]);
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://a.com/"), "a.com is used up");
        assert!(!active.blocks_page("https://b.com/"), "b.com has all its time");
        assert_eq!(active.used.take(), vec![("a".to_string(), "site:b.com".to_string())]);
        let items = running(&list, "a").allowance.unwrap().items;
        assert_eq!(items.iter().map(|i| (i.item.as_str(), i.left)).collect::<Vec<_>>(), vec![("a.com", 0)]);
    }

    #[test]
    fn a_cycle_that_is_blocking_enforces_and_what_is_used_survives_a_restart() {
        let list = sessions("cycle");
        let cycle = Terms::Pomodoro { blocked: "1h".into(), allowed: "10m".into(), anchor: allowance::Anchor::Start };
        list.start_with("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, Some(cycle), StartLock::None).unwrap();
        assert!(list.active().unwrap().blocks_page("https://a.com/"), "blocked first");
        assert!(running(&list, "a").allowance.is_some_and(|status| !status.open && status.change.is_some()));

        let list = sessions("budget-kept");
        let each = budget("1m", Scope::Total);
        list.start_with("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, Some(each), StartLock::None).unwrap();
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("a".to_string(), TOTAL.to_string())]);
        // Written to disk with the next save, then read back as a second run finds it.
        list.refresh("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, None, Some(budget("1m", Scope::Total)));
        let resumed = reopened("budget-kept");
        resumed.restore();
        let left = running(&resumed, "a").allowance.unwrap().left.unwrap();
        assert!(left < 60, "what was used is kept: {left}");
        // Taking the allowance away ends it.
        resumed.refresh("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, None, None);
        assert!(running(&resumed, "a").allowance.is_none());
    }

    /// The lists of these entries, checked.
    fn lists_of(entries: Entries) -> Lists {
        Lists::parse(&entries).unwrap()
    }

    fn strings(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|e| e.to_string()).collect()
    }

    /// Only these actions (by `Actions::all`'s field names) turned on.
    fn only(on: &[&str]) -> Actions {
        let mut fields = serde_json::to_value(Actions::default()).unwrap();
        assert!(on.iter().all(|name| fields.get(name).is_some()), "an unknown action in {on:?}");
        for (name, value) in fields.as_object_mut().unwrap() {
            *value = serde_json::Value::Bool(on.contains(&name.as_str()));
        }
        serde_json::from_value(fields).unwrap()
    }

    #[test]
    fn each_action_answers_for_its_own_kind_of_window() {
        let list = sessions("kinds");
        let lists = lists_of(Entries {
            filters: strings(&["a.com"]),
            store_apps: strings(&["Microsoft.XboxApp_8wekyb3d8bbwe", "Microsoft.Windows.Photos_8wekyb3d8bbwe"]),
            titles: strings(&["*Chat*"]),
            ..Entries::default()
        });
        let actions = only(&[]);
        list.start("a", "A", vec![part(lists, actions)], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        // Sites: wherever they show.
        assert!(active.blocks_page("https://a.com/x") && !active.blocks_page("https://b.com/"));
        assert!(active.watches_sites());
        // Store apps, never one of Windows' own whatever the list says.
        assert!(active.closes_store_apps());
        assert!(active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe"));
        assert!(!active.blocks_store_app("Microsoft.Windows.Photos_8wekyb3d8bbwe"));
        assert!(!active.blocks_store_app("Microsoft.XboxGameOverlay_8wekyb3d8bbwe"), "only the app named");
        // Windows by their title.
        assert!(
            active.watches_titles()
                && (active.title_action("Team chat - Browser", false) == Some(TitleAction::Close))
                && active.title_action("Notes", false).is_none()
        );
        // Nothing else.
        assert!(!active.closes_programs() && !active.closes_obsidian() && !active.blocks_program("chat.exe"));
        assert!(active.used.take().is_empty());

        // A list is its own switch: empty, it does nothing, whatever the actions say.
        let list = sessions("kinds-off");
        list.start("a", "A", vec![part(lists_of(Entries::default()), Actions::default())], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert!(!active.closes_store_apps() && !active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe"));
        assert!(!active.watches_titles() && active.title_action("Team chat", false).is_none());
        assert!(!active.closes_programs() && !active.blocks_program("chat.exe") && active.closes_obsidian());
        // With entries, it acts with no action on at all.
        let list = sessions("kinds-on");
        let lists = lists_of(Entries {
            programs: strings(&["chat.exe"]),
            store_apps: strings(&["Microsoft.XboxApp_8wekyb3d8bbwe"]),
            titles: strings(&["*Chat*"]),
            ..Entries::default()
        });
        list.start("a", "A", vec![part(lists, only(&[]))], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert!(active.closes_store_apps() && active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe"));
        assert!(active.watches_titles() && active.title_action("Team chat", false) == Some(TitleAction::Close));
        assert!(active.closes_programs() && active.blocks_program("chat.exe") && !active.closes_obsidian());
    }

    #[test]
    fn an_always_allowed_title_is_left_alone_and_never_in_use() {
        let lists =
            || lists_of(Entries { titles: strings(&["*"]), allowed_titles: strings(&["*Visual Studio Code"]), ..Entries::default() });
        let code = "main.rs - bullseye - Visual Studio Code";
        // Closing and minimizing alike, of its own target only.
        let list = sessions("allowed-titles");
        list.start("close", "Close", vec![part(lists(), only(&[]))], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.title_action("YouTube - Chrome", false), Some(TitleAction::Close));
        assert!(active.title_action(code, false).is_none());
        list.start(
            "other",
            "Other",
            vec![part(lists_of(Entries { titles: strings(&["*Code"]), ..Entries::default() }), only(&["window_titles_minimize"]))],
            None,
            StartLock::None,
        )
        .unwrap();
        assert_eq!(
            list.active().unwrap().title_action(code, false),
            Some(TitleAction::Minimize),
            "another session's exception is not this one's"
        );
        let list = sessions("allowed-titles-down");
        list.start("down", "Down", vec![part(lists(), only(&["window_titles_minimize"]))], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.title_action("YouTube - Chrome", false), Some(TitleAction::Minimize));
        assert!(active.title_action(code, false).is_none());
        // With a budget, for each title and for the whole: what is always allowed is not in use.
        for (name, scope, key) in [("each", Scope::Each, "title:*"), ("total", Scope::Total, "")] {
            let list = sessions(&format!("allowed-titles-{name}"));
            list.start_with("a", "A", vec![part(lists(), only(&[]))], None, Some(budget("1h", scope)), StartLock::None).unwrap();
            let active = list.active().unwrap();
            assert!(active.title_action(code, false).is_none() && active.used.take().is_empty());
            assert!(active.title_action("YouTube - Chrome", false).is_none());
            assert_eq!(active.used.take(), vec![("a".to_string(), key.to_string())]);
        }
    }

    #[test]
    fn a_budget_for_each_item_watches_store_apps_and_titles_too() {
        let list = sessions("budget-kinds");
        let lists = lists_of(Entries {
            store_apps: strings(&["Microsoft.XboxApp_8wekyb3d8bbwe"]),
            titles: strings(&["*Chat*"]),
            programs: strings(&["game.exe"]),
            ..Entries::default()
        });
        list.start_with("a", "A", vec![part(lists, Actions::default())], None, Some(budget("1s", Scope::Each)), StartLock::None).unwrap();
        let active = list.active().unwrap();
        // Let through, and what is seen is noted under each item's key.
        assert!(active.closes_store_apps() && active.watches_titles() && active.closes_programs());
        assert!(!active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe"));
        assert!(active.title_action("Team chat", false).is_none());
        assert!(!active.blocks_program("game.exe"));
        assert!(!active.blocks_page("https://a.com/"));
        let mut seen = active.used.take();
        seen.sort();
        let key = |k: &str| ("a".to_string(), k.to_string());
        assert_eq!(seen, vec![key("app:Microsoft.XboxApp_8wekyb3d8bbwe"), key("program:game.exe"), key("title:*Chat*")]);
        // What could not be reported is taken back, to go with the next report.
        active.used.put_back(seen.clone());
        assert_eq!(active.used.take(), seen);
        // Used up: closed again.
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[key("app:Microsoft.XboxApp_8wekyb3d8bbwe"), key("title:*Chat*")]);
        let active = list.active().unwrap();
        assert!(
            active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe")
                && (active.title_action("Team chat", false) == Some(TitleAction::Close))
        );
        assert!(!active.blocks_program("game.exe"), "the program has its own time");
    }

    #[test]
    fn windows_by_title_are_minimized_or_closed_and_closing_wins() {
        let list = sessions("minimize");
        let titles = |entries: &[&str]| lists_of(Entries { titles: strings(entries), ..Entries::default() });
        list.start("down", "Down", vec![part(titles(&["*"]), only(&["window_titles_minimize"]))], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert!(active.watches_titles() && active.minimizes_windows());
        assert_eq!(active.title_action("Report.docx - Word", false), Some(TitleAction::Minimize));
        // A window that is down already is left as it is.
        assert_eq!(active.title_action("Report.docx - Word", true), None);
        // Another session that closes some of them: those are closed, also when they are down.
        list.start("chat", "Chat", vec![part(titles(&["*Chat*"]), only(&[]))], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.title_action("Team Chat", false), Some(TitleAction::Close));
        assert_eq!(active.title_action("Team Chat", true), Some(TitleAction::Close));
        assert_eq!(active.title_action("Notes", false), Some(TitleAction::Minimize));
        // Only closing: nothing is minimized, and no window wakes the worker for it.
        list.stop("down", false).unwrap();
        let active = list.active().unwrap();
        assert!(!active.minimizes_windows() && active.title_action("Notes", false).is_none());
    }

    #[test]
    fn a_budget_counts_a_minimized_title_only_while_its_window_is_up() {
        let list = sessions("minimize-budget");
        let titles = || lists_of(Entries { titles: strings(&["*Chat*"]), ..Entries::default() });
        list.start_with(
            "a",
            "A",
            vec![part(titles(), only(&["window_titles_minimize"]))],
            None,
            Some(budget("1h", Scope::Total)),
            StartLock::None,
        )
        .unwrap();
        let active = list.active().unwrap();
        // With time left the window is let be, and counted while it is up, not while it is down.
        assert_eq!(active.title_action("Team Chat", false), None);
        assert_eq!(active.used.take(), vec![("a".to_string(), TOTAL.to_string())]);
        assert_eq!(active.title_action("Team Chat", true), None);
        assert!(active.used.take().is_empty());
        // A session that closes counts an open window, up or down.
        let list = sessions("close-budget");
        list.start_with("a", "A", vec![part(titles(), only(&[]))], None, Some(budget("1h", Scope::Total)), StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.title_action("Team Chat", true), None);
        assert_eq!(active.used.take(), vec![("a".to_string(), TOTAL.to_string())]);
    }

    #[test]
    fn a_budget_never_lets_obsidian_through_and_an_allowed_cycle_does() {
        // Obsidian's action has no list, so no item: a budget, total or for each item, leaves it on.
        for scope in [Scope::Total, Scope::Each] {
            let list = sessions(&format!("obsidian-{scope:?}"));
            list.start_with("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, Some(budget("1h", scope)), StartLock::None)
                .unwrap();
            let active = list.active().unwrap();
            assert!(!active.blocks_page("https://a.com/"), "{scope:?}: the site is let through");
            assert!(active.closes_obsidian(), "{scope:?}: Obsidian is not");
            assert!(active.used.take().iter().all(|(_, key)| key.starts_with("site:") || key == TOTAL));
        }
        // While a cycle allows, the session enforces nothing at all, Obsidian included.
        let list = sessions("obsidian-cycle");
        let cycle = Terms::Pomodoro { blocked: "1s".into(), allowed: "1h".into(), anchor: allowance::Anchor::Start };
        list.start_with("a", "A", vec![part(sites(&["a.com"]), Actions::default())], None, Some(cycle), StartLock::None).unwrap();
        assert!(list.active().unwrap().closes_obsidian(), "blocked first");
        list.clock.at_least(SystemTime::now() + Duration::from_secs(2));
        assert!(list.active().is_none(), "allowed: nothing to enforce");
    }

    #[test]
    fn a_budget_item_is_an_entry_as_typed_wildcards_and_all() {
        let list = sessions("wildcards");
        list.start_with(
            "a",
            "A",
            vec![part(sites(&["*.reddit.com", "youtube.com", "*"]), Actions::default())],
            None,
            Some(budget("1s", Scope::Each)),
            StartLock::None,
        )
        .unwrap();
        let key = |k: &str| ("a".to_string(), k.to_string());
        // A wildcard entry is one item for everything it matches; a page that two entries match is
        // seen as both.
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://old.reddit.com/r/x") && !active.blocks_page("https://www.reddit.com/"));
        assert_eq!(active.used.take(), vec![key("site:*"), key("site:*.reddit.com")]);
        assert!(!active.blocks_page("https://youtube.com/"));
        assert_eq!(active.used.take(), vec![key("site:*"), key("site:youtube.com")]);
        // Once one of the entries that match a page is used up, the page is closed, though the other
        // has time.
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[key("site:youtube.com")]);
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://youtube.com/"));
        assert!(!active.blocks_page("https://example.com/"), "* still has time for the rest");
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[key("site:*")]);
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://example.com/") && active.blocks_page("https://old.reddit.com/"));
        // The items as the window shows them: by the entry as typed.
        let mut items: Vec<String> = running(&list, "a").allowance.unwrap().items.into_iter().map(|i| i.item).collect();
        items.sort();
        assert_eq!(items, vec!["*", "youtube.com"]);
    }

    #[test]
    fn a_condition_lock_says_what_it_waits_for() {
        let list = sessions("lock-words");
        list.clock.at_least(monday(16 * 3600));
        let start =
            |id: &str, rule: Rule| list.start(id, id, vec![part(sites(&["a.com"]), Actions::default())], None, StartLock::Condition(rule));
        // A drive that is not connected (no drive has this serial number) holds the lock.
        start("Drive", Rule::FlashDrive { serial: "no-such-serial".into(), drive: "Key".into() }).unwrap();
        let why = |id: &str| list.stop(id, false).unwrap_err();
        assert!(why("Drive").starts_with("“Drive” is locked until the flash drive “Key” is connected,"), "{}", why("Drive"));
        // A webhook that does not answer goes by its fallback: active, so locked.
        let hook = crate::webhook::Hook {
            url: "http://127.0.0.1:9/".into(),
            method: crate::webhook::Method::Get,
            interval: 60,
            fallback: true,
            matcher: crate::webhook::Matcher { on: crate::webhook::Match::Status, status: Some(204), body: None },
        };
        start("Hook", Rule::Webhook(hook)).unwrap();
        assert!(why("Hook").starts_with("“Hook” is locked by its webhook condition,"), "{}", why("Hook"));
        // A combination says how long, when its conditions have times.
        let any = Rule::Combined {
            mode: crate::condition::Mode::Any,
            members: vec!["m".into(), "t".into()],
            rules: vec![weekly("mon 08:00-17:00").unwrap(), weekly("tue 08:00-17:00").unwrap()],
        };
        start("Both", any).unwrap();
        let both = why("Both");
        assert!(
            both.starts_with("“Both” is locked by its lock condition for the next 60 minutes") || both.contains("59 minutes"),
            "{both}"
        );
    }

    #[test]
    fn a_report_counts_only_against_a_budget_and_only_while_active() {
        let list = sessions("report-ignored");
        start(&list, "plain", sites(&["a.com"]), None).unwrap();
        // 1970-01-05 was a Monday; the session's condition is office hours, so 20:00 is outside it.
        list.clock.at_least(monday(20 * 3600));
        list.start_with(
            "quiet",
            "Q",
            vec![part(sites(&["b.com"]), Actions::default())],
            weekly("mon 08:00-17:00"),
            Some(budget("1s", Scope::Total)),
            StartLock::None,
        )
        .unwrap();
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("plain".to_string(), TOTAL.to_string()), ("quiet".to_string(), TOTAL.to_string())]);
        assert!(running(&list, "plain").allowance.is_none());
        assert_eq!(running(&list, "quiet").allowance.unwrap().left, Some(1), "not counted while the condition is off");
    }

    #[test]
    fn a_budget_for_each_item_works_out_what_is_left_once_for_each_set_of_items() {
        let list = sessions("remainder");
        list.start_with(
            "a",
            "A",
            vec![part(sites(&["a.com", "b.com"]), Actions::default())],
            None,
            Some(budget("1s", Scope::Each)),
            StartLock::None,
        )
        .unwrap();
        // The same items free twice: the same lists, worked out once.
        let first = list.active().unwrap();
        let again = list.active().unwrap();
        assert!(Arc::ptr_eq(&first.sessions[0].lists, &again.sessions[0].lists));
        // Every item used up: all of it enforced, as started.
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("a".to_string(), "site:a.com".to_string()), ("a".to_string(), "site:b.com".to_string())]);
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://a.com/") && active.blocks_page("https://b.com/"));
        assert!(active.sessions[0].free.is_empty());
    }

    #[test]
    fn a_saved_session_that_cannot_be_read_any_more_is_dropped() {
        let list = sessions("unreadable");
        start(&list, "a", sites(&["a.com"]), None).unwrap();
        start(&list, "b", sites(&["b.com"]), None).unwrap();
        // Session "a" now names a program that is never closed, which its list cannot be read with.
        let path = list.store.as_ref().unwrap().path().to_path_buf();
        let mut saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        fn spoil(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(fields) if fields.get("filters").is_some_and(|f| f == &serde_json::json!(["a.com"])) => {
                    fields.insert("programs".into(), serde_json::json!(["explorer.exe"]));
                }
                serde_json::Value::Object(fields) => fields.values_mut().for_each(spoil),
                serde_json::Value::Array(items) => items.iter_mut().for_each(spoil),
                _ => {}
            }
        }
        spoil(&mut saved);
        std::fs::write(&path, saved.to_string()).unwrap();
        let resumed = reopened("unreadable");
        resumed.restore();
        assert!(!resumed.runs("a") && resumed.runs("b"));
    }

    #[test]
    fn without_a_store_a_session_still_works_for_this_run() {
        let dir = std::env::temp_dir().join(format!("session-nostore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");

        let list =
            Sessions { inner: Arc::default(), clock: test_clock(), zone: Some(Zone::fixed(0)), store: None, boot: Some("boot".into()) };
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).unwrap();
        assert!(list.is_running());
        list.restore();
        assert!(list.is_running(), "restoring from no store keeps what runs");
        assert!(!path.exists(), "a sessions with no store must not write anything");
        assert!(!dir.exists(), "a sessions with no store must not create a folder");
    }
}
