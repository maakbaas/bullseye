//! What the user has defined, in three kinds, each a list of its own (`Library`):
//! - **block lists** (`BlockList`): what to block and how: sites, always-allowed sites, programs,
//!   and the enforcers that act on them;
//! - **schedules** (`NamedSchedule`): when, as a weekly schedule (`schedule.rs`); "always" is built
//!   in and is no entry here;
//! - **blocks** (`Definition`): one or more block lists, a schedule (or always) and a lock
//!   (`lock.rs`). A block is what is started and runs.
//!
//! All of it is kept by the service (`%ProgramData%\bullseye\blocks.json`), so it survives
//! restarts, and is only a template: starting a block copies its lists and schedule into a running
//! block (`block.rs`), which is then fixed until it ends. So that what is shown and what runs stay
//! the same, a running block's definition, and every block list and schedule it uses, cannot be
//! changed or deleted until it ends (`handler.rs`). A block list or schedule that any block uses
//! cannot be deleted either.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::block::Lists;
use crate::filters::FilterList;
use crate::lock::Lock;
use crate::programs::ProgramList;
use crate::schedule::Week;
use crate::state::{self, Enforcers};
use crate::sync::LockExt;
use crate::unlock;

/// More than anyone needs, of each kind; keeps the file and the status message small.
const MAX_ITEMS: usize = 50;
const MAX_NAME: usize = 60;
const MAX_ID: usize = 64;

/// What to block, and how.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BlockList {
    /// Made by the UI or the CLI; letters, digits, `-` and `_`.
    pub id: String,
    pub name: String,
    /// The sites to block, one entry per line, as typed.
    pub filters: Vec<String>,
    /// The sites that are always allowed, which win over `filters` (of this list only).
    pub allowed: Vec<String>,
    /// The programs to close (`programs.rs`).
    pub programs: Vec<String>,
    pub enforcers: Enforcers,
}

/// A weekly schedule with a name. **A new kind of schedule** (other than weekly) would become a
/// field here, with what it means in `block::Block::in_schedule`, and an editor in the UI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NamedSchedule {
    pub id: String,
    pub name: String,
    /// When it is active (`days`, Monday first).
    #[serde(flatten)]
    pub week: Week,
}

/// A block: what is started. Its lists by id, its schedule by id (`None`: always active) and its
/// lock.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Definition {
    pub id: String,
    pub name: String,
    /// The block lists it enforces, by id.
    pub lists: Vec<String>,
    /// The schedule it is active by, by id; `None` is always.
    pub schedule: Option<String>,
    /// How it is kept from being stopped once started (`lock.rs`).
    pub lock: Lock,
    /// The last settings of each other lock method it has had (a duration, an end, a lock
    /// schedule, a password's hash), so that choosing one again starts from them. Kept by the
    /// service (`Definitions::save_block`): what a client sends here is ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept_locks: Vec<Lock>,
}

/// Everything the user has defined, each kind in the order it was made.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Library {
    pub lists: Vec<BlockList>,
    pub schedules: Vec<NamedSchedule>,
    pub blocks: Vec<Definition>,
}

/// A block with its parts looked up and checked: what starting it needs.
pub struct Resolved {
    pub block: Definition,
    pub lists: Vec<BlockList>,
    pub week: Option<Week>,
    /// The week of its scheduled lock, if it has one.
    pub lock_week: Option<Week>,
}

/// An id and a name as every kind needs them.
fn check_name(kind: &str, id: &str, name: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_ID || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("Invalid {kind} id"));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("Give the {kind} a name"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("Keep the name under {MAX_NAME} characters"));
    }
    Ok(())
}

impl BlockList {
    /// The lists, read and checked. Also refuses programs Windows needs (`programs.rs`).
    pub fn lists(&self) -> Result<Lists, String> {
        let sites = FilterList::parse(&self.filters).map_err(|_| format!("{}: invalid block list syntax", self.name))?;
        let allowed = FilterList::parse(&self.allowed).map_err(|_| format!("{}: invalid syntax in the list of allowed sites", self.name))?;
        let programs = ProgramList::parse(&self.programs).map_err(|why| format!("{}: {why}", self.name))?;
        Ok(Lists { sites, allowed, programs })
    }

    /// What can be checked without starting a block with it. Whether it has anything to do is only
    /// checked when a block starts, so a list can be saved half done.
    fn check(&self) -> Result<(), String> {
        check_name("block list", &self.id, &self.name)?;
        self.lists().map(|_| ())
    }
}

impl NamedSchedule {
    /// The week, sorted, merged and checked: refused if it has no time in it.
    fn checked(mut self) -> Result<NamedSchedule, String> {
        check_name("schedule", &self.id, &self.name)?;
        self.week = self.week.normalized()?;
        Ok(self)
    }
}

impl Library {
    /// As a client may see it: password locks without their hashes (`Lock::redacted`).
    pub fn redacted(mut self) -> Library {
        for block in &mut self.blocks {
            block.lock = block.lock.redacted();
            block.kept_locks = block.kept_locks.iter().map(Lock::redacted).collect();
        }
        self
    }

    pub fn list(&self, id: &str) -> Option<&BlockList> {
        self.lists.iter().find(|l| l.id == id)
    }

    pub fn schedule(&self, id: &str) -> Option<&NamedSchedule> {
        self.schedules.iter().find(|s| s.id == id)
    }

    pub fn block(&self, id: &str) -> Option<&Definition> {
        self.blocks.iter().find(|b| b.id == id)
    }

    /// The blocks that use a block list or a schedule (as their schedule or their lock's), by id.
    pub fn users_of(&self, part: &str) -> Vec<&Definition> {
        self.blocks
            .iter()
            .filter(|b| b.lists.iter().any(|l| l == part) || b.schedule.as_deref() == Some(part) || b.lock.schedule() == Some(part))
            .collect()
    }

    /// A schedule's week, checked.
    fn week_of(&self, id: &str) -> Result<Week, String> {
        self.schedule(id).ok_or("A schedule of this block is gone")?.week.clone().normalized()
    }

    /// A block with its parts, ready to start. Refused if it has no block list, or a part is gone.
    pub fn resolve(&self, id: &str) -> Result<Resolved, String> {
        let block = self.block(id).ok_or("No such block")?.clone();
        if block.lists.is_empty() {
            return Err(format!("{}: choose at least one block list", block.name));
        }
        let lists = block.lists.iter().map(|l| self.list(l).cloned().ok_or("A block list of this block is gone")).collect::<Result<Vec<_>, _>>()?;
        let week = block.schedule.as_deref().map(|s| self.week_of(s)).transpose()?;
        let lock_week = block.lock.schedule().map(|s| self.week_of(s)).transpose()?;
        Ok(Resolved { block, lists, week, lock_week })
    }

    /// What can be checked of a block without starting it: its name, that its parts exist, its lock.
    fn check_block(&self, block: &Definition) -> Result<(), String> {
        check_name("block", &block.id, &block.name)?;
        for list in &block.lists {
            self.list(list).ok_or_else(|| format!("No block list with id {list}"))?;
        }
        if let Some(schedule) = &block.schedule {
            self.schedule(schedule).ok_or_else(|| format!("No schedule with id {schedule}"))?;
        }
        block.lock.check()?;
        if let Some(schedule) = block.lock.schedule() {
            self.schedule(schedule).ok_or_else(|| format!("No schedule with id {schedule} for the lock"))?;
        }
        Ok(())
    }
}

/// Adds `item`, or replaces the one with its id, in its place.
fn put<T>(items: &mut Vec<T>, item: T, same: impl Fn(&T) -> bool) -> Result<(), String> {
    match items.iter().position(same) {
        Some(at) => items[at] = item,
        None if items.len() >= MAX_ITEMS => return Err(format!("At most {MAX_ITEMS} of each")),
        None => items.push(item),
    }
    Ok(())
}

/// The library, and the file it is kept in.
/// What a block keeps of the other lock methods once its lock is `lock`: what it kept before and
/// the lock it had, the latest of each method that has settings, without `lock`'s own method.
fn kept_locks(old: Option<&Definition>, lock: &Lock) -> Vec<Lock> {
    let mut kept: Vec<Lock> = old.map(|old| old.kept_locks.clone()).unwrap_or_default();
    if let Some(previous) = old.map(|old| &old.lock).filter(|previous| previous.has_settings()) {
        kept.retain(|k| !k.same_method(previous));
        kept.push(previous.clone());
    }
    kept.retain(|k| !k.same_method(lock));
    kept
}

pub struct Definitions {
    library: Mutex<Library>,
    /// `None`: kept for this run only (no data folder, or a test).
    path: Option<PathBuf>,
}

impl Definitions {
    /// The service's: `%ProgramData%\bullseye\blocks.json`, read back from the last run.
    pub fn new() -> Definitions {
        Definitions::at(state::dir().map(|dir| dir.join("blocks.json")))
    }

    pub fn at(path: Option<PathBuf>) -> Definitions {
        let library = path.as_ref().and_then(load).unwrap_or_default();
        Definitions { library: Mutex::new(library), path }
    }

    pub fn library(&self) -> Library {
        self.library.locked().clone()
    }

    /// Changes the library with `change`, which may refuse; written to disk before it counts.
    fn change(&self, change: impl FnOnce(&mut Library) -> Result<(), String>) -> Result<(), String> {
        let mut library = self.library.locked();
        let mut new = library.clone();
        change(&mut new)?;
        self.write(&new)?;
        *library = new;
        Ok(())
    }

    /// A password lock's password is turned into a salted hash here and never kept itself; saved
    /// without a new password, a password lock keeps the hash it has.
    pub fn save_block(&self, mut block: Definition) -> Result<(), String> {
        block.name = block.name.trim().to_string();
        self.change(|library| {
            let old = library.block(&block.id).cloned();
            if let Lock::Password { password, hash } = &mut block.lock {
                *hash = match password.take() {
                    Some(new) if new.is_empty() => return Err("The password of a password lock cannot be empty".into()),
                    Some(new) => Some(unlock::new_hash(&new)?),
                    // A hash from a client is not taken: only one this service made, the block's
                    // own or the one kept from when it last had a password lock.
                    None => old.as_ref().and_then(|old| old.lock.password_hash().or_else(|| old.kept_locks.iter().find_map(Lock::password_hash))).map(str::to_string),
                };
                if hash.is_none() {
                    return Err("Set a password for the password lock".into());
                }
            }
            library.check_block(&block)?;
            block.kept_locks = kept_locks(old.as_ref(), &block.lock);
            let id = block.id.clone();
            put(&mut library.blocks, block, |b| b.id == id)
        })
    }

    pub fn delete_block(&self, id: &str) -> Result<(), String> {
        self.change(|library| {
            library.block(id).ok_or("No such block")?;
            library.blocks.retain(|b| b.id != id);
            Ok(())
        })
    }

    pub fn save_list(&self, mut list: BlockList) -> Result<(), String> {
        list.name = list.name.trim().to_string();
        list.check()?;
        self.change(|library| {
            let id = list.id.clone();
            put(&mut library.lists, list, |l| l.id == id)
        })
    }

    pub fn delete_list(&self, id: &str) -> Result<(), String> {
        self.change(|library| {
            library.list(id).ok_or("No such block list")?;
            refuse_if_used(library, id, "block list")?;
            library.lists.retain(|l| l.id != id);
            Ok(())
        })
    }

    pub fn save_schedule(&self, mut schedule: NamedSchedule) -> Result<(), String> {
        schedule.name = schedule.name.trim().to_string();
        let schedule = schedule.checked()?;
        self.change(|library| {
            let id = schedule.id.clone();
            put(&mut library.schedules, schedule, |s| s.id == id)
        })
    }

    pub fn delete_schedule(&self, id: &str) -> Result<(), String> {
        self.change(|library| {
            library.schedule(id).ok_or("No such schedule")?;
            refuse_if_used(library, id, "schedule")?;
            library.schedules.retain(|s| s.id != id);
            Ok(())
        })
    }

    /// Through a temporary file, like `state::Store::save`, so a crash leaves the old file or the new.
    fn write(&self, library: &Library) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_string_pretty(library).map_err(|e| e.to_string())?;
        let dir = path.parent().ok_or_else(|| format!("no folder for {}", path.display()))?;
        fs::create_dir_all(dir).map_err(|e| format!("Cannot save: {e}"))?;
        let temporary = path.with_extension("new");
        fs::write(&temporary, json).map_err(|e| format!("Cannot save: {e}"))?;
        fs::rename(&temporary, path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("Cannot save: {e}")
        })
    }
}

/// A block list or schedule that blocks use cannot be deleted; the message says which blocks.
fn refuse_if_used(library: &Library, id: &str, kind: &str) -> Result<(), String> {
    let users: Vec<&str> = library.users_of(id).iter().map(|b| b.name.as_str()).collect();
    match users.as_slice() {
        [] => Ok(()),
        names => Err(format!("This {kind} is used by {}; take it out of them first", names.join(", "))),
    }
}

fn load(path: &PathBuf) -> Option<Library> {
    let bytes = fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(library) => Some(library),
        Err(e) => {
            eprintln!("definitions: cannot read {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A block list with these sites and the default enforcers.
    pub fn block_list(id: &str, filters: &[&str]) -> BlockList {
        BlockList {
            id: id.into(),
            name: format!("List {id}"),
            filters: filters.iter().map(|f| f.to_string()).collect(),
            allowed: Vec::new(),
            programs: Vec::new(),
            enforcers: Enforcers::default(),
        }
    }

    /// A block of these lists, always active and not locked.
    pub fn block(id: &str, lists: &[&str]) -> Definition {
        Definition { id: id.into(), name: format!("Block {id}"), lists: lists.iter().map(|l| l.to_string()).collect(), schedule: None, lock: Lock::None, kept_locks: Vec::new() }
    }

    pub fn weekly(id: &str, text: &str) -> NamedSchedule {
        NamedSchedule { id: id.into(), name: format!("Schedule {id}"), week: Week::parse(text).unwrap().unwrap() }
    }

    fn path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("definitions-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("blocks.json")
    }

    #[test]
    fn everything_is_kept_across_runs() {
        let path = path("kept");
        let definitions = Definitions::at(Some(path.clone()));
        definitions.save_list(block_list("social", &["youtube.com"])).unwrap();
        definitions.save_list(block_list("games", &[])).unwrap();
        definitions.save_schedule(weekly("work", "mon-fri 8-17")).unwrap();
        let mut work = block("w", &["social", "games"]);
        work.schedule = Some("work".into());
        work.lock = Lock::Duration { duration: "1h".into() };
        definitions.save_block(work.clone()).unwrap();
        // Saving one with the same id replaces it, in its place.
        definitions.save_list(block_list("social", &["reddit.com"])).unwrap();

        let again = Definitions::at(Some(path)).library();
        assert_eq!(again.lists.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), vec!["social", "games"]);
        assert_eq!(again.list("social").unwrap().filters, vec!["reddit.com"]);
        assert_eq!(again.block("w").unwrap().lists, vec!["social", "games"]);
        let resolved = again.resolve("w").unwrap();
        assert_eq!(resolved.lists.len(), 2);
        assert_eq!(resolved.week.unwrap().describe(), "Mon-Fri 08:00-17:00");
    }

    #[test]
    fn what_cannot_be_saved() {
        let definitions = Definitions::at(None);
        definitions.save_list(block_list("l", &["a.com"])).unwrap();
        let bad_lists = [
            BlockList { name: "  ".into(), ..block_list("x", &[]) },
            BlockList { name: "x".repeat(MAX_NAME + 1), ..block_list("x", &[]) },
            block_list("../x", &[]),
            block_list("x", &["has space.com"]),
            BlockList { programs: vec!["explorer.exe".into()], ..block_list("x", &[]) },
        ];
        for list in bad_lists {
            assert!(definitions.save_list(list.clone()).is_err(), "{list:?}");
        }
        // A schedule with no time in it is never active.
        assert!(definitions.save_schedule(NamedSchedule { id: "s".into(), name: "S".into(), week: Week::default() }).is_err());
        let bad_blocks = [
            block("b", &["nope"]),
            Definition { schedule: Some("nope".into()), ..block("b", &["l"]) },
            Definition { lock: Lock::Duration { duration: "soon".into() }, ..block("b", &["l"]) },
            Definition { lock: Lock::Duration { duration: "".into() }, ..block("b", &["l"]) },
            Definition { lock: Lock::Scheduled { schedule: "nope".into() }, ..block("b", &["l"]) },
        ];
        for b in bad_blocks {
            assert!(definitions.save_block(b.clone()).is_err(), "{b:?}");
        }
        assert!(definitions.library().blocks.is_empty());
        // Half done is fine to save: no lists yet. It is checked when it starts.
        definitions.save_block(block("b", &[])).unwrap();
        assert!(definitions.library().resolve("b").is_err());
    }

    #[test]
    fn a_list_or_schedule_in_use_cannot_be_deleted() {
        let definitions = Definitions::at(None);
        definitions.save_list(block_list("l", &["a.com"])).unwrap();
        definitions.save_schedule(weekly("s", "weekends")).unwrap();
        definitions.save_schedule(weekly("s2", "weekdays")).unwrap();
        definitions.save_block(Definition { schedule: Some("s".into()), lock: Lock::Scheduled { schedule: "s2".into() }, ..block("b", &["l"]) }).unwrap();
        let why = definitions.delete_list("l").unwrap_err();
        assert!(why.contains("Block b"), "{why}");
        assert!(definitions.delete_schedule("s").is_err());
        // A schedule used by a lock is in use too.
        assert!(definitions.delete_schedule("s2").is_err());
        definitions.delete_block("b").unwrap();
        definitions.delete_list("l").unwrap();
        definitions.delete_schedule("s").unwrap();
        definitions.delete_schedule("s2").unwrap();
        assert!(definitions.delete_list("l").is_err(), "gone already");
    }

    #[test]
    fn a_password_lock_keeps_only_a_hash_and_shows_neither() {
        let path = path("password");
        let definitions = Definitions::at(Some(path.clone()));
        definitions.save_list(block_list("l", &["a.com"])).unwrap();
        let with = |password: Option<&str>, hash: Option<&str>| Definition {
            lock: Lock::Password { password: password.map(str::to_string), hash: hash.map(str::to_string) },
            ..block("b", &["l"])
        };
        // A password lock needs a password, not an empty one.
        assert!(definitions.save_block(with(None, None)).is_err());
        assert!(definitions.save_block(with(Some(""), None)).is_err());
        definitions.save_block(with(Some("partner's secret"), None)).unwrap();
        let kept = definitions.library().block("b").unwrap().lock.password_hash().unwrap().to_string();
        assert!(unlock::check_hash("partner's secret", &kept));
        // Not on disk as text, nor in what a client sees.
        assert!(!fs::read_to_string(&path).unwrap().contains("partner's secret"));
        assert_eq!(definitions.library().redacted().block("b").unwrap().lock, Lock::Password { password: None, hash: None });
        // Saved again without a password (another change), it keeps its hash; a hash sent by a
        // client is not taken.
        definitions.save_block(Definition { name: "Renamed".into(), ..with(None, Some("00$00")) }).unwrap();
        assert_eq!(definitions.library().block("b").unwrap().lock.password_hash(), Some(kept.as_str()));
    }

    #[test]
    fn the_settings_of_other_lock_methods_are_kept() {
        let definitions = Definitions::at(Some(path("kept")));
        definitions.save_list(block_list("l", &["a.com"])).unwrap();
        let with = |lock: Lock| Definition { lock, ..block("b", &["l"]) };
        definitions.save_block(with(Lock::Duration { duration: "5m".into() })).unwrap();
        definitions.save_block(with(Lock::Password { password: Some("secret".into()), hash: None })).unwrap();
        definitions.save_block(with(Lock::None)).unwrap();
        let kept = definitions.library().block("b").unwrap().kept_locks.clone();
        assert_eq!(kept.len(), 2);
        assert!(kept.contains(&Lock::Duration { duration: "5m".into() }));
        // What a client sees has the duration, and that there was a password, without its hash.
        let seen = definitions.library().redacted().block("b").unwrap().kept_locks.clone();
        assert!(seen.contains(&Lock::Password { password: None, hash: None }));
        // A password lock chosen again without a new password has the one it had.
        definitions.save_block(with(Lock::Password { password: None, hash: None })).unwrap();
        let hash = definitions.library().block("b").unwrap().lock.password_hash().unwrap().to_string();
        assert!(unlock::check_hash("secret", &hash));
        // The current method is not kept twice; a client's kept locks are ignored.
        let sent = Definition { kept_locks: vec![Lock::Duration { duration: "1d".into() }], ..with(Lock::Duration { duration: "10m".into() }) };
        definitions.save_block(sent).unwrap();
        let kept = definitions.library().block("b").unwrap().kept_locks.clone();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].same_method(&Lock::Password { password: None, hash: None }) && kept[0].password_hash() == Some(hash.as_str()));
    }

    #[test]
    fn a_schedule_travels_with_its_days_beside_its_name() {
        let json = serde_json::to_string(&weekly("s", "sat")).unwrap();
        assert!(json.starts_with(r#"{"id":"s","name":"Schedule s","days":[[],[],[],[],[],[[0,1440]],[]]"#), "{json}");
        assert_eq!(serde_json::from_str::<NamedSchedule>(&json).unwrap(), weekly("s", "sat"));
    }
}
