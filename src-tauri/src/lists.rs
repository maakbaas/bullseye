//! The lists of a block list, in the two shapes every layer needs, so a new list is made here and
//! nowhere else in the backend:
//! - `Entries`: the lists as text, one entry per line as typed. What is saved (`definitions`,
//!   `state`), sent over the pipe (`proto`) and shown (`block::Running`) carries these, flattened
//!   into its own fields, so each list is a field of the JSON as ever.
//! - `Lists`: the same, read and checked, ready to match with (`block::Enforced` holds one).
//!
//! **A new list** is a field of both, a line in `Lists::parse`, `Lists::entries` and
//! `Lists::has`, an arm in `Entries::field`, a `state::Uses` variant for the enforcer that needs
//! it, an entry in the window's `LISTS` and its key in a `SECTIONS` group, and an option in
//! `cli::LIST_OPTIONS`.

use serde::{Deserialize, Serialize};

use crate::filters::FilterList;
use crate::patterns::Patterns;
use crate::programs::ProgramList;
use crate::state::Uses;

/// The lists as typed. Missing ones are empty, so a list saved before a list was added reads.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Entries {
    /// The sites to block.
    pub filters: Vec<String>,
    /// The sites that are always allowed, which win over `filters` (of this list only).
    pub allowed: Vec<String>,
    /// The programs to close (`programs.rs`).
    pub programs: Vec<String>,
    /// The Store apps to close, by package family name (`store_apps.rs`).
    pub store_apps: Vec<String>,
    /// The titles of windows to close (`patterns.rs`).
    pub titles: Vec<String>,
}

impl Entries {
    /// Every list in order, a to z regardless of case, as saving a block list leaves them.
    pub fn sort(&mut self) {
        for list in [&mut self.filters, &mut self.allowed, &mut self.programs, &mut self.store_apps, &mut self.titles] {
            list.sort_by_cached_key(|entry| entry.to_lowercase());
        }
    }

    /// A list by its field name, for what works on any of them (the CLI's options).
    pub fn field(&mut self, key: &str) -> Option<&mut Vec<String>> {
        match key {
            "filters" => Some(&mut self.filters),
            "allowed" => Some(&mut self.allowed),
            "programs" => Some(&mut self.programs),
            "store_apps" => Some(&mut self.store_apps),
            "titles" => Some(&mut self.titles),
            _ => None,
        }
    }
}

/// What a block is told to block, read and checked.
#[derive(Default)]
pub struct Lists {
    pub sites: FilterList,
    /// The sites that are always allowed, which win over `sites`. They are not something an
    /// enforcer needs on its own (`Uses::Sites` is about the block list), they only make
    /// exceptions to it.
    pub allowed: FilterList,
    pub programs: ProgramList,
    pub store_apps: Patterns,
    pub titles: Patterns,
}

impl Lists {
    /// Reads and checks the lists. The error says in words what is wrong (also what Windows
    /// needs: `programs.rs`).
    pub fn parse(entries: &Entries) -> Result<Lists, String> {
        Ok(Lists {
            sites: FilterList::parse(&entries.filters).map_err(|_| "invalid block list syntax")?,
            allowed: FilterList::parse(&entries.allowed).map_err(|_| "invalid syntax in the list of allowed sites")?,
            programs: ProgramList::parse(&entries.programs)?,
            store_apps: Patterns::parse(&entries.store_apps).map_err(|why| format!("Store apps: {why}"))?,
            titles: Patterns::parse(&entries.titles).map_err(|why| format!("Window titles: {why}"))?,
        })
    }

    /// The lists as written down, which are normalized and so match exactly what they matched.
    pub fn entries(&self) -> Entries {
        Entries {
            filters: self.sites.entries(),
            allowed: self.allowed.entries(),
            programs: self.programs.entries(),
            store_apps: self.store_apps.entries(),
            titles: self.titles.entries(),
        }
    }

    /// Whether the list an enforcer needs has an entry (`Enforcers::can_run`).
    pub fn has(&self, uses: Uses) -> bool {
        match uses {
            Uses::Nothing => true,
            Uses::Sites => !self.sites.is_empty(),
            Uses::Programs => !self.programs.is_empty(),
            Uses::StoreApps => !self.store_apps.is_empty(),
            Uses::Titles => !self.titles.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_list_is_reached_by_its_field_name() {
        let mut entries = Entries::default();
        for (n, key) in ["filters", "allowed", "programs", "store_apps", "titles"].into_iter().enumerate() {
            entries.field(key).unwrap().push(format!("entry{n}"));
        }
        assert_eq!(entries.titles, vec!["entry4"]);
        assert!(entries.field("sites").is_none(), "the field's name, not the CLI's");
    }

    #[test]
    fn an_enforcer_has_something_to_do_when_its_list_has_entries() {
        let empty = Lists::default();
        assert!(empty.has(Uses::Nothing));
        assert!(!empty.has(Uses::Sites) && !empty.has(Uses::Programs) && !empty.has(Uses::StoreApps) && !empty.has(Uses::Titles));
        let full = Lists::parse(&Entries {
            filters: vec!["a.com".into()],
            programs: vec!["game.exe".into()],
            store_apps: vec!["Microsoft.Xbox*".into()],
            titles: vec!["*Chat*".into()],
            ..Entries::default()
        })
        .unwrap();
        assert!([Uses::Sites, Uses::Programs, Uses::StoreApps, Uses::Titles].into_iter().all(|uses| full.has(uses)));
        // What a list cannot read says which list.
        let why = |entries: Entries| Lists::parse(&entries).err().unwrap_or_default();
        assert!(why(Entries { titles: vec!["*".into()], ..Entries::default() }).starts_with("Window titles:"));
        assert!(why(Entries { store_apps: vec!["*".into()], ..Entries::default() }).starts_with("Store apps:"));
    }
}
