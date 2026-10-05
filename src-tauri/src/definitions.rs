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
    #[serde(default)]
    pub filters: Vec<String>,
    /// The sites that are always allowed, which win over `filters` (of this list only).
    #[serde(default)]
    pub allowed: Vec<String>,
    /// The programs to close (`programs.rs`).
    #[serde(default)]
    pub programs: Vec<String>,
    #[serde(default)]
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
    #[serde(default)]
    pub lists: Vec<String>,
    /// The schedule it is active by, by id; `None` is always.
    #[serde(default)]
    pub schedule: Option<String>,
    /// How it is kept from being stopped once started (`lock.rs`).
    #[serde(default)]
    pub lock: Lock,
}

/// Everything the user has defined, each kind in the order it was made.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Library {
    #[serde(default)]
    pub lists: Vec<BlockList>,
    #[serde(default)]
    pub schedules: Vec<NamedSchedule>,
    #[serde(default)]
    pub blocks: Vec<Definition>,
}

/// A block with its parts looked up and checked: what starting it needs.
pub struct Resolved {
    pub block: Definition,
    pub lists: Vec<BlockList>,
    pub week: Option<Week>,
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
    pub fn list(&self, id: &str) -> Option<&BlockList> {
        self.lists.iter().find(|l| l.id == id)
    }

    pub fn schedule(&self, id: &str) -> Option<&NamedSchedule> {
        self.schedules.iter().find(|s| s.id == id)
    }

    pub fn block(&self, id: &str) -> Option<&Definition> {
        self.blocks.iter().find(|b| b.id == id)
    }

    /// The blocks that use a block list or a schedule, by id.
    pub fn users_of(&self, part: &str) -> Vec<&Definition> {
        self.blocks.iter().filter(|b| b.lists.iter().any(|l| l == part) || b.schedule.as_deref() == Some(part)).collect()
    }

    /// A block with its parts, ready to start. Refused if it has no block list, or a part is gone.
    pub fn resolve(&self, id: &str) -> Result<Resolved, String> {
        let block = self.block(id).ok_or("No such block")?.clone();
        if block.lists.is_empty() {
            return Err(format!("{}: choose at least one block list", block.name));
        }
        let lists = block.lists.iter().map(|l| self.list(l).cloned().ok_or("A block list of this block is gone")).collect::<Result<Vec<_>, _>>()?;
        let week = match &block.schedule {
            Some(s) => Some(self.schedule(s).ok_or("The schedule of this block is gone")?.week.clone().normalized()?),
            None => None,
        };
        Ok(Resolved { block, lists, week })
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
        block.lock.duration()?;
        if block.lock == Lock::WhileActive && block.schedule.is_none() {
            return Err("Locked while active needs a weekly schedule: with Always, only the unlock password would end it".into());
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

    pub fn save_block(&self, mut block: Definition) -> Result<(), String> {
        block.name = block.name.trim().to_string();
        self.change(|library| {
            library.check_block(&block)?;
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

/// The file: today's library, or the list of blocks from before block lists and schedules had
/// their own lists.
#[derive(Deserialize)]
#[serde(untagged)]
enum OnDisk {
    Library(Library),
    Blocks(Vec<OldDefinition>),
}

/// A block from before: its lists, enforcers, schedule and lock all in one.
#[derive(Deserialize)]
struct OldDefinition {
    id: String,
    name: String,
    #[serde(default)]
    filters: Vec<String>,
    #[serde(default)]
    allowed: Vec<String>,
    #[serde(default)]
    programs: Vec<String>,
    #[serde(default)]
    enforcers: Enforcers,
    #[serde(default)]
    schedule: OldSchedule,
    #[serde(default)]
    lock: Lock,
}

#[derive(Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OldSchedule {
    #[default]
    Always,
    Weekly(Week),
    /// From before lock methods: the time a block ran, which is a timer lock now.
    Duration {
        #[serde(default)]
        duration: String,
    },
}

/// Older blocks in today's terms: each becomes a block list and a block of the same name and id;
/// a weekly schedule becomes a named schedule (one per different week), and a duration a timer lock.
fn migrate(old: Vec<OldDefinition>) -> Library {
    let mut library = Library::default();
    for block in old {
        library.lists.push(BlockList {
            id: block.id.clone(),
            name: block.name.clone(),
            filters: block.filters,
            allowed: block.allowed,
            programs: block.programs,
            enforcers: block.enforcers,
        });
        let mut lock = block.lock;
        let schedule = match block.schedule {
            OldSchedule::Always => None,
            OldSchedule::Duration { duration } => {
                lock = match duration.trim() {
                    "" => Lock::None,
                    duration => Lock::Timer { duration: duration.to_string() },
                };
                None
            }
            OldSchedule::Weekly(week) => {
                let week = week.normalized().unwrap_or_default();
                match library.schedules.iter().find(|s| s.week == week) {
                    Some(same) => Some(same.id.clone()),
                    None => {
                        let id = format!("schedule-{}", library.schedules.len() + 1);
                        library.schedules.push(NamedSchedule { id: id.clone(), name: format!("{} schedule", block.name), week });
                        Some(id)
                    }
                }
            }
        };
        library.blocks.push(Definition { id: block.id, name: block.name, lists: vec![library.lists.last().unwrap().id.clone()], schedule, lock });
    }
    library
}

fn load(path: &PathBuf) -> Option<Library> {
    let mut bytes = fs::read(path).ok()?;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.drain(..3);
    }
    match serde_json::from_slice::<OnDisk>(&bytes) {
        Ok(OnDisk::Library(library)) => Some(library),
        Ok(OnDisk::Blocks(old)) => Some(migrate(old)),
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
        Definition { id: id.into(), name: format!("Block {id}"), lists: lists.iter().map(|l| l.to_string()).collect(), schedule: None, lock: Lock::None }
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
        work.lock = Lock::Timer { duration: "1h".into() };
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
            Definition { lock: Lock::Timer { duration: "soon".into() }, ..block("b", &["l"]) },
            Definition { lock: Lock::Timer { duration: "".into() }, ..block("b", &["l"]) },
            // Locked while active with Always would only ever end with the password.
            Definition { lock: Lock::WhileActive, ..block("b", &["l"]) },
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
        definitions.save_block(Definition { schedule: Some("s".into()), lock: Lock::WhileActive, ..block("b", &["l"]) }).unwrap();
        let why = definitions.delete_list("l").unwrap_err();
        assert!(why.contains("Block b"), "{why}");
        assert!(definitions.delete_schedule("s").is_err());
        definitions.delete_block("b").unwrap();
        definitions.delete_list("l").unwrap();
        definitions.delete_schedule("s").unwrap();
        assert!(definitions.delete_list("l").is_err(), "gone already");
    }

    #[test]
    fn blocks_from_before_become_lists_schedules_and_blocks() {
        let path = path("migrate");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let week = r#"{"kind":"weekly","days":[[[480,1020]],[[480,1020]],[[480,1020]],[[480,1020]],[[480,1020]],[],[]]}"#;
        fs::write(
            &path,
            format!(
                r#"[
                    {{"id":"a","name":"Work","filters":["a.com"],"schedule":{week},"lock":{{"method":"timer","duration":"2h"}}}},
                    {{"id":"b","name":"Also work","programs":["steam.exe"],"schedule":{week}}},
                    {{"id":"c","name":"Old","filters":["c.com"],"schedule":{{"kind":"duration","duration":"1h"}}}}
                ]"#
            ),
        )
        .unwrap();
        let library = Definitions::at(Some(path)).library();
        assert_eq!(library.lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Work", "Also work", "Old"]);
        // The same week twice is one schedule, used by both.
        assert_eq!(library.schedules.len(), 1);
        assert_eq!(library.block("a").unwrap().schedule, library.block("b").unwrap().schedule);
        assert_eq!(library.block("a").unwrap().lock, Lock::Timer { duration: "2h".into() });
        let old = library.block("c").unwrap();
        assert_eq!((old.schedule.clone(), old.lock.clone(), old.lists.clone()), (None, Lock::Timer { duration: "1h".into() }, vec!["c".to_string()]));
        assert!(library.resolve("b").is_ok());
    }

    #[test]
    fn a_schedule_travels_with_its_days_beside_its_name() {
        let json = serde_json::to_string(&weekly("s", "sat")).unwrap();
        assert!(json.starts_with(r#"{"id":"s","name":"Schedule s","days":[[],[],[],[],[],[[0,1440]],[]]"#), "{json}");
        assert_eq!(serde_json::from_str::<NamedSchedule>(&json).unwrap(), weekly("s", "sat"));
    }
}
