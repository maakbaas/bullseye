//! Syncing: computers (and the sync server) that share one log of what was done on any of them, so
//! each has the same settings, runs the same sessions and unlocks together.
//!
//! **What another computer did is done here as if it was asked for here** (`handler::apply`): through
//! the same rules, so no computer, nor the server, can do anything here that could not be done at
//! this computer's own keyboard. What the rules refuse for now waits (`Held`) and is tried again
//! (`Replica::retry`) until it can be done or has nothing left to do: a stop waits while the session is
//! locked here (its own restart, its own drive), an edit while a locked session here uses it.
//!
//! What is done here counts at once and is written to the outbox (`record`); the server numbers every
//! entry, and every computer applies the entries in that order (`receive`). An entry of this computer's
//! own that comes back is not done again. One from elsewhere that changes something this computer has
//! changed since (a pending entry of its own) is not applied: its own comes later in the order, so it
//! is what every computer ends up with. An import from elsewhere is applied and the pending edits of
//! this computer done again on top. So no state between is ever enforced, and every computer ends up
//! the same, apart from what it holds back.
//!
//! **The way out is never in sync's hands.** Every lock still ends by its own rule here, Unlock here
//! works as it always does, and disconnecting (`disconnect`) always works, whatever the state: it
//! forgets everything that waits and applies nothing more. A fault in what is kept on disk is a
//! computer that is not connected, never one that replays the log from its start.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::definitions::{Condition, Definition, Kind, Library, Target};
use crate::handler::{self, Context, Outcome};
use crate::lock::Lock;
use crate::state;
use crate::sync::LockExt;

/// How much earlier than now a countdown started elsewhere may have begun here: an entry that takes a
/// while to arrive keeps its time, one that claims an older one waits the full time less this.
pub const GRACE: Duration = Duration::from_secs(120);
/// The longest entry taken, in bytes: an import of the most items there can be fits easily.
pub const MAX_ENTRY: usize = 1024 * 1024;
/// At most this many entries wait at once; the oldest is forgotten first (it then never happens here,
/// which loosens nothing).
pub const MAX_HELD: usize = 200;
/// At most this many entries of this computer's own wait to be sent (offline for a long time).
pub const MAX_PENDING: usize = 5000;
/// The longest id of an entry or a computer.
const MAX_ID: usize = 64;

/// A new random id, for an entry, a start or a computer.
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What was done, as every computer does it again: the request as it came out where it was done,
/// with what was decided there (a start's end, a drawn text, a password's hash; never a password).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    SaveSession {
        session: Definition,
    },
    DeleteSession {
        id: String,
    },
    SaveTarget {
        target: Target,
    },
    DeleteTarget {
        id: String,
    },
    SaveCondition {
        condition: Condition,
    },
    DeleteCondition {
        id: String,
    },
    Duplicate {
        kind: Kind,
        id: String,
        new_id: String,
    },
    Import {
        library: Library,
    },
    /// `start` is the start's own id (`session::Origin`); `ends` a duration lock's end there, `text`
    /// a random text drawn there (milliseconds since the Unix epoch); `zone` the time zone it is read
    /// in there (`localtime::Zone::name`), which only the sync server goes by.
    Start {
        id: String,
        start: String,
        ends: Option<u64>,
        text: Option<String>,
        #[serde(default)]
        zone: Option<String>,
    },
    Lock {
        id: String,
        lock: Lock,
        ends: Option<u64>,
        text: Option<String>,
    },
    /// `starts`: the starts the session stood for where it was stopped; `text` a text lock's text as
    /// typed; `proof` the signature its password made there (`unlock::sign`), never the password.
    Stop {
        id: String,
        starts: Vec<String>,
        text: Option<String>,
        #[serde(default)]
        proof: Option<String>,
    },
    /// Use of what a budget lets through, counted there since it last said so (milliseconds by key,
    /// `allowance::Run::charge`): it counts here too, so a budget is one for all computers.
    Used {
        id: String,
        used: std::collections::BTreeMap<String, u64>,
    },
    AskToStop {
        id: String,
    },
    AbortStop {
        id: String,
    },
    TakeBreak {
        id: String,
    },
    EndBreak {
        id: String,
    },
    AskForUnlock,
    AbortUnlock,
    /// `text`: the answer to a text way to unlock; `next`: the random text drawn there for the next
    /// unlock; `proof`: made with the password of the way to unlock, its signature of the time.
    Unlock {
        text: Option<String>,
        next: Option<String>,
        #[serde(default)]
        proof: Option<Proof>,
    },
    SetUnlockDelay {
        delay: String,
    },
    SetUnlockWay {
        way: Option<Lock>,
    },
}

/// That a password was typed, without the password: its signature (`unlock::sign`) of what was done
/// at `at` (milliseconds since the Unix epoch).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Proof {
    pub at: u64,
    pub signature: String,
}

/// What an import changes: everything defined.
const EVERYTHING: &str = "*";

impl Action {
    /// What of the shared settings it sets as a whole (an item by its kind and id, the unlock delay or
    /// way): a later entry that sets the same is what stays. Nothing for what happens to a run.
    fn keys(&self) -> Vec<String> {
        match self {
            Action::SaveSession { session } => vec![format!("session:{}", session.id)],
            Action::DeleteSession { id } => vec![format!("session:{id}")],
            Action::SaveTarget { target } => vec![format!("target:{}", target.id)],
            Action::DeleteTarget { id } => vec![format!("target:{id}")],
            Action::SaveCondition { condition } => vec![format!("condition:{}", condition.id)],
            Action::DeleteCondition { id } => vec![format!("condition:{id}")],
            Action::Duplicate { kind, new_id, .. } => vec![format!("{}:{new_id}", kind_name(*kind))],
            Action::Import { .. } => vec![EVERYTHING.to_string()],
            Action::SetUnlockDelay { .. } => vec!["unlock:delay".to_string()],
            Action::SetUnlockWay { .. } => vec!["unlock:way".to_string()],
            _ => Vec::new(),
        }
    }

    /// Whether it changes what is defined (rather than the unlock settings or a run).
    fn is_library(&self) -> bool {
        self.keys().iter().any(|key| !key.starts_with("unlock:"))
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Session => "session",
        Kind::Target => "target",
        Kind::Condition => "condition",
    }
}

/// One entry of the log: an action, which computer did it and when (milliseconds since the Unix
/// epoch, its internet time).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub device: String,
    pub at: u64,
    pub action: Action,
}

impl Entry {
    /// An entry as it came from the log: refused when it is too long, not an entry, or its ids are
    /// empty or too long.
    pub fn parse(text: &str) -> Result<Entry, String> {
        if text.len() > MAX_ENTRY {
            return Err("An entry is too long".into());
        }
        let entry: Entry = serde_json::from_str(text).map_err(|e| format!("Not an entry: {e}"))?;
        let fits = |id: &str| !id.is_empty() && id.len() <= MAX_ID;
        if !fits(&entry.id) || !fits(&entry.device) {
            return Err("An entry's ids are empty or too long".into());
        }
        Ok(entry)
    }

    pub fn text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// An entry from elsewhere that is refused here for now, and why.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Held {
    pub entry: Entry,
    pub why: String,
}

/// What a computer keeps of syncing (`replica.json`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
struct Kept {
    /// This computer's id in the log, new with each connect; empty while not connected.
    #[serde(default)]
    device: String,
    /// The number of the last entry of the log applied; `None` is not connected.
    #[serde(default)]
    confirmed: Option<u64>,
    /// The id of that entry, sent with the next log request so a server that is not the one synced with
    /// any more (wiped, restored from a backup) is told apart (`Failure::Diverged`); none: not known.
    #[serde(default)]
    last: Option<String>,
    /// When the last entry applied was done, or the snapshot joined from was last changed (`Entry::at`).
    #[serde(default)]
    seen: Option<u64>,
    /// What was done here and not yet numbered by the server, in the order it was done.
    #[serde(default)]
    pending: Vec<Entry>,
    /// What waits here.
    #[serde(default)]
    held: Vec<Held>,
}

/// A computer's side of syncing.
pub struct Replica {
    kept: Mutex<Kept>,
    /// `None`: kept for this run only (a test, or the server's own).
    path: Option<PathBuf>,
}

impl Replica {
    /// The service's: `replica.json` in its data folder.
    #[allow(dead_code)] // used by the sync client, still to come
    pub fn new() -> Replica {
        Replica::at(state::dir().map(|dir| dir.join("replica.json")))
    }

    /// Whatever is wrong with the file (missing, unreadable, nonsense), this computer is not connected.
    pub fn at(path: Option<PathBuf>) -> Replica {
        let kept = path
            .as_ref()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Kept>(&bytes).ok())
            .filter(|kept| kept.confirmed.is_none() || !kept.device.is_empty())
            .unwrap_or_default();
        Replica { kept: Mutex::new(kept), path }
    }

    /// Joins the log from its entry `head` on (what came before is in the settings taken over when
    /// connecting, never replayed), as a new computer.
    pub fn connect(&self, head: u64, last: Option<String>, seen: Option<u64>) -> Result<String, String> {
        let device = new_id();
        let next = Kept { device: device.clone(), confirmed: Some(head), last, seen, pending: Vec::new(), held: Vec::new() };
        let mut kept = self.kept.locked();
        self.write(&next)?;
        *kept = next;
        Ok(device)
    }

    /// Joins the log again from its entry `head` (whose id is `last`) on, as the same computer, keeping what
    /// is still to be sent and what waits: after being away longer than the log keeps (the snapshot taken
    /// over then is applied by `adopt`), or when the server's log is not the one synced with (what is here
    /// is sent again then: `syncing.rs`).
    pub fn rejoin(&self, head: u64, last: Option<String>, seen: Option<u64>) {
        let mut kept = self.kept.locked();
        if kept.confirmed.is_some() {
            kept.confirmed = Some(head);
            kept.last = last;
            kept.seen = seen.or(kept.seen);
            self.save(&kept);
        }
    }

    /// Applies an action taken over from the server's snapshot when joining (`syncing.rs`), as one from
    /// elsewhere: under this computer's rules, waiting if they refuse it.
    pub fn adopt(&self, context: &Context, action: Action, now: SystemTime) -> Outcome {
        let outcome = handler::apply(context, &action, now);
        if let Outcome::Held(why) = &outcome {
            let mut kept = self.kept.locked();
            let entry = Entry { id: new_id(), device: "server".into(), at: state::to_unix(now).unwrap_or(0), action };
            kept.held.retain(|held| !supersedes(&entry.action, &held.entry.action));
            hold(&mut kept.held, entry, why.clone());
            self.save(&kept);
        }
        outcome
    }

    /// Stops syncing: nothing more is applied, and what waits or was not sent is forgotten. Always
    /// works: the file is removed rather than written, and if that fails a fault in it reads as not
    /// connected anyway.
    pub fn disconnect(&self) {
        let mut kept = self.kept.locked();
        *kept = Kept::default();
        if let Some(path) = &self.path {
            if path.exists() {
                if let Err(e) = fs::remove_file(path) {
                    // Overwritten with nothing as a fallback: an unreadable file is not connected too.
                    eprintln!("replica: {e}");
                    let _ = fs::write(path, b"");
                }
            }
        }
    }

    pub fn connected(&self) -> bool {
        self.kept.locked().confirmed.is_some()
    }

    /// This computer's id in the log (empty while not connected).
    pub fn device(&self) -> String {
        self.kept.locked().device.clone()
    }

    /// The number of the last entry applied.
    pub fn confirmed(&self) -> Option<u64> {
        self.kept.locked().confirmed
    }

    /// When what this computer last took in was done (`Kept::seen`).
    pub fn seen(&self) -> Option<u64> {
        self.kept.locked().seen
    }

    /// The id of the last entry applied, if it is known.
    pub fn last(&self) -> Option<String> {
        self.kept.locked().last.clone()
    }

    /// What waits here.
    pub fn held(&self) -> Vec<Held> {
        self.kept.locked().held.clone()
    }

    /// What was done here and is to be sent, oldest first.
    pub fn outbox(&self) -> Vec<Entry> {
        self.kept.locked().pending.clone()
    }

    /// Something was done here (`handler::handle_shared`) at `now`: it goes into the outbox. Nothing while
    /// not connected.
    pub fn record(&self, action: Action, now: SystemTime) -> Option<Entry> {
        let mut kept = self.kept.locked();
        kept.confirmed?;
        let entry = Entry { id: new_id(), device: kept.device.clone(), at: state::to_unix(now).unwrap_or(0), action };
        if kept.pending.len() >= MAX_PENDING {
            kept.pending.remove(0);
        }
        kept.pending.push(entry.clone());
        self.save(&kept);
        Some(entry)
    }

    /// Applies the entries of the log `batch` (each with its number, as the server sends them, in
    /// order), from after the last one applied. One that is not an entry is skipped. A start and its stop
    /// in one batch (a computer catching up after a while) do nothing at all: that run is over. Says
    /// what became of each entry applied.
    pub fn receive(&self, context: &Context, batch: &[(u64, String)]) -> Vec<(String, Outcome)> {
        let mut kept = self.kept.locked();
        let Some(mut confirmed) = kept.confirmed else { return Vec::new() };
        let entries: Vec<(u64, Option<Entry>)> =
            batch.iter().filter(|(seq, _)| *seq > confirmed).map(|(seq, text)| (*seq, Entry::parse(text).ok())).collect();
        let over = runs_over(entries.iter().filter_map(|(_, entry)| entry.as_ref()));
        let mut outcomes = Vec::new();
        for (seq, entry) in entries {
            if seq <= confirmed {
                continue;
            }
            confirmed = seq;
            kept.confirmed = Some(seq);
            kept.last = entry.as_ref().map(|entry| entry.id.clone());
            kept.seen = entry.as_ref().map(|entry| entry.at).or(kept.seen);
            let Some(entry) = entry else { continue };
            if entry.device == kept.device {
                kept.pending.retain(|own| own.id != entry.id);
                continue;
            }
            if over.contains(&entry.id) {
                outcomes.push((entry.id.clone(), Outcome::Dropped("Started and stopped while away".into())));
                continue;
            }
            let keys = entry.action.keys();
            if !keys.is_empty() {
                kept.held.retain(|held| !supersedes(&entry.action, &held.entry.action));
                if overridden(&entry.action, &kept.pending) {
                    outcomes.push((entry.id.clone(), Outcome::Dropped("Changed here since".into())));
                    continue;
                }
            }
            let outcome = handler::apply(context, &entry.action, state::from_unix(entry.at));
            if matches!(outcome, Outcome::Applied) && keys.iter().any(|key| key == EVERYTHING) {
                // What was changed here since comes after the import: done again on top of it.
                let pending: Vec<Entry> = kept.pending.iter().filter(|own| own.action.is_library()).cloned().collect();
                for own in pending {
                    if let Outcome::Held(why) = handler::apply(context, &own.action, state::from_unix(own.at)) {
                        hold(&mut kept.held, own, why);
                    }
                }
            }
            if let Outcome::Held(why) = &outcome {
                hold(&mut kept.held, entry.clone(), why.clone());
            }
            outcomes.push((entry.id, outcome));
        }
        self.save(&kept);
        outcomes
    }

    /// Tries what waits again, in the order it came: what can be done now is done, what has nothing left
    /// to do is forgotten. Says how many are done or forgotten.
    pub fn retry(&self, context: &Context) -> usize {
        let mut kept = self.kept.locked();
        let before = kept.held.clone();
        let mut still = Vec::new();
        for held in before.iter().cloned() {
            match handler::apply(context, &held.entry.action, state::from_unix(held.entry.at)) {
                Outcome::Held(why) => still.push(Held { why, ..held }),
                Outcome::Applied | Outcome::Dropped(_) => {}
            }
        }
        let done = before.len() - still.len();
        if still != before {
            kept.held = still;
            self.save(&kept);
        }
        done
    }

    /// Written down; not being able to only costs what waits a restart (and a fault reads as not
    /// connected, never as more locked).
    fn save(&self, kept: &Kept) {
        if let Err(e) = self.write(kept) {
            eprintln!("replica: {e}");
        }
    }

    /// Through a temporary file, so a crash leaves the old file or the new.
    fn write(&self, kept: &Kept) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_string(kept).map_err(|e| e.to_string())?;
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

/// Puts an entry among what waits, forgetting the oldest beyond `MAX_HELD`.
fn hold(held: &mut Vec<Held>, entry: Entry, why: String) {
    held.retain(|h| h.entry.id != entry.id);
    if held.len() >= MAX_HELD {
        held.remove(0);
    }
    held.push(Held { entry, why });
}

/// Whether `new` sets everything `old` set, so `old` waits for nothing any more.
fn supersedes(new: &Action, old: &Action) -> bool {
    let (new, old_keys) = (new.keys(), old.keys());
    !old_keys.is_empty() && (old_keys.iter().all(|key| new.contains(key)) || (new.iter().any(|key| key == EVERYTHING) && old.is_library()))
}

/// Whether an entry from elsewhere sets only what an entry of this computer's own that is not numbered
/// yet sets too: that comes later in the order, so it is what stays.
fn overridden(action: &Action, pending: &[Entry]) -> bool {
    let keys = action.keys();
    if keys.iter().any(|key| key == EVERYTHING) {
        return pending.iter().any(|own| own.action.keys().iter().any(|key| key == EVERYTHING));
    }
    let own: BTreeSet<String> = pending.iter().flat_map(|own| own.action.keys()).collect();
    keys.iter().all(|key| own.contains(key) || (own.contains(EVERYTHING) && !key.starts_with("unlock:")))
}

/// The entries of runs that start and stop within `entries`: neither is to be done. A stop is left
/// out only when every start it names starts there.
fn runs_over<'a>(entries: impl Iterator<Item = &'a Entry> + Clone) -> BTreeSet<String> {
    let started: Vec<(&str, &str)> = entries
        .clone()
        .filter_map(|entry| match &entry.action {
            Action::Start { start, .. } => Some((start.as_str(), entry.id.as_str())),
            _ => None,
        })
        .collect();
    let mut over = BTreeSet::new();
    for entry in entries {
        if let Action::Stop { starts, .. } = &entry.action {
            let starting: Vec<&str> =
                starts.iter().filter_map(|s| started.iter().find(|(start, _)| start == s).map(|(_, id)| *id)).collect();
            if !starts.is_empty() && starting.len() == starts.len() {
                over.insert(entry.id.clone());
                over.extend(starting.into_iter().map(str::to_string));
            }
        }
    }
    over
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowance;
    use crate::definitions::tests::{session, target};
    use crate::definitions::Definitions;
    use crate::handler::handle_shared;
    use crate::proto::{Request, Response};
    use crate::session::{RunLock, Sessions};
    use crate::state::Store;
    use crate::unlock::{self, UnlockDelay};
    use crate::watchdog::AppSlot;
    use std::sync::{Arc, OnceLock};

    const HOUR: Duration = Duration::from_secs(3600);

    /// A computer: its service (`Context`) and its side of syncing, in a folder of its own.
    struct Node {
        context: Context,
        replica: Replica,
        dir: PathBuf,
    }

    fn node(name: &str) -> Node {
        let dir = std::env::temp_dir().join(format!("replica-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let sessions = Sessions::with_store(Store::new(dir.join("state.json")));
        let context = Context::new(sessions, Definitions::at(Some(dir.join("sessions.json"))), Arc::new(AppSlot::new(false)))
            .with_unlock(UnlockDelay::at(Some(dir.join("unlock.json"))));
        let replica = Replica::at(Some(dir.join("replica.json")));
        replica.connect(0, None, None).unwrap();
        Node { context, replica, dir }
    }

    impl Node {
        /// Asks this computer's service, as its window or CLI does; what it shares goes to the outbox.
        fn ask(&self, request: Request) -> Response {
            let (response, _, action) = handle_shared(&self.context, 0, request);
            if let Some(action) = action {
                self.replica.record(action, self.now());
            }
            response
        }

        fn ok(&self, request: Request) {
            assert_eq!(self.ask(request), Response::Done);
        }

        fn refused(&self, request: Request) -> String {
            match self.ask(request) {
                Response::Failed(why) | Response::Refused(why) => why,
                other => panic!("not refused: {other:?}"),
            }
        }

        fn now(&self) -> SystemTime {
            self.context.sessions().now()
        }

        fn runs(&self, id: &str) -> bool {
            self.context.sessions().running_ids().iter().any(|r| r == id)
        }

        fn locked(&self) -> bool {
            self.context.sessions().locked().is_some()
        }

        /// Moves its clock on to `time`.
        fn at(&self, time: SystemTime) {
            self.context.sessions().move_to(time);
        }

        /// The way out, as at this computer's keyboard: ask to unlock, wait out the longest delay there
        /// can be, unlock. True when nothing runs afterwards.
        fn unlocks(&self) -> bool {
            self.ok(Request::AskForUnlock);
            self.at(self.now() + Duration::from_secs(unlock::MAX_DELAY + 1));
            self.ask(Request::Unlock) == Response::Done && self.context.sessions().running_ids().is_empty()
        }
    }

    /// The server's log: every entry sent, numbered in the order it came.
    #[derive(Default)]
    struct Log {
        entries: Vec<(u64, String)>,
    }

    impl Log {
        fn append(&mut self, text: String) {
            let seq = self.entries.len() as u64 + 1;
            self.entries.push((seq, text));
        }

        /// What a computer sent and the log does not have yet.
        fn send(&mut self, node: &Node) {
            for entry in node.replica.outbox() {
                let known = self.entries.iter().any(|(_, text)| Entry::parse(text).is_ok_and(|e| e.id == entry.id));
                if !known {
                    self.append(entry.text());
                }
            }
        }

        /// Every computer sends, then every computer takes in the log and tries again what waits.
        fn sync(&mut self, nodes: &[&Node]) {
            for node in nodes {
                self.send(node);
            }
            for node in nodes {
                node.replica.receive(&node.context, &self.entries);
                node.replica.retry(&node.context);
            }
        }

        /// An entry made up by a computer `device` (or anyone with the token), as it would be sent.
        fn forge(&mut self, action: Action, at: u64) {
            self.append(Entry { id: new_id(), device: "forger".into(), at, action }.text());
        }
    }

    fn millis(time: SystemTime) -> u64 {
        state::to_unix(time).unwrap()
    }

    /// A password's hash, made once (it takes a moment).
    fn hash() -> String {
        static HASH: OnceLock<String> = OnceLock::new();
        HASH.get_or_init(|| unlock::new_hash("secret").unwrap()).clone()
    }

    /// Two computers sharing a target "social" and a session "work" of it with `lock`.
    fn pair(name: &str, lock: Lock) -> (Node, Node, Log) {
        let (a, b, mut log) = (node(&format!("{name}-a")), node(&format!("{name}-b")), Log::default());
        a.ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        a.ok(Request::SaveSession { session: Definition { lock, ..session("work", &["social"]) } });
        log.sync(&[&a, &b]);
        assert!(b.context.library().session("work").is_some());
        (a, b, log)
    }

    #[test]
    fn a_start_on_one_computer_runs_on_the_other_and_so_does_its_stop() {
        let (a, b, mut log) = pair("start", Lock::None);
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        assert!(b.runs("work"));
        assert_eq!(a.context.sessions().starts_of("work"), b.context.sessions().starts_of("work"));
        b.ok(Request::Stop { id: "work".into(), password: None, text: None });
        log.sync(&[&a, &b]);
        assert!(!a.runs("work") && !b.runs("work"));
        // Nothing waits, and nothing is sent twice.
        assert!(a.replica.held().is_empty() && b.replica.held().is_empty());
        assert!(a.replica.outbox().is_empty() && b.replica.outbox().is_empty());
    }

    #[test]
    fn settings_edits_and_deletes_reach_every_computer() {
        let (a, b, mut log) = pair("edits", Lock::None);
        b.ok(Request::SaveTarget { target: target("social", &["youtube.com", "reddit.com"]) });
        b.ok(Request::Duplicate { kind: Kind::Target, id: "social".into(), new_id: "copy".into() });
        b.ok(Request::SetUnlockDelay { delay: "2h".into() });
        log.sync(&[&a, &b]);
        assert_eq!(a.context.library(), b.context.library());
        assert_eq!(a.context.unlock().delay(), 7200);
        a.ok(Request::DeleteTarget { id: "copy".into() });
        a.ok(Request::DeleteSession { id: "work".into() });
        log.sync(&[&a, &b]);
        assert_eq!(a.context.library(), b.context.library());
        assert!(b.context.library().target("copy").is_none());
    }

    #[test]
    fn a_timed_lock_ends_at_the_same_moment_everywhere_and_a_stop_before_waits() {
        let (a, b, mut log) = pair("timed", Lock::timed_for("2h"));
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        assert_eq!(a.context.sessions().lock_of("work"), b.context.sessions().lock_of("work"));
        assert!(b.locked());
        // An entry claiming a stop is refused here as at the keyboard: it waits.
        log.forge(
            Action::Stop { id: "work".into(), starts: b.context.sessions().starts_of("work"), text: None, proof: None },
            millis(b.now()),
        );
        log.sync(&[&a, &b]);
        assert!(b.runs("work"));
        assert_eq!(b.replica.held().len(), 1);
        assert!(b.replica.held()[0].why.contains("locked"));
        // Once the time is up the session ends by itself, and the stop has nothing left to do.
        let later = b.now() + 2 * HOUR + Duration::from_secs(1);
        a.at(later);
        b.at(later);
        log.sync(&[&a, &b]);
        assert!(!b.runs("work") && !a.runs("work"));
        assert!(b.replica.held().is_empty());
    }

    #[test]
    fn a_start_from_elsewhere_never_locks_longer_than_a_start_here_would() {
        let (_a, b, mut log) = pair("longer", Lock::timed_for("1h"));
        log.forge(Action::Start { id: "work".into(), start: new_id(), ends: Some(u64::MAX / 2), text: None, zone: None }, millis(b.now()));
        log.sync(&[&b]);
        let Some(RunLock::Ends { at }) = b.context.sessions().lock_of("work") else { panic!("not timed") };
        assert!(at <= millis(b.now() + HOUR) + 1000);
        // A lock that has ended there starts nothing here.
        b.context.sessions().reset();
        log.forge(Action::Start { id: "work".into(), start: new_id(), ends: Some(1), text: None, zone: None }, 1);
        log.sync(&[&b]);
        assert!(!b.runs("work"));
    }

    #[test]
    fn a_start_from_elsewhere_has_the_lock_this_computer_has_defined() {
        // The entry names a random text, but the session here has a text of its own to type.
        let (_a, b, mut log) = pair("defined", Lock::Text { text: "my own text".into() });
        log.forge(Action::Start { id: "work".into(), start: new_id(), ends: Some(u64::MAX), text: Some("x".into()), zone: None }, 0);
        log.sync(&[&b]);
        assert_eq!(b.context.sessions().lock_of("work"), Some(RunLock::Text { text: "my own text".into() }));
    }

    #[test]
    fn starts_made_at_once_on_two_computers_end_only_when_both_are_stopped() {
        let (a, b, mut log) = pair("both", Lock::None);
        a.ok(Request::Start { id: "work".into() });
        b.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        assert_eq!(a.context.sessions().starts_of("work").len(), 2);
        assert_eq!(b.context.sessions().starts_of("work").len(), 2);
        // A stop names both starts it stood for, so it ends the session everywhere.
        a.ok(Request::Stop { id: "work".into(), password: None, text: None });
        log.sync(&[&a, &b]);
        assert!(!a.runs("work") && !b.runs("work"));
        // A stop made before another start was heard of ends only what it knew.
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        b.context.sessions().join("work", "unheard", crate::session::StartLock::None).unwrap();
        a.ok(Request::Stop { id: "work".into(), password: None, text: None });
        log.sync(&[&a, &b]);
        assert!(b.runs("work"));
        assert_eq!(b.context.sessions().starts_of("work"), vec!["unheard".to_string()]);
    }

    #[test]
    fn a_held_stop_never_stands_in_the_way_of_unlocking_here() {
        let (a, b, mut log) = pair("held", Lock::None);
        // A password lock needs a password, which never travels: the hash does.
        a.ok(Request::SaveSession {
            session: Definition { lock: Lock::Password { password: Some("secret".into()), hash: None }, ..session("work", &["social"]) },
        });
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        assert!(b.runs("work") && b.locked());
        // A stop without the password's signature waits here for the password.
        log.forge(Action::Stop { id: "work".into(), starts: b.context.sessions().starts_of("work"), text: None, proof: None }, 0);
        log.sync(&[&a, &b]);
        assert!(a.runs("work") && b.runs("work"));
        assert_eq!(b.replica.held().len(), 1);
        b.ok(Request::Stop { id: "work".into(), password: Some("secret".into()), text: None });
        log.sync(&[&a, &b]);
        assert!(!b.runs("work"));
        assert!(b.replica.held().is_empty());
        // And unlocking ends it too, whatever waits.
        a.ok(Request::Start { id: "work".into() });
        log.forge(Action::Stop { id: "work".into(), starts: vec!["other".into()], text: None, proof: None }, 0);
        log.sync(&[&a, &b]);
        assert!(b.unlocks());
    }

    #[test]
    fn a_stop_with_the_password_travels_as_its_signature_for_that_run_only() {
        let (a, b, mut log) = pair("signed", Lock::None);
        a.ok(Request::SaveSession {
            session: Definition { lock: Lock::Password { password: Some("secret".into()), hash: None }, ..session("work", &["social"]) },
        });
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        let first = b.context.sessions().starts_of("work");
        a.ok(Request::Stop { id: "work".into(), password: Some("secret".into()), text: None });
        let Some(Entry { action: Action::Stop { proof: Some(proof), .. }, .. }) = a.replica.outbox().last().cloned() else {
            panic!("no proof")
        };
        log.sync(&[&a, &b]);
        assert!(!b.runs("work"));
        // The same signature for the next run is worth nothing: it signed the starts it ended.
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        let second = b.context.sessions().starts_of("work");
        assert_ne!(first, second);
        log.forge(Action::Stop { id: "work".into(), starts: second.clone(), text: None, proof: Some(proof.clone()) }, 0);
        log.forge(Action::Stop { id: "work".into(), starts: second, text: None, proof: Some("ab".repeat(64)) }, 0);
        log.sync(&[&b]);
        assert!(b.runs("work"));
        assert_eq!(b.replica.held().len(), 2);
    }

    #[test]
    fn an_unlock_with_the_password_travels_as_its_signature_of_the_time() {
        let (a, b, mut log) = pair("signed-unlock", Lock::UntilRestart);
        a.ok(Request::SetUnlockDelay { delay: "24h".into() });
        a.ok(Request::SetUnlockWay { way: Some(Lock::Password { password: Some("out".into()), hash: None }) });
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        assert!(b.locked());
        a.ok(Request::UnlockWith { password: Some("out".into()), text: None });
        let Some(Entry { action: Action::Unlock { proof: Some(proof), .. }, .. }) = a.replica.outbox().last().cloned() else {
            panic!("no proof")
        };
        log.sync(&[&a, &b]);
        assert!(!b.runs("work"));
        // Sent again later, it ends nothing started after it was made.
        b.ok(Request::Start { id: "work".into() });
        log.forge(Action::Unlock { text: None, next: None, proof: Some(proof.clone()) }, millis(b.now()));
        // Nor does a signature of another time, or a made-up one.
        log.forge(Action::Unlock { text: None, next: None, proof: Some(Proof { at: millis(b.now()), ..proof }) }, millis(b.now()));
        log.forge(
            Action::Unlock { text: None, next: None, proof: Some(Proof { at: millis(b.now()), signature: "00".repeat(64) }) },
            millis(b.now()),
        );
        log.sync(&[&b]);
        assert!(b.runs("work"));
    }

    #[test]
    fn a_budget_is_one_for_all_computers() {
        use crate::allowance::{Anchor, Reset, Scope, Terms};
        let budget =
            Terms::Budget { allowed: "1h".into(), scope: Scope::Total, reset: Reset::Never, every: String::new(), anchor: Anchor::Clock };
        let (a, b, mut log) = pair("budget", Lock::None);
        a.ok(Request::SaveSession { session: Definition { allowance: Some(budget), ..session("work", &["social"]) } });
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        let left = |node: &Node| node.context.sessions().status().running[0].allowance.as_ref().unwrap().left;
        assert_eq!(left(&b), Some(3600));
        // What A counts is told to the others, once.
        a.context.sessions().polled();
        std::thread::sleep(Duration::from_millis(20));
        a.context.sessions().charge(&[("work".into(), allowance::TOTAL.into())]);
        let unshared = a.context.sessions().take_unshared();
        assert_eq!(unshared.len(), 1);
        assert!(a.context.sessions().take_unshared().is_empty());
        for (id, mut used) in unshared {
            // As if half an hour of it had been counted.
            used.insert(allowance::TOTAL.to_string(), 30 * 60 * 1000);
            a.replica.record(Action::Used { id, used }, a.now());
        }
        log.sync(&[&a, &b]);
        assert_eq!(left(&b), Some(1800));
        // Use from elsewhere only ever counts against a session, and never beyond its budget.
        log.forge(
            Action::Used { id: "work".into(), used: [(allowance::TOTAL.to_string(), u64::MAX), ("site:nope".to_string(), 5)].into() },
            0,
        );
        log.sync(&[&b]);
        assert_eq!(left(&b), Some(0));
        log.forge(Action::Used { id: "gone".into(), used: [(allowance::TOTAL.to_string(), 1)].into() }, 0);
        log.sync(&[&b]);
        assert!(b.replica.held().is_empty());
    }

    #[test]
    fn a_restart_lock_waits_for_this_computers_own_restart() {
        let (a, b, mut log) = pair("restart", Lock::UntilRestart);
        b.ok(Request::SetUnlockDelay { delay: "1h".into() });
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        log.forge(Action::Stop { id: "work".into(), starts: b.context.sessions().starts_of("work"), text: None, proof: None }, 0);
        log.sync(&[&a, &b]);
        assert!(b.runs("work"));
        assert!(b.replica.held()[0].why.contains("restarts"));
        // Windows restarts here: the session ends with it, and the stop has nothing left to do.
        let restarted = b.context.sessions().restarted("after");
        let context = Context::new(restarted, Definitions::at(Some(b.dir.join("sessions.json"))), Arc::new(AppSlot::new(false)))
            .with_unlock(UnlockDelay::at(Some(b.dir.join("unlock.json"))));
        let b = Node { context, replica: Replica::at(Some(b.dir.join("replica.json"))), dir: b.dir.clone() };
        assert!(b.replica.connected());
        assert!(!b.runs("work"));
        log.sync(&[&a, &b]);
        assert!(b.replica.held().is_empty());
    }

    #[test]
    fn the_servers_view_takes_what_a_computer_did_as_done() {
        let (_a, server, _log) = pair("mirror", Lock::UntilRestart);
        server.ok(Request::SetUnlockDelay { delay: "24h".into() });
        server.ok(Request::Start { id: "work".into() });
        let starts = server.context.sessions().starts_of("work");
        let at = server.now();
        let mirror = |action: Action| handler::mirror(&server.context, &action, at);
        // What its own rules refuse is taken as done: a computer did it under its own.
        // A target made looser waits there as here, while what it runs is locked; then it is made.
        assert_eq!(mirror(Action::SaveTarget { target: target("social", &[]) }), Outcome::Applied);
        assert!(server.context.library().target("social").unwrap().desired().entries.filters.is_empty());
        assert_eq!(mirror(Action::Stop { id: "work".into(), starts, text: None, proof: None }), Outcome::Applied);
        assert!(!server.runs("work"));
        server.ask(Request::Status);
        assert!(server.context.library().target("social").unwrap().entries.filters.is_empty());
        server.ok(Request::SaveTarget { target: target("social", &["a.com"]) });
        server.ok(Request::Start { id: "work".into() });
        for action in [
            Action::SaveSession { session: session("work", &["social"]) },
            Action::SetUnlockDelay { delay: "0".into() },
            Action::SetUnlockWay { way: Some(Lock::Text { text: "out".into() }) },
            Action::SaveCondition { condition: crate::definitions::tests::weekly("c", "mon 9-17") },
            Action::DeleteCondition { id: "c".into() },
        ] {
            assert_eq!(mirror(action), Outcome::Applied);
        }
        assert_eq!(server.context.unlock().delay(), 0);
        server.context.sessions().reset();
        server.ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        server.ok(Request::SetUnlockDelay { delay: "24h".into() });
        server.ok(Request::Start { id: "work".into() });
        assert_eq!(mirror(Action::DeleteTarget { id: "social".into() }), Outcome::Applied);
        assert_eq!(mirror(Action::Import { library: Library::default() }), Outcome::Applied);
        assert!(server.context.library().sessions.is_empty());
        // An unlock is taken as done, for what started by then.
        server.ok(Request::SaveTarget { target: target("social", &["a.com"]) });
        server.ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        server.ok(Request::Start { id: "work".into() });
        assert_eq!(
            handler::mirror(&server.context, &Action::Unlock { text: None, next: None, proof: None }, server.now()),
            Outcome::Applied
        );
        assert!(!server.runs("work"));
        // What has nothing to do is dropped, as anywhere.
        assert!(matches!(mirror(Action::DeleteSession { id: "nope".into() }), Outcome::Dropped(_)));
        assert!(matches!(mirror(Action::SaveTarget { target: target("bad", &["not a site at all !"]) }), Outcome::Dropped(_)));
    }

    #[test]
    fn a_password_never_travels() {
        let (a, b, mut log) = pair("password", Lock::None);
        a.ok(Request::SaveSession {
            session: Definition { lock: Lock::Password { password: Some("secret".into()), hash: None }, ..session("work", &["social"]) },
        });
        a.ok(Request::SetUnlockWay { way: Some(Lock::Password { password: Some("another".into()), hash: None }) });
        a.ok(Request::Start { id: "work".into() });
        a.ok(Request::Stop { id: "work".into(), password: Some("secret".into()), text: None });
        a.ok(Request::UnlockWith { password: Some("another".into()), text: None });
        let sent: Vec<String> = a.replica.outbox().iter().map(Entry::text).collect();
        assert!(sent.iter().all(|text| !text.contains("secret") && !text.contains("another")), "{sent:?}");
        log.sync(&[&a, &b]);
        // The hashes made there are used here: the passwords work on this computer too.
        assert_eq!(
            b.context.library().session("work").unwrap().lock.password_hash(),
            a.context.library().session("work").unwrap().lock.password_hash()
        );
        b.ok(Request::Start { id: "work".into() });
        b.ok(Request::Stop { id: "work".into(), password: Some("secret".into()), text: None });
        b.ok(Request::UnlockWith { password: Some("another".into()), text: None });
    }

    #[test]
    fn a_random_text_is_drawn_once_and_typed_once() {
        let (a, b, mut log) = pair("random", Lock::RandomText { length: 12, text: None });
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        let text = match a.context.sessions().lock_of("work") {
            Some(RunLock::Text { text }) => text,
            other => panic!("{other:?}"),
        };
        assert_eq!(b.context.sessions().lock_of("work"), Some(RunLock::Text { text: text.clone() }));
        a.ok(Request::Stop { id: "work".into(), password: None, text: Some(text) });
        log.sync(&[&a, &b]);
        assert!(!b.runs("work"));
        // A wrong text from elsewhere waits, as a wrong text typed here is refused.
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        log.forge(
            Action::Stop { id: "work".into(), starts: b.context.sessions().starts_of("work"), text: Some("wrong".into()), proof: None },
            0,
        );
        log.sync(&[&a, &b]);
        assert!(b.runs("work"));
    }

    #[test]
    fn unlocking_travels_and_waits_for_the_countdown_here() {
        let (a, b, mut log) = pair("unlock", Lock::UntilRestart);
        a.ok(Request::SetUnlockDelay { delay: "1h".into() });
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        assert!(b.locked());
        a.ok(Request::AskForUnlock);
        log.sync(&[&a, &b]);
        let asked_a = a.context.unlock().requested().unwrap();
        let asked_b = b.context.unlock().requested().unwrap();
        assert!(asked_b.abs_diff(asked_a) < 5000, "one countdown for both");
        // An unlock that claims to be ready before the countdown is over here waits.
        log.forge(Action::Unlock { text: None, next: None, proof: None }, millis(b.now()));
        log.sync(&[&a, &b]);
        assert!(b.runs("work"));
        let later = b.now() + HOUR + Duration::from_secs(5);
        a.at(later);
        b.at(later);
        log.sync(&[&a, &b]);
        // The countdown is over on both now: the unlock that waited is done on both, and it used up
        // the countdown, as an unlock does.
        assert!(!b.runs("work") && !a.runs("work"));
        assert_eq!(a.context.unlock().requested(), None);
        assert_eq!(b.context.unlock().requested(), None);
        assert!(a.replica.held().is_empty() && b.replica.held().is_empty());
        // An unlock made on A itself travels as well.
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        a.ok(Request::AskForUnlock);
        log.sync(&[&a, &b]);
        let later = b.now() + HOUR + Duration::from_secs(5);
        a.at(later);
        b.at(later);
        a.ok(Request::Unlock);
        log.sync(&[&a, &b]);
        assert!(!a.runs("work") && !b.runs("work"));
    }

    #[test]
    fn times_beyond_any_calendar_are_taken_as_the_latest() {
        let (_a, b, mut log) = pair("overflow", Lock::UntilRestart);
        b.ok(Request::SetUnlockDelay { delay: "1h".into() });
        b.ok(Request::Start { id: "work".into() });
        for action in [
            Action::AskForUnlock,
            Action::AbortUnlock,
            Action::Unlock { text: None, next: None, proof: None },
            Action::AskToStop { id: "work".into() },
        ] {
            log.forge(action, u64::MAX);
        }
        log.sync(&[&b]);
        assert!(b.runs("work"));
        assert!(b.unlocks());
    }

    #[test]
    fn a_countdown_from_elsewhere_never_began_earlier_than_the_grace() {
        let (_a, b, mut log) = pair("grace", Lock::UntilRestart);
        b.ok(Request::SetUnlockDelay { delay: "1h".into() });
        b.ok(Request::Start { id: "work".into() });
        log.forge(Action::AskForUnlock, 0);
        log.sync(&[&b]);
        let left = b.context.unlock().remaining(b.now()).unwrap();
        assert!(left >= 3600 - GRACE.as_secs() - 1, "{left}");
        assert!(b.runs("work"));
        // The same for a delay lock's wait.
        let (_a, b, mut log) = pair("grace-stop", Lock::Delay { delay: "1h".into() });
        b.ok(Request::Start { id: "work".into() });
        log.forge(Action::AskToStop { id: "work".into() }, 0);
        log.sync(&[&b]);
        assert!(b.locked());
        assert!(b.refused(Request::Stop { id: "work".into(), password: None, text: None }).contains("can be stopped in"));
    }

    /// A time from elsewhere that is still to come (a computer whose clock is far ahead, or a forged
    /// entry) never moves a countdown here into the future: it would never end, and asking again
    /// changes nothing while one runs. It counts from now at the latest.
    #[test]
    fn a_countdown_from_elsewhere_never_begins_later_than_now() {
        let (_a, b, mut log) = pair("future", Lock::UntilRestart);
        b.ok(Request::SetUnlockDelay { delay: "1h".into() });
        b.ok(Request::Start { id: "work".into() });
        let year = millis(b.now()) + 365 * 86_400_000;
        log.forge(Action::AskForUnlock, year);
        log.sync(&[&b]);
        assert!(b.context.unlock().requested().is_some_and(|asked| asked <= millis(b.now())));
        b.at(b.now() + Duration::from_secs(3601));
        assert_eq!(b.ask(Request::Unlock), Response::Done);
        assert!(!b.runs("work"));
        // The same for a delay lock's wait.
        let (_a, b, mut log) = pair("future-stop", Lock::Delay { delay: "1h".into() });
        b.ok(Request::Start { id: "work".into() });
        log.forge(Action::AskToStop { id: "work".into() }, millis(b.now()) + 365 * 86_400_000);
        log.sync(&[&b]);
        b.at(b.now() + Duration::from_secs(3601));
        b.ok(Request::Stop { id: "work".into(), password: None, text: None });
    }

    /// A session locked while a condition is active (here a flash drive that is not plugged in) stays
    /// locked whatever arrives about that condition or the session: the condition deleted, made one
    /// that is never active, imported away, or the lock saved away.
    #[test]
    fn a_condition_lock_is_not_opened_from_elsewhere() {
        let drive = || Condition {
            id: "key".into(),
            name: "Key".into(),
            rule: crate::condition::Rule::FlashDrive { serial: "4E0985D05EC9".into(), drive: "Key".into() },
        };
        let (a, b, mut log) = pair("condition-lock", Lock::None);
        a.ok(Request::SaveCondition { condition: drive() });
        a.ok(Request::SaveSession {
            session: Definition { lock: Lock::Condition { condition: "key".into() }, ..session("work", &["social"]) },
        });
        log.sync(&[&a, &b]);
        b.ok(Request::Start { id: "work".into() });
        assert!(b.locked());
        let never = Condition { rule: crate::definitions::tests::weekly("key", "mon 9-10").rule, ..drive() };
        for action in [
            Action::DeleteCondition { id: "key".into() },
            Action::SaveCondition { condition: never },
            Action::Import { library: Library::default() },
            Action::SaveSession { session: session("work", &["social"]) },
            Action::DeleteSession { id: "work".into() },
        ] {
            log.forge(action, millis(b.now()));
            log.sync(&[&b]);
            b.ask(Request::Status);
            assert!(b.locked(), "{:?}", b.context.library());
            assert!(b.refused(Request::Stop { id: "work".into(), password: None, text: None }).contains("flash drive"));
        }
    }

    #[test]
    fn an_unlock_from_elsewhere_ends_only_what_started_before_it() {
        let (a, b, mut log) = pair("before", Lock::None);
        a.ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        log.sync(&[&a, &b]);
        let before = millis(b.now()) - 10_000;
        b.ok(Request::Start { id: "work".into() });
        log.forge(Action::Unlock { text: None, next: None, proof: None }, before);
        log.sync(&[&a, &b]);
        assert!(b.runs("work"));
        // With nothing to end, it waits for nothing.
        assert!(b.replica.held().is_empty());
    }

    #[test]
    fn an_abort_from_elsewhere_never_ends_a_countdown_asked_for_after_it() {
        let (_a, b, mut log) = pair("abort", Lock::UntilRestart);
        b.ok(Request::SetUnlockDelay { delay: "1h".into() });
        b.ok(Request::Start { id: "work".into() });
        let earlier = millis(b.now()) - 10_000;
        b.ok(Request::AskForUnlock);
        log.forge(Action::AbortUnlock, earlier);
        log.sync(&[&b]);
        assert!(b.context.unlock().requested().is_some());
        log.forge(Action::AbortUnlock, millis(b.now()));
        log.sync(&[&b]);
        assert!(b.context.unlock().requested().is_none());
    }

    #[test]
    fn a_shorter_delay_or_a_new_way_to_unlock_from_elsewhere_waits_while_locked() {
        let (a, b, mut log) = pair("shorter", Lock::UntilRestart);
        b.ok(Request::SetUnlockDelay { delay: "2h".into() });
        log.sync(&[&a, &b]);
        b.ok(Request::Start { id: "work".into() });
        a.ok(Request::SetUnlockDelay { delay: "0".into() });
        a.ok(Request::SetUnlockWay { way: Some(Lock::Text { text: "let me out".into() }) });
        log.sync(&[&a, &b]);
        assert_eq!(b.context.unlock().delay(), 7200);
        assert!(b.context.unlock().way().is_none());
        assert_eq!(b.replica.held().len(), 2);
        // A longer delay is taken at once, and takes the place of the shorter one that waited.
        a.ok(Request::SetUnlockDelay { delay: "3h".into() });
        log.sync(&[&a, &b]);
        assert_eq!(b.context.unlock().delay(), 3 * 3600);
        assert_eq!(b.replica.held().len(), 1);
        // Never more than 24 hours, whatever an entry says.
        log.forge(Action::SetUnlockDelay { delay: "999d".into() }, 0);
        log.sync(&[&b]);
        assert_eq!(b.context.unlock().delay(), 3 * 3600);
        assert!(b.unlocks());
    }

    #[test]
    fn an_edit_from_elsewhere_that_loosens_a_locked_session_waits() {
        let (a, b, mut log) = pair("loosen", Lock::UntilRestart);
        b.ok(Request::Start { id: "work".into() });
        a.ok(Request::SaveTarget { target: target("social", &[]) });
        a.ok(Request::DeleteSession { id: "work".into() });
        a.ok(Request::Import { library: Library::default() });
        log.sync(&[&a, &b]);
        assert_eq!(b.context.library().target("social").unwrap().entries.filters, vec!["youtube.com".to_string()]);
        assert!(b.runs("work"));
        // The import takes the place of the edits before it: only it waits.
        assert_eq!(b.replica.held().len(), 1);
        assert!(b.unlocks());
        log.sync(&[&a, &b]);
        assert!(b.replica.held().is_empty());
        assert_eq!(a.context.library(), b.context.library());
    }

    #[test]
    fn a_target_made_looser_elsewhere_waits_here_while_locked_and_is_made_once_it_is_not() {
        let (a, b, mut log) = pair("waits", Lock::UntilRestart);
        b.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        a.ok(Request::SaveTarget { target: target("social", &["reddit.com"]) });
        log.sync(&[&a, &b]);
        // Nothing is held: what blocks more is made at once, what blocks less waits in the target.
        assert!(b.replica.held().is_empty());
        let social = b.context.library().target("social").unwrap().clone();
        assert_eq!(social.entries.filters, vec!["reddit.com".to_string(), "youtube.com".to_string()]);
        assert_eq!(social.desired().entries.filters, vec!["reddit.com".to_string()]);
        // A runs the locked session too (what runs is shared): there it waits as well.
        assert_eq!(a.context.library(), b.context.library());
        // Unlocked: made on both, each by itself.
        assert!(b.unlocks());
        log.sync(&[&a, &b]);
        for node in [&a, &b] {
            node.ask(Request::Status);
            let social = node.context.library().target("social").unwrap().clone();
            assert_eq!((social.entries.filters, social.waiting), (vec!["reddit.com".to_string()], None));
        }
        assert_eq!(a.context.library(), b.context.library());
    }

    #[test]
    fn an_edit_made_here_and_not_yet_sent_is_what_every_computer_ends_up_with() {
        let (a, b, mut log) = pair("order", Lock::None);
        a.ok(Request::SaveTarget { target: target("social", &["a.com"]) });
        log.send(&a);
        // B changes it before hearing of A's change: B's comes later, so B's is what stays, and B
        // never shows A's in between.
        b.ok(Request::SaveTarget { target: target("social", &["b.com"]) });
        b.replica.receive(&b.context, &log.entries);
        assert_eq!(b.context.library().target("social").unwrap().entries.filters, vec!["b.com".to_string()]);
        log.sync(&[&a, &b]);
        assert_eq!(a.context.library().target("social").unwrap().entries.filters, vec!["b.com".to_string()]);
        assert_eq!(a.context.library(), b.context.library());
    }

    #[test]
    fn an_import_from_elsewhere_keeps_what_was_changed_here_since() {
        let (a, b, mut log) = pair("import", Lock::None);
        let mut library = a.context.library();
        library.targets.push(target("news", &["news.com"]));
        a.ok(Request::Import { library });
        log.send(&a);
        b.ok(Request::SaveTarget { target: target("social", &["b.com"]) });
        log.sync(&[&a, &b]);
        assert!(b.context.library().target("news").is_some());
        assert_eq!(b.context.library().target("social").unwrap().entries.filters, vec!["b.com".to_string()]);
        assert_eq!(a.context.library(), b.context.library());
    }

    #[test]
    fn a_run_that_started_and_stopped_while_away_does_nothing() {
        let (a, b, mut log) = pair("away", Lock::None);
        a.ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        log.sync(&[&a, &b]);
        // On A a restart lock, stopped later (here by unlocking, which shares no stop: Stop for it
        // is made up as if A had restarted and stopped it).
        a.ok(Request::Start { id: "work".into() });
        log.send(&a);
        log.forge(Action::Stop { id: "work".into(), starts: a.context.sessions().starts_of("work"), text: None, proof: None }, 0);
        // B was away for all of it: it starts nothing, so nothing waits for its restart.
        log.sync(&[&b]);
        assert!(!b.runs("work"));
        assert!(b.replica.held().is_empty());
    }

    #[test]
    fn entries_are_taken_once_and_garbage_changes_nothing() {
        let (a, b, mut log) = pair("garbage", Lock::None);
        let library = b.context.library();
        log.append("not json".into());
        log.append("{}".into());
        log.append(r#"{"id":"","device":"x","at":0,"action":{"action":"ask_for_unlock"}}"#.to_string());
        log.append(format!("{{\"id\":\"{}\",\"device\":\"x\",\"at\":0,\"action\":{{\"action\":\"no_such\"}}}}", "a".repeat(10)));
        log.append(format!("{{\"id\":\"{}\",\"device\":\"x\",\"at\":0,\"action\":{{\"action\":\"ask_for_unlock\"}}}}", "a".repeat(65)));
        log.append(" ".repeat(MAX_ENTRY + 1));
        log.sync(&[&a, &b]);
        assert_eq!(b.context.library(), library);
        assert_eq!(b.context.unlock().requested(), None);
        assert_eq!(b.replica.confirmed(), Some(log.entries.len() as u64));
        // The same entries again, or older ones, are not applied twice.
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        b.context.sessions().reset();
        b.replica.receive(&b.context, &log.entries);
        assert!(!b.runs("work"));
    }

    #[test]
    fn nothing_is_applied_or_recorded_while_not_connected() {
        let (a, b, mut log) = pair("offline", Lock::None);
        b.replica.disconnect();
        assert!(!b.replica.connected() && b.replica.device().is_empty());
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        assert!(!b.runs("work"));
        assert!(b.replica.record(Action::AskForUnlock, b.now()).is_none());
        assert!(b.replica.outbox().is_empty());
    }

    #[test]
    fn disconnecting_always_works_and_forgets_what_waits() {
        let (a, b, mut log) = pair("disconnect", Lock::UntilRestart);
        a.ok(Request::Start { id: "work".into() });
        log.sync(&[&a, &b]);
        log.forge(Action::Stop { id: "work".into(), starts: b.context.sessions().starts_of("work"), text: None, proof: None }, 0);
        log.sync(&[&a, &b]);
        assert_eq!(b.replica.held().len(), 1);
        // Even with its file made unwritable by a folder in its place.
        let file = b.dir.join("replica.json");
        let _ = fs::remove_file(&file);
        fs::create_dir_all(&file).unwrap();
        b.replica.disconnect();
        assert!(!b.replica.connected());
        assert!(b.replica.held().is_empty());
        assert!(Replica::at(Some(file)).held().is_empty());
        // Abort floods from elsewhere reach it no more: the countdown here runs out.
        b.ok(Request::AskForUnlock);
        for _ in 0..3 {
            log.forge(Action::AbortUnlock, millis(b.now()));
        }
        log.sync(&[&b]);
        assert!(b.context.unlock().requested().is_some());
        assert!(b.unlocks());
    }

    #[test]
    fn a_broken_or_missing_file_is_not_connected() {
        let dir = std::env::temp_dir().join(format!("replica-{}-broken", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("replica.json");
        assert!(!Replica::at(Some(file.clone())).connected());
        fs::write(&file, b"{nonsense").unwrap();
        assert!(!Replica::at(Some(file.clone())).connected());
        // Connected without a device is a fault too.
        fs::write(&file, br#"{"device":"","confirmed":5}"#).unwrap();
        assert!(!Replica::at(Some(file.clone())).connected());
        // What a connected computer keeps comes back.
        let replica = Replica::at(Some(file.clone()));
        let device = replica.connect(7, None, None).unwrap();
        replica.record(Action::AskForUnlock, SystemTime::now()).unwrap();
        let again = Replica::at(Some(file));
        assert_eq!((again.device(), again.confirmed(), again.outbox().len()), (device, Some(7), 1));
        assert!(Replica::at(None).connect(0, None, None).is_ok());
    }

    #[test]
    fn what_waits_is_bounded_and_the_oldest_goes_first() {
        let mut held = Vec::new();
        let entry = |n: usize| Entry { id: format!("e{n}"), device: "x".into(), at: 0, action: Action::AskForUnlock };
        for n in 0..MAX_HELD + 5 {
            hold(&mut held, entry(n), "why".into());
        }
        assert_eq!(held.len(), MAX_HELD);
        assert_eq!(held[0].entry.id, "e5");
        // The same entry held again is held once.
        hold(&mut held, entry(5), "again".into());
        assert_eq!(held.len(), MAX_HELD);
        assert_eq!(held.last().unwrap().why, "again");
        // And the outbox too.
        let replica = Replica::at(None);
        replica.connect(0, None, None).unwrap();
        for _ in 0..MAX_PENDING + 2 {
            replica.record(Action::AskForUnlock, SystemTime::now());
        }
        assert_eq!(replica.outbox().len(), MAX_PENDING);
    }

    #[test]
    fn what_an_entry_sets_and_what_takes_its_place() {
        let save = Action::SaveTarget { target: target("t", &[]) };
        let delete = Action::DeleteTarget { id: "t".into() };
        let import = Action::Import { library: Library::default() };
        let delay = Action::SetUnlockDelay { delay: "1h".into() };
        assert!(supersedes(&delete, &save) && supersedes(&import, &save));
        assert!(!supersedes(&import, &delay) && !supersedes(&save, &delay));
        assert!(!supersedes(&save, &Action::AskForUnlock));
        let duplicate = Action::Duplicate { kind: Kind::Condition, id: "c".into(), new_id: "d".into() };
        assert_eq!(duplicate.keys(), vec!["condition:d".to_string()]);
        let session = Action::Duplicate { kind: Kind::Session, id: "c".into(), new_id: "d".into() };
        assert_eq!(session.keys(), vec!["session:d".to_string()]);
        let pending = |action: Action| vec![Entry { id: "p".into(), device: "me".into(), at: 0, action }];
        assert!(overridden(&save, &pending(delete.clone())));
        assert!(overridden(&save, &pending(import.clone())));
        assert!(!overridden(&delay, &pending(import.clone())));
        assert!(!overridden(&import, &pending(save.clone())));
        assert!(overridden(&import, &pending(import.clone())));
        assert!(!overridden(&save, &[]));
    }

    /// A small random number source, so the long test is the same each run.
    struct Dice(u64);

    impl Dice {
        fn roll(&mut self, sides: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % sides as u64) as usize
        }
        fn pick<T: Clone>(&mut self, from: &[T]) -> T {
            from[self.roll(from.len())].clone()
        }
    }

    /// Any action at all, as anyone with the server's token could send: sensible, odd or hostile.
    fn any_action(dice: &mut Dice, b: &Node) -> Action {
        let id = dice.pick(&["work".to_string(), "play".to_string(), "gone".to_string()]);
        let now = millis(b.now());
        let time = dice.pick(&[None, Some(0), Some(1), Some(now), Some(now + 3_600_000), Some(u64::MAX / 2), Some(u64::MAX)]);
        let text = dice.pick(&[None, Some(String::new()), Some("abcdefgh".to_string()), Some("x".repeat(2000))]);
        let mut starts = b.context.sessions().starts_of(&id);
        starts.push(new_id());
        let starts = dice.pick(&[Vec::new(), starts.clone(), starts[..starts.len() - 1].to_vec(), vec!["nope".to_string()]]);
        let lock = dice.pick(&[
            Lock::None,
            Lock::timed_for("1h"),
            Lock::timed_until("2099-01-01 00:00"),
            Lock::UntilRestart,
            Lock::Delay { delay: "1h".into() },
            Lock::Text { text: "type me".into() },
            Lock::RandomText { length: 8, text: Some("abcdefgh".into()) },
            Lock::Password { password: None, hash: Some(hash()) },
            Lock::Password { password: None, hash: Some("not a hash".into()) },
            Lock::Password { password: None, hash: None },
            Lock::Condition { condition: "nope".into() },
        ]);
        let mut definition = session(&id, &[]);
        definition.targets = dice.pick(&[vec![], vec!["social".to_string()], vec!["nope".to_string()]]);
        definition.lock = lock.clone();
        let sites: &[&str] = dice.pick(&[&[][..], &["youtube.com"][..], &["*"][..], &["a.com", "b.com"][..]]);
        let mut library = b.context.library();
        if dice.roll(2) == 0 {
            library.sessions.clear();
        }
        match dice.roll(21) {
            0 => Action::SaveSession { session: definition },
            1 => Action::DeleteSession { id },
            // Whatever `waiting` comes with it is no part of what is saved.
            2 => {
                let waiting =
                    dice.pick(&[None, Some(crate::definitions::Waiting { entries: Default::default(), actions: Default::default() })]);
                Action::SaveTarget { target: crate::definitions::Target { waiting, ..target(dice.pick(&["social", "other"]), sites) } }
            }
            3 => Action::DeleteTarget { id: dice.pick(&["social".to_string(), id]) },
            4 => Action::Duplicate { kind: Kind::Session, id, new_id: new_id() },
            5 => Action::Import { library },
            6 | 7 => Action::Start { id, start: new_id(), ends: time, text, zone: None },
            8 => Action::Lock { id, lock, ends: time, text },
            9 => Action::Stop { id, starts, text, proof: dice.pick(&[None, Some("00".repeat(64)), Some("not hex".to_string())]) },
            10 => Action::Used { id, used: [("".to_string(), u64::MAX), ("site:youtube.com".to_string(), 1000)].into() },
            11 => Action::AskToStop { id },
            12 => Action::AbortStop { id },
            13 => Action::TakeBreak { id },
            14 => Action::EndBreak { id },
            15 => Action::AskForUnlock,
            16 => Action::AbortUnlock,
            17 => Action::Unlock { text, next: dice.pick(&[None, Some("abcdefgh".to_string()), Some("x".to_string())]), proof: None },
            18 => Action::SetUnlockDelay { delay: dice.pick(&["0", "1h", "24h", "25h", "999d", "nonsense"]).to_string() },
            19 => Action::SetUnlockWay {
                way: dice.pick(&[None, Some(Lock::None), Some(lock), Some(Lock::Text { text: "out".into() }), Some(Lock::timed_for("1h"))]),
            },
            _ => Action::Lock { id, lock: Lock::UntilRestart, ends: None, text: None },
        }
    }

    /// **No sequence of entries from elsewhere takes the way out away.** Whatever arrives, in whatever
    /// order, with whatever times (and garbage among it, and this computer doing things of its own in
    /// between): the unlock delay stays at most 24 hours, disconnecting works, and once disconnected,
    /// asking to unlock and waiting out the longest delay there can be ends every session.
    #[test]
    fn no_sequence_of_entries_from_elsewhere_takes_the_way_out_away() {
        let mut dice = Dice(0x5eed_cafe_f00d_d00d);
        for round in 0..60 {
            let (a, b, mut log) = pair(
                &format!("fuzz{round}"),
                dice.pick(&[
                    Lock::None,
                    Lock::UntilRestart,
                    Lock::Delay { delay: "1h".into() },
                    Lock::timed_for("1h"),
                    Lock::RandomText { length: 8, text: None },
                ]),
            );
            a.ok(Request::SaveSession { session: session("play", &["social"]) });
            log.sync(&[&a, &b]);
            for _ in 0..25 {
                match dice.roll(10) {
                    0 => log.append(dice.pick(&["".to_string(), "{".to_string(), "null".to_string(), "[]".to_string()])),
                    1 => {
                        // This computer does something of its own.
                        let local = dice.pick(&[
                            Request::Start { id: "work".into() },
                            Request::Start { id: "play".into() },
                            Request::Stop { id: "work".into(), password: None, text: None },
                            Request::AskForUnlock,
                            Request::SetUnlockDelay { delay: "3h".into() },
                        ]);
                        let _ = b.ask(local);
                    }
                    2 => {
                        let time = b.now() + Duration::from_secs(dice.pick(&[1, 60, 3600, 86_400]));
                        b.at(time);
                    }
                    _ => {
                        let action = any_action(&mut dice, &b);
                        let at = dice.pick(&[0, millis(b.now()), millis(b.now()) + 86_400_000, u64::MAX]);
                        log.forge(action, at);
                    }
                }
                // What a session that stays locked blocks never gets less by what arrives.
                let blocked = || -> Vec<(String, Vec<String>)> {
                    let status = b.context.sessions().status();
                    let locked = b.context.sessions().locked_ids();
                    status
                        .running
                        .iter()
                        .filter(|r| locked.contains(&r.id))
                        .map(|r| (r.id.clone(), r.targets.iter().flat_map(|t| t.entries.filters.clone()).collect()))
                        .collect()
                };
                let before = blocked();
                log.sync(&[&b]);
                for (id, after) in blocked() {
                    if let Some((_, had)) = before.iter().find(|(was, _)| *was == id) {
                        assert!(had.iter().all(|site| after.contains(site)), "round {round}: {id} lost a site: {had:?} to {after:?}");
                    }
                }
                assert!(b.context.unlock().delay() <= unlock::MAX_DELAY, "round {round}");
                assert!(b.replica.held().len() <= MAX_HELD);
                // Nothing that arrives makes the service crash: not what it does every second (a crash
                // there ends the process, and Windows starts it again over the same files), nor what
                // the app and the window ask for, nor reading the sessions back after a restart.
                b.context.sessions().active();
                assert!(matches!(b.ask(Request::Policy), Response::Policy(_)), "round {round}");
                assert!(matches!(b.ask(Request::Status), Response::Status(_)), "round {round}");
                b.context.sessions().restarted("boot").active();
            }
            b.replica.disconnect();
            assert!(!b.replica.connected() && b.replica.held().is_empty(), "round {round}");
            assert!(b.unlocks(), "round {round}: {:?}", b.context.sessions().locked());
        }
    }

    /// The same, staying connected: what waits never keeps unlocking here from ending everything.
    #[test]
    fn unlocking_here_works_while_connected_whatever_waits() {
        let mut dice = Dice(0x0dd_ba11);
        for round in 0..30 {
            let (a, b, mut log) = pair(&format!("stay{round}"), Lock::UntilRestart);
            a.ok(Request::SaveSession { session: session("play", &["social"]) });
            b.ok(Request::Start { id: "work".into() });
            log.sync(&[&a, &b]);
            for _ in 0..20 {
                let action = any_action(&mut dice, &b);
                let at = dice.pick(&[0, millis(b.now()), millis(b.now()) + 86_400_000, u64::MAX]);
                log.forge(action, at);
            }
            log.sync(&[&a, &b]);
            // Unlocking here, with nothing more arriving in between, ends what runs.
            assert!(b.unlocks(), "round {round}: {:?}", b.context.sessions().locked());
        }
    }
}
