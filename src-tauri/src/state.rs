//! The running sessions on disk, so they survive the app ending and the machine restarting.
//!
//! The file is written when a session starts or ends, and deleted when the last one ends.
//!
//! The end time is a timestamp in internet time (`clock.rs`) rather than a running stopwatch,
//! because a stopwatch cannot be carried across a restart. Moving the Windows clock does not
//! change it.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::allowance::Run;
use crate::condition::Rule;
use crate::lists::Entries;
use crate::session::RunLock;

/// A running session as written to disk.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Saved {
    /// Which definition it was started from (`definitions.rs`).
    pub id: String,
    /// The definition's name when it started.
    pub name: String,
    /// Its targets, each with its own actions.
    pub parts: Vec<SavedPart>,
    /// The latest time the session was known to be running at, so the clock cannot start from an
    /// earlier one while offline (`clock::at_least`); milliseconds since the Unix epoch.
    pub seen: Option<u64>,
    /// How it is locked; a password lock only as its hash, never the password.
    pub lock: RunLock,
    /// When the session is active (`condition.rs`); `None` is always.
    pub condition: Option<Rule>,
    /// The time zone a schedule is read in, by name (`localtime::Zone::named`), fixed when the
    /// session started.
    pub zone: String,
    /// Its allowance and what is used of it (`allowance.rs`); `None`: it has none.
    #[serde(default)]
    pub allowance: Option<Run>,
}

/// One target of a running session, as written to disk.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedPart {
    pub name: String,
    #[serde(flatten)]
    pub entries: Entries,
    pub actions: Actions,
}

/// The ways a session is enforced, chosen when it starts and fixed while it runs, so turning one
/// off is not a way around a timed session. A list has no switch of its own: the sites, programs,
/// Store apps and window titles are acted on when the list has entries (an empty list is "off"), the
/// sites in every way a browser can show them (`tab_closer.rs`). Obsidian's web viewer, which has no
/// list, has a switch.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Actions {
    /// The windows whose title is on the list of titles are minimized instead of closed: nothing is
    /// lost. Not an action of its own but how that list is acted on.
    #[serde(default)]
    pub window_titles_minimize: bool,
    /// Closes Obsidian while it shows web content; not linked to the target.
    pub obsidian_web_viewer: bool,
    /// Closes Steam's built-in browser window (its title never says which page it shows); not
    /// linked to the target. Off in a new target.
    #[serde(default)]
    pub steam_browser: bool,
}

/// What an action needs to have something to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Uses {
    /// No list: it does its work on its own (Obsidian's).
    Nothing,
    /// The target of sites (`filters.rs`).
    Sites,
    /// The list of programs (`programs.rs`).
    Programs,
    /// The list of Store apps (`store_apps.rs`).
    StoreApps,
    /// The list of window titles (`patterns.rs`).
    Titles,
}

impl Actions {
    /// Every action as (turned on, what it needs). **The one place to list a new action**: add
    /// its field above (if it has a switch) and a line here; what may start and what runs follow
    /// from it. (A new kind of list also needs a `Uses` variant and an arm in `lists::Lists::has`.)
    fn all(&self) -> [(bool, Uses); 6] {
        [
            (self.obsidian_web_viewer, Uses::Nothing),
            (self.steam_browser, Uses::Nothing),
            // Acted on whenever their list has entries.
            (true, Uses::Sites),
            (true, Uses::Programs),
            (true, Uses::StoreApps),
            (true, Uses::Titles),
        ]
    }

    /// Whether `new` loosens these (for a locked session, that is a way out): an action turned off,
    /// or windows by title minimized where they were closed (closing them where they were minimized
    /// is stricter).
    pub fn turned_off_by(&self, new: &Actions) -> bool {
        (new.window_titles_minimize && !self.window_titles_minimize)
            || self.all().iter().zip(new.all()).any(|(&(was, _), (now, _))| was && !now)
    }

    /// Whether any action that is turned on needs this kind of list.
    pub fn uses(&self, list: Uses) -> bool {
        self.all().iter().any(|&(on, needs)| on && needs == list)
    }

    /// Whether a session with these actions has anything to do, given which lists have entries
    /// (`has_entries`): an action that needs a list has nothing to do with an empty one, the
    /// others do not need one. A session that cannot do anything is not worth starting.
    pub fn can_run(&self, has_entries: impl Fn(Uses) -> bool) -> bool {
        self.all().iter().any(|&(on, needs)| on && has_entries(needs))
    }
}

impl Default for Actions {
    fn default() -> Actions {
        Actions { obsidian_web_viewer: true, window_titles_minimize: false, steam_browser: false }
    }
}

/// Seconds since the Unix epoch on the Windows clock, for log lines.
pub fn now() -> Option<Duration> {
    SystemTime::now().duration_since(UNIX_EPOCH).ok()
}

/// A wall-clock time as milliseconds since the Unix epoch, the form times are stored in (whole
/// seconds would end a lock up to a second early).
pub fn to_unix(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().and_then(|d| u64::try_from(d.as_millis()).ok())
}

/// A wall-clock time read back from milliseconds since the Unix epoch.
pub fn from_unix(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

/// The file: the running sessions.
#[derive(Serialize, Deserialize)]
struct OnDisk {
    sessions: Vec<Saved>,
}

/// One state file, so tests can work in a directory of their own.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Store {
        Store { path }
    }

    /// The file this session is kept in, for tests and for telling the user where it is.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The saved sessions; none if there is no file, it is unreadable, or it is not ours.
    pub fn load(&self) -> Vec<Saved> {
        let read = fs::read(&self.path).ok().and_then(|bytes| serde_json::from_slice::<OnDisk>(&bytes).ok());
        read.map(|file| file.sessions).unwrap_or_default()
    }

    /// Writes the running sessions, replacing what was there; none at all removes the file. Through a
    /// temporary file, so that a crash or a power cut can never leave a half-written file behind
    /// that then fails to parse.
    pub fn save(&self, sessions: &[Saved]) -> Result<(), String> {
        if sessions.is_empty() {
            self.clear();
            return Ok(());
        }
        let json = serde_json::to_string(&OnDisk { sessions: sessions.to_vec() }).map_err(|e| e.to_string())?;
        let dir = self.path.parent().ok_or_else(|| format!("no folder for {}", self.path.display()))?;
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let temporary = self.path.with_extension("new");
        fs::write(&temporary, json).map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
        // `rename` replaces an existing file in one step, on Windows too, so the file is only
        // ever the old sessions or the new ones: never partly written, and never briefly absent.
        fs::rename(&temporary, &self.path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("cannot replace {}: {e}", self.path.display())
        })
    }

    /// Deletes the saved sessions: there are none left to resume.
    pub fn clear(&self) {
        if let Err(e) = fs::remove_file(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("state: cannot remove {}: {e}", self.path.display());
            }
        }
    }
}

/// A folder to use instead of the real one: development and the pipe test (`service.rs`).
static DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

#[cfg(any(debug_assertions, test))]
pub fn use_dir(dir: PathBuf) {
    let _ = DIR_OVERRIDE.set(dir);
}

/// The folder the service keeps its files in: `%ProgramData%\bullseye`, which the service
/// locks down to SYSTEM (`service::lock_down_data_dir`). The running sessions and the definitions
/// live here, so the user whose sessions they are cannot edit or delete them.
///
/// `None` only if there is no such folder, in which case the service still blocks for as long as
/// it runs, it just does not survive a restart.
pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = DIR_OVERRIDE.get() {
        return Some(dir.clone());
    }
    Some(PathBuf::from(std::env::var_os("ProgramData")?).join("bullseye"))
}

/// The app's own folder, for its log: `%LOCALAPPDATA%\bullseye`. Nothing in it matters to a session.
pub fn user_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("bullseye"))
}

/// The store the service uses: `%ProgramData%\bullseye\state.json`.
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
        let lock = until.map_or(RunLock::None, |at| RunLock::Ends { at });
        let part = SavedPart {
            name: "A".into(),
            entries: Entries { filters: vec!["tweakers.net".to_string()], ..Entries::default() },
            actions: Actions::default(),
        };
        Saved {
            id: "a".into(),
            name: "A".into(),
            parts: vec![part],
            seen: None,
            lock,
            condition: None,
            zone: "fixed:+0".into(),
            allowance: None,
        }
    }

    #[test]
    fn no_file_is_no_session() {
        let store = store("absent");
        assert!(store.load().is_empty());
        // Clearing something that is not there is not an error.
        store.clear();
    }

    #[test]
    fn saved_session_comes_back() {
        let store = store("roundtrip");
        let saved = timed(Some(to_unix(SystemTime::now()).unwrap() + 600_000));
        store.save(std::slice::from_ref(&saved)).unwrap();
        assert_eq!(store.load(), vec![saved.clone()]);
        // Saving again replaces rather than appends.
        let mut shorter = saved.clone();
        shorter.lock = RunLock::Ends { at: to_unix(SystemTime::now()).unwrap() + 60_000 };
        let other = Saved { id: "b".into(), ..timed(None) };
        store.save(&[shorter.clone(), other.clone()]).unwrap();
        assert_eq!(store.load(), vec![shorter, other]);
        // None left removes the file.
        store.save(&[]).unwrap();
        assert!(!store.path().exists());
        assert!(store.load().is_empty());
    }

    #[test]
    fn leftovers_are_cleaned_up() {
        let store = store("leftovers");
        store.save(&[timed(Some(to_unix(SystemTime::now()).unwrap() + 600_000))]).unwrap();
        // A power cut during the write, or a file edited by hand: unreadable, and must not
        // keep the app from starting.
        fs::write(store.path(), "{ not json").unwrap();
        assert!(store.load().is_empty());
        store.clear();
        // The temporary file the save goes through must not outlive it either.
        store.save(&[timed(None)]).unwrap();
        assert!(!store.path().with_extension("new").exists());
        store.clear();
    }

    #[test]
    fn what_can_run_depends_on_the_actions_and_the_lists() {
        let only = |obsidian| Actions { obsidian_web_viewer: obsidian, window_titles_minimize: false, steam_browser: false };
        // Which lists have entries.
        let lists = |sites: bool, programs: bool| {
            move |uses: Uses| match uses {
                Uses::Nothing => true,
                Uses::Sites => sites,
                Uses::Programs => programs,
                Uses::StoreApps | Uses::Titles => false,
            }
        };
        // Every list is acted on by its entries.
        assert!(only(false).can_run(lists(true, false)));
        assert!(only(false).can_run(lists(false, true)));
        for list in [Uses::Sites, Uses::Programs, Uses::StoreApps, Uses::Titles] {
            assert!(only(false).uses(list), "{list:?}");
        }
        // Obsidian's needs no list; without it and without entries, nothing runs.
        assert!(only(true).can_run(lists(false, false)) && only(true).uses(Uses::Nothing));
        assert!(!only(false).can_run(lists(false, false)) && !only(false).uses(Uses::Nothing));
    }

    #[test]
    fn wall_clock_round_trips() {
        let time = from_unix(1_700_000_000_123);
        assert_eq!(to_unix(time), Some(1_700_000_000_123));
    }

    /// The only test that points the service's folder somewhere (it can be set once per process):
    /// everything the service keeps goes there.
    #[test]
    fn the_service_keeps_its_files_in_its_data_folder() {
        let dir = std::env::temp_dir().join(format!("state-{}-data", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        use_dir(dir.clone());
        assert_eq!(super::dir(), Some(dir.clone()));
        assert_eq!(super::store().map(|store| store.path().to_path_buf()), Some(dir.join("state.json")));
        crate::definitions::Definitions::new().save_target(crate::definitions::tests::target("l", &[])).unwrap();
        assert!(dir.join("sessions.json").exists());
        crate::unlock::UnlockDelay::new().set_delay(3600, SystemTime::now()).unwrap();
        assert!(dir.join("unlock.json").exists());
        let sessions = crate::session::Sessions::new();
        assert!(!sessions.is_running());
        // The app's own folder, for its log, is the user's.
        assert_eq!(user_dir(), std::env::var_os("LOCALAPPDATA").map(|local| PathBuf::from(local).join("bullseye")));
        assert!(now().is_some_and(|since| since.as_secs() > 1_700_000_000));
    }

    #[test]
    fn windows_by_title_are_closed_unless_minimized_and_closing_is_stricter() {
        let minimize = |on| Actions { window_titles_minimize: on, ..Actions::default() };
        assert!(!Actions::default().window_titles_minimize, "a new target closes them");
        // A locked session may go from minimizing to closing, never back; nor turn an action off.
        assert!(!minimize(true).turned_off_by(&minimize(false)), "closing instead of minimizing is stricter");
        assert!(minimize(false).turned_off_by(&minimize(true)), "minimizing instead of closing is not");
        assert!(!minimize(false).turned_off_by(&minimize(false)));
        let obsidian = |on| Actions { obsidian_web_viewer: on, ..Actions::default() };
        assert!(obsidian(true).turned_off_by(&obsidian(false)) && !obsidian(false).turned_off_by(&obsidian(true)));
    }

    #[test]
    fn steams_browser_is_an_action_of_its_own_that_is_off_in_a_new_target() {
        let steam = |on| Actions { steam_browser: on, ..Actions::default() };
        assert!(!Actions::default().steam_browser, "a new target leaves Steam's browser alone");
        // It needs no list, so a session with only it has something to do.
        let nothing = |uses: Uses| uses == Uses::Nothing; // no list has entries; the action needs none
        let only_steam = Actions { obsidian_web_viewer: false, window_titles_minimize: false, steam_browser: true };
        assert!(only_steam.can_run(nothing) && only_steam.uses(Uses::Nothing));
        // A locked session cannot turn it off; turning it on is stricter.
        assert!(steam(true).turned_off_by(&steam(false)));
        assert!(!steam(false).turned_off_by(&steam(true)) && !steam(true).turned_off_by(&steam(true)));
    }
}
