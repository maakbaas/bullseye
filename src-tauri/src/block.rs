//! The running blocks: for each, its block lists (each with its own enforcers), its condition
//! if it has one, and its lock (`lock.rs`): when a duration or until lock ends, the lock condition,
//! the start of Windows a restart lock waits out, a password's hash. Each block is started from a
//! definition (`definitions.rs`) and runs on its own: several can run at once, and what is enforced
//! is all of them together. A block with a duration or until lock cannot be stopped early and stops
//! by itself once its time is up; one locked until Windows restarts ends with the restart. Outside
//! its condition a block keeps running, and stays locked, but enforces nothing.
//!
//! The running blocks are also kept on disk (`state.rs`), so ending the app, or restarting
//! Windows, does not end them: the next run picks them up again while their time is not up.
//!
//! Time is internet time (`clock.rs`), not the Windows clock, so setting that clock forward
//! does not end a timed block early.

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
use crate::state::{self, Enforcers, Store, Uses};
use crate::sync::LockExt;
use crate::unlock;

/// How often running timed blocks write down the time they have reached (`Saved::seen`).
const CHECKPOINT: Duration = Duration::from_secs(60);

/// The longest time that one report of use is counted for, whatever the time since the last look:
/// the app asks every second, so more means it was not asking.
const MAX_GAP: Duration = Duration::from_secs(5);

/// Shared between the request handling and the ticker thread.
#[derive(Clone)]
pub struct Blocklist {
    inner: Arc<Mutex<Inner>>,
    /// Where the blocks are kept between runs; `None` disables surviving a restart.
    store: Option<Arc<Store>>,
    /// The app's internet clock; tests have their own.
    clock: &'static Clock,
    /// The time zone a new block's schedule is read in; `None` is Windows' own (`localtime.rs`).
    /// Tests use a fixed one.
    zone: Option<Zone>,
    /// This start of Windows, for locks until it restarts; `None` is Windows' own (`boot.rs`).
    /// Tests use their own.
    boot: Option<String>,
}

struct Inner {
    blocks: Vec<Block>,
    /// When the blocks were last written to disk.
    saved_at: Instant,
    /// When the app last asked what to enforce: the time since is what a report of use counts for.
    polled: Instant,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner { blocks: Vec::new(), saved_at: Instant::now(), polled: Instant::now() }
    }
}

/// Something an allowance lets through for now, which the app is to watch rather than close: when
/// it is in use, that is counted against `key` of the block (`allowance.rs`). Its lists include
/// the always-allowed sites of its block list, which are not in use as they are not blocked.
#[derive(Clone)]
pub struct Free {
    /// The running block, by its definition's id.
    pub block: String,
    /// What it is counted against: `allowance::TOTAL` or an item's key (`allowance::item_keys`).
    pub key: String,
    pub lists: Arc<Lists>,
}

/// What one running block enforces.
#[derive(Clone)]
pub struct Enforced {
    pub lists: Arc<Lists>,
    pub enforcers: Enforcers,
    /// What its allowance lets through now, to be watched (usually nothing).
    pub free: Vec<Free>,
}

impl Enforced {
    pub fn new(lists: Arc<Lists>, enforcers: Enforcers) -> Enforced {
        Enforced { lists, enforcers, free: Vec::new() }
    }

    /// Whether a page is to be closed: on the block list, and not on the list of allowed sites.
    /// Allowed wins, whatever the block list says, so `*` on the block list with `google.com`
    /// allowed blocks every site but that one. Each block's allowed sites are exceptions to its
    /// own block list only.
    pub fn blocks_site(&self, url: &str) -> bool {
        self.lists.sites.is_blocked(url) && !self.lists.allowed.is_blocked(url)
    }
}

/// What the app has seen in use that an allowance counts (`Free`), as (block, key), until it is
/// reported to the service (`client::Remote::active`). Shared by every copy of the same `Active`.
#[derive(Clone, Default)]
pub struct Usage(Arc<Mutex<BTreeSet<(String, String)>>>);

impl Usage {
    fn note(&self, block: &str, key: &str) {
        self.0.locked().insert((block.to_string(), key.to_string()));
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

/// What is to be enforced right now: every running block's part, each with its own enforcers.
#[derive(Clone)]
pub struct Active {
    pub blocks: Vec<Enforced>,
    /// What an allowance's items were seen in use (written by the `blocks_*` questions below).
    pub used: Usage,
}

impl Active {
    /// What the closing asks of every page, program and window: whether any block closes it
    /// (`closes`, among those that are `on`: have the enforcer for it). If none does, what an
    /// allowance lets through and that matches (`in_use`) is noted as in use, to be counted.
    fn judge(&self, on: impl Fn(&Enforced) -> bool, closes: impl Fn(&Enforced) -> bool, in_use: impl Fn(&Lists) -> bool) -> bool {
        if self.blocks.iter().any(|b| on(b) && closes(b)) {
            return true;
        }
        for block in self.blocks.iter().filter(|b| on(b)) {
            for free in block.free.iter().filter(|f| in_use(&f.lists)) {
                self.used.note(&free.block, &free.key);
            }
        }
        false
    }

    /// Whether a page is on a block list and not allowed (`Enforced::blocks_site`), for the free items.
    fn site_in_use(lists: &Lists, url: &str) -> bool {
        lists.sites.is_blocked(url) && !lists.allowed.is_blocked(url)
    }

    /// Whether a page shown in a browser is to be closed by any block that closes such pages:
    /// `split_view` for a window showing two pages, otherwise an ordinary tab.
    pub fn blocks_page(&self, url: &str, split_view: bool) -> bool {
        self.judge(
            |b| if split_view { b.enforcers.chromium_split_view } else { b.enforcers.chromium_tabs },
            |b| b.blocks_site(url),
            |lists| Active::site_in_use(lists, url),
        )
    }

    /// Whether a page shown in an app window (a Chromium window with no tabs) is to be closed, with
    /// the window, by any block that closes those.
    pub fn blocks_app_window(&self, url: &str) -> bool {
        self.judge(|b| b.enforcers.chromium_app_windows, |b| b.blocks_site(url), |lists| Active::site_in_use(lists, url))
    }

    /// Whether a page open in a Firefox tab is to be closed, by any block that closes those:
    /// `split_view` for a page in either half of a split view, otherwise an ordinary tab.
    pub fn blocks_firefox_page(&self, url: &str, split_view: bool) -> bool {
        self.judge(
            |b| if split_view { b.enforcers.firefox_split_view } else { b.enforcers.firefox_tabs },
            |b| b.blocks_site(url),
            |lists| Active::site_in_use(lists, url),
        )
    }

    /// Whether any block has sites to look for, so the browsers are worth looking at: to close, or
    /// to see in use for an allowance.
    pub fn watches_sites(&self) -> bool {
        self.blocks
            .iter()
            .any(|b| b.enforcers.uses(Uses::Sites) && (!b.lists.sites.is_empty() || b.free.iter().any(|f| !f.lists.sites.is_empty())))
    }

    /// Whether any block closes programs and has some to close (or to see in use).
    pub fn closes_programs(&self) -> bool {
        self.blocks.iter().any(|b| {
            b.enforcers.uses(Uses::Programs) && (!b.lists.programs.is_empty() || b.free.iter().any(|f| !f.lists.programs.is_empty()))
        })
    }

    /// Whether a program (by file name) is closed by any block that closes programs.
    pub fn blocks_program(&self, file_name: &str) -> bool {
        self.judge(
            |b| b.enforcers.uses(Uses::Programs),
            |b| b.lists.programs.is_blocked(file_name),
            |lists| lists.programs.is_blocked(file_name),
        )
    }

    /// Whether any block closes Store apps and has some to close (or to see in use).
    pub fn closes_store_apps(&self) -> bool {
        self.blocks.iter().any(|b| {
            b.enforcers.uses(Uses::StoreApps) && (!b.lists.store_apps.is_empty() || b.free.iter().any(|f| !f.lists.store_apps.is_empty()))
        })
    }

    /// Whether a Store app (by package family name) is closed by any block that closes them. Never
    /// one that is part of Windows (`store_apps::is_protected`), whatever the lists say.
    pub fn blocks_store_app(&self, family: &str) -> bool {
        !crate::store_apps::is_protected(family)
            && self.judge(
                |b| b.enforcers.uses(Uses::StoreApps),
                |b| b.lists.store_apps.is_match(family),
                |lists| lists.store_apps.is_match(family),
            )
    }

    /// Whether any block closes windows by their title and has some titles (or to see in use).
    pub fn closes_titles(&self) -> bool {
        self.blocks
            .iter()
            .any(|b| b.enforcers.uses(Uses::Titles) && (!b.lists.titles.is_empty() || b.free.iter().any(|f| !f.lists.titles.is_empty())))
    }

    /// Whether a window with this title is closed by any block that closes windows by title.
    pub fn blocks_title(&self, title: &str) -> bool {
        self.judge(|b| b.enforcers.uses(Uses::Titles), |b| b.lists.titles.is_match(title), |lists| lists.titles.is_match(title))
    }

    /// Whether any block closes Obsidian when it shows web pages.
    pub fn closes_obsidian(&self) -> bool {
        self.blocks.iter().any(|b| b.enforcers.obsidian_web_viewer)
    }
}

/// One block list of a block, as it is started: its name, its lists and its enforcers.
pub struct Part {
    pub name: String,
    pub lists: Lists,
    pub enforcers: Enforcers,
}

impl Part {
    /// Whether it has anything to do: an enforcer that is on and has its list (`Enforcers::can_run`).
    pub fn can_run(&self) -> bool {
        self.enforcers.can_run(|uses| self.lists.has(uses))
    }
}

/// How a block is to be locked when it starts: its definition's lock (`lock::Lock`) with what it
/// refers to looked up (a lock condition's rule, a password's hash).
pub enum StartLock {
    None,
    /// For this long from now.
    For(Duration),
    /// Until this local date and time, read in the block's time zone.
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

/// How a running block is locked, as it runs and as it is saved (`state::Saved::lock`). A
/// duration or until lock is the moment it ends by then.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RunLock {
    None,
    /// Cannot be stopped, and ends at `at` (milliseconds since the Unix epoch, internet time), the block
    /// with it.
    Ends {
        at: u64,
    },
    /// Cannot be stopped while `condition` is active (a schedule is read in the block's time zone).
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

/// A part of a running block: its block list's name and what it enforces.
struct RunPart {
    name: String,
    enforced: Enforced,
    /// Each entry of its lists on its own, with the always-allowed sites, by its key: what a budget
    /// for each item watches.
    items: Vec<(String, Arc<Lists>)>,
    /// Only its always-allowed sites: what is left to enforce when everything of it is let through.
    exceptions: Arc<Lists>,
    /// Its lists without the items that are let through, for the ones that were (`remainder`).
    remaining: Option<(Vec<String>, Arc<Lists>)>,
}

impl RunPart {
    fn new(name: String, lists: Lists, enforcers: Enforcers) -> RunPart {
        let entries = lists.entries();
        let mut source = entries.clone();
        let mut items = Vec::new();
        for (prefix, field) in ITEMS {
            for entry in source.field(field).cloned().unwrap_or_default() {
                let mut one = Entries { allowed: entries.allowed.clone(), ..Entries::default() };
                if let Some(list) = one.field(field) {
                    list.push(entry.clone());
                }
                if let Ok(parsed) = Lists::parse(&one) {
                    items.push((format!("{prefix}:{entry}"), Arc::new(parsed)));
                }
            }
        }
        let only_allowed = Entries { allowed: entries.allowed, ..Entries::default() };
        let exceptions = Arc::new(Lists::parse(&only_allowed).unwrap_or_default());
        RunPart { name, enforced: Enforced::new(Arc::new(lists), enforcers), items, exceptions, remaining: None }
    }

    /// Its lists without the items `free`, as long as it is the same ones (parsed once). If they cannot
    /// be read, all of them: the block errs on the side of blocking.
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

struct Block {
    /// The definition it was started from.
    id: String,
    name: String,
    /// One per block list, each by its own enforcers and allowed sites.
    parts: Vec<RunPart>,
    lock: RunLock,
    /// When the block is active; `None` is always.
    condition: Option<Rule>,
    /// The time zone a schedule is read in: Windows' when the block started, kept until it ends,
    /// so changing the time zone does not move the schedule.
    zone: Zone,
    /// What lets it go quiet for a while, and what of that is used (`allowance.rs`).
    allowance: Option<Run>,
}

impl Block {
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
    /// (`Free`), so its use can be counted. Obsidian's enforcer has no list to watch, so it stays.
    fn enforced(&mut self, now: SystemTime) -> Vec<Enforced> {
        let Block { id, zone, allowance, parts, .. } = self;
        let Some(run) = allowance.as_ref() else { return parts.iter().map(|p| p.enforced.clone()).collect() };
        match run.scope() {
            None if run.everything_allowed(zone, now) => Vec::new(),
            None => parts.iter().map(|p| p.enforced.clone()).collect(),
            Some(Scope::Total) if !run.is_free(TOTAL) => parts.iter().map(|p| p.enforced.clone()).collect(),
            Some(Scope::Total) => parts
                .iter()
                .map(|p| Enforced {
                    lists: p.exceptions.clone(),
                    enforcers: p.enforced.enforcers,
                    free: vec![Free { block: id.clone(), key: TOTAL.to_string(), lists: p.enforced.lists.clone() }],
                })
                .collect(),
            Some(Scope::Each) => parts
                .iter_mut()
                .map(|p| {
                    let free: Vec<String> = p.items.iter().filter(|(key, _)| run.is_free(key)).map(|(key, _)| key.clone()).collect();
                    let lists = p.remainder(&free);
                    let watched = p.items.iter().filter(|(key, _)| free.contains(key)).map(|(key, lists)| Free {
                        block: id.clone(),
                        key: key.clone(),
                        lists: lists.clone(),
                    });
                    Enforced { lists, enforcers: p.enforced.enforcers, free: watched.collect() }
                })
                .collect(),
        }
    }

    /// Whether `key` is something this block's allowance counts: the block as a whole, or one of its items.
    fn counts(&self, key: &str) -> bool {
        match self.allowance.as_ref().and_then(Run::scope) {
            Some(Scope::Total) => key == TOTAL,
            Some(Scope::Each) => self.parts.iter().any(|p| p.items.iter().any(|(item, _)| item == key)),
            None => false,
        }
    }

    /// Whether its condition has the block active at `now`. If it cannot be known (the local time,
    /// the connected drives), it is: a block errs on the side of blocking.
    fn in_condition(&self, now: SystemTime) -> bool {
        self.condition.as_ref().is_none_or(|rule| rule.active(&self.zone, now))
    }

    /// Seconds until the condition turns the block on or off, if it has a time for that.
    fn condition_change(&self, now: SystemTime) -> Option<u64> {
        self.condition.as_ref()?.next_change(&self.zone, now)
    }

    /// When a duration or until lock ends, and the block with it.
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
            RunLock::Ends { .. } => {
                self.remaining(now).map(|left| format!("“{name}” is locked for the next {}", duration::countdown(left)))
            }
            // Like `in_condition`, locked if it cannot be known.
            RunLock::Condition { condition } => condition.active(&self.zone, now).then(|| {
                let until = self.lock_change(now).map(|s| format!(" for the next {}", duration::countdown(s))).unwrap_or_default();
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
                Some(left) => Some(format!("“{name}” can be stopped in {}", duration::countdown(left))),
                None => Some(format!("“{name}” is locked: asking to stop it starts a wait of {}", duration::countdown(*delay))),
            },
            RunLock::Text { .. } => Some(format!("“{name}” is locked until its text is typed in")),
            RunLock::Password { .. } => Some(format!("“{name}” is locked with a password")),
        }
    }
}

/// One block list of a running block, as the UI shows it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct RunningList {
    pub name: String,
    #[serde(flatten)]
    pub entries: Entries,
    pub enforcers: Enforcers,
}

/// One running block, as the UI shows it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Running {
    pub id: String,
    pub name: String,
    /// What it blocks and how, as it was started, so the UI can show what a running block does.
    pub lists: Vec<RunningList>,
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

impl Blocklist {
    /// The service's block list, over the real store in its data folder.
    pub fn new() -> Blocklist {
        Blocklist { inner: Arc::default(), store: state::store().map(Arc::new), clock: clock::global(), zone: None, boot: None }
    }

    /// A block list over a store of its own and a clock of its own, for other modules' tests.
    #[cfg(test)]
    pub fn with_store(store: Store) -> Blocklist {
        Blocklist {
            inner: Arc::default(),
            store: Some(Arc::new(store)),
            clock: Box::leak(Box::default()),
            zone: Some(Zone::fixed(0)),
            boot: Some("boot".into()),
        }
    }

    /// Starts a block from the definition `id`, unless that one is running: checked here rather
    /// than only in the UI, so a locked block cannot be swapped for a shorter or unlocked one.
    /// `condition` says when it is active (`None`: always), `lock` how it is locked (`lock.rs`).
    /// Other blocks are not affected.
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
        if inner.blocks.iter().any(|b| b.id == id) {
            return Err("This block is already running".into());
        }
        let zone = self.zone.clone().unwrap_or_else(Zone::current);
        let now = self.clock.now();
        let ends = |end: SystemTime| state::to_unix(end).map(|at| RunLock::Ends { at }).ok_or("Cannot tell the time");
        let lock = match lock {
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
        };
        let parts = parts.into_iter().map(|p| RunPart::new(p.name, p.lists, p.enforcers)).collect();
        let allowance = allowance.map(|terms| Run::new(terms, now));
        let new = Block { id: id.to_string(), name: name.to_string(), parts, lock, condition, zone, allowance };
        // Written before the block goes live, so a block cannot be missed after a restart. A
        // block that cannot be written would not survive the service ending, so it does not start.
        let mut all: Vec<&Block> = inner.blocks.iter().collect();
        all.push(&new);
        self.save(&all).map_err(|e| format!("Cannot save the block, so it would not survive a restart: {e}"))?;
        inner.blocks.push(new);
        inner.saved_at = Instant::now();
        Ok(())
    }

    /// Stops the block `id` unless it is locked right now. `answered`: the right password of its
    /// password lock, or the right text of its text lock, was given (`password_matches`,
    /// `text_matches`), which is what lets those stop.
    pub fn stop(&self, id: &str, answered: bool) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let Some(at) = inner.blocks.iter().position(|b| b.id == id) else {
            return Err("This block is not running".into());
        };
        let block = &inner.blocks[at];
        let password_locked = block.password_hash().is_some();
        let text_locked = block.lock_text().is_some();
        if !(answered && (password_locked || text_locked)) {
            if let Some(why) = block.locked(self.clock.now()) {
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
        inner.blocks.remove(at);
        self.save_all(&mut inner);
        Ok(())
    }

    /// Starts the wait of a block locked with a delay; asking again keeps the wait that runs.
    pub fn ask_to_stop(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = state::to_unix(self.clock.now()).ok_or("Cannot tell the time")?;
        let block = inner.blocks.iter_mut().find(|b| b.id == id).ok_or("This block is not running")?;
        match &mut block.lock {
            RunLock::Delay { asked, .. } => {
                asked.get_or_insert(now);
            }
            _ => return Err("This block is not locked with a delay".into()),
        }
        self.save_all(&mut inner);
        Ok(())
    }

    /// Ends that wait, also when it is over: locked again.
    pub fn abort_stop(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let block = inner.blocks.iter_mut().find(|b| b.id == id).ok_or("This block is not running")?;
        match &mut block.lock {
            RunLock::Delay { asked, .. } => *asked = None,
            _ => return Err("This block is not locked with a delay".into()),
        }
        self.save_all(&mut inner);
        Ok(())
    }

    /// Picks up the blocks left behind by an earlier run, so restarting Windows does not end them.
    /// A block whose time is up while the service was not running is not resumed.
    pub fn restore(&self) {
        let Some(store) = &self.store else { return };
        let saved = store.load();
        let mut blocks = Vec::new();
        for saved in &saved {
            // Until internet time is in, the clock carries on from where the blocks last were,
            // rather than from a Windows clock that may have been set forward meanwhile.
            if let Some(seen) = saved.seen {
                self.clock.at_least(state::from_unix(seen));
            }
        }
        for saved in saved {
            // A block list we cannot read is dropped rather than resumed as something unknown.
            let parts: Result<Vec<RunPart>, String> =
                saved.parts.into_iter().map(|p| Ok(RunPart::new(p.name, Lists::parse(&p.entries)?, p.enforcers))).collect();
            let condition = saved.condition.clone().map(Rule::checked).transpose();
            let (Ok(parts), Ok(condition)) = (parts, condition) else {
                eprintln!("block: dropping an unreadable saved block");
                continue;
            };
            let ended = match &saved.lock {
                RunLock::Ends { at } => self.clock.now() >= state::from_unix(*at),
                // Locked until Windows restarts, and it has: the block ended with it. If this
                // start of Windows cannot be told, it errs on the side of blocking.
                RunLock::UntilRestart { boot } => self.boot().is_some_and(|now| now != *boot),
                _ => false,
            };
            if ended {
                continue;
            }
            blocks.push(Block {
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
        inner.blocks = blocks;
        // What was dropped is cleaned up, rather than being re-checked on every run.
        self.save_all(&mut inner);
    }

    /// What is to be enforced right now: every block list of every running block that its condition
    /// has active. Ends timed blocks whose time is up. `None` when nothing is to be enforced.
    pub fn active(&self) -> Option<Active> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        // Keeps the time on disk moving, for a restart while offline (`Saved::seen`), and what a
        // budget has used.
        if inner.blocks.iter().any(|b| b.ends().is_some() || b.keeps_count()) && inner.saved_at.elapsed() >= CHECKPOINT {
            self.save_all(&mut inner);
        }
        let now = self.clock.now();
        for block in inner.blocks.iter_mut() {
            block.advance(now);
        }
        let blocks: Vec<Enforced> = inner.blocks.iter_mut().filter(|b| b.in_condition(now)).flat_map(|b| b.enforced(now)).collect();
        (!blocks.is_empty()).then_some(Active { blocks, used: Usage::default() })
    }

    /// The app has been told what to enforce: what use it reports from now on counts from here
    /// (`charge`). Only its own question does this, not the service looking for itself.
    pub fn polled(&self) {
        self.inner.locked().polled = Instant::now();
    }

    /// Counts what the app has seen in use (block id and key, `Free`) against the allowances of the
    /// blocks, for the time since the app last asked what to enforce (`MAX_GAP` at most). Only what
    /// a block's allowance counts and its condition has active now; anything else is ignored.
    pub fn charge(&self, used: &[(String, String)]) {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let millis = u64::try_from(inner.polled.elapsed().min(MAX_GAP).as_millis()).unwrap_or(0);
        let now = self.clock.now();
        for block in inner.blocks.iter_mut() {
            block.advance(now);
            let active = block.in_condition(now);
            let keys: Vec<&String> = used.iter().filter(|(id, key)| *id == block.id && block.counts(key)).map(|(_, key)| key).collect();
            if !active || keys.is_empty() {
                continue;
            }
            let keys: Vec<String> = keys.into_iter().cloned().collect();
            if let Some(run) = &mut block.allowance {
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

    /// Whether any block runs, also one that its condition has inactive right now: it is still
    /// there and maybe locked, so the app is kept running and the exe is not replaced.
    pub fn is_running(&self) -> bool {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        !inner.blocks.is_empty()
    }

    /// The ids of the running blocks.
    pub fn running_ids(&self) -> Vec<String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        inner.blocks.iter().map(|b| b.id.clone()).collect()
    }

    /// The ids of the running blocks that cannot be stopped right now (`Block::locked`): what a
    /// block list or condition they use may only be made stricter by (`handler::edit_shared`).
    pub fn locked_ids(&self) -> Vec<String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        inner.blocks.iter().filter(|b| b.locked(now).is_some()).map(|b| b.id.clone()).collect()
    }

    /// Gives the running block `id` its name, block lists and conditions as its definition has them
    /// now, after it or one of them was changed: all of them, its lock condition's rule too. Its
    /// lock (but for a condition lock, which follows its condition), its time zone and what else it started with stay. Whether the change may be made is
    /// the handler's to judge (`handler::edit_shared`, `edit_block`); nothing happens if the block
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
        let Some(block) = inner.blocks.iter_mut().find(|b| b.id == id) else { return };
        block.name = name.to_string();
        block.parts = parts.into_iter().map(|p| RunPart::new(p.name, p.lists, p.enforcers)).collect();
        block.condition = condition;
        // What is used of an allowance stays when it is changed; with another one added or taken
        // away it starts or ends.
        block.allowance = match (allowance, block.allowance.take()) {
            (Some(terms), Some(run)) => Some(run.retermed(terms)),
            (Some(terms), None) => Some(Run::new(terms, now)),
            (None, _) => None,
        };
        // A condition lock follows its condition: its rule, or no lock once the condition is deleted
        // (it was free, or the change would not have been made).
        if matches!(block.lock, RunLock::Condition { .. }) {
            block.lock = lock_rule.map_or(RunLock::None, |condition| RunLock::Condition { condition });
        }
        self.save_all(&mut inner);
    }

    /// Whether `password` is the password of the running block `id`'s password lock.
    /// Whether `text` is exactly the text of the running block `id`'s text lock (line breaks
    /// whichever way they were typed).
    pub fn text_matches(&self, id: &str, text: &str) -> bool {
        let inner = self.inner.locked();
        let same = |a: &str, b: &str| a.replace("\r\n", "\n") == b.replace("\r\n", "\n");
        inner.blocks.iter().find(|b| b.id == id).and_then(|b| b.lock_text()).is_some_and(|expected| same(expected, text))
    }

    pub fn password_matches(&self, id: &str, password: &str) -> bool {
        let inner = self.inner.locked();
        let hash = inner.blocks.iter().find(|b| b.id == id).and_then(|b| b.password_hash().map(str::to_string));
        // The hash is computed without holding the lock: it takes a moment.
        drop(inner);
        hash.is_some_and(|hash| unlock::check_hash(password, &hash))
    }

    /// Whether the block `id` runs.
    #[cfg(test)]
    pub fn runs(&self, id: &str) -> bool {
        self.running_ids().iter().any(|running| running == id)
    }

    /// Why some running block cannot be stopped right now, if one cannot: then neither may the
    /// service be uninstalled.
    pub fn locked(&self) -> Option<String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        inner.blocks.iter().find_map(|b| b.locked(now))
    }

    pub fn status(&self) -> Status {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        for block in inner.blocks.iter_mut() {
            block.advance(now);
        }
        let running = inner
            .blocks
            .iter()
            .map(|b| Running {
                id: b.id.clone(),
                name: b.name.clone(),
                lists: b
                    .parts
                    .iter()
                    .map(|p| RunningList { name: p.name.clone(), entries: p.enforced.lists.entries(), enforcers: p.enforced.enforcers })
                    .collect(),
                remaining: b.remaining(now),
                lock_condition: b.lock_rule().cloned(),
                until_restart: matches!(b.lock, RunLock::UntilRestart { .. }),
                lock_text: b.lock_text().map(str::to_string),
                stop_delay: b.stop_delay(),
                stop_delay_words: b.stop_delay().map(duration::countdown),
                stop_remaining: b.stop_remaining(now),
                stop_remaining_words: b.stop_remaining(now).map(duration::countdown),
                password: b.password_hash().is_some(),
                locked: b.locked(now).is_some(),
                condition: b.condition.clone(),
                enforcing: b.in_condition(now),
                condition_change: b.condition_change(now),
                remaining_words: b.remaining(now).map(duration::countdown),
                condition_change_words: b.condition_change(now).map(duration::countdown),
                lock_change: b.lock_change(now),
                lock_change_words: b.lock_change(now).map(duration::countdown),
                allowance: b.allowance.as_ref().map(|run| run.status(&b.zone, now)),
            })
            .collect();
        Status { running, clock_synced: self.clock.synced() }
    }

    /// Ends every block whatever its lock, and forgets them. Only for unlocking
    /// (`handler::unlock`): everything else goes through `stop`, which keeps a locked block.
    pub fn reset(&self) {
        let mut inner = self.inner.locked();
        inner.blocks.clear();
        self.save_all(&mut inner);
    }

    fn boot(&self) -> Option<String> {
        self.boot.clone().or_else(crate::boot::id)
    }

    /// Removes blocks whose duration or until lock is up; they end by themselves and leave nothing to resume.
    fn expire(&self, inner: &mut Inner) {
        let now = self.clock.now();
        let before = inner.blocks.len();
        inner.blocks.retain(|b| b.ends().is_none_or(|end| now < end));
        if inner.blocks.len() != before {
            self.save_all(inner);
        }
    }

    fn save_all(&self, inner: &mut Inner) {
        inner.saved_at = Instant::now();
        if let Err(e) = self.save(&inner.blocks.iter().collect::<Vec<_>>()) {
            eprintln!("block: {e}");
        }
    }

    fn save(&self, blocks: &[&Block]) -> Result<(), String> {
        let Some(store) = &self.store else { return Ok(()) };
        let seen = state::to_unix(self.clock.now());
        let saved: Vec<state::Saved> = blocks
            .iter()
            .map(|b| state::Saved {
                id: b.id.clone(),
                name: b.name.clone(),
                parts: b
                    .parts
                    .iter()
                    .map(|p| state::SavedPart {
                        name: p.name.clone(),
                        entries: p.enforced.lists.entries(),
                        enforcers: p.enforced.enforcers,
                    })
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

    /// A clock of its own per block list, so one test restoring a block does not move another's.
    fn test_clock() -> &'static Clock {
        Box::leak(Box::default())
    }

    /// A block list over a store of its own, so the tests never touch the real one.
    /// `wiped` starts from an empty store, for a test that starts a block of its own.
    fn blocklist_of(name: &str, wiped: bool) -> Blocklist {
        let dir = std::env::temp_dir().join(format!("block-{}-{name}", std::process::id()));
        if wiped {
            let _ = std::fs::remove_dir_all(&dir);
        }
        Blocklist {
            inner: Arc::default(),
            clock: test_clock(),
            zone: Some(Zone::fixed(0)),
            store: Some(Arc::new(Store::new(dir.join("state.json")))),
            boot: Some("boot".into()),
        }
    }

    fn blocklist(name: &str) -> Blocklist {
        blocklist_of(name, true)
    }

    /// The same store, as a second run of the service would find it.
    fn reopened(name: &str) -> Blocklist {
        blocklist_of(name, false)
    }

    /// Only sites to block; the program list is empty.
    fn sites(entries: &[&str]) -> Lists {
        Lists { sites: FilterList::parse(entries).unwrap(), ..Lists::default() }
    }

    fn start(list: &Blocklist, id: &str, lists: Lists, duration: Option<Duration>) -> Result<(), String> {
        list.start(
            id,
            &format!("Block {id}"),
            vec![part(lists, Enforcers::default())],
            None,
            duration.map_or(StartLock::None, StartLock::For),
        )
    }

    /// One block list of a block.
    fn part(lists: Lists, enforcers: Enforcers) -> Part {
        Part { name: "List".into(), lists, enforcers }
    }

    fn running(list: &Blocklist, id: &str) -> Running {
        list.status().running.into_iter().find(|r| r.id == id).expect("not running")
    }

    #[test]
    fn timed_block_cannot_stop_early_and_ends_by_itself() {
        let list = blocklist("timed");
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
        let list = blocklist("delay");
        let begin =
            || list.start("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, StartLock::Delay(Duration::from_secs(1)));
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
    fn a_running_block_cannot_be_started_again() {
        let list = blocklist("replace");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(3600))).unwrap();
        // Not as a shorter one, nor as an untimed one that could then be stopped.
        assert!(start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(1))).is_err());
        assert!(start(&list, "a", sites(&["a.com"]), None).is_err());
        assert!(running(&list, "a").remaining.is_some_and(|left| left > 3500));

        // An untimed block is stopped first, then it can start again, with new lists.
        let untimed = blocklist("replace-untimed");
        start(&untimed, "a", sites(&["a.com"]), None).unwrap();
        assert!(start(&untimed, "a", sites(&["b.com"]), None).is_err());
        untimed.stop("a", false).unwrap();
        start(&untimed, "a", sites(&["b.com"]), None).unwrap();
        assert_eq!(running(&untimed, "a").lists[0].entries.filters, vec!["b.com"]);
    }

    #[test]
    fn blocks_run_side_by_side_and_end_on_their_own() {
        let list = blocklist("several");
        start(&list, "timed", sites(&["a.com"]), Some(Duration::from_millis(50))).unwrap();
        start(&list, "untimed", sites(&["b.com"]), None).unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.blocks.len(), 2);
        assert!(active.blocks_page("https://a.com/", false) && active.blocks_page("https://b.com/", false));
        assert!(list.locked().is_some_and(|why| why.contains("Block timed")));

        // The timed one ends by itself; the other goes on.
        std::thread::sleep(Duration::from_millis(60));
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/", false) && active.blocks_page("https://b.com/", false));
        assert!(!list.runs("timed") && list.runs("untimed"));
        assert_eq!(list.locked(), None);
        // Stopping one leaves the others.
        start(&list, "other", sites(&["c.com"]), None).unwrap();
        list.stop("untimed", false).unwrap();
        assert_eq!(list.status().running.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["other"]);
    }

    #[test]
    fn each_block_goes_by_its_own_enforcers_and_allowed_sites() {
        let list = blocklist("own-rules");
        let tabs_only = Enforcers {
            chromium_tabs: true,
            chromium_split_view: false,
            obsidian_web_viewer: false,
            exe_names: false,
            chromium_app_windows: false,
            firefox_tabs: false,
            firefox_split_view: false,
            store_apps: false,
            window_titles: false,
        };
        let split_only = Enforcers {
            chromium_tabs: false,
            chromium_split_view: true,
            obsidian_web_viewer: false,
            exe_names: false,
            chromium_app_windows: false,
            firefox_tabs: false,
            firefox_split_view: false,
            store_apps: false,
            window_titles: false,
        };
        let everything_but_google =
            Lists { sites: FilterList::parse(&["*"]).unwrap(), allowed: FilterList::parse(&["google.com"]).unwrap(), ..Lists::default() };
        list.start("tabs", "Tabs", vec![part(everything_but_google, tabs_only)], None, StartLock::None).unwrap();
        list.start("split", "Split", vec![part(sites(&["google.com"]), split_only)], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        // One block's allowed site is not an exception to another block's list.
        assert!(!active.blocks_page("https://google.com/", false));
        assert!(active.blocks_page("https://google.com/", true));
        assert!(active.blocks_page("https://youtube.com/", false));
        assert!(!active.blocks_page("https://youtube.com/", true));
        assert!(active.watches_sites() && !active.closes_programs() && !active.closes_obsidian());
    }

    #[test]
    fn a_block_that_cannot_be_saved_does_not_start() {
        // A store whose folder is a file, so the state file can never be written.
        let dir = std::env::temp_dir().join(format!("block-{}-unsaved", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("not-a-folder"), "").unwrap();
        let list = Blocklist {
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
    fn blocks_outlive_the_service_that_started_them() {
        let list = blocklist("resume");
        start(&list, "a", sites(&["https://tweakers.net/", "b.com"]), Some(Duration::from_secs(3600))).unwrap();
        start(&list, "b", sites(&["c.com"]), None).unwrap();
        assert_eq!(running(&list, "a").lists[0].entries.filters, vec!["tweakers.net", "b.com"]);

        // A new run over the same store: both are still going, with their names.
        let resumed = reopened("resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.blocks_page("https://tweakers.net/nieuws", false) && active.blocks_page("https://c.com/", false));
        let a = running(&resumed, "a");
        assert_eq!(a.name, "Block a");
        assert!(a.remaining.is_some_and(|left| (3595..=3600).contains(&left)));
        assert!(resumed.stop("a", false).is_err());
        assert_eq!(running(&resumed, "b").remaining, None);
        assert!(resumed.stop("b", false).is_ok());
    }

    /// A block that blocks `block` and always allows `allow`.
    fn with_allowed(block: &[&str], allow: &[&str]) -> Active {
        let list = Blocklist::with_store(Store::new(
            std::env::temp_dir()
                .join(format!("block-{}-allowed-{}", std::process::id(), block.join("_").len() + allow.len()))
                .join("state.json"),
        ));
        let lists = Lists { sites: FilterList::parse(block).unwrap(), allowed: FilterList::parse(allow).unwrap(), ..Lists::default() };
        // `Enforcers::default()` has an enforcer that needs no list, so an empty block list starts too.
        start(&list, "a", lists, None).unwrap();
        list.active().unwrap()
    }

    fn blocks_site(active: &Active, url: &str) -> bool {
        active.blocks_page(url, false)
    }

    #[test]
    fn allowed_sites_win_over_the_block_list() {
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

        // A specific block and an exception inside it: the exception wins.
        let active = with_allowed(&["youtube.com"], &["youtube.com/watch*"]);
        assert!(blocks_site(&active, "https://youtube.com/"));
        assert!(blocks_site(&active, "https://youtube.com/shorts/abc"));
        assert!(!blocks_site(&active, "https://youtube.com/watch?v=1"));

        // Allowed everything is allowed everything, whatever the block list says.
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
        let list = blocklist("allowed-resume");
        let lists = Lists {
            sites: FilterList::parse(&["*"]).unwrap(),
            allowed: FilterList::parse(&["Google.com", "*.wikipedia.org"]).unwrap(),
            ..Lists::default()
        };
        start(&list, "a", lists, Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(running(&list, "a").lists[0].entries.allowed, vec!["google.com", "*.wikipedia.org"]);

        let resumed = reopened("allowed-resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(blocks_site(&active, "https://youtube.com/") && !blocks_site(&active, "https://google.com/"));
        assert_eq!(running(&resumed, "a").lists[0].entries.allowed, vec!["google.com", "*.wikipedia.org"]);
    }

    #[test]
    fn a_resumed_block_keeps_its_programs() {
        let list = blocklist("programs");
        let lists = Lists { programs: ProgramList::parse(&["Steam*", "game"]).unwrap(), ..Lists::default() };
        start(&list, "a", lists, Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(running(&list, "a").lists[0].entries.programs, vec!["steam*.exe", "game.exe"]);

        // A new run over the same store: the programs are still on the block.
        let resumed = reopened("programs");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.closes_programs());
        assert!(active.blocks_program("steamwebhelper.exe") && active.blocks_program("GAME.EXE"));
        assert!(!active.blocks_program("chrome.exe"));
        assert_eq!(running(&resumed, "a").lists[0].entries.programs, vec!["steam*.exe", "game.exe"]);
    }

    #[test]
    fn a_resumed_block_keeps_its_enforcers() {
        let only_obsidian = Enforcers {
            chromium_tabs: false,
            chromium_split_view: false,
            obsidian_web_viewer: true,
            exe_names: false,
            chromium_app_windows: false,
            firefox_tabs: false,
            firefox_split_view: false,
            store_apps: false,
            window_titles: false,
        };
        let list = blocklist("enforcers");
        list.start("a", "A", vec![part(sites(&[]), only_obsidian)], None, StartLock::For(Duration::from_secs(3600))).unwrap();
        let resumed = reopened("enforcers");
        resumed.restore();
        assert_eq!(resumed.active().map(|active| active.blocks[0].enforcers), Some(only_obsidian));
        assert_eq!(running(&resumed, "a").lists[0].enforcers, only_obsidian);
    }

    /// 2030-01-07 00:00:00 UTC, a Monday; the tests' time zone is UTC.
    const MONDAY: u64 = 1_893_974_400;

    /// `MONDAY` plus `seconds`, as a time.
    fn monday(seconds: u64) -> SystemTime {
        state::from_unix((MONDAY + seconds) * 1000)
    }

    #[test]
    fn outside_its_schedule_a_block_runs_and_stays_locked_but_enforces_nothing() {
        let list = blocklist("schedule");
        // Monday 16:00, office hours until 17:00, locked for two hours: the user's own example.
        list.clock.at_least(monday(16 * 3600));
        let office = weekly("mon-fri 08:00-17:00");
        list.start(
            "a",
            "A",
            vec![part(sites(&["a.com"]), Enforcers::default())],
            office.clone(),
            StartLock::For(Duration::from_secs(2 * 3600)),
        )
        .unwrap();
        // An always-active block next to it, to show the other one going quiet on its own.
        start(&list, "b", sites(&["b.com"]), None).unwrap();
        assert!(list.active().unwrap().blocks_page("https://a.com/", false));
        let a = running(&list, "a");
        assert!(a.enforcing && a.condition == office);
        // The test runs on a real clock as well, so a few seconds may pass along the way.
        assert!(a.condition_change.is_some_and(|left| (3590..=3600).contains(&left)));
        assert!(running(&list, "b").enforcing && running(&list, "b").condition_change.is_none());

        // 17:00: the schedule has it inactive, so nothing of it is enforced; it still runs and is
        // still locked.
        list.clock.at_least(monday(17 * 3600));
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/", false) && active.blocks_page("https://b.com/", false));
        assert!(list.runs("a") && list.stop("a", false).is_err());
        let a = running(&list, "a");
        assert!(!a.enforcing);
        // In words too, for the window and the CLI.
        assert!(
            a.remaining_words.as_deref().is_some_and(|words| words.starts_with("1 hour") || words.starts_with("59 minutes")),
            "{:?}",
            a.remaining_words
        );
        assert!(a.condition_change_words.as_deref().is_some_and(|words| words.starts_with("14 hours") || words.starts_with("15 hours")));
        // Rounded up, and the start took a moment of the clock: a little over the hour.
        assert!(a.remaining.is_some_and(|left| (3590..=3610).contains(&left)));
        // Active again on Tuesday at 08:00.
        assert!(a.condition_change.is_some_and(|left| (15 * 3600 - 10..=15 * 3600).contains(&left)));
        // With only it running, nothing is enforced, but a block still runs (the app is kept alive).
        list.stop("b", false).unwrap();
        assert!(list.active().is_none() && list.is_running());

        // Just after 18:00 (the lock started a moment after 16:00): the lock is over, and with it
        // the block.
        list.clock.at_least(monday(18 * 3600 + 30));
        assert!(!list.runs("a") && !list.is_running());
    }

    #[test]
    fn a_lock_schedule_locks_only_while_it_is_active() {
        let list = blocklist("lock-schedule");
        // Monday 16:00. Active all day, locked during office hours, until 17:00.
        list.clock.at_least(monday(16 * 3600));
        let office = weekly("mon-fri 08:00-17:00");
        let locks = || StartLock::Condition(office.clone().unwrap());
        list.start("a", "Work", vec![part(sites(&["a.com"]), Enforcers::default())], None, locks()).unwrap();
        let why = list.stop("a", false).unwrap_err();
        assert!(why.contains("locked by its lock schedule for the next 1 hour") || why.contains("for the next 59 minutes"), "{why}");
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
        list.start("a", "Work", vec![part(sites(&["a.com"]), Enforcers::default())], None, locks()).unwrap();
        let resumed = reopened("lock-schedule");
        resumed.clock.at_least(monday(7 * 86400 + 9 * 3600));
        resumed.restore();
        assert!(resumed.stop("a", false).is_err() && running(&resumed, "a").lock_condition == office);
    }

    #[test]
    fn a_password_lock_stops_only_with_its_password() {
        let list = blocklist("password");
        let locks = StartLock::Password(unlock::new_hash("partner").unwrap());
        list.start("a", "Games", vec![part(sites(&["a.com"]), Enforcers::default())], None, locks).unwrap();
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
        let list = blocklist("text");
        let locks = StartLock::Text("Write this\nout by hand".into());
        list.start("a", "Games", vec![part(sites(&["a.com"]), Enforcers::default())], None, locks).unwrap();
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
    fn a_block_of_several_lists_enforces_each_by_its_own_rules() {
        let list = blocklist("parts");
        let tabs_only = Enforcers {
            chromium_tabs: true,
            chromium_split_view: false,
            obsidian_web_viewer: false,
            exe_names: false,
            chromium_app_windows: false,
            firefox_tabs: false,
            firefox_split_view: false,
            store_apps: false,
            window_titles: false,
        };
        let programs_only = Enforcers {
            chromium_tabs: false,
            chromium_split_view: false,
            obsidian_web_viewer: false,
            exe_names: true,
            chromium_app_windows: false,
            firefox_tabs: false,
            firefox_split_view: false,
            store_apps: false,
            window_titles: false,
        };
        let games = Lists { programs: ProgramList::parse(&["steam"]).unwrap(), ..Lists::default() };
        let social = Part { name: "Social".into(), lists: sites(&["youtube.com"]), enforcers: tabs_only };
        list.start(
            "a",
            "Evening",
            vec![social, Part { name: "Games".into(), lists: games, enforcers: programs_only }],
            None,
            StartLock::None,
        )
        .unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.blocks.len(), 2);
        assert!(active.blocks_page("https://youtube.com/", false) && active.blocks_program("steam.exe"));
        let lists = running(&list, "a").lists;
        assert_eq!(lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Social", "Games"]);
        // And again after a restart.
        let resumed = reopened("parts");
        resumed.restore();
        assert_eq!(running(&resumed, "a").lists, lists);
    }

    #[test]
    fn an_until_lock_ends_at_its_local_time() {
        let list = blocklist("until");
        // Monday 16:00 UTC; the test zone is UTC, so 18:00 is two hours away.
        list.clock.at_least(monday(16 * 3600));
        let at = |text: &str| StartLock::Until(DateTime::parse(text).unwrap());
        let why = list.start("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, at("2030-01-07 12:00")).unwrap_err();
        assert!(why.contains("has passed"), "{why}");
        list.start("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, at("2030-01-07 18:00")).unwrap();
        assert!(list.stop("a", false).is_err());
        assert!(running(&list, "a").remaining.is_some_and(|left| (7190..=7200).contains(&left)));
        list.clock.at_least(monday(18 * 3600 + 1));
        assert!(!list.runs("a"));
    }

    #[test]
    fn a_lock_until_restart_holds_until_windows_restarts() {
        let list = blocklist("until-restart");
        list.start("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, StartLock::UntilRestart).unwrap();
        let why = list.stop("a", false).unwrap_err();
        assert!(why.contains("until Windows restarts"), "{why}");
        assert!(running(&list, "a").until_restart && list.locked().is_some());
        // The service started again without a restart of Windows: still there, still locked.
        let resumed = reopened("until-restart");
        resumed.restore();
        assert!(resumed.stop("a", false).is_err());
        // After a restart the block has ended.
        let mut restarted = reopened("until-restart");
        restarted.boot = Some("next boot".into());
        restarted.restore();
        assert!(!restarted.is_running());
    }

    #[test]
    fn a_resumed_block_keeps_its_schedule_and_time_zone() {
        let list = blocklist("schedule-resume");
        list.clock.at_least(monday(12 * 3600));
        let weekend = weekly("weekends");
        list.start("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], weekend.clone(), StartLock::None).unwrap();
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
    fn a_block_whose_time_is_up_is_not_resumed() {
        let list = blocklist("expired");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(3600))).unwrap();
        // Rewritten as though it had been left behind by a run that ended an hour ago, next to one
        // that still runs.
        let store = list.store.clone().unwrap();
        let part = state::SavedPart {
            name: "List".into(),
            entries: Entries { filters: vec!["a.com".into()], ..Entries::default() },
            enforcers: Enforcers::default(),
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
    fn ending_the_blocks_clears_them() {
        let list = blocklist("cleared");
        let store = list.store.clone().unwrap();
        start(&list, "a", sites(&["a.com"]), None).unwrap();
        assert_eq!(store.load().len(), 1);
        list.stop("a", false).unwrap();
        assert!(store.load().is_empty());

        // A timed block that ends by itself is cleared too, so nothing is left to resume.
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

    fn budget(allowed: &str, scope: Scope) -> Terms {
        Terms::Budget { allowed: allowed.into(), scope, reset: allowance::Reset::Never }
    }

    #[test]
    fn a_budget_for_the_block_lets_it_through_until_it_is_used_up() {
        let list = blocklist("budget-total");
        list.start_with(
            "a",
            "A",
            vec![part(sites(&["a.com"]), Enforcers::default())],
            None,
            Some(budget("1s", Scope::Total)),
            StartLock::None,
        )
        .unwrap();
        // With time left the site is let through and its use is noted, to be counted.
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/x", false));
        assert_eq!(active.used.take(), vec![("a".to_string(), TOTAL.to_string())]);
        assert!(running(&list, "a").allowance.is_some_and(|status| status.open && status.left.is_some_and(|left| left > 0)));
        // What is reported counts from the last time the app asked; ignored: another block, another key.
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("other".to_string(), TOTAL.to_string()), ("a".to_string(), "site:a.com".to_string())]);
        assert!(!list.active().unwrap().blocks_page("https://a.com/x", false));
        list.charge(&[("a".to_string(), TOTAL.to_string())]);
        // Used up: closed again.
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://a.com/x", false));
        assert!(running(&list, "a").allowance.is_some_and(|status| !status.open && status.left == Some(0)));
    }

    #[test]
    fn a_budget_for_each_item_counts_each_on_its_own() {
        let list = blocklist("budget-each");
        list.start_with(
            "a",
            "A",
            vec![part(sites(&["a.com", "b.com"]), Enforcers::default())],
            None,
            Some(budget("1s", Scope::Each)),
            StartLock::None,
        )
        .unwrap();
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/", false) && !active.blocks_page("https://b.com/", false));
        assert_eq!(active.used.take().len(), 2);
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("a".to_string(), "site:a.com".to_string())]);
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://a.com/", false), "a.com is used up");
        assert!(!active.blocks_page("https://b.com/", false), "b.com has all its time");
        assert_eq!(active.used.take(), vec![("a".to_string(), "site:b.com".to_string())]);
        let items = running(&list, "a").allowance.unwrap().items;
        assert_eq!(items.iter().map(|i| (i.item.as_str(), i.left)).collect::<Vec<_>>(), vec![("a.com", 0)]);
    }

    #[test]
    fn a_cycle_that_is_blocking_enforces_and_what_is_used_survives_a_restart() {
        let list = blocklist("cycle");
        let cycle = Terms::Pomodoro { blocked: "1h".into(), allowed: "10m".into(), anchor: allowance::Anchor::Start };
        list.start_with("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, Some(cycle), StartLock::None).unwrap();
        assert!(list.active().unwrap().blocks_page("https://a.com/", false), "blocked first");
        assert!(running(&list, "a").allowance.is_some_and(|status| !status.open && status.change.is_some()));

        let list = blocklist("budget-kept");
        let each = budget("1m", Scope::Total);
        list.start_with("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, Some(each), StartLock::None).unwrap();
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[("a".to_string(), TOTAL.to_string())]);
        // Written to disk with the next save, then read back as a second run finds it.
        list.refresh("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, None, Some(budget("1m", Scope::Total)));
        let resumed = reopened("budget-kept");
        resumed.restore();
        let left = running(&resumed, "a").allowance.unwrap().left.unwrap();
        assert!(left < 60, "what was used is kept: {left}");
        // Taking the allowance away ends it.
        resumed.refresh("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, None, None);
        assert!(running(&resumed, "a").allowance.is_none());
    }

    /// The lists of these entries, checked.
    fn lists_of(entries: Entries) -> Lists {
        Lists::parse(&entries).unwrap()
    }

    fn strings(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|e| e.to_string()).collect()
    }

    /// Only these enforcers (by `Enforcers::all`'s field names) turned on.
    fn only(on: &[&str]) -> Enforcers {
        let mut fields = serde_json::to_value(Enforcers::default()).unwrap();
        for (name, value) in fields.as_object_mut().unwrap() {
            *value = serde_json::Value::Bool(on.contains(&name.as_str()));
        }
        serde_json::from_value(fields).unwrap()
    }

    #[test]
    fn each_enforcer_answers_for_its_own_kind_of_window() {
        let list = blocklist("kinds");
        let lists = lists_of(Entries {
            filters: strings(&["a.com"]),
            store_apps: strings(&["Microsoft.Xbox*", "Microsoft.Windows.*"]),
            titles: strings(&["*Chat*"]),
            ..Entries::default()
        });
        let enforcers = only(&["chromium_app_windows", "firefox_split_view", "store_apps", "window_titles"]);
        list.start("a", "A", vec![part(lists, enforcers)], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        // Sites: only where an enforcer for it is on.
        assert!(active.blocks_app_window("https://a.com/x") && !active.blocks_app_window("https://b.com/"));
        assert!(active.blocks_firefox_page("https://a.com/", true) && !active.blocks_firefox_page("https://a.com/", false));
        assert!(!active.blocks_page("https://a.com/", false) && !active.blocks_page("https://a.com/", true));
        assert!(active.watches_sites());
        // Store apps, never one of Windows' own whatever the list says.
        assert!(active.closes_store_apps());
        assert!(active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe"));
        assert!(!active.blocks_store_app("Microsoft.Windows.Photos_8wekyb3d8bbwe"));
        assert!(!active.blocks_store_app("Microsoft.WindowsCalculator_8wekyb3d8bbwe"));
        // Windows by their title.
        assert!(active.closes_titles() && active.blocks_title("Team chat - Browser") && !active.blocks_title("Notes"));
        // Nothing else.
        assert!(!active.closes_programs() && !active.closes_obsidian() && !active.blocks_program("chat.exe"));
        assert!(active.used.take().is_empty());

        // Without those enforcers, nothing of it is closed.
        let list = blocklist("kinds-off");
        let lists = lists_of(Entries { store_apps: strings(&["Microsoft.Xbox*"]), titles: strings(&["*Chat*"]), ..Entries::default() });
        list.start("a", "A", vec![part(lists, only(&["exe_names", "obsidian_web_viewer"]))], None, StartLock::None).unwrap();
        let active = list.active().unwrap();
        assert!(!active.closes_store_apps() && !active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe"));
        assert!(!active.closes_titles() && !active.blocks_title("Team chat"));
        assert!(!active.closes_programs() && active.closes_obsidian());
    }

    #[test]
    fn a_budget_for_each_item_watches_store_apps_and_titles_too() {
        let list = blocklist("budget-kinds");
        let lists = lists_of(Entries {
            store_apps: strings(&["Microsoft.Xbox*"]),
            titles: strings(&["*Chat*"]),
            programs: strings(&["game.exe"]),
            ..Entries::default()
        });
        list.start_with("a", "A", vec![part(lists, Enforcers::default())], None, Some(budget("1s", Scope::Each)), StartLock::None).unwrap();
        let active = list.active().unwrap();
        // Let through, and what is seen is noted under each item's key.
        assert!(active.closes_store_apps() && active.closes_titles() && active.closes_programs());
        assert!(!active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe"));
        assert!(!active.blocks_title("Team chat"));
        assert!(!active.blocks_program("game.exe"));
        assert!(!active.blocks_app_window("https://a.com/") && !active.blocks_firefox_page("https://a.com/", false));
        let mut seen = active.used.take();
        seen.sort();
        let key = |k: &str| ("a".to_string(), k.to_string());
        assert_eq!(seen, vec![key("app:Microsoft.Xbox*"), key("program:game.exe"), key("title:*Chat*")]);
        // What could not be reported is taken back, to go with the next report.
        active.used.put_back(seen.clone());
        assert_eq!(active.used.take(), seen);
        // Used up: closed again.
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[key("app:Microsoft.Xbox*"), key("title:*Chat*")]);
        let active = list.active().unwrap();
        assert!(active.blocks_store_app("Microsoft.XboxApp_8wekyb3d8bbwe") && active.blocks_title("Team chat"));
        assert!(!active.blocks_program("game.exe"), "the program has its own time");
    }

    #[test]
    fn a_budget_never_lets_obsidian_through_and_an_allowed_cycle_does() {
        // Obsidian's enforcer has no list, so no item: a budget, total or for each item, leaves it on.
        for scope in [Scope::Total, Scope::Each] {
            let list = blocklist(&format!("obsidian-{scope:?}"));
            list.start_with(
                "a",
                "A",
                vec![part(sites(&["a.com"]), Enforcers::default())],
                None,
                Some(budget("1h", scope)),
                StartLock::None,
            )
            .unwrap();
            let active = list.active().unwrap();
            assert!(!active.blocks_page("https://a.com/", false), "{scope:?}: the site is let through");
            assert!(active.closes_obsidian(), "{scope:?}: Obsidian is not");
            assert!(active.used.take().iter().all(|(_, key)| key.starts_with("site:") || key == TOTAL));
        }
        // While a cycle allows, the block enforces nothing at all, Obsidian included.
        let list = blocklist("obsidian-cycle");
        let cycle = Terms::Pomodoro { blocked: "1s".into(), allowed: "1h".into(), anchor: allowance::Anchor::Start };
        list.start_with("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], None, Some(cycle), StartLock::None).unwrap();
        assert!(list.active().unwrap().closes_obsidian(), "blocked first");
        list.clock.at_least(SystemTime::now() + Duration::from_secs(2));
        assert!(list.active().is_none(), "allowed: nothing to enforce");
    }

    #[test]
    fn a_budget_item_is_an_entry_as_typed_wildcards_and_all() {
        let list = blocklist("wildcards");
        list.start_with(
            "a",
            "A",
            vec![part(sites(&["*.reddit.com", "youtube.com", "*"]), Enforcers::default())],
            None,
            Some(budget("1s", Scope::Each)),
            StartLock::None,
        )
        .unwrap();
        let key = |k: &str| ("a".to_string(), k.to_string());
        // A wildcard entry is one item for everything it matches; a page that two entries match is
        // seen as both.
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://old.reddit.com/r/x", false) && !active.blocks_page("https://www.reddit.com/", false));
        assert_eq!(active.used.take(), vec![key("site:*"), key("site:*.reddit.com")]);
        assert!(!active.blocks_page("https://youtube.com/", false));
        assert_eq!(active.used.take(), vec![key("site:*"), key("site:youtube.com")]);
        // Once one of the entries that match a page is used up, the page is closed, though the other
        // has time.
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[key("site:youtube.com")]);
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://youtube.com/", false));
        assert!(!active.blocks_page("https://example.com/", false), "* still has time for the rest");
        list.polled();
        std::thread::sleep(Duration::from_millis(1100));
        list.charge(&[key("site:*")]);
        let active = list.active().unwrap();
        assert!(active.blocks_page("https://example.com/", false) && active.blocks_page("https://old.reddit.com/", false));
        // The items as the window shows them: by the entry as typed.
        let mut items: Vec<String> = running(&list, "a").allowance.unwrap().items.into_iter().map(|i| i.item).collect();
        items.sort();
        assert_eq!(items, vec!["*", "youtube.com"]);
    }

    #[test]
    fn a_condition_lock_says_what_it_waits_for() {
        let list = blocklist("lock-words");
        list.clock.at_least(monday(16 * 3600));
        let start = |id: &str, rule: Rule| {
            list.start(id, id, vec![part(sites(&["a.com"]), Enforcers::default())], None, StartLock::Condition(rule))
        };
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
            when_true: crate::webhook::Matcher { on: crate::webhook::Match::Status, status: Some(200), body: None },
            when_false: crate::webhook::Matcher { on: crate::webhook::Match::Status, status: Some(204), body: None },
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
        assert!(both.starts_with("“Both” is locked by its lock condition for the next 1 hour") || both.contains("59 minutes"), "{both}");
    }

    #[test]
    fn without_a_store_a_block_still_works_for_this_run() {
        let dir = std::env::temp_dir().join(format!("block-nostore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");

        let list =
            Blocklist { inner: Arc::default(), clock: test_clock(), zone: Some(Zone::fixed(0)), store: None, boot: Some("boot".into()) };
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).unwrap();
        assert!(list.is_running());
        list.restore();
        assert!(list.is_running(), "restoring from no store keeps what runs");
        assert!(!path.exists(), "a blocklist with no store must not write anything");
        assert!(!dir.exists(), "a blocklist with no store must not create a folder");
    }
}
