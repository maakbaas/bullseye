//! The blocks the user has defined: each a name, its lists, its enforcers and its schedule. They are
//! kept by the service (`%ProgramData%\SiteBlocker\blocks.json`), so they survive restarts like
//! everything else, and are only templates: starting one copies it into a running block
//! (`block.rs`), which is then fixed until it ends. That is why a running block's definition cannot
//! be changed or deleted either.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::block::{parse_duration, Lists};
use crate::filters::FilterList;
use crate::programs::ProgramList;
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
    #[serde(default)]
    pub schedule: Schedule,
}

/// When a block runs. **A new kind of schedule goes here**, with an arm in `Schedule::duration`
/// (or whatever replaces it once a schedule is more than a length of time) and an entry in the UI's
/// `SCHEDULES` table.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// From when it is started, for `duration` (`5s`, `10m`, `3h`, `1d`); until stopped when empty.
    Duration {
        #[serde(default)]
        duration: String,
    },
}

impl Default for Schedule {
    fn default() -> Schedule {
        Schedule::Duration { duration: String::new() }
    }
}

impl Schedule {
    /// How long a started block runs; `None` until it is stopped.
    pub fn duration(&self) -> Result<Option<std::time::Duration>, String> {
        match self {
            Schedule::Duration { duration } => parse_duration(duration),
        }
    }
}

impl Definition {
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
        self.schedule.duration()?;
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
    /// The service's: `%ProgramData%\SiteBlocker\blocks.json`, read back from the last run.
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
    pub fn save(&self, mut definition: Definition) -> Result<(), String> {
        definition.check()?;
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
    match serde_json::from_slice(&bytes) {
        Ok(list) => Some(list),
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
        changed.schedule = Schedule::Duration { duration: "1h".into() };
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
            with(|d| d.schedule = Schedule::Duration { duration: "soon".into() }),
        ];
        for definition in bad {
            assert!(definitions.save(definition.clone()).is_err(), "{definition:?}");
        }
        assert!(definitions.all().is_empty());
        // Nothing to do yet is fine to save: it is checked when it starts.
        definitions.save(with(|d| d.filters.clear())).unwrap();
    }

    #[test]
    fn a_schedule_reads_with_or_without_its_fields() {
        let d: Definition = serde_json::from_str(r#"{"id":"a","name":"A"}"#).unwrap();
        assert_eq!(d.schedule, Schedule::default());
        let d: Definition = serde_json::from_str(r#"{"id":"a","name":"A","schedule":{"kind":"duration","duration":"2h"}}"#).unwrap();
        assert_eq!(d.schedule.duration(), Ok(Some(std::time::Duration::from_secs(7200))));
    }
}
