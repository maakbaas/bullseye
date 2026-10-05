//! The running blocks: for each, its block lists (each with its own enforcers), its weekly schedule
//! if it has one, and its lock (`lock.rs`): when a timer lock ends, and whether it is locked while
//! its schedule has it active. Each block is started from a definition (`definitions.rs`) and runs
//! on its own: several can run at once, and what is enforced is all of them together. A block with
//! a timer lock cannot be stopped early and stops by itself once its time is up. Outside its
//! schedule a block keeps running, and stays locked by a timer, but enforces nothing.
//!
//! The running blocks are also kept on disk (`state.rs`), so ending the app, or restarting
//! Windows, does not end them: the next run picks them up again while their time is not up.
//!
//! Time is internet time (`clock.rs`), not the Windows clock, so setting that clock forward
//! does not end a timed block early.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::clock::{self, Clock};
use crate::duration;
use crate::filters::FilterList;
use crate::localtime::Zone;
use crate::programs::ProgramList;
use crate::schedule::Week;
use crate::state::{self, Enforcers, Store, Uses};
use crate::sync::LockExt;
use crate::unlock;

/// How often running timed blocks write down the time they have reached (`Saved::seen`).
const CHECKPOINT: Duration = Duration::from_secs(60);

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
}

struct Inner {
    blocks: Vec<Block>,
    /// When the blocks were last written to disk.
    saved_at: Instant,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner { blocks: Vec::new(), saved_at: Instant::now() }
    }
}

/// What a block is told to block. **A new kind of list goes here**, together with a `state::Uses`
/// variant, an arm in `has`, a field in `Block`, `state::Saved` and `definitions::Definition`, and
/// an entry in the UI's `LISTS` table.
pub struct Lists {
    /// The sites to block.
    pub sites: FilterList,
    /// The sites that are always allowed, which win over `sites`. They are not something an
    /// enforcer needs on its own (`Uses::Sites` is about the block list), they only make
    /// exceptions to it.
    pub allowed: FilterList,
    pub programs: ProgramList,
}

impl Lists {
    /// Whether the list an enforcer needs has an entry (`Enforcers::can_run`).
    pub fn has(&self, uses: Uses) -> bool {
        match uses {
            Uses::Nothing => true,
            Uses::Sites => !self.sites.is_empty(),
            Uses::Programs => !self.programs.is_empty(),
        }
    }
}

/// What one running block enforces.
#[derive(Clone)]
pub struct Enforced {
    pub filters: Arc<FilterList>,
    pub allowed: Arc<FilterList>,
    pub programs: Arc<ProgramList>,
    pub enforcers: Enforcers,
}

impl Enforced {
    /// Whether a page is to be closed: on the block list, and not on the list of allowed sites.
    /// Allowed wins, whatever the block list says, so `*` on the block list with `google.com`
    /// allowed blocks every site but that one. Each block's allowed sites are exceptions to its
    /// own block list only.
    pub fn blocks_site(&self, url: &str) -> bool {
        self.filters.is_blocked(url) && !self.allowed.is_blocked(url)
    }
}

/// What is to be enforced right now: every running block's part, each with its own enforcers.
#[derive(Clone)]
pub struct Active {
    pub blocks: Vec<Enforced>,
}

impl Active {
    /// Whether a page shown in a browser is to be closed by any block that closes such pages:
    /// `split_view` for a window showing two pages, otherwise an ordinary tab.
    pub fn blocks_page(&self, url: &str, split_view: bool) -> bool {
        self.blocks.iter().any(|b| {
            let enforced = if split_view { b.enforcers.chromium_split_view } else { b.enforcers.chromium_tabs };
            enforced && b.blocks_site(url)
        })
    }

    /// Whether any block has sites to look for, so the browsers are worth looking at.
    pub fn watches_sites(&self) -> bool {
        self.blocks.iter().any(|b| b.enforcers.uses(Uses::Sites) && !b.filters.is_empty())
    }

    /// Whether any block closes programs and has some to close.
    pub fn closes_programs(&self) -> bool {
        self.blocks.iter().any(|b| b.enforcers.uses(Uses::Programs) && !b.programs.is_empty())
    }

    /// Whether a program (by file name) is closed by any block that closes programs.
    pub fn blocks_program(&self, file_name: &str) -> bool {
        self.blocks.iter().any(|b| b.enforcers.uses(Uses::Programs) && b.programs.is_blocked(file_name))
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

/// How a block is locked once started (`lock::Lock`): a timer's length, a lock schedule (locked
/// while it is active), a password's hash (`unlock::new_hash`). None of them: not locked.
#[derive(Default)]
pub struct Locks {
    pub timer: Option<Duration>,
    pub schedule: Option<Week>,
    pub password: Option<String>,
}

/// A part of a running block: its block list's name and what it enforces.
struct RunPart {
    name: String,
    enforced: Enforced,
}

struct Block {
    /// The definition it was started from.
    id: String,
    name: String,
    /// One per block list, each by its own enforcers and allowed sites.
    parts: Vec<RunPart>,
    /// When a timer lock ends, and the block with it, in internet time; `None`: no timer.
    until: Option<SystemTime>,
    /// Locked while this schedule is active (`Lock::Scheduled`), read in `zone` like `schedule`.
    lock_schedule: Option<Week>,
    /// Locked with a password: its salted hash (`Lock::Password`).
    password: Option<String>,
    /// When the block is active; `None` is always.
    schedule: Option<Week>,
    /// The time zone the schedule is read in: Windows' when the block started, kept until it ends,
    /// so changing the time zone does not move the schedule.
    zone: Zone,
}

impl Block {
    /// Whether its schedule has the block active at `now`. If the local time cannot be known, it
    /// is: a block errs on the side of blocking.
    fn in_schedule(&self, now: SystemTime) -> bool {
        let Some(week) = &self.schedule else { return true };
        self.zone.local(now).is_none_or(|local| week.is_on(local))
    }

    /// Seconds until the schedule turns the block on or off.
    fn schedule_change(&self, now: SystemTime) -> Option<u64> {
        self.schedule.as_ref()?.next_change(self.zone.local(now)?)
    }

    /// Seconds until its lock schedule locks or frees it.
    fn lock_change(&self, now: SystemTime) -> Option<u64> {
        self.lock_schedule.as_ref()?.next_change(self.zone.local(now)?)
    }

    /// Whether its lock schedule has it locked at `now`; like `in_schedule`, it is if the local
    /// time cannot be known.
    fn schedule_locked(&self, now: SystemTime) -> bool {
        let Some(week) = &self.lock_schedule else { return false };
        self.zone.local(now).is_none_or(|local| week.is_on(local))
    }

    /// Why it cannot be stopped at `now`, if it cannot (without its password, for a password lock).
    fn locked(&self, now: SystemTime) -> Option<String> {
        if self.password.is_some() {
            return Some(format!("“{}” is locked with a password", self.name));
        }
        if let Some(left) = remaining(self, now) {
            return Some(format!("“{}” is locked for another {}", self.name, duration::words(left)));
        }
        if self.schedule_locked(now) {
            let until = self.lock_change(now).map(|s| format!(", for another {}", duration::words(s))).unwrap_or_default();
            return Some(format!("“{}” is locked by its lock schedule{until}", self.name));
        }
        None
    }
}

/// One block list of a running block, as the UI shows it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct RunningList {
    pub name: String,
    pub filters: Vec<String>,
    pub allowed: Vec<String>,
    pub programs: Vec<String>,
    pub enforcers: Enforcers,
}

/// One running block, as the UI shows it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Running {
    pub id: String,
    pub name: String,
    /// What it blocks and how, as it was started, so the UI can show what a running block does.
    pub lists: Vec<RunningList>,
    /// Seconds left of a timer lock; `None`: no timer.
    pub remaining: Option<u64>,
    /// Locked while this schedule is active (`Lock::Scheduled`).
    pub lock_schedule: Option<Week>,
    /// Locked with a password: stopped only with it (`Lock::Password`).
    pub password: bool,
    /// Whether it cannot be stopped right now (without its password, for a password lock).
    pub locked: bool,
    /// Its weekly schedule; `None` is always active.
    pub schedule: Option<Week>,
    /// Whether it enforces anything right now: always, unless its schedule has it inactive.
    pub enforcing: bool,
    /// Seconds until its schedule turns it on or off, if it has one that ever does.
    pub schedule_change: Option<u64>,
    /// Seconds until its lock schedule locks or frees it, if it has one that ever does.
    pub lock_change: Option<u64>,
    /// The three times above in words (`duration::words`), so the window and the CLI show them the
    /// same way without a formatter of their own.
    pub remaining_words: Option<String>,
    pub schedule_change_words: Option<String>,
    pub lock_change_words: Option<String>,
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
        Blocklist { inner: Arc::default(), store: state::store().map(Arc::new), clock: clock::global(), zone: None }
    }

    /// A block list over a store of its own and a clock of its own, for other modules' tests.
    #[cfg(test)]
    pub fn with_store(store: Store) -> Blocklist {
        Blocklist { inner: Arc::default(), store: Some(Arc::new(store)), clock: Box::leak(Box::default()), zone: Some(Zone::fixed(0)) }
    }

    /// Starts a block from the definition `id`, unless that one is running: checked here rather
    /// than only in the UI, so a locked block cannot be swapped for a shorter or unlocked one.
    /// `schedule` says when it is active (`None`: always), `locks` how it is locked (`lock.rs`).
    /// Other blocks are not affected.
    pub fn start(&self, id: &str, name: &str, parts: Vec<Part>, schedule: Option<Week>, locks: Locks) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        if inner.blocks.iter().any(|b| b.id == id) {
            return Err("This block is already running".into());
        }
        let parts = parts
            .into_iter()
            .map(|p| RunPart {
                name: p.name,
                enforced: Enforced { filters: Arc::new(p.lists.sites), allowed: Arc::new(p.lists.allowed), programs: Arc::new(p.lists.programs), enforcers: p.enforcers },
            })
            .collect();
        let new = Block {
            id: id.to_string(),
            name: name.to_string(),
            parts,
            until: locks.timer.map(|d| self.clock.now() + d),
            lock_schedule: locks.schedule,
            password: locks.password,
            schedule,
            zone: self.zone.clone().unwrap_or_else(Zone::current),
        };
        // Written before the block goes live, so a block cannot be missed after a restart. A
        // block that cannot be written would not survive the service ending, so it does not start.
        let mut all: Vec<&Block> = inner.blocks.iter().collect();
        all.push(&new);
        self.save(&all).map_err(|e| format!("Cannot save the block, so it would not survive a restart: {e}"))?;
        inner.blocks.push(new);
        inner.saved_at = Instant::now();
        Ok(())
    }

    /// Stops the block `id` unless it is locked right now. `password_given`: the right password of
    /// its password lock was given (`password_matches`), which is what lets that one stop.
    pub fn stop(&self, id: &str, password_given: bool) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let Some(at) = inner.blocks.iter().position(|b| b.id == id) else {
            return Err("This block is not running".into());
        };
        let block = &inner.blocks[at];
        if !(password_given && block.password.is_some()) {
            if let Some(why) = block.locked(self.clock.now()) {
                let how = if block.password.is_some() { "; give its password to stop it" } else { ", so it cannot be stopped now" };
                return Err(format!("{why}{how}"));
            }
        }
        inner.blocks.remove(at);
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
                self.clock.at_least(seen);
            }
        }
        for saved in saved {
            // A block list we cannot read is dropped rather than resumed as something unknown.
            let parts: Result<Vec<RunPart>, String> = saved
                .parts
                .into_iter()
                .map(|p| {
                    Ok(RunPart {
                        name: p.name,
                        enforced: Enforced {
                            filters: Arc::new(FilterList::parse(&p.filters).map_err(|_| "sites")?),
                            allowed: Arc::new(FilterList::parse(&p.allowed).map_err(|_| "allowed")?),
                            programs: Arc::new(ProgramList::parse(&p.programs)?),
                            enforcers: p.enforcers,
                        },
                    })
                })
                .collect();
            let schedule = saved.schedule.clone().map(Week::normalized).transpose();
            let (Ok(parts), Ok(schedule)) = (parts, schedule) else {
                eprintln!("block: dropping an unreadable saved block");
                continue;
            };
            let until = saved.until.map(state::from_unix);
            if until.is_some_and(|u| self.clock.now() >= u) {
                continue;
            }
            blocks.push(Block {
                id: saved.id,
                name: saved.name,
                parts,
                until,
                lock_schedule: saved.lock_schedule,
                password: saved.password,
                schedule,
                zone: Zone::named(&saved.zone),
            });
        }
        let mut inner = self.inner.locked();
        inner.blocks = blocks;
        // What was dropped is cleaned up, rather than being re-checked on every run.
        self.save_all(&mut inner);
    }

    /// What is to be enforced right now: every block list of every running block that its schedule
    /// has active. Ends timed blocks whose time is up. `None` when nothing is to be enforced.
    pub fn active(&self) -> Option<Active> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        // Keeps the time on disk moving, for a restart while offline (`Saved::seen`).
        if inner.blocks.iter().any(|b| b.until.is_some()) && inner.saved_at.elapsed() >= CHECKPOINT {
            self.save_all(&mut inner);
        }
        let now = self.clock.now();
        let blocks: Vec<Enforced> =
            inner.blocks.iter().filter(|b| b.in_schedule(now)).flat_map(|b| b.parts.iter().map(|p| p.enforced.clone())).collect();
        (!blocks.is_empty()).then_some(Active { blocks })
    }

    /// Whether any block runs, also one that its schedule has inactive right now: it is still
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

    /// Whether `password` is the password of the running block `id`'s password lock.
    pub fn password_matches(&self, id: &str, password: &str) -> bool {
        let inner = self.inner.locked();
        let hash = inner.blocks.iter().find(|b| b.id == id).and_then(|b| b.password.clone());
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
        let running = inner
            .blocks
            .iter()
            .map(|b| Running {
                id: b.id.clone(),
                name: b.name.clone(),
                lists: b
                    .parts
                    .iter()
                    .map(|p| RunningList {
                        name: p.name.clone(),
                        filters: p.enforced.filters.entries(),
                        allowed: p.enforced.allowed.entries(),
                        programs: p.enforced.programs.entries(),
                        enforcers: p.enforced.enforcers,
                    })
                    .collect(),
                remaining: remaining(b, now),
                lock_schedule: b.lock_schedule.clone(),
                password: b.password.is_some(),
                locked: b.locked(now).is_some(),
                schedule: b.schedule.clone(),
                enforcing: b.in_schedule(now),
                schedule_change: b.schedule_change(now),
                remaining_words: remaining(b, now).map(duration::words),
                schedule_change_words: b.schedule_change(now).map(duration::words),
                lock_change: b.lock_change(now),
                lock_change_words: b.lock_change(now).map(duration::words),
            })
            .collect();
        Status { running, clock_synced: self.clock.synced() }
    }

    /// Ends every block whatever its lock, and forgets them. Only for the unlock password
    /// (`handler::unlock`): everything else goes through `stop`, which keeps a locked block.
    pub fn reset(&self) {
        let mut inner = self.inner.locked();
        inner.blocks.clear();
        self.save_all(&mut inner);
    }

    /// Removes blocks whose timer lock is up; they end by themselves and leave nothing to resume.
    fn expire(&self, inner: &mut Inner) {
        let now = self.clock.now();
        let before = inner.blocks.len();
        inner.blocks.retain(|b| b.until.is_none_or(|u| now < u));
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
                        filters: p.enforced.filters.entries(),
                        allowed: p.enforced.allowed.entries(),
                        programs: p.enforced.programs.entries(),
                        enforcers: p.enforced.enforcers,
                    })
                    .collect(),
                until: b.until.and_then(state::to_unix),
                seen,
                lock_schedule: b.lock_schedule.clone(),
                password: b.password.clone(),
                schedule: b.schedule.clone(),
                zone: b.zone.name().to_string(),
            })
            .collect();
        store.save(&saved)
    }
}

fn remaining(block: &Block, now: SystemTime) -> Option<u64> {
    let left = block.until?.duration_since(now).ok()?;
    Some(left.as_secs_f64().ceil() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Saved;

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
        Blocklist { inner: Arc::default(), clock: test_clock(), zone: Some(Zone::fixed(0)), store: Some(Arc::new(Store::new(dir.join("state.json")))) }
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
        Lists { sites: FilterList::parse(entries).unwrap(), allowed: FilterList::default(), programs: ProgramList::default() }
    }

    fn start(list: &Blocklist, id: &str, lists: Lists, duration: Option<Duration>) -> Result<(), String> {
        list.start(id, &format!("Block {id}"), vec![part(lists, Enforcers::default())], None, Locks { timer: duration, ..Locks::default() })
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
        assert_eq!(running(&untimed, "a").lists[0].filters, vec!["b.com"]);
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
        let tabs_only = Enforcers { chromium_tabs: true, chromium_split_view: false, obsidian_web_viewer: false, exe_names: false };
        let split_only = Enforcers { chromium_tabs: false, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let everything_but_google = Lists { sites: FilterList::parse(&["*"]).unwrap(), allowed: FilterList::parse(&["google.com"]).unwrap(), programs: ProgramList::default() };
        list.start("tabs", "Tabs", vec![part(everything_but_google, tabs_only)], None, Locks::default()).unwrap();
        list.start("split", "Split", vec![part(sites(&["google.com"]), split_only)], None, Locks::default()).unwrap();
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
        let list = Blocklist { inner: Arc::default(), clock: test_clock(), zone: Some(Zone::fixed(0)), store: Some(Arc::new(Store::new(dir.join("not-a-folder").join("state.json")))) };
        assert!(start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).is_err());
        assert!(!list.is_running());
    }

    #[test]
    fn blocks_outlive_the_service_that_started_them() {
        let list = blocklist("resume");
        start(&list, "a", sites(&["https://tweakers.net/", "b.com"]), Some(Duration::from_secs(3600))).unwrap();
        start(&list, "b", sites(&["c.com"]), None).unwrap();
        assert_eq!(running(&list, "a").lists[0].filters, vec!["tweakers.net", "b.com"]);

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
            std::env::temp_dir().join(format!("block-{}-allowed-{}", std::process::id(), block.join("_").len() + allow.len())).join("state.json"),
        ));
        let lists = Lists {
            sites: FilterList::parse(block).unwrap(),
            allowed: FilterList::parse(allow).unwrap(),
            programs: ProgramList::default(),
        };
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
        // An allowed entry is exactly as wide as it is written: not the sites around it.
        assert!(blocks_site(&active, "https://accounts.google.com/"));
        assert!(blocks_site(&active, "https://wikipedia.org/"));

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
            programs: ProgramList::default(),
        };
        start(&list, "a", lists, Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(running(&list, "a").lists[0].allowed, vec!["google.com", "*.wikipedia.org"]);

        let resumed = reopened("allowed-resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(blocks_site(&active, "https://youtube.com/") && !blocks_site(&active, "https://google.com/"));
        assert_eq!(running(&resumed, "a").lists[0].allowed, vec!["google.com", "*.wikipedia.org"]);
    }

    #[test]
    fn a_resumed_block_keeps_its_programs() {
        let list = blocklist("programs");
        let lists = Lists {
            sites: FilterList::default(),
            allowed: FilterList::default(),
            programs: ProgramList::parse(&["Steam*", "game"]).unwrap(),
        };
        start(&list, "a", lists, Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(running(&list, "a").lists[0].programs, vec!["steam*.exe", "game.exe"]);

        // A new run over the same store: the programs are still on the block.
        let resumed = reopened("programs");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.closes_programs());
        assert!(active.blocks_program("steamwebhelper.exe") && active.blocks_program("GAME.EXE"));
        assert!(!active.blocks_program("chrome.exe"));
        assert_eq!(running(&resumed, "a").lists[0].programs, vec!["steam*.exe", "game.exe"]);
    }

    #[test]
    fn a_resumed_block_keeps_its_enforcers() {
        let only_obsidian = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true, exe_names: false };
        let list = blocklist("enforcers");
        list.start("a", "A", vec![part(sites(&[]), only_obsidian)], None, Locks { timer: Some(Duration::from_secs(3600)), ..Locks::default() }).unwrap();
        let resumed = reopened("enforcers");
        resumed.restore();
        assert_eq!(resumed.active().map(|active| active.blocks[0].enforcers), Some(only_obsidian));
        assert_eq!(running(&resumed, "a").lists[0].enforcers, only_obsidian);
    }

    /// 2030-01-07 00:00:00 UTC, a Monday; the tests' time zone is UTC.
    const MONDAY: u64 = 1_893_974_400;

    #[test]
    fn outside_its_schedule_a_block_runs_and_stays_locked_but_enforces_nothing() {
        let list = blocklist("schedule");
        // Monday 16:00, office hours until 17:00, locked for two hours: the user's own example.
        list.clock.at_least(MONDAY + 16 * 3600);
        let office = Week::parse("mon-fri 08:00-17:00").unwrap();
        list.start("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], office.clone(), Locks { timer: Some(Duration::from_secs(2 * 3600)), ..Locks::default() }).unwrap();
        // An always-active block next to it, to show the other one going quiet on its own.
        start(&list, "b", sites(&["b.com"]), None).unwrap();
        assert!(list.active().unwrap().blocks_page("https://a.com/", false));
        let a = running(&list, "a");
        assert!(a.enforcing && a.schedule == office);
        // The test runs on a real clock as well, so a few seconds may pass along the way.
        assert!(a.schedule_change.is_some_and(|left| (3590..=3600).contains(&left)));
        assert!(running(&list, "b").enforcing && running(&list, "b").schedule_change.is_none());

        // 17:00: the schedule has it inactive, so nothing of it is enforced; it still runs and is
        // still locked.
        list.clock.at_least(MONDAY + 17 * 3600);
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/", false) && active.blocks_page("https://b.com/", false));
        assert!(list.runs("a") && list.stop("a", false).is_err());
        let a = running(&list, "a");
        assert!(!a.enforcing);
        // In words too, for the window and the CLI.
        assert!(a.remaining_words.as_deref().is_some_and(|words| words.starts_with("1 hour ")), "{:?}", a.remaining_words);
        assert!(a.schedule_change_words.as_deref().is_some_and(|words| words.starts_with("14 hours") || words.starts_with("15 hours")));
        // Rounded up, and the start took a moment of the clock: a little over the hour.
        assert!(a.remaining.is_some_and(|left| (3590..=3610).contains(&left)));
        // Active again on Tuesday at 08:00.
        assert!(a.schedule_change.is_some_and(|left| (15 * 3600 - 10..=15 * 3600).contains(&left)));
        // With only it running, nothing is enforced, but a block still runs (the app is kept alive).
        list.stop("b", false).unwrap();
        assert!(list.active().is_none() && list.is_running());

        // Just after 18:00 (the lock started a moment after 16:00): the lock is over, and with it
        // the block.
        list.clock.at_least(MONDAY + 18 * 3600 + 30);
        assert!(!list.runs("a") && !list.is_running());
    }

    #[test]
    fn a_lock_schedule_locks_only_while_it_is_active() {
        let list = blocklist("lock-schedule");
        // Monday 16:00. Active all day, locked during office hours, until 17:00.
        list.clock.at_least(MONDAY + 16 * 3600);
        let office = Week::parse("mon-fri 08:00-17:00").unwrap();
        let locks = || Locks { schedule: office.clone(), ..Locks::default() };
        list.start("a", "Work", vec![part(sites(&["a.com"]), Enforcers::default())], None, locks()).unwrap();
        let why = list.stop("a", false).unwrap_err();
        assert!(why.contains("locked by its lock schedule, for another 1 hour") || why.contains("for another 59 minutes"), "{why}");
        let a = running(&list, "a");
        assert!(a.locked && a.lock_schedule == office && a.enforcing);
        assert!(a.lock_change.is_some_and(|left| (3590..=3600).contains(&left)), "{:?}", a.lock_change);
        assert!(list.locked().is_some(), "the service may not be uninstalled now");
        // 17:00: outside the lock schedule it can be stopped, and still blocks until it is.
        list.clock.at_least(MONDAY + 17 * 3600 + 5);
        let a = running(&list, "a");
        assert!(!a.locked && a.enforcing && list.locked().is_none());
        // Locked again on Tuesday at 08:00.
        assert!(a.lock_change.is_some_and(|left| (15 * 3600 - 10..=15 * 3600).contains(&left)));
        list.stop("a", false).unwrap();
        assert!(!list.is_running());

        // It survives a restart as it was.
        list.clock.at_least(MONDAY + 7 * 86400 + 9 * 3600);
        list.start("a", "Work", vec![part(sites(&["a.com"]), Enforcers::default())], None, locks()).unwrap();
        let resumed = reopened("lock-schedule");
        resumed.clock.at_least(MONDAY + 7 * 86400 + 9 * 3600);
        resumed.restore();
        assert!(resumed.stop("a", false).is_err() && running(&resumed, "a").lock_schedule == office);
    }

    #[test]
    fn a_password_lock_stops_only_with_its_password() {
        let list = blocklist("password");
        let locks = Locks { password: Some(unlock::new_hash("partner").unwrap()), ..Locks::default() };
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
    fn a_block_of_several_lists_enforces_each_by_its_own_rules() {
        let list = blocklist("parts");
        let tabs_only = Enforcers { chromium_tabs: true, chromium_split_view: false, obsidian_web_viewer: false, exe_names: false };
        let programs_only = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: false, exe_names: true };
        let games = Lists { sites: FilterList::default(), allowed: FilterList::default(), programs: ProgramList::parse(&["steam"]).unwrap() };
        let social = Part { name: "Social".into(), lists: sites(&["youtube.com"]), enforcers: tabs_only };
        list.start("a", "Evening", vec![social, Part { name: "Games".into(), lists: games, enforcers: programs_only }], None, Locks::default()).unwrap();
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
    fn a_resumed_block_keeps_its_schedule_and_time_zone() {
        let list = blocklist("schedule-resume");
        list.clock.at_least(MONDAY + 12 * 3600);
        let weekend = Week::parse("weekends").unwrap();
        list.start("a", "A", vec![part(sites(&["a.com"]), Enforcers::default())], weekend.clone(), Locks::default()).unwrap();
        assert!(list.is_running() && list.active().is_none(), "Monday is not in the weekend");
        let saved = list.store.clone().unwrap().load().remove(0);
        assert_eq!((saved.schedule, saved.zone.as_str()), (weekend.clone(), "fixed:+0"));

        let resumed = reopened("schedule-resume");
        resumed.clock.at_least(MONDAY + 5 * 86400 + 3600);
        resumed.restore();
        assert_eq!(running(&resumed, "a").schedule, weekend);
        assert!(resumed.active().is_some(), "Saturday is");
    }

    #[test]
    fn a_block_whose_time_is_up_is_not_resumed() {
        let list = blocklist("expired");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(3600))).unwrap();
        // Rewritten as though it had been left behind by a run that ended an hour ago, next to one
        // that still runs.
        let store = list.store.clone().unwrap();
        let part = state::SavedPart { name: "List".into(), filters: vec!["a.com".into()], allowed: vec![], programs: vec![], enforcers: Enforcers::default() };
        let saved = |id: &str, until| Saved {
            id: id.into(),
            name: id.into(),
            parts: vec![part.clone()],
            until,
            seen: None,
            lock_schedule: None,
            password: None,
            schedule: None,
            zone: "fixed:+0".into(),
        };
        store.save(&[saved("old", Some(state::now().unwrap().as_secs() - 3600)), saved("new", None)]).unwrap();

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

    #[test]
    fn without_a_store_a_block_still_works_for_this_run() {
        let dir = std::env::temp_dir().join(format!("block-nostore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");

        let list = Blocklist { inner: Arc::default(), clock: test_clock(), zone: Some(Zone::fixed(0)), store: None };
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).unwrap();
        assert!(list.is_running());
        list.restore();
        assert!(list.is_running(), "restoring from no store keeps what runs");
        assert!(!path.exists(), "a blocklist with no store must not write anything");
        assert!(!dir.exists(), "a blocklist with no store must not create a folder");
    }
}
