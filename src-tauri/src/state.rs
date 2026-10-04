//! The running block on disk, so it survives the app ending and the machine restarting.
//!
//! The file is written when a block starts and deleted when one ends, which doubles as how
//! the watchdog (`watchdog.rs`) knows there is a timed block to keep alive.
//!
//! The end time is a timestamp in internet time (`clock.rs`) rather than a running stopwatch,
//! because a stopwatch cannot be carried across a restart. Moving the Windows clock does not
//! change it; deleting the file still ends the block, as an administrator can.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A block as written to disk.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Saved {
    /// One blocked entry per line, as typed in the UI.
    pub filters: Vec<String>,
    /// Seconds since the Unix epoch at which the block ends; `None` blocks until stopped.
    pub until: Option<u64>,
    /// The latest time the block was known to be running at, so the clock cannot start from an
    /// earlier one while offline (`clock::at_least`). Missing in files from older versions.
    #[serde(default)]
    pub seen: Option<u64>,
    /// Which enforcers the block runs. Missing in files from older versions, which ran all.
    #[serde(default)]
    pub enforcers: Enforcers,
    /// The programs to close, one entry per line, as typed in the UI. Missing in files from older
    /// versions, which had none.
    #[serde(default)]
    pub programs: Vec<String>,
}

/// The ways a block is enforced, chosen when it starts and fixed while it runs, so turning one
/// off is not a way around a timed block.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Enforcers {
    /// Closes a Chromium tab whose page is on the block list (`tab_closer.rs`).
    pub chromium_tabs: bool,
    /// Closes a blocked page shown in either half of a Chromium split view.
    pub chromium_split_view: bool,
    /// Closes Obsidian while it shows web content; not linked to the block list.
    pub obsidian_web_viewer: bool,
    /// Closes the programs on the program list (`programs.rs`).
    pub exe_names: bool,
}

/// What an enforcer needs to have something to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Uses {
    /// No list: it does its work on its own (Obsidian's).
    Nothing,
    /// The block list of sites (`filters.rs`).
    Sites,
    /// The list of programs (`programs.rs`).
    Programs,
}

impl Enforcers {
    /// Every enforcer as (turned on, what it needs). **The one place to list a new enforcer**: add
    /// its field above and a line here; what may start and what runs follow from it. (A new kind of
    /// list also needs a `Uses` variant and an arm in `block::Lists::has`.)
    fn all(&self) -> Vec<(bool, Uses)> {
        vec![
            (self.chromium_tabs, Uses::Sites),
            (self.chromium_split_view, Uses::Sites),
            (self.obsidian_web_viewer, Uses::Nothing),
            (self.exe_names, Uses::Programs),
        ]
    }

    /// Whether any enforcer is turned on.
    pub fn any(&self) -> bool {
        self.all().iter().any(|&(on, _)| on)
    }

    /// Whether any enforcer that is turned on needs this kind of list.
    pub fn uses(&self, list: Uses) -> bool {
        self.all().iter().any(|&(on, needs)| on && needs == list)
    }

    /// Whether a block with these enforcers has anything to do, given which lists have entries
    /// (`has_entries`): an enforcer that needs a list has nothing to do with an empty one, the
    /// others do not need one. A block that cannot do anything is not worth starting.
    pub fn can_run(&self, has_entries: impl Fn(Uses) -> bool) -> bool {
        self.all().iter().any(|&(on, needs)| on && has_entries(needs))
    }
}

impl Default for Enforcers {
    fn default() -> Enforcers {
        Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: true, exe_names: true }
    }
}

/// Seconds since the Unix epoch on the Windows clock, for log lines.
pub fn now() -> Option<Duration> {
    SystemTime::now().duration_since(UNIX_EPOCH).ok()
}

/// A wall-clock time as seconds since the Unix epoch, the form a deadline is stored in.
pub fn to_unix(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// A wall-clock time read back from seconds since the Unix epoch.
pub fn from_unix(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

/// One state file, so tests can work in a directory of their own.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Store {
        Store { path }
    }

    /// The file this block is kept in, for tests and for telling the user where it is.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The saved block, or `None` if there is none, it is unreadable, or it is not ours.
    pub fn load(&self) -> Option<Saved> {
        let mut bytes = fs::read(&self.path).ok()?;
        // Windows tools write a UTF-8 byte order mark, which serde_json rejects as part of the
        // JSON. Skipping it means such a file still resumes rather than silently reading as no
        // block at all.
        if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            bytes.drain(..3);
        }
        serde_json::from_slice(&bytes).ok()
    }

    /// Writes the block, replacing any previous one. Through a temporary file, so that a crash
    /// or a power cut can never leave a half-written block behind that then fails to parse.
    pub fn save(&self, saved: &Saved) -> Result<(), String> {
        let json = serde_json::to_string(saved).map_err(|e| e.to_string())?;
        let dir = self.path.parent().ok_or_else(|| format!("no folder for {}", self.path.display()))?;
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let temporary = self.path.with_extension("new");
        fs::write(&temporary, json).map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
        // `rename` replaces an existing file in one step, on Windows too, so the file is only
        // ever the old block or the new one: never partly written, and never briefly absent
        // (which the launcher would read as "no block left to watch").
        fs::rename(&temporary, &self.path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("cannot replace {}: {e}", self.path.display())
        })
    }

    /// Deletes the saved block, which is what tells the watchdog there is nothing to keep alive.
    pub fn clear(&self) {
        if let Err(e) = fs::remove_file(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("state: cannot remove {}: {e}", self.path.display());
            }
        }
    }
}

/// A folder to use instead of the real one, for `--service-console` (`service.rs`).
static DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

pub fn use_dir(dir: PathBuf) {
    let _ = DIR_OVERRIDE.set(dir);
}

/// The folder the service keeps its files in: `%ProgramData%\SiteBlocker`, which the service
/// locks down to SYSTEM (`service::lock_down_data_dir`). The saved block lives here, so the user
/// whose block it is cannot edit or delete it.
///
/// `None` only if there is no such folder, in which case the service still blocks for as long as
/// it runs, it just does not survive a restart.
pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = DIR_OVERRIDE.get() {
        return Some(dir.clone());
    }
    Some(PathBuf::from(std::env::var_os("ProgramData")?).join("SiteBlocker"))
}

/// The app's own folder, for its log: `%LOCALAPPDATA%\SiteBlocker`. Nothing in it matters to a block.
pub fn user_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("SiteBlocker"))
}

/// The store the service uses: `%ProgramData%\SiteBlocker\state.json`.
pub fn store() -> Option<Store> {
    Some(Store::new(dir()?.join("state.json")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store in a directory of its own, removed again when the test ends.
    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("site-block-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::new(dir.join("state.json"))
    }

    fn timed(until: Option<u64>) -> Saved {
        Saved { filters: vec!["tweakers.net".to_string()], until, seen: None, enforcers: Enforcers::default(), programs: Vec::new() }
    }

    #[test]
    fn no_file_is_no_block() {
        let store = store("absent");
        assert_eq!(store.load(), None);
        // Clearing something that is not there is not an error.
        store.clear();
    }

    #[test]
    fn saved_block_comes_back() {
        let store = store("roundtrip");
        let saved = timed(Some(now().unwrap().as_secs() + 600));
        store.save(&saved).unwrap();
        assert_eq!(store.load().as_ref(), Some(&saved));
        // Saving again replaces rather than appends.
        let mut shorter = saved.clone();
        shorter.until = Some(now().unwrap().as_secs() + 60);
        store.save(&shorter).unwrap();
        assert_eq!(store.load().as_ref(), Some(&shorter));
        store.clear();
        assert_eq!(store.load(), None);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_block() {
        let store = store("bom");
        let saved = timed(Some(now().unwrap().as_secs() + 600));
        store.save(&saved).unwrap();
        // PowerShell's `Set-Content -Encoding utf8` and other Windows tools add one.
        let mut with_bom = vec![0xef, 0xbb, 0xbf];
        with_bom.extend_from_slice(serde_json::to_string(&saved).unwrap().as_bytes());
        fs::write(store.path(), with_bom).unwrap();
        assert_eq!(store.load().as_ref(), Some(&saved));
        store.clear();
    }

    #[test]
    fn leftovers_are_cleaned_up() {
        let store = store("leftovers");
        store.save(&timed(Some(now().unwrap().as_secs() + 600))).unwrap();
        // A power cut during the write, or a file edited by hand: unreadable, and must not
        // keep the app from starting.
        fs::write(store.path(), "{ not json").unwrap();
        assert_eq!(store.load(), None);
        store.clear();
        // The temporary file the save goes through must not outlive it either.
        store.save(&timed(None)).unwrap();
        assert!(!store.path().with_extension("new").exists());
        store.clear();
    }


    #[test]
    fn files_from_before_seen_still_load() {
        let store = store("old");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), r#"{"filters":["a.com"],"until":1800000000}"#).unwrap();
        assert_eq!(store.load(), Some(Saved { filters: vec!["a.com".into()], until: Some(1_800_000_000), seen: None, enforcers: Enforcers::default(), programs: Vec::new() }));
        store.clear();
    }

    #[test]
    fn what_can_run_depends_on_the_enforcers_and_the_lists() {
        let only = |tabs, split, obsidian, exe| Enforcers {
            chromium_tabs: tabs,
            chromium_split_view: split,
            obsidian_web_viewer: obsidian,
            exe_names: exe,
        };
        // Which lists have entries.
        let lists = |sites: bool, programs: bool| move |uses: Uses| match uses {
            Uses::Nothing => true,
            Uses::Sites => sites,
            Uses::Programs => programs,
        };
        // With entries, any enforcer that is on can run.
        assert!(only(true, false, false, false).can_run(lists(true, false)));
        assert!(only(false, true, false, false).can_run(lists(true, false)));
        assert!(only(false, false, true, false).can_run(lists(false, false)));
        assert!(only(false, false, false, true).can_run(lists(false, true)));
        // An enforcer needs its own list: sites do not help the programs one, nor the other way round.
        assert!(!only(false, false, false, true).can_run(lists(true, false)));
        assert!(!only(true, false, false, false).can_run(lists(false, true)));
        // Without any entries, only one that needs no list can run, also next to ones that do.
        assert!(only(true, true, true, true).can_run(lists(false, false)));
        assert!(Enforcers::default().can_run(lists(false, false)));
        assert!(!only(true, true, false, true).can_run(lists(false, false)));
        // And with nothing turned on, nothing can.
        assert!(!only(false, false, false, false).can_run(lists(true, true)));
        // The checks the closing code and the start use agree with it.
        let programs_only = only(false, false, false, true);
        assert!(programs_only.uses(Uses::Programs) && !programs_only.uses(Uses::Sites) && programs_only.any());
        assert!(only(true, false, false, false).uses(Uses::Sites));
        assert!(!only(false, false, false, false).any());
    }

    #[test]
    fn wall_clock_round_trips() {
        let time = from_unix(1_700_000_000);
        assert_eq!(to_unix(time), Some(1_700_000_000));
    }
}