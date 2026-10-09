//! The lists of a target, in the two shapes every layer needs, so a new list is made here and
//! nowhere else in the backend:
//! - `Entries`: the lists as text, one entry per line as typed. What is saved (`definitions`,
//!   `state`), sent over the pipe (`proto`) and shown (`session::Running`) carries these, flattened
//!   into its own fields, so each list is a field of the JSON as ever.
//! - `Lists`: the same, read and checked, ready to match with (`session::Enforced` holds one).
//!
//! **A new list** is a field of both, a line in `Lists::parse`, `Lists::entries` and
//! `Lists::has`, an arm in `Entries::field`, a `state::Uses` variant for the action that needs
//! it, an entry in the window's `LISTS` and its key in a `SECTIONS` group, and an option in
//! `LIST_OPTIONS` (`cli/parse.rs`).

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
    /// The titles of windows that are always left alone, which win over `titles` (of this list
    /// only), as the always-allowed sites do over the sites.
    pub allowed_titles: Vec<String>,
}

impl Entries {
    /// Every list in order, a to z regardless of case, as saving a target leaves them.
    pub fn sort(&mut self) {
        for list in
            [&mut self.filters, &mut self.allowed, &mut self.programs, &mut self.store_apps, &mut self.titles, &mut self.allowed_titles]
        {
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
            "allowed_titles" => Some(&mut self.allowed_titles),
            _ => None,
        }
    }
}

/// What a session is told to block, read and checked.
#[derive(Default)]
pub struct Lists {
    pub sites: FilterList,
    /// The sites that are always allowed, which win over `sites`. They are not something an
    /// action needs on its own (`Uses::Sites` is about the target), they only make
    /// exceptions to it.
    pub allowed: FilterList,
    pub programs: ProgramList,
    pub store_apps: Patterns,
    pub titles: Patterns,
    /// The window titles that are always left alone, which win over `titles`: exceptions, like
    /// `allowed`.
    pub allowed_titles: Patterns,
}

impl Lists {
    /// Reads and checks the lists. The error says in words what is wrong (also what Windows
    /// needs: `programs.rs`).
    pub fn parse(entries: &Entries) -> Result<Lists, String> {
        Ok(Lists {
            sites: FilterList::parse(&entries.filters).map_err(|_| "invalid target syntax")?,
            allowed: FilterList::parse(&entries.allowed).map_err(|_| "invalid syntax in the list of allowed sites")?,
            programs: ProgramList::parse(&entries.programs)?,
            store_apps: store_apps(&entries.store_apps)?,
            titles: Patterns::parse(&entries.titles).map_err(|why| format!("Window titles: {why}"))?,
            allowed_titles: Patterns::parse(&entries.allowed_titles).map_err(|why| format!("Always allowed window titles: {why}"))?,
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
            allowed_titles: self.allowed_titles.entries(),
        }
    }

    /// Whether a page is to be closed by these lists: on the sites, and not always allowed.
    pub fn blocks_site(&self, url: &str) -> bool {
        self.sites.is_blocked(url) && !self.allowed.is_blocked(url)
    }

    /// Whether a window of this title is to be closed or minimized by these lists: one of the
    /// titles, and not always allowed.
    pub fn blocks_title(&self, title: &str) -> bool {
        self.titles.is_match(title) && !self.allowed_titles.is_match(title)
    }

    /// Only the exceptions of these lists (the always-allowed sites and titles), which go with
    /// whatever an allowance lets through of it.
    pub fn exceptions(entries: &Entries) -> Entries {
        Entries { allowed: entries.allowed.clone(), allowed_titles: entries.allowed_titles.clone(), ..Entries::default() }
    }

    /// Whether the list an action needs has an entry (`Actions::can_run`).
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

/// The Store apps, by their exact package family names, letter case aside: the window ticks them from
/// the installed ones (`store_apps::installed`), and the CLI takes them as `list apps` shows them.
/// A `*` is refused, so what a session closes is always an app someone chose.
fn store_apps(entries: &[String]) -> Result<Patterns, String> {
    let wild: Vec<&str> = entries.iter().map(|e| e.trim()).filter(|e| e.contains('*')).collect();
    if !wild.is_empty() {
        return Err(format!("Store apps: give each by its exact package family name, without *: {}", wild.join(", ")));
    }
    Patterns::parse(entries).map_err(|why| format!("Store apps: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_list_is_reached_by_its_field_name() {
        let mut entries = Entries::default();
        for (n, key) in ["filters", "allowed", "programs", "store_apps", "titles", "allowed_titles"].into_iter().enumerate() {
            entries.field(key).unwrap().push(format!("entry{n}"));
        }
        assert_eq!(entries.titles, vec!["entry4"]);
        assert_eq!(entries.allowed_titles, vec!["entry5"]);
        assert!(entries.field("sites").is_none(), "the field's name, not the CLI's");
    }

    #[test]
    fn an_action_has_something_to_do_when_its_list_has_entries() {
        let empty = Lists::default();
        assert!(empty.has(Uses::Nothing));
        assert!(!empty.has(Uses::Sites) && !empty.has(Uses::Programs) && !empty.has(Uses::StoreApps) && !empty.has(Uses::Titles));
        let full = Lists::parse(&Entries {
            filters: vec!["a.com".into()],
            programs: vec!["game.exe".into()],
            store_apps: vec!["Microsoft.XboxApp_8wekyb3d8bbwe".into()],
            titles: vec!["*Chat*".into()],
            ..Entries::default()
        })
        .unwrap();
        assert!([Uses::Sites, Uses::Programs, Uses::StoreApps, Uses::Titles].into_iter().all(|uses| full.has(uses)));
        // A lone * is a title list of its own; what a list cannot read says which list.
        assert!(Lists::parse(&Entries { titles: vec!["*".into()], ..Entries::default() }).is_ok());
        let why = |entries: Entries| Lists::parse(&entries).err().unwrap_or_default();
        assert!(
            why(Entries { programs: vec!["*".into()], ..Entries::default() }).starts_with("Would close programs Windows or this app needs")
        );
    }

    #[test]
    fn an_always_allowed_title_wins_over_the_titles() {
        let lists = Lists::parse(&Entries {
            titles: vec!["*".into()],
            allowed_titles: vec!["*Visual Studio Code".into(), "Calculator".into()],
            ..Entries::default()
        })
        .unwrap();
        assert!(lists.blocks_title("YouTube - Chrome"));
        assert!(!lists.blocks_title("main.rs - bullseye - visual studio code") && !lists.blocks_title("Calculator"));
        // On their own they do nothing: an exception, not something to watch.
        let only = Lists::parse(&Entries { allowed_titles: vec!["Calculator".into()], ..Entries::default() }).unwrap();
        assert!(!only.has(Uses::Titles) && !only.blocks_title("Calculator") && !only.blocks_title("Notes"));
        // What the exceptions of a target are.
        let entries = Entries {
            filters: vec!["*".into()],
            allowed: vec!["google.com".into()],
            titles: vec!["*".into()],
            allowed_titles: vec!["Notes".into()],
            ..Entries::default()
        };
        let exceptions = Lists::exceptions(&entries);
        assert_eq!((exceptions.allowed, exceptions.allowed_titles), (vec!["google.com".to_string()], vec!["Notes".to_string()]));
        assert!(exceptions.filters.is_empty() && exceptions.titles.is_empty());
    }

    #[test]
    fn a_store_app_is_named_exactly() {
        let lists = Lists::parse(&Entries { store_apps: vec!["Microsoft.XboxApp_8wekyb3d8bbwe".into()], ..Entries::default() }).unwrap();
        assert!(lists.store_apps.is_match("microsoft.xboxapp_8wekyb3d8bbwe"), "letter case aside");
        assert!(!lists.store_apps.is_match("Microsoft.XboxApp_8wekyb3d8bbwe2"));
        for wild in ["*", "Microsoft.Xbox*", "*Xbox*"] {
            let why = Lists::parse(&Entries {
                store_apps: vec!["Microsoft.WindowsCalculator_8wekyb3d8bbwe".into(), wild.into()],
                ..Entries::default()
            })
            .err()
            .unwrap_or_default();
            assert_eq!(why, format!("Store apps: give each by its exact package family name, without *: {wild}"));
        }
    }
}
