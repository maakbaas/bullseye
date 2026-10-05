//! The blocks the user has defined: each a name, its lists and enforcers (what it enforces), its
//! schedule (when it is active) and its lock (how it is kept from being stopped). They are
//! kept by the service (`%ProgramData%\bullseye\blocks.json`), so they survive restarts like
//! everything else, and are only templates: starting one copies it into a running block
//! (`block.rs`), which is then fixed until it ends. That is why a running block's definition cannot
//! be changed or deleted either.

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

/// More than anyone needs; keeps the file and the status message small.
const MAX_BLOCKS: usize = 50;
const MAX_NAME: usize = 60;
const MAX_ID: usize = 64;

/// One block as the user defined it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Definition {
    /// Made by the UI; letters, digits, `-` and `_`.
    pub id: String,
    pub name: String,
    /// The sites to block, one entry per line, as typed.
    #[serde(default)]
    pub filters: Vec<String>,
    /// The sites that are always allowed, which win over `filters`.
    #[serde(default)]
    pub allowed: Vec<String>,
    /// The programs to close (`programs.rs`).
    #[serde(default)]
    pub programs: Vec<String>,
    #[serde(default)]
    pub enforcers: Enforcers,
    /// When the block is active while it runs.
    #[serde(default)]
    pub schedule: Schedule,
    /// How a started block is kept from being stopped (`lock.rs`).
    #[serde(default)]
    pub lock: Lock,
}

/// When a running block is active. **A new kind of schedule goes here**, with an arm in
/// `Schedule::week` (or a new way for `block::Block` to decide) and an entry in the UI's
/// `SCHEDULES` table.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// All the time it runs.
    #[default]
    Always,
    /// At the times of a weekly schedule (`schedule.rs`); outside them it still runs and stays
    /// locked, but enforces nothing.
    Weekly(Week),
    /// From before lock methods: the time a block ran was its schedule. Only ever read from an older
    /// `blocks.json` or client, and turned into a timer lock at once (`Definition::upgraded`).
    Duration {
        #[serde(default)]
        duration: String,
    },
}

impl Schedule {
    /// The weekly schedule, checked; `None` is always active.
    pub fn week(&self) -> Result<Option<Week>, String> {
        match self {
            Schedule::Always | Schedule::Duration { .. } => Ok(None),
            Schedule::Weekly(week) => week.clone().normalized().map(Some),
        }
    }
}

impl Definition {
    /// The same block in today's terms: an old duration "schedule" is a timer lock now, and the
    /// block is always active.
    pub fn upgraded(mut self) -> Definition {
        if let Schedule::Duration { duration } = &self.schedule {
            self.lock = match duration.trim() {
                "" => Lock::None,
                duration => Lock::Timer { duration: duration.to_string() },
            };
            self.schedule = Schedule::Always;
        }
        self
    }

    /// The lists, read and checked. Also refuses programs Windows needs (`programs.rs`).
    pub fn lists(&self) -> Result<Lists, String> {
        let sites = FilterList::parse(&self.filters).map_err(|_| "Invalid block list syntax".to_string())?;
        let allowed = FilterList::parse(&self.allowed).map_err(|_| "Invalid syntax in the list of allowed sites".to_string())?;
        let programs = ProgramList::parse(&self.programs)?;
        Ok(Lists { sites, allowed, programs })
    }

    /// What can be checked without starting it: a name, an id, lists that can be read and a
    /// schedule that makes sense. Whether it has anything to do is only checked when it starts, so
    /// a block can be saved half done.
    pub fn check(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.len() > MAX_ID || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err("Invalid block id".into());
        }
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Give the block a name".into());
        }
        if name.chars().count() > MAX_NAME {
            return Err(format!("Keep the name under {MAX_NAME} characters"));
        }
        self.lists()?;
        self.schedule.week()?;
        self.lock.duration()?;
        Ok(())
    }
}

/// The definitions, in the order they were made, and the file they are kept in.
pub struct Definitions {
    list: Mutex<Vec<Definition>>,
    /// `None`: kept for this run only (no data folder, or a test).
    path: Option<PathBuf>,
}

impl Definitions {
    /// The service's: `%ProgramData%\bullseye\blocks.json`, read back from the last run.
    pub fn new() -> Definitions {
        Definitions::at(state::dir().map(|dir| dir.join("blocks.json")))
    }

    pub fn at(path: Option<PathBuf>) -> Definitions {
        let list = path.as_ref().and_then(load).unwrap_or_default();
        Definitions { list: Mutex::new(list), path }
    }

    pub fn all(&self) -> Vec<Definition> {
        self.list.locked().clone()
    }

    pub fn get(&self, id: &str) -> Option<Definition> {
        self.list.locked().iter().find(|d| d.id == id).cloned()
    }

    /// Adds a definition, or replaces the one with its id. Checked first (`Definition::check`).
    pub fn save(&self, definition: Definition) -> Result<(), String> {
        let mut definition = definition.upgraded();
        definition.check()?;
        // Kept sorted and merged, as it is compared and shown.
        if let Schedule::Weekly(week) = &definition.schedule {
            definition.schedule = Schedule::Weekly(week.clone().normalized()?);
        }
        definition.name = definition.name.trim().to_string();
        let mut list = self.list.locked();
        let mut new = list.clone();
        match new.iter().position(|d| d.id == definition.id) {
            Some(at) => new[at] = definition,
            None if new.len() >= MAX_BLOCKS => return Err(format!("At most {MAX_BLOCKS} blocks")),
            None => new.push(definition),
        }
        self.write(&new)?;
        *list = new;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut list = self.list.locked();
        let new: Vec<Definition> = list.iter().filter(|d| d.id != id).cloned().collect();
        if new.len() == list.len() {
            return Err("No such block".into());
        }
        self.write(&new)?;
        *list = new;
        Ok(())
    }

    /// Through a temporary file, like `state::Store::save`, so a crash leaves the old list or the new.
    fn write(&self, list: &[Definition]) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
        let dir = path.parent().ok_or_else(|| format!("no folder for {}", path.display()))?;
        fs::create_dir_all(dir).map_err(|e| format!("Cannot save the blocks: {e}"))?;
        let temporary = path.with_extension("new");
        fs::write(&temporary, json).map_err(|e| format!("Cannot save the blocks: {e}"))?;
        fs::rename(&temporary, path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("Cannot save the blocks: {e}")
        })
    }
}

fn load(path: &PathBuf) -> Option<Vec<Definition>> {
    let mut bytes = fs::read(path).ok()?;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.drain(..3);
    }
    match serde_json::from_slice::<Vec<Definition>>(&bytes) {
        Ok(list) => Some(list.into_iter().map(Definition::upgraded).collect()),
        Err(e) => {
            eprintln!("definitions: cannot read {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A definition with these sites and the default enforcers, untimed.
    pub fn definition(id: &str, filters: &[&str]) -> Definition {
        Definition {
            id: id.into(),
            name: format!("Block {id}"),
            filters: filters.iter().map(|f| f.to_string()).collect(),
            allowed: Vec::new(),
            programs: Vec::new(),
            enforcers: Enforcers::default(),
            schedule: Schedule::default(),
            lock: Lock::default(),
        }
    }

    fn path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("definitions-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("blocks.json")
    }

    #[test]
    fn definitions_are_kept_across_runs() {
        let path = path("kept");
        let definitions = Definitions::at(Some(path.clone()));
        definitions.save(definition("a", &["a.com"])).unwrap();
        definitions.save(definition("b", &["b.com"])).unwrap();
        // Saving one with the same id replaces it, in its place.
        let mut changed = definition("a", &["c.com"]);
        changed.lock = Lock::Timer { duration: "1h".into() };
        definitions.save(changed.clone()).unwrap();

        let again = Definitions::at(Some(path));
        assert_eq!(again.all().iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(again.get("a"), Some(changed));
        again.delete("b").unwrap();
        assert!(again.get("b").is_none());
        assert!(again.delete("b").is_err());
    }

    #[test]
    fn what_cannot_be_saved() {
        let definitions = Definitions::at(None);
        let with = |change: fn(&mut Definition)| {
            let mut d = definition("x", &["a.com"]);
            change(&mut d);
            d
        };
        let bad = [
            with(|d| d.name = "  ".into()),
            with(|d| d.name = "x".repeat(MAX_NAME + 1)),
            with(|d| d.id = String::new()),
            with(|d| d.id = "../x".into()),
            with(|d| d.filters = vec!["has space.com".into()]),
            with(|d| d.programs = vec!["explorer.exe".into()]),
            with(|d| d.lock = Lock::Timer { duration: "soon".into() }),
            with(|d| d.lock = Lock::Timer { duration: "".into() }),
            with(|d| d.schedule = Schedule::Weekly(Week::default())),
        ];
        for definition in bad {
            assert!(definitions.save(definition.clone()).is_err(), "{definition:?}");
        }
        assert!(definitions.all().is_empty());
        // Nothing to do yet is fine to save: it is checked when it starts.
        definitions.save(with(|d| d.filters.clear())).unwrap();
    }

    #[test]
    fn a_schedule_and_a_lock_read_with_or_without_their_fields() {
        let d: Definition = serde_json::from_str(r#"{"id":"a","name":"A"}"#).unwrap();
        assert_eq!((d.schedule, d.lock), (Schedule::Always, Lock::None));
        let json = r#"{"id":"a","name":"A","schedule":{"kind":"weekly","days":[[[480,1020]],[],[],[],[],[],[]]},"lock":{"method":"timer","duration":"2h 30m"}}"#;
        let d: Definition = serde_json::from_str(json).unwrap();
        assert_eq!(d.schedule.week().unwrap().unwrap().describe(), "Mon 08:00-17:00");
        assert_eq!(d.lock.duration(), Ok(Some(std::time::Duration::from_secs(9000))));
    }

    #[test]
    fn a_duration_from_before_locks_becomes_a_timer_lock() {
        let old = |duration: &str| -> Definition {
            serde_json::from_str(&format!(r#"{{"id":"a","name":"A","schedule":{{"kind":"duration","duration":"{duration}"}}}}"#)).unwrap()
        };
        let d = old("2h").upgraded();
        assert_eq!((d.schedule, d.lock), (Schedule::Always, Lock::Timer { duration: "2h".into() }));
        let d = old("").upgraded();
        assert_eq!((d.schedule, d.lock), (Schedule::Always, Lock::None));

        // In a file from before, and from a client that still sends one.
        let path = path("upgrade");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"[{"id":"a","name":"A","filters":["a.com"],"schedule":{"kind":"duration","duration":"1h"}}]"#).unwrap();
        let definitions = Definitions::at(Some(path));
        assert_eq!(definitions.get("a").unwrap().lock, Lock::Timer { duration: "1h".into() });
        definitions.save(Definition { schedule: Schedule::Duration { duration: "5m".into() }, ..definition("b", &[]) }).unwrap();
        let b = definitions.get("b").unwrap();
        assert_eq!((b.schedule, b.lock), (Schedule::Always, Lock::Timer { duration: "5m".into() }));
    }

    #[test]
    fn a_weekly_schedule_is_kept_sorted_and_merged() {
        let definitions = Definitions::at(None);
        let mut week = Week::default();
        week.days[0] = vec![crate::schedule::Span(600, 700), crate::schedule::Span(480, 620)];
        definitions.save(Definition { schedule: Schedule::Weekly(week), ..definition("a", &[]) }).unwrap();
        assert_eq!(definitions.get("a").unwrap().schedule.week().unwrap().unwrap().describe(), "Mon 08:00-11:40");
    }
}
