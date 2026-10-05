//! The running blocks: for each, its lists, its enforcers and, for a timed block, when it ends.
//! Each block is started from a definition (`definitions.rs`) and runs on its own: several can run
//! at once, and what is enforced is all of them together. A timed block cannot be stopped early
//! and stops by itself once its time is up.
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
use crate::filters::FilterList;
use crate::programs::ProgramList;
use crate::state::{self, Enforcers, Store, Uses};
use crate::sync::LockExt;

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

struct Block {
    /// The definition it was started from.
    id: String,
    name: String,
    filters: Arc<FilterList>,
    allowed: Arc<FilterList>,
    programs: Arc<ProgramList>,
    /// When a timed block ends, in internet time; `None` blocks until stopped.
    until: Option<SystemTime>,
    enforcers: Enforcers,
}

impl Block {
    fn enforced(&self) -> Enforced {
        Enforced { filters: self.filters.clone(), allowed: self.allowed.clone(), programs: self.programs.clone(), enforcers: self.enforcers }
    }
}

/// One running block, as the UI shows it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Running {
    pub id: String,
    pub name: String,
    /// Seconds left of a timed block; `None` runs until stopped.
    pub remaining: Option<u64>,
    /// What it blocks and how, so the UI can show what a running block does even if its definition
    /// has changed meanwhile or is gone.
    pub filters: Vec<String>,
    pub allowed: Vec<String>,
    pub programs: Vec<String>,
    pub enforcers: Enforcers,
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
        Blocklist { inner: Arc::default(), store: state::store().map(Arc::new), clock: clock::global() }
    }

    /// A block list over a store of its own and a clock of its own, for other modules' tests.
    #[cfg(test)]
    pub fn with_store(store: Store) -> Blocklist {
        Blocklist { inner: Arc::default(), store: Some(Arc::new(store)), clock: Box::leak(Box::default()) }
    }

    /// Starts a block from the definition `id`, unless that one is running: checked here rather
    /// than only in the UI, so a timed block cannot be swapped for a shorter or untimed one.
    /// Other blocks are not affected.
    pub fn start(&self, id: &str, name: &str, lists: Lists, duration: Option<Duration>, enforcers: Enforcers) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        if inner.blocks.iter().any(|b| b.id == id) {
            return Err("This block is already running".into());
        }
        let new = Block {
            id: id.to_string(),
            name: name.to_string(),
            filters: Arc::new(lists.sites),
            allowed: Arc::new(lists.allowed),
            programs: Arc::new(lists.programs),
            until: duration.map(|d| self.clock.now() + d),
            enforcers,
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

    /// Stops the block `id` unless it is timed and still running.
    pub fn stop(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let Some(at) = inner.blocks.iter().position(|b| b.id == id) else {
            return Err("This block is not running".into());
        };
        if let Some(left) = remaining(&inner.blocks[at], self.clock.now()) {
            return Err(format!("A timed block cannot be stopped early ({left}s left)"));
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
            let (Ok(filters), Ok(allowed), Ok(programs)) =
                (FilterList::parse(&saved.filters), FilterList::parse(&saved.allowed), ProgramList::parse(&saved.programs))
            else {
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
                filters: Arc::new(filters),
                allowed: Arc::new(allowed),
                programs: Arc::new(programs),
                until,
                enforcers: saved.enforcers,
            });
        }
        let mut inner = self.inner.locked();
        inner.blocks = blocks;
        // What was dropped is cleaned up, rather than being re-checked on every run.
        self.save_all(&mut inner);
    }

    /// What is to be enforced, ending timed blocks whose time is up. `Some` while any block runs.
    pub fn active(&self) -> Option<Active> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        // Keeps the time on disk moving, for a restart while offline (`Saved::seen`).
        if inner.blocks.iter().any(|b| b.until.is_some()) && inner.saved_at.elapsed() >= CHECKPOINT {
            self.save_all(&mut inner);
        }
        (!inner.blocks.is_empty()).then(|| Active { blocks: inner.blocks.iter().map(Block::enforced).collect() })
    }

    /// Whether any block runs.
    pub fn is_running(&self) -> bool {
        self.active().is_some()
    }

    /// Whether the block `id` runs.
    pub fn runs(&self, id: &str) -> bool {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        inner.blocks.iter().any(|b| b.id == id)
    }

    /// The seconds left of the timed block that ends last, if any is running.
    pub fn longest_remaining(&self) -> Option<u64> {
        let mut inner = self.inner.locked();
        self.expire(&mut inner);
        let now = self.clock.now();
        inner.blocks.iter().filter_map(|b| remaining(b, now)).max()
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
                remaining: remaining(b, now),
                filters: b.filters.entries(),
                allowed: b.allowed.entries(),
                programs: b.programs.entries(),
                enforcers: b.enforcers,
            })
            .collect();
        Status { running, clock_synced: self.clock.synced() }
    }

    /// Ends every block whatever its time, and forgets them. Only for the unlock password
    /// (`handler::unlock`): everything else goes through `stop`, which keeps a timed block.
    pub fn reset(&self) {
        let mut inner = self.inner.locked();
        inner.blocks.clear();
        self.save_all(&mut inner);
    }

    /// Removes timed blocks whose time is up; they end by themselves and leave nothing to resume.
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
                filters: b.filters.entries(),
                until: b.until.and_then(state::to_unix),
                seen,
                enforcers: b.enforcers,
                programs: b.programs.entries(),
                allowed: b.allowed.entries(),
            })
            .collect();
        store.save(&saved)
    }
}

fn remaining(block: &Block, now: SystemTime) -> Option<u64> {
    let left = block.until?.duration_since(now).ok()?;
    Some(left.as_secs_f64().ceil() as u64)
}

/// Parses `5s`, `10m`, `3h` or `1d`; an empty string means no time limit.
pub fn parse_duration(text: &str) -> Result<Option<Duration>, String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Ok(None);
    }
    let invalid = || format!("Invalid duration \"{text}\"; use e.g. 5s, 10m, 3h or 1d");
    let (number, unit) = text.split_at(text.len() - 1);
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return Err(invalid()),
    };
    let n: u64 = number.trim().parse().map_err(|_| invalid())?;
    if n == 0 {
        return Err(invalid());
    }
    Ok(Some(Duration::from_secs(n.checked_mul(seconds).ok_or_else(invalid)?)))
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
        Blocklist { inner: Arc::default(), clock: test_clock(), store: Some(Arc::new(Store::new(dir.join("state.json")))) }
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
        list.start(id, &format!("Block {id}"), lists, duration, Enforcers::default())
    }

    fn running(list: &Blocklist, id: &str) -> Running {
        list.status().running.into_iter().find(|r| r.id == id).expect("not running")
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration(""), Ok(None));
        assert_eq!(parse_duration("5s"), Ok(Some(Duration::from_secs(5))));
        assert_eq!(parse_duration(" 10M "), Ok(Some(Duration::from_secs(600))));
        assert_eq!(parse_duration("3h"), Ok(Some(Duration::from_secs(10800))));
        assert_eq!(parse_duration("1d"), Ok(Some(Duration::from_secs(86400))));
        for bad in ["5", "m", "0m", "5x", "1.5h", "-1h", "5 minutes"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn timed_block_cannot_stop_early_and_ends_by_itself() {
        let list = blocklist("timed");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_millis(50))).unwrap();
        assert!(list.stop("a").is_err());
        assert!(list.is_running());
        std::thread::sleep(Duration::from_millis(60));
        assert!(!list.is_running());
        assert!(list.stop("a").is_err(), "nothing left to stop");

        start(&list, "a", sites(&["a.com"]), None).unwrap();
        assert!(list.stop("a").is_ok());
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
        untimed.stop("a").unwrap();
        start(&untimed, "a", sites(&["b.com"]), None).unwrap();
        assert_eq!(running(&untimed, "a").filters, vec!["b.com"]);
    }

    #[test]
    fn blocks_run_side_by_side_and_end_on_their_own() {
        let list = blocklist("several");
        start(&list, "timed", sites(&["a.com"]), Some(Duration::from_millis(50))).unwrap();
        start(&list, "untimed", sites(&["b.com"]), None).unwrap();
        let active = list.active().unwrap();
        assert_eq!(active.blocks.len(), 2);
        assert!(active.blocks_page("https://a.com/", false) && active.blocks_page("https://b.com/", false));
        assert_eq!(list.longest_remaining(), Some(1));

        // The timed one ends by itself; the other goes on.
        std::thread::sleep(Duration::from_millis(60));
        let active = list.active().unwrap();
        assert!(!active.blocks_page("https://a.com/", false) && active.blocks_page("https://b.com/", false));
        assert!(!list.runs("timed") && list.runs("untimed"));
        assert_eq!(list.longest_remaining(), None);
        // Stopping one leaves the others.
        start(&list, "other", sites(&["c.com"]), None).unwrap();
        list.stop("untimed").unwrap();
        assert_eq!(list.status().running.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["other"]);
    }

    #[test]
    fn each_block_goes_by_its_own_enforcers_and_allowed_sites() {
        let list = blocklist("own-rules");
        let tabs_only = Enforcers { chromium_tabs: true, chromium_split_view: false, obsidian_web_viewer: false, exe_names: false };
        let split_only = Enforcers { chromium_tabs: false, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let everything_but_google = Lists { sites: FilterList::parse(&["*"]).unwrap(), allowed: FilterList::parse(&["google.com"]).unwrap(), programs: ProgramList::default() };
        list.start("tabs", "Tabs", everything_but_google, None, tabs_only).unwrap();
        list.start("split", "Split", sites(&["google.com"]), None, split_only).unwrap();
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
        let list = Blocklist { inner: Arc::default(), clock: test_clock(), store: Some(Arc::new(Store::new(dir.join("not-a-folder").join("state.json")))) };
        assert!(start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).is_err());
        assert!(!list.is_running());
    }

    #[test]
    fn blocks_outlive_the_service_that_started_them() {
        let list = blocklist("resume");
        start(&list, "a", sites(&["https://tweakers.net/", "b.com"]), Some(Duration::from_secs(3600))).unwrap();
        start(&list, "b", sites(&["c.com"]), None).unwrap();
        assert_eq!(running(&list, "a").filters, vec!["tweakers.net", "b.com"]);

        // A new run over the same store: both are still going, with their names.
        let resumed = reopened("resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.blocks_page("https://tweakers.net/nieuws", false) && active.blocks_page("https://c.com/", false));
        let a = running(&resumed, "a");
        assert_eq!(a.name, "Block a");
        assert!(a.remaining.is_some_and(|left| (3595..=3600).contains(&left)));
        assert!(resumed.stop("a").is_err());
        assert_eq!(running(&resumed, "b").remaining, None);
        assert!(resumed.stop("b").is_ok());
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
    fn the_allowed_sites_survive_a_restart_and_older_blocks_have_none() {
        let list = blocklist("allowed-resume");
        let lists = Lists {
            sites: FilterList::parse(&["*"]).unwrap(),
            allowed: FilterList::parse(&["Google.com", "*.wikipedia.org"]).unwrap(),
            programs: ProgramList::default(),
        };
        start(&list, "a", lists, Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(running(&list, "a").allowed, vec!["google.com", "*.wikipedia.org"]);

        let resumed = reopened("allowed-resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(blocks_site(&active, "https://youtube.com/") && !blocks_site(&active, "https://google.com/"));
        assert_eq!(running(&resumed, "a").allowed, vec!["google.com", "*.wikipedia.org"]);

        // A saved block from before the allowed list (and before several blocks) resumes with none.
        let older = blocklist("allowed-older");
        let store = older.store.clone().unwrap();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        let until = state::now().unwrap().as_secs() + 3600;
        std::fs::write(store.path(), format!(r#"{{"filters":["a.com"],"until":{until}}}"#)).unwrap();
        let resumed = reopened("allowed-older");
        resumed.restore();
        assert!(running(&resumed, "").allowed.is_empty() && blocks_site(&resumed.active().unwrap(), "https://a.com/"));
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
        assert_eq!(running(&list, "a").programs, vec!["steam*.exe", "game.exe"]);

        // A new run over the same store: the programs are still on the block.
        let resumed = reopened("programs");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.closes_programs());
        assert!(active.blocks_program("steamwebhelper.exe") && active.blocks_program("GAME.EXE"));
        assert!(!active.blocks_program("chrome.exe"));
        assert_eq!(running(&resumed, "a").programs, vec!["steam*.exe", "game.exe"]);
    }

    #[test]
    fn a_block_from_before_programs_still_resumes_without_any() {
        let list = blocklist("before-programs");
        let store = list.store.clone().unwrap();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        let until = state::now().unwrap().as_secs() + 3600;
        std::fs::write(store.path(), format!(r#"{{"filters":["a.com"],"until":{until}}}"#)).unwrap();
        let resumed = reopened("before-programs");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.blocks[0].filters.is_blocked("a.com") && !active.closes_programs());
    }

    #[test]
    fn a_resumed_block_keeps_its_enforcers() {
        let only_obsidian = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true, exe_names: false };
        let list = blocklist("enforcers");
        list.start("a", "A", sites(&[]), Some(Duration::from_secs(3600)), only_obsidian).unwrap();
        let resumed = reopened("enforcers");
        resumed.restore();
        assert_eq!(resumed.active().map(|active| active.blocks[0].enforcers), Some(only_obsidian));
        assert_eq!(running(&resumed, "a").enforcers, only_obsidian);
    }

    #[test]
    fn a_block_whose_time_is_up_is_not_resumed() {
        let list = blocklist("expired");
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(3600))).unwrap();
        // Rewritten as though it had been left behind by a run that ended an hour ago, next to one
        // that still runs.
        let store = list.store.clone().unwrap();
        let saved = |id: &str, until| Saved {
            id: id.into(),
            name: id.into(),
            filters: vec!["a.com".to_string()],
            until,
            seen: None,
            enforcers: Enforcers::default(),
            programs: Vec::new(),
            allowed: Vec::new(),
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
        list.stop("a").unwrap();
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

        let list = Blocklist { inner: Arc::default(), clock: test_clock(), store: None };
        start(&list, "a", sites(&["a.com"]), Some(Duration::from_secs(60))).unwrap();
        assert!(list.is_running());
        list.restore();
        assert!(list.is_running(), "restoring from no store keeps what runs");
        assert!(!path.exists(), "a blocklist with no store must not write anything");
        assert!(!dir.exists(), "a blocklist with no store must not create a folder");
    }
}
