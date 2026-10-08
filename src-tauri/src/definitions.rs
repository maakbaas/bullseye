//! What the user has defined, in three kinds, each a list of its own (`Library`):
//! - **targets** (`Target`): what to block and how: sites, always-allowed sites, programs,
//!   and the enforcers that act on them;
//! - **conditions** (`Condition`): when, as a weekly schedule or while a flash drive is not
//!   connected (`condition.rs`); "always" is built in and is no entry here;
//! - **sessions** (`Definition`): one or more targets, a condition (or always), an allowance
//!   (or none: a budget of time or a Pomodoro cycle, `allowance.rs`, set on the session itself like
//!   its lock) and a lock (`lock.rs`). A session is what is started and runs.
//!
//! All of it is kept by the service (`%ProgramData%\bullseye\sessions.json`), so it survives
//! restarts, and is only a template: starting a session copies its lists and condition into a running
//! session (`session.rs`), which is then fixed until it ends. So that what is shown and what runs stay
//! the same, a running session's definition cannot be changed or deleted until it ends, nor can a
//! target or condition it uses be deleted. A target or condition can be changed, and the
//! change goes to the running sessions that use it; but while one of them is locked, only to be
//! stricter (`Target::tightened_by`, `Rule::tightened_by`, `handler::edit_shared`). Deleting a
//! target or condition takes it out of the sessions that use it (`Library::fallout`).

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::allowance::Terms;
use crate::condition::{Mode, Rule};
use crate::lists::{Entries, Lists};
use crate::lock::Lock;
use crate::state::{self, Enforcers};
use crate::sync::LockExt;
use crate::unlock;

/// More than anyone needs, of each kind; keeps the file and the status message small.
const MAX_ITEMS: usize = 50;
const MAX_NAME: usize = 60;
const MAX_ID: usize = 64;

/// What to block, and how.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Target {
    /// Made by the UI or the CLI; letters, digits, `-` and `_`.
    pub id: String,
    pub name: String,
    /// What it blocks: its lists, one entry per line as typed (fields of the list in the file).
    #[serde(flatten)]
    pub entries: Entries,
    pub enforcers: Enforcers,
}

/// When a session is active, and what a condition lock locks by, with a name: a weekly schedule or a
/// flash drive (`Rule`, `condition.rs`, which says how to make a new kind). Its rule is fields of
/// the condition in the file: `type` and what the type has.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Condition {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub rule: Rule,
}

/// Which kind of thing: what `Definitions::duplicate` copies.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Session,
    Target,
    Condition,
}

/// A session: what is started. Its lists by id, its condition by id (`None`: always active) and its
/// lock.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Definition {
    pub id: String,
    pub name: String,
    /// The targets it enforces, by id.
    pub targets: Vec<String>,
    /// The condition it is active by, by id; `None` is always.
    pub condition: Option<String>,
    /// Its allowance, set on the session itself like its lock; `None` is none (`allowance.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance: Option<Terms>,
    /// How it is kept from being stopped once started (`lock.rs`).
    pub lock: Lock,
    /// The last settings of each other lock method it has had (a duration, an end, a lock
    /// condition, a password's hash), so that choosing one again starts from them. Kept by the
    /// service (`Definitions::save_session`): what a client sends here is ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept_locks: Vec<Lock>,
}

/// Everything the user has defined, each kind in the order it was made.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Library {
    pub targets: Vec<Target>,
    pub conditions: Vec<Condition>,
    pub sessions: Vec<Definition>,
}

/// How a session uses a condition that is not its own: through a combination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Via {
    /// As its condition or its lock's.
    Directly,
    /// In a combination of which any one is enough (`Mode::Any`): more active time in it is more
    /// active time in the combination.
    Any,
    /// In a combination that needs all of them (`Mode::All`): more active time in it is not.
    All,
}

/// A session with its parts looked up and checked: what starting it needs.
pub struct Resolved {
    pub session: Definition,
    pub lists: Vec<Target>,
    pub condition: Option<Rule>,
    /// The rule of its condition lock, if it has one.
    pub lock_condition: Option<Rule>,
    /// The terms of its allowance, if it has one.
    pub allowance: Option<Terms>,
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

impl Target {
    /// The lists, read and checked. Also refuses programs Windows needs (`programs.rs`).
    pub fn lists(&self) -> Result<Lists, String> {
        Lists::parse(&self.entries).map_err(|why| format!("{}: {why}", self.name))
    }

    /// Whether `new` blocks everything this does and more, which is all a target a locked session
    /// uses may be changed by: entries may be added to what is blocked, taken out of what is
    /// allowed, and enforcers turned on, never the other way. Lists are compared as they are read
    /// (`Lists::entries`), so case and a missing `.exe` make no difference. The error says what is
    /// wrong, for a person.
    pub fn tightened_by(&self, new: &Target) -> Result<(), String> {
        let (old, new_entries) = (self.lists()?.entries(), new.lists()?.entries());
        let missing = |had: &[String], now: &[String]| had.iter().filter(|e| !now.contains(e)).cloned().collect::<Vec<_>>().join(", ");
        for (what, had, now) in [
            ("sites to block", &old.filters, &new_entries.filters),
            ("programs", &old.programs, &new_entries.programs),
            ("Store apps", &old.store_apps, &new_entries.store_apps),
            ("window titles", &old.titles, &new_entries.titles),
        ] {
            let gone = missing(had, now);
            if !gone.is_empty() {
                return Err(format!("A locked session uses this target, so {what} can be added but not taken out: {gone}"));
            }
        }
        for (what, had, now) in [
            ("always-allowed sites", &old.allowed, &new_entries.allowed),
            ("always-allowed window titles", &old.allowed_titles, &new_entries.allowed_titles),
        ] {
            let added = missing(now, had);
            if !added.is_empty() {
                return Err(format!("A locked session uses this target, so {what} can be taken out but not added: {added}"));
            }
        }
        if self.enforcers.turned_off_by(&new.enforcers) {
            return Err("A locked session uses this target, so enforcers can be turned on but not off".into());
        }
        Ok(())
    }

    /// What can be checked without starting a session with it. Whether it has anything to do is only
    /// checked when a session starts, so a list can be saved half done.
    fn check(&self) -> Result<(), String> {
        check_name("target", &self.id, &self.name)?;
        self.lists().map(|_| ())
    }
}

impl Condition {
    /// The rule, in order and checked: a week sorted and merged, refused if it has no time in it;
    /// a drive chosen.
    fn checked(mut self) -> Result<Condition, String> {
        check_name("condition", &self.id, &self.name)?;
        self.rule = self.rule.checked()?;
        // A definition holds the ids of a combination's conditions, never copies of them.
        if let Rule::Combined { rules, .. } = &mut self.rule {
            rules.clear();
        }
        Ok(self)
    }
}

impl Library {
    /// Targets, conditions and sessions each by name, a to z regardless of case (the order every
    /// client shows them in).
    fn sort(&mut self) {
        self.targets.sort_by_cached_key(|l| l.name.to_lowercase());
        self.conditions.sort_by_cached_key(|c| c.name.to_lowercase());
        self.sessions.sort_by_cached_key(|b| b.name.to_lowercase());
    }

    /// As a client may see it: password locks without their hashes (`Lock::redacted`).
    pub fn redacted(mut self) -> Library {
        for session in &mut self.sessions {
            session.lock = session.lock.redacted();
            session.kept_locks = session.kept_locks.iter().map(Lock::redacted).collect();
        }
        self
    }

    pub fn target(&self, id: &str) -> Option<&Target> {
        self.targets.iter().find(|l| l.id == id)
    }

    pub fn condition(&self, id: &str) -> Option<&Condition> {
        self.conditions.iter().find(|s| s.id == id)
    }

    pub fn session(&self, id: &str) -> Option<&Definition> {
        self.sessions.iter().find(|b| b.id == id)
    }

    /// The sessions that use a target or a condition (as their condition or their
    /// lock's, or through a combination that has it), with how, once for each way.
    pub fn usages(&self, part: &str) -> Vec<(&Definition, Via)> {
        let mut found = Vec::new();
        for session in &self.sessions {
            if session.targets.iter().any(|l| l == part)
                || session.condition.as_deref() == Some(part)
                || session.lock.condition() == Some(part)
            {
                found.push((session, Via::Directly));
            }
            for used in [session.condition.as_deref(), session.lock.condition()].into_iter().flatten() {
                if let Some(Rule::Combined { mode, members, .. }) = self.condition(used).map(|c| &c.rule) {
                    if members.iter().any(|m| m == part) {
                        found.push((session, if *mode == Mode::Any { Via::Any } else { Via::All }));
                    }
                }
            }
        }
        found
    }

    /// The sessions that use a target or a condition, once each (`usages`), by id.
    pub fn users_of(&self, part: &str) -> Vec<&Definition> {
        let mut users: Vec<&Definition> = Vec::new();
        for (session, _) in self.usages(part) {
            if !users.iter().any(|u| u.id == session.id) {
                users.push(session);
            }
        }
        users
    }

    /// A condition's rule, checked; a combination with copies of the rules of its conditions, as
    /// a running session holds it.
    pub fn rule_of(&self, id: &str) -> Result<Rule, String> {
        let rule = self.condition(id).ok_or("A condition of this session is gone")?.rule.clone().checked()?;
        let Rule::Combined { mode, members, .. } = rule else { return Ok(rule) };
        let rules = members
            .iter()
            .map(|member| match self.condition(member).map(|c| c.rule.clone().checked()) {
                Some(Ok(Rule::Combined { .. })) => Err("A combination cannot hold another combination".to_string()),
                Some(rule) => rule,
                None => Err("A condition of this combination is gone".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Rule::Combined { mode, members, rules })
    }

    /// The conditions a combination is of exist, and are not combinations, nor the combination.
    fn check_combinations(&self) -> Result<(), String> {
        for condition in &self.conditions {
            for member in condition.rule.members() {
                let found = self.condition(member).ok_or_else(|| format!("{}: no condition with id {member}", condition.name))?;
                if found.id == condition.id || matches!(found.rule, Rule::Combined { .. }) {
                    return Err(format!("{}: a combination holds conditions that are not combinations", condition.name));
                }
            }
        }
        Ok(())
    }

    /// A session with its parts, ready to start. Refused if it has no target, or a part is gone.
    pub fn resolve(&self, id: &str) -> Result<Resolved, String> {
        let session = self.session(id).ok_or("No such session")?.clone();
        if session.targets.is_empty() {
            return Err(format!("{}: choose at least one target", session.name));
        }
        let lists = session
            .targets
            .iter()
            .map(|l| self.target(l).cloned().ok_or("A target of this session is gone"))
            .collect::<Result<Vec<_>, _>>()?;
        let condition = session.condition.as_deref().map(|c| self.rule_of(c)).transpose()?;
        let lock_condition = session.lock.condition().map(|c| self.rule_of(c)).transpose()?;
        let allowance = session.allowance.clone().map(Terms::checked).transpose()?;
        Ok(Resolved { session, lists, condition, lock_condition, allowance })
    }

    /// What can be checked of a session without starting it: its name, that its parts exist, its lock.
    fn check_session(&self, session: &Definition) -> Result<(), String> {
        check_name("session", &session.id, &session.name)?;
        for list in &session.targets {
            self.target(list).ok_or_else(|| format!("No target with id {list}"))?;
        }
        if let Some(condition) = &session.condition {
            self.condition(condition).ok_or_else(|| format!("No condition with id {condition}"))?;
        }
        if let Some(allowance) = &session.allowance {
            allowance.clone().checked()?;
        }
        session.lock.check()?;
        if let Some(condition) = session.lock.condition() {
            self.condition(condition).ok_or_else(|| format!("No condition with id {condition} for the lock"))?;
        }
        Ok(())
    }
}

/// What every kind has: an id and a name.
trait Named: Clone {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn rename(&mut self, id: String, name: String);
}

macro_rules! named {
    ($($kind:ty),*) => {$(
        impl Named for $kind {
            fn id(&self) -> &str {
                &self.id
            }
            fn name(&self) -> &str {
                &self.name
            }
            fn rename(&mut self, id: String, name: String) {
                self.id = id;
                self.name = name;
            }
        }
    )*};
}
named!(Target, Condition, Definition);

/// A copy of the item `id` as `new_id`, at the end, named after it with the next number free among
/// its kind (`copy_name`).
fn duplicate<T: Named>(items: &mut Vec<T>, kind: &str, id: &str, new_id: &str) -> Result<(), String> {
    let mut copy = items.iter().find(|i| i.id() == id).ok_or_else(|| format!("No such {kind}"))?.clone();
    if items.iter().any(|i| i.id() == new_id) {
        return Err(format!("There already is a {kind} with id {new_id}"));
    }
    let name = copy_name(copy.name(), &items.iter().map(Named::name).collect::<Vec<_>>());
    check_name(kind, new_id, &name)?;
    copy.rename(new_id.to_string(), name);
    put(items, copy, |_| false)
}

/// The name of a copy: `Work` and `Work (2)` both give `Work (2)`, or the next number not taken.
fn copy_name(name: &str, taken: &[&str]) -> String {
    let base = match name.strip_suffix(')').and_then(|rest| rest.rsplit_once(" (")) {
        Some((base, number)) if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    };
    (2..)
        .map(|n| {
            let number = format!(" ({n})");
            // Within the longest name there may be.
            let base: String = base.chars().take(MAX_NAME - number.len()).collect();
            format!("{}{number}", base.trim_end())
        })
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .expect("a free number")
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

/// What a session keeps of the other lock methods once its lock is `lock`: what it kept before and
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

/// The library, and the file it is kept in.
pub struct Definitions {
    library: Mutex<Library>,
    /// `None`: kept for this run only (no data folder, or a test).
    path: Option<PathBuf>,
}

impl Definitions {
    /// The service's: `%ProgramData%\bullseye\sessions.json`, read back from the last run.
    pub fn new() -> Definitions {
        Definitions::at(state::dir().map(|dir| dir.join("sessions.json")))
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
        new.sort();
        self.write(&new)?;
        *library = new;
        Ok(())
    }

    /// A password lock's password is turned into a salted hash here and never kept itself; saved
    /// without a new password, a password lock keeps the hash it has.
    pub fn save_session(&self, mut session: Definition) -> Result<(), String> {
        session.name = session.name.trim().to_string();
        session.allowance = session.allowance.map(Terms::checked).transpose()?;
        self.change(|library| {
            let old = library.session(&session.id).cloned();
            if let Lock::Password { password, hash } = &mut session.lock {
                *hash = match password.take() {
                    Some(new) if new.is_empty() => return Err("The password of a password lock cannot be empty".into()),
                    Some(new) => Some(unlock::new_hash(&new)?),
                    // A hash from a client is not taken: only one this service made, the session's
                    // own or the one kept from when it last had a password lock.
                    None => old
                        .as_ref()
                        .and_then(|old| old.lock.password_hash().or_else(|| old.kept_locks.iter().find_map(Lock::password_hash)))
                        .map(str::to_string),
                };
                if hash.is_none() {
                    return Err("Set a password for the password lock".into());
                }
            }
            library.check_session(&session)?;
            session.kept_locks = kept_locks(old.as_ref(), &session.lock);
            let id = session.id.clone();
            put(&mut library.sessions, session, |b| b.id == id)
        })
    }

    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        self.change(|library| {
            library.session(id).ok_or("No such session")?;
            library.sessions.retain(|b| b.id != id);
            Ok(())
        })
    }

    pub fn save_target(&self, mut list: Target) -> Result<(), String> {
        list.name = list.name.trim().to_string();
        list.entries.sort();
        list.check()?;
        self.change(|library| {
            let id = list.id.clone();
            put(&mut library.targets, list, |l| l.id == id)
        })
    }

    /// Deletes a target, and takes it out of the sessions that use it; a session that had no
    /// other target is deleted with it (`Library::fallout`). Whether a running session uses it
    /// is the handler's to refuse (`handler::edit`).
    pub fn delete_target(&self, id: &str) -> Result<(), String> {
        self.change(|library| {
            library.target(id).ok_or("No such target")?;
            library.targets.retain(|l| l.id != id);
            library.sessions.retain_mut(|session| {
                let had = session.targets.len();
                session.targets.retain(|l| l != id);
                !(had > 0 && session.targets.is_empty())
            });
            Ok(())
        })
    }

    pub fn save_condition(&self, mut condition: Condition) -> Result<(), String> {
        condition.name = condition.name.trim().to_string();
        let condition = condition.checked()?;
        self.change(|library| {
            let id = condition.id.clone();
            put(&mut library.conditions, condition, |c| c.id == id)?;
            library.check_combinations()
        })
    }

    pub fn delete_condition(&self, id: &str) -> Result<(), String> {
        self.change(|library| {
            library.condition(id).ok_or("No such condition")?;
            // It is taken out of the combinations it is in; one left with a single condition goes
            // too (`Library::going_with`), and its sessions are treated as for any condition.
            let going = library.going_with(id);
            for condition in &mut library.conditions {
                if let Rule::Combined { members, .. } = &mut condition.rule {
                    members.retain(|m| m != id);
                }
            }
            library.conditions.retain(|c| !going.contains(&c.id));
            // The sessions it was the condition of are always active; one it was the lock condition of
            // has no lock (`Library::fallout`).
            let gone = |id: Option<&str>| id.is_some_and(|id| going.iter().any(|g| g == id));
            for session in &mut library.sessions {
                if gone(session.condition.as_deref()) {
                    session.condition = None;
                }
                if gone(session.lock.condition()) {
                    session.lock = Lock::None;
                }
            }
            // A session may have kept it as the condition of a lock it no longer has: forgotten too.
            for session in &mut library.sessions {
                session.kept_locks.retain(|lock| !gone(lock.condition()));
            }
            Ok(())
        })
    }

    /// A copy of a session, target or condition, as `new_id` (made by the client, as for anything
    /// new), named after it with a number (`copy_name`). A session's copy has its lists, condition,
    /// allowance and lock, a password lock's hash included, and the lock settings it kept.
    pub fn duplicate(&self, kind: Kind, id: &str, new_id: &str) -> Result<(), String> {
        self.change(|library| match kind {
            Kind::Session => duplicate(&mut library.sessions, "session", id, new_id),
            Kind::Target => duplicate(&mut library.targets, "target", id, new_id),
            Kind::Condition => duplicate(&mut library.conditions, "condition", id, new_id),
        })
    }

    /// Replaces everything with `library` (Import), once all of it checks out as it would one by
    /// one. A password lock comes without its password, so as no lock (`SettingsFile`); a lock
    /// settings kept for a password is dropped. Whether a session runs is the handler's to refuse.
    pub fn replace(&self, mut library: Library) -> Result<(), String> {
        if library.targets.len().max(library.conditions.len()).max(library.sessions.len()) > MAX_ITEMS {
            return Err(format!("At most {MAX_ITEMS} of each"));
        }
        for list in &mut library.targets {
            list.name = list.name.trim().to_string();
            list.check()?;
        }
        library.conditions = library.conditions.into_iter().map(Condition::checked).collect::<Result<_, _>>()?;
        library.check_combinations()?;
        for session in &mut library.sessions {
            session.name = session.name.trim().to_string();
            session.allowance = session.allowance.take().map(Terms::checked).transpose()?;
            // A password lock comes with its hash from the file, never a password; without a hash it
            // is no lock.
            if let Lock::Password { password, hash } = &mut session.lock {
                *password = None;
                if hash.is_none() {
                    session.lock = Lock::None;
                }
            }
            session.kept_locks.retain(|lock| !matches!(lock, Lock::Password { hash: None, .. }));
            for lock in &mut session.kept_locks {
                if let Lock::Password { password, .. } = lock {
                    *password = None;
                }
            }
        }
        for session in &library.sessions {
            library.check_session(session)?;
        }
        let unique = |ids: Vec<&str>| ids.iter().enumerate().all(|(at, id)| !ids[..at].contains(id));
        if !unique(library.targets.iter().map(|l| l.id.as_str()).collect())
            || !unique(library.conditions.iter().map(|c| c.id.as_str()).collect())
            || !unique(library.sessions.iter().map(|b| b.id.as_str()).collect())
        {
            return Err("Two items of one kind in the file have the same id".into());
        }
        self.change(|old| {
            *old = library;
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

/// What deleting a target or condition does to the sessions that use it, by their names: what
/// the service does (`Definitions::delete_target`, `delete_condition`), and what the window and the
/// CLI say before it.
#[derive(Serialize, Default, Debug, PartialEq)]
pub struct Fallout {
    /// Sessions that had only this target: deleted with it.
    pub deleted: Vec<String>,
    /// Sessions that keep their other targets.
    pub without_it: Vec<String>,
    /// Blocks it was the condition of: always active from then on.
    pub always: Vec<String>,
    /// Blocks it was the lock condition of: no lock from then on.
    pub unlocked: Vec<String>,
    /// Combinations it is in, which go on without it.
    pub shrinks: Vec<String>,
    /// Combinations it is in that are left with one condition, and so are deleted too (and the
    /// sessions that used them are in `always` and `unlocked`).
    pub removed: Vec<String>,
}

impl Library {
    /// What deleting the target or condition `id` does (`Definitions::delete_target`,
    /// `delete_condition`), for the question before it.
    pub fn fallout(&self, id: &str) -> Fallout {
        let names = |keep: &dyn Fn(&Definition) -> bool| self.sessions.iter().filter(|b| keep(b)).map(|b| b.name.clone()).collect();
        let going = self.going_with(id);
        let has = |condition: Option<&str>| condition.is_some_and(|c| going.iter().any(|g| g == c));
        let combinations = |gone: bool| {
            self.conditions
                .iter()
                .filter(|c| c.rule.members().iter().any(|m| m == id) && going.contains(&c.id) == gone)
                .map(|c| c.name.clone())
                .collect()
        };
        Fallout {
            deleted: names(&|b| b.targets.iter().all(|l| l == id) && !b.targets.is_empty()),
            without_it: names(&|b| b.targets.iter().any(|l| l == id) && b.targets.iter().any(|l| l != id)),
            always: names(&|b| has(b.condition.as_deref())),
            unlocked: names(&|b| has(b.lock.condition())),
            shrinks: combinations(false),
            removed: combinations(true),
        }
    }

    /// The conditions that go when the condition `id` is deleted: itself, and each combination
    /// it is in that would be left with fewer than two conditions.
    pub fn going_with(&self, id: &str) -> Vec<String> {
        let mut going = vec![id.to_string()];
        going.extend(
            self.conditions
                .iter()
                .filter(|c| c.rule.members().iter().any(|m| m == id) && c.rule.members().len() <= 2)
                .map(|c| c.id.clone()),
        );
        going
    }
}

/// The version of the settings file; another is refused rather than guessed at.
const SETTINGS_VERSION: u32 = 1;

/// The file Export writes and Import reads, to move everything the user defined to another
/// computer: the library with a version. Password locks are in it with their hash (never the password;
/// a short password could be guessed at from the hash offline, by whoever has the file).
#[derive(Serialize, Deserialize)]
struct SettingsFile {
    bullseye_settings: u32,
    #[serde(flatten)]
    library: Library,
}

/// What a settings file holds, for the question before importing it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Contents {
    pub targets: usize,
    pub conditions: usize,
    pub sessions: usize,
    /// Sessions with a password lock but no hash in the file, which come without a lock.
    pub unlocked: Vec<String>,
}

impl Library {
    /// The settings file of this library (`SettingsFile`).
    pub fn settings_text(self) -> Result<String, String> {
        let file = SettingsFile { bullseye_settings: SETTINGS_VERSION, library: self };
        serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
    }

    /// A settings file read back; whether its contents can be used is the import's to check.
    pub fn from_settings_text(text: &str) -> Result<Library, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|_| "This is not a Bullseye settings file")?;
        match value.get("bullseye_settings").and_then(serde_json::Value::as_u64) {
            Some(version) if version == u64::from(SETTINGS_VERSION) => {}
            Some(_) => return Err("This settings file is from another version of Bullseye".into()),
            None => return Err("This is not a Bullseye settings file".into()),
        }
        let file: SettingsFile = serde_json::from_value(value).map_err(|e| format!("This settings file cannot be read: {e}"))?;
        Ok(file.library)
    }

    pub fn contents(&self) -> Contents {
        Contents {
            targets: self.targets.len(),
            conditions: self.conditions.len(),
            sessions: self.sessions.len(),
            unlocked: self
                .sessions
                .iter()
                .filter(|b| matches!(b.lock, Lock::Password { hash: None, .. }))
                .map(|b| b.name.clone())
                .collect(),
        }
    }
}

/// A Store app of a settings file that this computer has not installed: left out on Import, as a
/// family name is one exact package (`store_apps.rs`) and another computer may have the app as
/// another package, or not at all.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct LeftOut {
    /// The target it was in.
    pub target: String,
    pub family: String,
    /// What its family name calls it (`store_apps::readable`).
    pub name: String,
}

impl Library {
    /// Whether any of its targets has a Store app.
    pub fn has_store_apps(&self) -> bool {
        self.targets.iter().any(|list| !list.entries.store_apps.is_empty())
    }

    /// Without the Store apps that are not among `installed` (family names, letter case aside), and
    /// what was left out. `None`: what is installed is not known, so everything stays.
    pub fn without_missing_apps(mut self, installed: Option<&[String]>) -> (Library, Vec<LeftOut>) {
        let Some(installed) = installed else { return (self, Vec::new()) };
        let mut left_out = Vec::new();
        for list in &mut self.targets {
            list.entries.store_apps.retain(|family| {
                let here = installed.iter().any(|i| i.eq_ignore_ascii_case(family.trim()));
                if !here {
                    left_out.push(LeftOut { target: list.name.clone(), family: family.clone(), name: crate::store_apps::readable(family) });
                }
                here
            });
        }
        (self, left_out)
    }
}

/// Writes the settings file of `library` (Export).
pub fn write_settings(path: &std::path::Path, library: Library) -> Result<(), String> {
    fs::write(path, library.settings_text()?).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// Reads a settings file (Import).
pub fn read_settings(path: &std::path::Path) -> Result<Library, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    Library::from_settings_text(&text)
}

fn load(path: &PathBuf) -> Option<Library> {
    let bytes = fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(mut library) => {
            Library::sort(&mut library);
            Some(library)
        }
        Err(e) => {
            eprintln!("definitions: cannot read {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::schedule::Week;

    /// A target with these sites and the default enforcers.
    pub fn target(id: &str, filters: &[&str]) -> Target {
        Target {
            id: id.into(),
            name: format!("List {id}"),
            entries: Entries { filters: filters.iter().map(|f| f.to_string()).collect(), ..Entries::default() },
            enforcers: Enforcers::default(),
        }
    }

    /// A session of these lists, always active and not locked.
    pub fn session(id: &str, lists: &[&str]) -> Definition {
        Definition {
            id: id.into(),
            name: format!("Session {id}"),
            targets: lists.iter().map(|l| l.to_string()).collect(),
            condition: None,
            allowance: None,
            lock: Lock::None,
            kept_locks: Vec::new(),
        }
    }

    pub fn weekly(id: &str, text: &str) -> Condition {
        Condition { id: id.into(), name: format!("Schedule {id}"), rule: Rule::Schedule { week: Week::parse(text).unwrap().unwrap() } }
    }

    fn path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("definitions-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("sessions.json")
    }

    #[test]
    fn everything_is_kept_across_runs() {
        let path = path("kept");
        let definitions = Definitions::at(Some(path.clone()));
        definitions.save_target(target("social", &["youtube.com"])).unwrap();
        definitions.save_target(target("games", &[])).unwrap();
        definitions.save_condition(weekly("work", "mon-fri 8-17")).unwrap();
        let mut work = session("w", &["social", "games"]);
        work.condition = Some("work".into());
        work.lock = Lock::Duration { duration: "1h".into() };
        definitions.save_session(work.clone()).unwrap();
        // Saving one with the same id replaces it, in its place.
        definitions.save_target(target("social", &["reddit.com"])).unwrap();

        let again = Definitions::at(Some(path)).library();
        assert_eq!(again.targets.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), vec!["games", "social"]);
        assert_eq!(again.target("social").unwrap().entries.filters, vec!["reddit.com"]);
        assert_eq!(again.session("w").unwrap().targets, vec!["social", "games"]);
        let resolved = again.resolve("w").unwrap();
        assert_eq!(resolved.lists.len(), 2);
        assert_eq!(resolved.condition.unwrap().week().unwrap().describe(), "Mon-Fri 08:00-17:00");
    }

    #[test]
    fn what_cannot_be_saved() {
        let definitions = Definitions::at(None);
        definitions.save_target(target("l", &["a.com"])).unwrap();
        let bad_lists = [
            Target { name: "  ".into(), ..target("x", &[]) },
            Target { name: "x".repeat(MAX_NAME + 1), ..target("x", &[]) },
            target("../x", &[]),
            target("x", &["has space.com"]),
            Target { entries: Entries { programs: vec!["explorer.exe".into()], ..Entries::default() }, ..target("x", &[]) },
        ];
        for list in bad_lists {
            assert!(definitions.save_target(list.clone()).is_err(), "{list:?}");
        }
        // A schedule with no time in it is never active.
        assert!(definitions
            .save_condition(Condition { id: "s".into(), name: "S".into(), rule: Rule::Schedule { week: Week::default() } })
            .is_err());
        let bad_sessions = [
            session("b", &["nope"]),
            Definition { condition: Some("nope".into()), ..session("b", &["l"]) },
            Definition { lock: Lock::Duration { duration: "soon".into() }, ..session("b", &["l"]) },
            Definition { lock: Lock::Duration { duration: "".into() }, ..session("b", &["l"]) },
            Definition { lock: Lock::Condition { condition: "nope".into() }, ..session("b", &["l"]) },
        ];
        for b in bad_sessions {
            assert!(definitions.save_session(b.clone()).is_err(), "{b:?}");
        }
        assert!(definitions.library().sessions.is_empty());
        // Half done is fine to save: no lists yet. It is checked when it starts.
        definitions.save_session(session("b", &[])).unwrap();
        assert!(definitions.library().resolve("b").is_err());
    }

    #[test]
    fn deleting_a_target_or_condition_takes_it_out_of_its_sessions() {
        let definitions = Definitions::at(None);
        definitions.save_target(target("l", &["a.com"])).unwrap();
        definitions.save_target(target("l2", &["b.com"])).unwrap();
        definitions.save_condition(weekly("s", "weekends")).unwrap();
        definitions.save_condition(weekly("s2", "weekdays")).unwrap();
        // "a" has only l; "b" has l and l2, schedule s and lock schedule s2.
        definitions.save_session(session("a", &["l"])).unwrap();
        definitions
            .save_session(Definition {
                condition: Some("s".into()),
                lock: Lock::Condition { condition: "s2".into() },
                ..session("b", &["l", "l2"])
            })
            .unwrap();
        let library = definitions.library();
        let fallout = library.fallout("l");
        assert_eq!((fallout.deleted, fallout.without_it), (vec!["Session a".to_string()], vec!["Session b".to_string()]));
        assert_eq!(
            (library.fallout("s").always, library.fallout("s2").unlocked),
            (vec!["Session b".to_string()], vec!["Session b".to_string()])
        );
        assert_eq!(library.fallout("nothing"), Fallout::default());

        // The target: "a" goes with it, "b" keeps l2.
        definitions.delete_target("l").unwrap();
        let library = definitions.library();
        assert!(library.session("a").is_none());
        assert_eq!(library.session("b").unwrap().targets, vec!["l2"]);
        // The conditions: "b" is always active, and has no lock.
        definitions.delete_condition("s").unwrap();
        definitions.delete_condition("s2").unwrap();
        let b = definitions.library().session("b").unwrap().clone();
        assert_eq!((b.condition, b.lock), (None, Lock::None));
        assert!(b.kept_locks.is_empty(), "nor is it kept: {:?}", b.kept_locks);
        assert!(definitions.delete_target("l").is_err(), "gone already");
    }

    #[test]
    fn a_kept_lock_condition_is_forgotten_with_it() {
        let definitions = Definitions::at(None);
        definitions.save_target(target("l", &["a.com"])).unwrap();
        definitions.save_condition(weekly("s2", "weekdays")).unwrap();
        definitions.save_session(Definition { lock: Lock::Condition { condition: "s2".into() }, ..session("b", &["l"]) }).unwrap();
        definitions.save_session(session("b", &["l"])).unwrap();
        assert_eq!(definitions.library().session("b").unwrap().kept_locks, vec![Lock::Condition { condition: "s2".into() }]);
        definitions.delete_condition("s2").unwrap();
        assert!(definitions.library().session("b").unwrap().kept_locks.is_empty());
    }

    #[test]
    fn a_password_lock_keeps_only_a_hash_and_shows_neither() {
        let path = path("password");
        let definitions = Definitions::at(Some(path.clone()));
        definitions.save_target(target("l", &["a.com"])).unwrap();
        let with = |password: Option<&str>, hash: Option<&str>| Definition {
            lock: Lock::Password { password: password.map(str::to_string), hash: hash.map(str::to_string) },
            ..session("b", &["l"])
        };
        // A password lock needs a password, not an empty one.
        assert!(definitions.save_session(with(None, None)).is_err());
        assert!(definitions.save_session(with(Some(""), None)).is_err());
        definitions.save_session(with(Some("partner's secret"), None)).unwrap();
        let kept = definitions.library().session("b").unwrap().lock.password_hash().unwrap().to_string();
        assert!(unlock::check_hash("partner's secret", &kept));
        // Not on disk as text, nor in what a client sees.
        assert!(!fs::read_to_string(&path).unwrap().contains("partner's secret"));
        assert_eq!(definitions.library().redacted().session("b").unwrap().lock, Lock::Password { password: None, hash: None });
        // Saved again without a password (another change), it keeps its hash; a hash sent by a
        // client is not taken.
        definitions.save_session(Definition { name: "Renamed".into(), ..with(None, Some("00$00")) }).unwrap();
        assert_eq!(definitions.library().session("b").unwrap().lock.password_hash(), Some(kept.as_str()));
    }

    #[test]
    fn the_settings_of_other_lock_methods_are_kept() {
        let definitions = Definitions::at(Some(path("kept-lock-settings")));
        definitions.save_target(target("l", &["a.com"])).unwrap();
        let with = |lock: Lock| Definition { lock, ..session("b", &["l"]) };
        definitions.save_session(with(Lock::Duration { duration: "5m".into() })).unwrap();
        definitions.save_session(with(Lock::Password { password: Some("secret".into()), hash: None })).unwrap();
        definitions.save_session(with(Lock::None)).unwrap();
        let kept = definitions.library().session("b").unwrap().kept_locks.clone();
        assert_eq!(kept.len(), 2);
        assert!(kept.contains(&Lock::Duration { duration: "5m".into() }));
        // What a client sees has the duration, and that there was a password, without its hash.
        let seen = definitions.library().redacted().session("b").unwrap().kept_locks.clone();
        assert!(seen.contains(&Lock::Password { password: None, hash: None }));
        // A password lock chosen again without a new password has the one it had.
        definitions.save_session(with(Lock::Password { password: None, hash: None })).unwrap();
        let hash = definitions.library().session("b").unwrap().lock.password_hash().unwrap().to_string();
        assert!(unlock::check_hash("secret", &hash));
        // The current method is not kept twice; a client's kept locks are ignored.
        let sent =
            Definition { kept_locks: vec![Lock::Duration { duration: "1d".into() }], ..with(Lock::Duration { duration: "10m".into() }) };
        definitions.save_session(sent).unwrap();
        let kept = definitions.library().session("b").unwrap().kept_locks.clone();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].same_method(&Lock::Password { password: None, hash: None }) && kept[0].password_hash() == Some(hash.as_str()));
    }

    #[test]
    fn a_copy_is_named_with_the_next_free_number() {
        assert_eq!(copy_name("Work", &["Work"]), "Work (2)");
        assert_eq!(copy_name("Work", &["Work", "Work (2)"]), "Work (3)");
        assert_eq!(copy_name("Work (2)", &["Work", "Work (2)"]), "Work (3)");
        assert_eq!(copy_name("Work (later)", &["Work (later)"]), "Work (later) (2)");
        assert_eq!(copy_name("(2)", &["(2)"]), "(2) (2)");
        let long = "x".repeat(MAX_NAME);
        assert_eq!(copy_name(&long, &[&long]).chars().count(), MAX_NAME);
    }

    #[test]
    fn everything_can_be_duplicated() {
        let definitions = Definitions::at(Some(path("duplicate")));
        definitions.save_target(target("l", &["a.com"])).unwrap();
        definitions.save_condition(weekly("s", "Mon 08:00-09:00")).unwrap();
        let session = Definition {
            condition: Some("s".into()),
            lock: Lock::Password { password: Some("secret".into()), hash: None },
            ..session("b", &["l"])
        };
        definitions.save_session(session).unwrap();

        definitions.duplicate(Kind::Target, "l", "l2").unwrap();
        definitions.duplicate(Kind::Condition, "s", "s2").unwrap();
        definitions.duplicate(Kind::Session, "b", "b2").unwrap();
        definitions.duplicate(Kind::Session, "b", "b3").unwrap();
        let library = definitions.library();
        assert_eq!(library.target("l2").unwrap().entries.filters, vec!["a.com"]);
        assert_eq!(library.condition("s2").unwrap().rule, library.condition("s").unwrap().rule);
        let (original, copy) = (library.session("b").unwrap(), library.session("b2").unwrap());
        assert_eq!((copy.name.as_str(), library.session("b3").unwrap().name.as_str()), ("Session b (2)", "Session b (3)"));
        // The copy uses the same target and schedule, and has the same lock, hash included.
        assert_eq!((&copy.targets, &copy.condition, &copy.lock), (&original.targets, &original.condition, &original.lock));
        assert!(unlock::check_hash("secret", copy.lock.password_hash().unwrap()));

        // Not under an id that is taken, and not of what is not there.
        assert!(definitions.duplicate(Kind::Session, "b", "b2").is_err());
        assert!(definitions.duplicate(Kind::Target, "nope", "x").is_err());
        assert!(definitions.duplicate(Kind::Condition, "s", "not an id").is_err());
    }

    #[test]
    fn everything_moves_through_a_settings_file_password_hashes_too() {
        let here = Definitions::at(Some(path("export")));
        here.save_target(target("l", &["a.com"])).unwrap();
        here.save_condition(weekly("s", "Mon 08:00-09:00")).unwrap();
        let with = |id: &str, lock: Lock| Definition { condition: Some("s".into()), lock, ..session(id, &["l"]) };
        here.save_session(with("b", Lock::Password { password: Some("secret".into()), hash: None })).unwrap();
        here.save_session(with("b", Lock::Duration { duration: "1h".into() })).unwrap();
        here.save_session(with("p", Lock::Password { password: Some("secret".into()), hash: None })).unwrap();

        let text = here.library().settings_text().unwrap();
        let hash = here.library().session("p").unwrap().lock.password_hash().unwrap().to_string();
        assert!(text.contains(&hash) && !text.contains("secret"), "{text}");
        let read = Library::from_settings_text(&text).unwrap();
        assert_eq!(read.contents(), Contents { targets: 1, conditions: 1, sessions: 2, unlocked: vec![] });

        let there = Definitions::at(Some(path("import")));
        there.save_target(target("other", &["b.com"])).unwrap();
        there.replace(read).unwrap();
        let library = there.library();
        assert!(library.target("other").is_none(), "replaced, not added to");
        assert_eq!(library.session("b").unwrap().lock, Lock::Duration { duration: "1h".into() });
        // A password lock comes with its hash, so the same password stops it there.
        assert!(unlock::check_hash("secret", library.session("p").unwrap().lock.password_hash().unwrap()));
        assert_eq!(library.condition("s").unwrap().rule, here.library().condition("s").unwrap().rule);
    }

    #[test]
    fn what_cannot_be_imported() {
        let definitions = Definitions::at(None);
        assert!(Library::from_settings_text("{}").is_err());
        assert!(Library::from_settings_text("not json").is_err());
        assert!(Library::from_settings_text(r#"{"bullseye_settings": 2, "targets": [], "conditions": [], "sessions": []}"#).is_err());
        let file = |library: Library| Library::from_settings_text(&library.settings_text().unwrap()).unwrap();
        // A session naming a list that is not in the file, two lists with one id, a list Windows needs.
        let missing = Library { sessions: vec![session("b", &["gone"])], ..Library::default() };
        assert!(definitions.replace(file(missing)).is_err());
        let twice = Library { targets: vec![target("l", &[]), target("l", &[])], ..Library::default() };
        assert!(definitions.replace(file(twice)).is_err());
        let mut windows = target("w", &[]);
        windows.entries.programs = vec!["explorer.exe".into()];
        assert!(definitions.replace(file(Library { targets: vec![windows], ..Library::default() })).is_err());
        // An empty file empties everything.
        definitions.save_target(target("l", &[])).unwrap();
        definitions.replace(file(Library::default())).unwrap();
        assert_eq!(definitions.library(), Library::default());
    }

    #[test]
    fn a_file_with_too_much_or_a_password_without_its_hash() {
        let definitions = Definitions::at(None);
        let many = Library { targets: (0..=MAX_ITEMS).map(|n| target(&format!("l{n}"), &[])).collect(), ..Library::default() };
        assert_eq!(definitions.replace(many), Err(format!("At most {MAX_ITEMS} of each")));
        // A password lock with no hash comes without a lock, and a kept one without a hash is dropped.
        let mut locked = session("b", &["l"]);
        locked.lock = Lock::Password { password: Some("sent".into()), hash: None };
        locked.kept_locks = vec![Lock::Password { password: None, hash: None }, Lock::Duration { duration: "1h".into() }];
        let library = Library { targets: vec![target("l", &[])], sessions: vec![locked], ..Library::default() };
        assert_eq!(library.contents().unlocked, vec!["Session b"]);
        definitions.replace(library).unwrap();
        let imported = definitions.library().sessions.remove(0);
        assert_eq!(imported.lock, Lock::None);
        assert_eq!(imported.kept_locks, vec![Lock::Duration { duration: "1h".into() }]);
        // An allowance comes with its session, checked.
        let mut budget = session("a", &["l"]);
        budget.allowance = Some(Terms::Budget {
            allowed: "soon".into(),
            scope: crate::allowance::Scope::Total,
            reset: crate::allowance::Reset::Never,
            every: String::new(),
            anchor: crate::allowance::Anchor::Start,
        });
        let bad = Library { targets: vec![target("l", &[])], sessions: vec![budget], ..Library::default() };
        assert!(definitions.replace(bad).is_err());
    }

    #[test]
    fn a_store_app_this_computer_has_not_installed_is_left_out() {
        let mut claude = target("c", &["claude.ai"]);
        claude.name = "Claude".into();
        claude.entries.store_apps = vec!["AnthropicPBC.Claude_fnn82j28hfe8t".into(), "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into()];
        let library = Library { targets: vec![claude], ..Library::default() };
        let installed = vec!["spotifyab.spotifymusic_zpdnekdrzrea0".to_string(), "Claude_pzs8sxrjxfjjc".to_string()];
        let (kept, left_out) = library.clone().without_missing_apps(Some(&installed));
        assert_eq!(kept.targets[0].entries.store_apps, vec!["SpotifyAB.SpotifyMusic_zpdnekdrzrea0"], "letter case aside");
        assert_eq!(
            left_out,
            vec![LeftOut { target: "Claude".into(), family: "AnthropicPBC.Claude_fnn82j28hfe8t".into(), name: "Claude".into() }],
            "another package of the same app is not a match"
        );
        assert_eq!(kept.targets[0].entries.filters, vec!["claude.ai"], "the rest of the list stays");
        // Not known what is installed: everything stays.
        assert!(library.has_store_apps() && !Library::default().has_store_apps());
        assert_eq!(library.clone().without_missing_apps(None), (library, Vec::new()));
    }

    #[test]
    fn a_settings_file_is_written_and_read_back() {
        let dir = path("settings-file").parent().unwrap().to_path_buf();
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bullseye.json");
        let library = Library { targets: vec![target("l", &["a.com"])], sessions: vec![session("b", &["l"])], ..Library::default() };
        write_settings(&file, library.clone()).unwrap();
        assert_eq!(read_settings(&file), Ok(library.clone()));
        // A file that is not there, or a folder where the file should go.
        assert!(read_settings(&dir.join("missing.json")).unwrap_err().starts_with("Cannot read"));
        assert!(write_settings(&dir, library).unwrap_err().starts_with("Cannot write"));
    }

    #[test]
    fn a_library_file_that_cannot_be_read_is_an_empty_library() {
        let file = path("unreadable");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "not json").unwrap();
        assert_eq!(Definitions::at(Some(file)).library(), Library::default());
    }

    #[test]
    fn a_combination_is_resolved_from_its_conditions_as_they_are() {
        let mut both = weekly("both", "mon 8-9");
        both.rule = Rule::Combined { mode: Mode::Any, members: vec!["a".into(), "b".into()], rules: vec![] };
        let mut nested = weekly("nested", "mon 8-9");
        nested.rule = Rule::Combined { mode: Mode::Any, members: vec!["a".into(), "both".into()], rules: vec![] };
        let library = Library { conditions: vec![weekly("a", "mon 8-9"), weekly("b", "tue 8-9"), both, nested], ..Library::default() };
        assert!(matches!(library.rule_of("both"), Ok(Rule::Combined { rules, .. }) if rules.len() == 2));
        assert_eq!(library.rule_of("nested"), Err("A combination cannot hold another combination".into()));
        assert_eq!(library.rule_of("gone"), Err("A condition of this session is gone".into()));
        let without_b = Library { conditions: library.conditions.into_iter().filter(|c| c.id != "b").collect(), ..Library::default() };
        assert_eq!(without_b.rule_of("both"), Err("A condition of this combination is gone".into()));
    }

    #[test]
    fn a_schedule_travels_with_its_days_beside_its_name() {
        let json = serde_json::to_string(&weekly("s", "sat")).unwrap();
        assert!(json.starts_with(r#"{"id":"s","name":"Schedule s","type":"schedule","days":[[],[],[],[],[],[[0,1440]],[]]"#), "{json}");
        assert_eq!(serde_json::from_str::<Condition>(&json).unwrap(), weekly("s", "sat"));
    }

    #[test]
    fn deleting_a_condition_takes_it_out_of_its_combinations_and_deletes_those_left_with_one() {
        let definitions = Definitions::at(Some(path("combinations")));
        let combination = |id: &str, mode: Mode, members: &[&str]| Condition {
            id: id.into(),
            name: format!("Combination {id}"),
            rule: Rule::Combined { mode, members: members.iter().map(|m| m.to_string()).collect(), rules: Vec::new() },
        };
        for id in ["a", "b", "c"] {
            definitions.save_condition(weekly(id, "mon")).unwrap();
        }
        definitions.save_condition(combination("abc", Mode::Any, &["a", "b", "c"])).unwrap();
        definitions.save_condition(combination("ab", Mode::All, &["a", "b"])).unwrap();
        definitions.save_target(target("l", &["a.com"])).unwrap();
        definitions
            .save_session(Definition {
                condition: Some("ab".into()),
                lock: Lock::Condition { condition: "abc".into() },
                ..session("x", &["l"])
            })
            .unwrap();

        // "c" is only in "abc", which has two left.
        let fallout = definitions.library().fallout("c");
        assert_eq!((fallout.shrinks, fallout.removed), (vec!["Combination abc".to_string()], Vec::<String>::new()));
        definitions.delete_condition("c").unwrap();
        let library = definitions.library();
        assert_eq!(library.condition("abc").unwrap().rule.members(), ["a", "b"]);
        assert!(library.session("x").unwrap().condition.is_some());

        // "a" is in both, and each is left with one: both go, and "x" is always active and unlocked.
        let fallout = library.fallout("a");
        assert_eq!(fallout.removed, vec!["Combination ab".to_string(), "Combination abc".to_string()]);
        assert_eq!((fallout.always, fallout.unlocked), (vec!["Session x".to_string()], vec!["Session x".to_string()]));
        definitions.delete_condition("a").unwrap();
        let library = definitions.library();
        assert_eq!(library.conditions.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        let x = library.session("x").unwrap();
        assert!(x.condition.is_none() && x.lock == Lock::None);
    }
}
