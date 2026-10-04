//! The running block: its filter list and, for a timed block, when it ends.
//! A timed block cannot be stopped early and stops by itself once its time is up.
//!
//! The block is also kept on disk (`state.rs`), so ending the app, or restarting Windows, does
//! not end it: the next run picks the block up again while its time is not up.
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

/// How often a running timed block writes down the time it has reached (`Saved::seen`).
const CHECKPOINT: Duration = Duration::from_secs(60);

/// Shared between the UI commands and the worker thread.
#[derive(Clone)]
pub struct Blocklist {
    block: Arc<Mutex<Option<Block>>>,
    /// Where the block is kept between runs; `None` disables surviving a restart.
    store: Option<Arc<Store>>,
    /// The app's internet clock; tests have their own.
    clock: &'static Clock,
}

/// What a block is told to block. **A new kind of list goes here**, together with a `state::Uses`
/// variant, an arm in `has`, and a field in `Block` and `state::Saved`.
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

/// What is to be enforced right now.
#[derive(Clone)]
pub struct Active {
    pub filters: Arc<FilterList>,
    pub allowed: Arc<FilterList>,
    pub programs: Arc<ProgramList>,
    pub enforcers: Enforcers,
}

impl Active {
    /// Whether a page is to be closed: on the block list, and not on the list of allowed sites.
    /// Allowed wins, whatever the block list says, so `*` on the block list with `google.com`
    /// allowed blocks every site but that one.
    pub fn blocks_site(&self, url: &str) -> bool {
        self.filters.is_blocked(url) && !self.allowed.is_blocked(url)
    }
}

struct Block {
    filters: Arc<FilterList>,
    allowed: Arc<FilterList>,
    programs: Arc<ProgramList>,
    /// When a timed block ends, in internet time; `None` blocks until stopped.
    until: Option<SystemTime>,
    enforcers: Enforcers,
    /// When the block was last written to disk.
    saved_at: Instant,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Status {
    blocking: bool,
    /// Seconds left of a timed block.
    pub remaining: Option<u64>,
    /// The blocked entries, so the UI can show what a resumed block is blocking.
    pub filters: Vec<String>,
    /// The allowed sites, for the same reason.
    #[serde(default)]
    pub allowed: Vec<String>,
    /// The blocked programs, for the same reason.
    #[serde(default)]
    pub programs: Vec<String>,
    /// The running block's enforcers, so the UI shows what a resumed block enforces.
    enforcers: Option<Enforcers>,
    /// Whether the time comes from the internet yet; until then, time with the app not running
    /// does not count.
    clock_synced: bool,
}

impl Blocklist {
    /// The app's own block list, over the real store in the local app-data folder.
    pub fn new() -> Blocklist {
        Blocklist { block: Arc::default(), store: state::store().map(Arc::new), clock: clock::global() }
    }

    /// A block list over a store of its own and a clock of its own, for other modules' tests.
    #[cfg(test)]
    pub fn with_store(store: Store) -> Blocklist {
        Blocklist { block: Arc::default(), store: Some(Arc::new(store)), clock: Box::leak(Box::default()) }
    }

    /// Starts a block, unless one is running: checked here rather than only in the UI, so a
    /// timed block cannot be swapped for a shorter or untimed one.
    pub fn start(&self, lists: Lists, duration: Option<Duration>, enforcers: Enforcers) -> Result<(), String> {
        let mut block = self.block.lock().unwrap();
        if block.as_ref().is_some_and(|b| b.until.is_none_or(|u| self.clock.now() < u)) {
            return Err("A block is already running".into());
        }
        let until = duration.map(|d| self.clock.now() + d);
        let new = Block {
            filters: Arc::new(lists.sites),
            allowed: Arc::new(lists.allowed),
            programs: Arc::new(lists.programs),
            until,
            enforcers,
            saved_at: Instant::now(),
        };
        // Written before the block goes live, so a block cannot be missed after a restart. A
        // block that cannot be written would not survive the app ending, so it does not start.
        self.save(&new).map_err(|e| format!("Cannot save the block, so it would not survive a restart: {e}"))?;
        *block = Some(new);
        Ok(())
    }

    /// Stops the block unless it is timed and still running; then returns the seconds left.
    pub fn stop(&self) -> Result<(), u64> {
        let mut block = self.block.lock().unwrap();
        if let Some(left) = block.as_ref().and_then(|b| remaining(b, self.clock.now())) {
            return Err(left);
        }
        *block = None;
        self.clear_saved();
        Ok(())
    }

    /// Picks up the block left behind by an earlier run, so restarting Windows does not end it.
    /// A block whose time is up while the app was not running is not resumed.
    pub fn restore(&self) {
        let Some(store) = &self.store else { return };
        let Some(saved) = store.load() else { return };
        // A block list we cannot read is dropped rather than resumed as something unknown.
        let (Ok(filters), Ok(allowed), Ok(programs)) =
            (FilterList::parse(&saved.filters), FilterList::parse(&saved.allowed), ProgramList::parse(&saved.programs))
        else {
            eprintln!("block: dropping an unreadable saved block");
            store.clear();
            return;
        };
        // Until internet time is in, the clock carries on from where the block last was, rather
        // than from a Windows clock that may have been set forward meanwhile.
        if let Some(seen) = saved.seen {
            self.clock.at_least(seen);
        }
        let until = saved.until.map(state::from_unix);
        if until.is_some_and(|u| self.clock.now() >= u) {
            store.clear();
            return;
        }
        let enforcers = saved.enforcers;
        *self.block.lock().unwrap() = Some(Block {
            filters: Arc::new(filters),
            allowed: Arc::new(allowed),
            programs: Arc::new(programs),
            until,
            enforcers,
            saved_at: Instant::now(),
        });
    }

    /// The active filters, ending a timed block whose time is up. `Some` while any block runs.
    pub fn filters(&self) -> Option<Arc<FilterList>> {
        self.active().map(|active| active.filters)
    }

    /// What is to be enforced, ending a timed block whose time is up.
    pub fn active(&self) -> Option<Active> {
        let mut block = self.block.lock().unwrap();
        if block.as_ref().is_some_and(|b| b.until.is_some_and(|u| self.clock.now() >= u)) {
            // A timed block ends by itself and leaves nothing behind to resume.
            *block = None;
            self.clear_saved();
        }
        // Keeps the time on disk moving, for a restart while offline (`Saved::seen`).
        if let Some(b) = block.as_mut().filter(|b| b.until.is_some() && b.saved_at.elapsed() >= CHECKPOINT) {
            b.saved_at = Instant::now();
            if let Err(e) = self.save(b) {
                eprintln!("block: {e}");
            }
        }
        block.as_ref().map(|b| Active {
            filters: b.filters.clone(),
            allowed: b.allowed.clone(),
            programs: b.programs.clone(),
            enforcers: b.enforcers,
        })
    }

    pub fn status(&self) -> Status {
        let blocking = self.filters().is_some();
        let block = self.block.lock().unwrap();
        Status {
            blocking,
            remaining: block.as_ref().and_then(|b| remaining(b, self.clock.now())),
            filters: block.as_ref().map(|b| b.filters.entries()).unwrap_or_default(),
            allowed: block.as_ref().map(|b| b.allowed.entries()).unwrap_or_default(),
            programs: block.as_ref().map(|b| b.programs.entries()).unwrap_or_default(),
            enforcers: block.as_ref().map(|b| b.enforcers),
            clock_synced: self.clock.synced(),
        }
    }

    fn save(&self, block: &Block) -> Result<(), String> {
        let Some(store) = &self.store else { return Ok(()) };
        store.save(&state::Saved {
            filters: block.filters.entries(),
            until: block.until.and_then(state::to_unix),
            seen: state::to_unix(self.clock.now()),
            enforcers: block.enforcers,
            programs: block.programs.entries(),
            allowed: block.allowed.entries(),
        })
    }

    /// Ends the block whatever its time, and forgets it. Only for the unlock password
    /// (`service::unlock`): everything else goes through `stop`, which keeps a timed block.
    pub fn reset(&self) {
        *self.block.lock().unwrap() = None;
        self.clear_saved();
    }

    /// Removes the saved block: there is no longer one to resume.
    pub fn clear_saved(&self) {
        if let Some(store) = &self.store {
            store.clear();
        }
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
        Blocklist { block: Arc::default(), clock: test_clock(), store: Some(Arc::new(Store::new(dir.join("state.json")))) }
    }

    fn blocklist(name: &str) -> Blocklist {
        blocklist_of(name, true)
    }

    /// The same store, as a second run of the app would find it.
    fn reopened(name: &str) -> Blocklist {
        blocklist_of(name, false)
    }

    /// Only sites to block; the program list is empty.
    fn sites(entries: &[&str]) -> Lists {
        Lists { sites: FilterList::parse(entries).unwrap(), allowed: FilterList::default(), programs: ProgramList::default() }
    }

    fn entries(blocklist: &Blocklist) -> Vec<String> {
        blocklist.status().filters
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
        list.start(sites(&["a.com"]), Some(Duration::from_millis(50)), Enforcers::default()).unwrap();
        assert_eq!(list.stop(), Err(1));
        assert!(list.filters().is_some());
        std::thread::sleep(Duration::from_millis(60));
        assert!(list.filters().is_none());
        assert!(list.stop().is_ok());

        list.start(sites(&["a.com"]), None, Enforcers::default()).unwrap();
        assert!(list.stop().is_ok());
        assert!(list.filters().is_none());
    }

    #[test]
    fn a_running_block_cannot_be_replaced() {
        let list = blocklist("replace");
        list.start(sites(&["a.com"]), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
        // Not for a shorter one, nor for an untimed one that could then be stopped.
        assert!(list.start(sites(&["a.com"]), Some(Duration::from_secs(1)), Enforcers::default()).is_err());
        assert!(list.start(sites(&["a.com"]), None, Enforcers::default()).is_err());
        assert!(list.status().remaining.is_some_and(|left| left > 3500));

        // An untimed block is stopped first, then a new one can start.
        let untimed = blocklist("replace-untimed");
        untimed.start(sites(&["a.com"]), None, Enforcers::default()).unwrap();
        assert!(untimed.start(sites(&["b.com"]), None, Enforcers::default()).is_err());
        untimed.stop().unwrap();
        untimed.start(sites(&["b.com"]), None, Enforcers::default()).unwrap();
        assert_eq!(entries(&untimed), vec!["b.com"]);
    }

    #[test]
    fn a_block_that_cannot_be_saved_does_not_start() {
        // A store whose folder is a file, so the state file can never be written.
        let dir = std::env::temp_dir().join(format!("block-{}-unsaved", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("not-a-folder"), "").unwrap();
        let list = Blocklist { block: Arc::default(), clock: test_clock(), store: Some(Arc::new(Store::new(dir.join("not-a-folder").join("state.json")))) };
        assert!(list.start(sites(&["a.com"]), Some(Duration::from_secs(60)), Enforcers::default()).is_err());
        assert!(list.filters().is_none());
    }

    #[test]
    fn a_block_outlives_the_app_that_started_it() {
        let list = blocklist("resume");
        list.start(sites(&["https://tweakers.net/", "b.com"]), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
        assert_eq!(entries(&list), vec!["tweakers.net", "b.com"]);

        // A new run of the app over the same store: the block is still going.
        let resumed = reopened("resume");
        resumed.restore();
        assert!(resumed.filters().unwrap().is_blocked("https://tweakers.net/nieuws"));
        assert!(resumed.status().remaining.is_some_and(|left| (3595..=3600).contains(&left)));
        assert_eq!(resumed.stop(), Err(resumed.status().remaining.unwrap()));

        // An untimed block is picked up as well: it runs until it is stopped.
        let untimed = blocklist("resume-untimed");
        untimed.start(sites(&["b.com"]), None, Enforcers::default()).unwrap();
        let resumed = reopened("resume-untimed");
        resumed.restore();
        assert!(resumed.filters().is_some());
        assert_eq!(resumed.status().remaining, None);
        assert!(resumed.stop().is_ok());
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
        list.start(lists, None, Enforcers::default()).unwrap();
        list.active().unwrap()
    }

    #[test]
    fn allowed_sites_win_over_the_block_list() {
        // Everything is blocked, except what is allowed.
        let active = with_allowed(&["*"], &["google.com", "*.wikipedia.org"]);
        assert!(active.blocks_site("https://youtube.com/watch?v=1"));
        assert!(active.blocks_site("https://tweakers.net/"));
        assert!(!active.blocks_site("https://google.com/search?q=a"));
        assert!(!active.blocks_site("https://www.google.com/"));
        assert!(!active.blocks_site("https://nl.wikipedia.org/wiki/Rust"));
        // An allowed entry is exactly as wide as it is written: not the sites around it.
        assert!(active.blocks_site("https://accounts.google.com/"));
        assert!(active.blocks_site("https://wikipedia.org/"));

        // A specific block and an exception inside it: the exception wins.
        let active = with_allowed(&["youtube.com"], &["youtube.com/watch*"]);
        assert!(active.blocks_site("https://youtube.com/"));
        assert!(active.blocks_site("https://youtube.com/shorts/abc"));
        assert!(!active.blocks_site("https://youtube.com/watch?v=1"));

        // Allowed everything is allowed everything, whatever the block list says.
        let active = with_allowed(&["youtube.com", "*.tweakers.net"], &["*"]);
        assert!(!active.blocks_site("https://youtube.com/") && !active.blocks_site("https://a.tweakers.net/"));
    }

    #[test]
    fn any_combination_of_the_two_lists_is_possible() {
        // Neither: nothing is blocked.
        let active = with_allowed(&[], &[]);
        assert!(!active.blocks_site("https://youtube.com/"));
        // Allowed alone: there is nothing to make exceptions to.
        let active = with_allowed(&[], &["google.com"]);
        assert!(!active.blocks_site("https://youtube.com/") && !active.blocks_site("https://google.com/"));
        // Blocked alone.
        let active = with_allowed(&["youtube.com"], &[]);
        assert!(active.blocks_site("https://youtube.com/") && !active.blocks_site("https://google.com/"));
        // The browser's own pages are neither blocked nor in need of an exception.
        let active = with_allowed(&["*"], &[]);
        assert!(!active.blocks_site("chrome://newtab/") && !active.blocks_site("about:blank"));
    }

    #[test]
    fn the_allowed_sites_survive_a_restart_and_older_blocks_have_none() {
        let list = blocklist("allowed-resume");
        let lists = Lists {
            sites: FilterList::parse(&["*"]).unwrap(),
            allowed: FilterList::parse(&["Google.com", "*.wikipedia.org"]).unwrap(),
            programs: ProgramList::default(),
        };
        list.start(lists, Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
        assert_eq!(list.status().allowed, vec!["google.com", "*.wikipedia.org"]);

        let resumed = reopened("allowed-resume");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.blocks_site("https://youtube.com/") && !active.blocks_site("https://google.com/"));
        assert_eq!(resumed.status().allowed, vec!["google.com", "*.wikipedia.org"]);

        // A saved block from before the allowed list resumes with none.
        let older = blocklist("allowed-older");
        let store = older.store.clone().unwrap();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        let until = state::now().unwrap().as_secs() + 3600;
        std::fs::write(store.path(), format!(r#"{{"filters":["a.com"],"until":{until}}}"#)).unwrap();
        let resumed = reopened("allowed-older");
        resumed.restore();
        assert!(resumed.status().allowed.is_empty() && resumed.active().unwrap().blocks_site("https://a.com/"));
    }

    #[test]
    fn a_resumed_block_keeps_its_programs() {
        let list = blocklist("programs");
        let lists = Lists {
            sites: FilterList::default(),
            allowed: FilterList::default(),
            programs: ProgramList::parse(&["Steam*", "game"]).unwrap(),
        };
        list.start(lists, Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
        assert_eq!(list.status().programs, vec!["steam*.exe", "game.exe"]);

        // A new run over the same store: the programs are still on the block.
        let resumed = reopened("programs");
        resumed.restore();
        let active = resumed.active().unwrap();
        assert!(active.programs.is_blocked("steamwebhelper.exe") && active.programs.is_blocked("GAME.EXE"));
        assert!(!active.programs.is_blocked("chrome.exe"));
        assert_eq!(resumed.status().programs, vec!["steam*.exe", "game.exe"]);
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
        assert!(active.filters.is_blocked("a.com") && active.programs.is_empty());
    }

    #[test]
    fn a_resumed_block_keeps_its_enforcers() {
        let only_obsidian = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true, exe_names: false };
        let list = blocklist("enforcers");
        list.start(sites(&[]), Some(Duration::from_secs(3600)), only_obsidian).unwrap();
        let resumed = reopened("enforcers");
        resumed.restore();
        assert_eq!(resumed.active().map(|active| active.enforcers), Some(only_obsidian));
        assert_eq!(resumed.status().enforcers, Some(only_obsidian));
    }

    #[test]
    fn a_block_whose_time_is_up_is_not_resumed() {
        let list = blocklist("expired");
        list.start(sites(&["a.com"]), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
        // Rewritten as though it had been left behind by a run that ended an hour ago.
        let store = list.store.clone().unwrap();
        store.save(&Saved {
            filters: vec!["a.com".to_string()],
            until: Some(state::now().unwrap().as_secs() - 3600),
            seen: None,
            enforcers: Enforcers::default(),
            programs: Vec::new(),
            allowed: Vec::new(),
        }).unwrap();

        let resumed = reopened("expired");
        resumed.restore();
        assert!(resumed.filters().is_none());
        assert_eq!(resumed.status().filters, Vec::<String>::new());
        // And it is cleaned up, rather than being re-checked on every run.
        assert_eq!(store.load(), None);
    }

    #[test]
    fn ending_a_block_clears_it() {
        let list = blocklist("cleared");
        let store = list.store.clone().unwrap();
        list.start(sites(&["a.com"]), None, Enforcers::default()).unwrap();
        assert!(store.load().is_some());
        list.stop().unwrap();
        assert_eq!(store.load(), None);

        // A timed block that ends by itself is cleared too, so nothing is left to resume.
        list.start(sites(&["a.com"]), Some(Duration::from_millis(50)), Enforcers::default()).unwrap();
        assert!(store.load().is_some());
        std::thread::sleep(Duration::from_millis(60));
        assert!(list.filters().is_none());
        assert_eq!(store.load(), None);
    }

    #[test]
    fn without_a_store_a_block_still_works_for_this_run() {
        let dir = std::env::temp_dir().join(format!("block-nostore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");

        let list = Blocklist { block: Arc::default(), clock: test_clock(), store: None };
        list.start(sites(&["a.com"]), Some(Duration::from_secs(60)), Enforcers::default()).unwrap();
        assert!(list.filters().is_some());
        list.restore();
        assert!(list.filters().is_some());
        assert!(!path.exists(), "a blocklist with no store must not write anything");
        assert!(!dir.exists(), "a blocklist with no store must not create a folder");
    }
}
