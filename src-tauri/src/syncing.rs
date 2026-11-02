//! A computer's syncing with the server (`sync_api.rs`), on top of its side of the log (`replica.rs`):
//! connecting, disconnecting, and the rounds of the service's sync thread (`Syncer::round`): send what
//! was done here, take in the log, try again what waits, tell the others the use counted here and say
//! how this computer is.
//!
//! The server's address and token are kept in `sync.json` in the service's folder. **Disconnecting
//! always works**, also while a session is locked, and nothing about syncing ever stands in the way of
//! a lock ending here by its own rule or of Unlock here (`replica.rs`).

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::definitions::Library;
use crate::handler::{self, Context, Outcome};
use crate::replica::{Action, Entry, Replica};
use crate::sync::LockExt;
use crate::sync_api::{DeviceReport, DeviceView, Failure, HeldView, Http, Posted, Snapshot, SyncStatus, Transport, MAX_POST, MAX_WAIT};
use crate::watchdog::note;
use crate::{state, strings};

/// How often the use counted here is told to the others.
const SHARE_USE: Duration = Duration::from_secs(30);
/// How often this computer says how it is.
const REPORT: Duration = Duration::from_secs(60);
/// The first wait after the server did not answer; it doubles up to `MAX_PAUSE`.
const MIN_PAUSE: Duration = Duration::from_secs(2);
const MAX_PAUSE: Duration = Duration::from_secs(60);

/// Makes the transport for an address and a token: `Http` in the service, the server itself in tests.
pub type Connector = Box<dyn Fn(&str, &str) -> Result<Arc<dyn Transport>, String> + Send + Sync>;

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
struct Settings {
    url: String,
    token: String,
    /// When disconnecting was asked for while the server could not be reached (milliseconds since the
    /// Unix epoch): it counts down as unlocking does (`handler::disconnect`), and ends when the server
    /// is reached again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disconnect_asked: Option<u64>,
}

#[derive(Default)]
struct Runtime {
    last_contact: Option<u64>,
    error: Option<String>,
    devices: Vec<crate::sync_api::Device>,
    shared_use: Option<Instant>,
    reported: Option<Instant>,
}

pub struct Syncer {
    replica: Replica,
    settings: Mutex<Settings>,
    /// `sync.json`; `None`: for this run only (tests).
    path: Option<PathBuf>,
    transport: Mutex<Option<Arc<dyn Transport>>>,
    connector: Connector,
    runtime: Mutex<Runtime>,
    /// This computer's name, for the others.
    name: String,
    /// Held while something is done here and recorded, and while entries from elsewhere are applied:
    /// what was done here is always either before an entry from elsewhere or pending when it comes.
    turn: Mutex<()>,
    /// Set when something was recorded, for the sending thread.
    nudge: (Mutex<bool>, Condvar),
    /// This computer's id among those that sync (`computer`): the same across connecting again, so the
    /// server lists it once. Not the id its entries go by in the log, which is new with each connect.
    computer: String,
}

impl Syncer {
    /// The service's: its files in the service's folder, the server over HTTP.
    pub fn new() -> Syncer {
        let connector: Connector = Box::new(|url, token| Ok(Arc::new(Http::new(url, token)?) as Arc<dyn Transport>));
        let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Computer".into());
        Syncer::at(state::dir(), connector, name)
    }

    /// Over the files in `dir` (`None`: for this run only), reaching the server through `connector`.
    pub fn at(dir: Option<PathBuf>, connector: Connector, name: String) -> Syncer {
        let path = dir.as_ref().map(|dir| dir.join("sync.json"));
        let computer = dir.as_ref().map_or_else(crate::replica::new_id, |dir| computer_id(&dir.join("computer")));
        let replica = Replica::at(dir.map(|dir| dir.join("replica.json")));
        let settings: Settings =
            path.as_ref().and_then(|path| fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        // Without both an address and its side of the log, it is not connected.
        if settings.url.is_empty() && replica.connected() {
            replica.disconnect();
        }
        let transport = (replica.connected()).then(|| connector(&settings.url, &settings.token).ok()).flatten();
        if replica.connected() && transport.is_none() {
            replica.disconnect();
        }
        Syncer {
            replica,
            settings: Mutex::new(settings),
            path,
            transport: Mutex::new(transport),
            connector,
            runtime: Mutex::default(),
            name,
            computer,
            turn: Mutex::new(()),
            nudge: (Mutex::new(false), Condvar::new()),
        }
    }

    pub fn connected(&self) -> bool {
        self.replica.connected()
    }

    /// The runs that ended here with Windows' restart (`Sessions::take_restarted`) go to the others as
    /// stops, as if stopped here: nothing else would ever end them in the server's view, which has no
    /// restart of its own, nor on a computer that takes its sessions over later. Each other computer
    /// stops them by its own rules (one locked until its own restart waits for that). Called once, as
    /// the service starts; while not connected there is nobody to tell.
    pub fn restarted(&self, context: &Context) {
        let _turn = self.turn();
        for (id, starts) in context.sessions().take_restarted() {
            self.record(Action::Stop { id, starts, text: None, proof: None }, context.now());
        }
    }

    /// Something was done here: it goes to the others.
    pub fn record(&self, action: Action, now: SystemTime) {
        self.replica.record(action, now);
        *self.nudge.0.locked() = true;
        self.nudge.1.notify_all();
    }

    /// Held while a request is handled here and its action recorded (`handler::handle`).
    pub fn turn(&self) -> MutexGuard<'_, ()> {
        self.turn.locked()
    }

    /// Starts the service's two sync threads: one takes in the log (waiting on the server for something
    /// new), the other sends what is done here as soon as it is done. A crash in either disconnects: a
    /// way out, never a service that ends again and again.
    pub fn spawn(self: &Arc<Self>, context: Arc<Context>) {
        let syncer = self.clone();
        let receiving = context.clone();
        let _ = thread::Builder::new().name("sync".into()).spawn(move || {
            let mut pause = MIN_PAUSE;
            loop {
                if !syncer.connected() {
                    thread::sleep(MIN_PAUSE);
                    continue;
                }
                match syncer.guarded(|| syncer.round(&receiving, Duration::from_secs(MAX_WAIT))) {
                    Some(Err(_)) => {
                        thread::sleep(pause);
                        pause = (pause * 2).min(MAX_PAUSE);
                    }
                    _ => pause = MIN_PAUSE,
                }
            }
        });
        let syncer = self.clone();
        let _ = thread::Builder::new().name("sync-send".into()).spawn(move || loop {
            if syncer.waited_for_work(Duration::from_secs(MAX_WAIT)) {
                let _ = syncer.guarded(|| syncer.push());
            }
        });
    }

    /// Runs `work`; a crash in it disconnects (`None`).
    fn guarded<T>(&self, work: impl FnOnce() -> T) -> Option<T> {
        let done = catch_unwind(AssertUnwindSafe(work)).ok();
        if done.is_none() {
            note("sync", "syncing crashed; disconnected");
            self.disconnect();
        }
        done
    }

    /// Waits until something is recorded here, at most `most`. Whether something was.
    fn waited_for_work(&self, most: Duration) -> bool {
        let mut flag = self.nudge.0.locked();
        if !*flag {
            flag = self.nudge.1.wait_timeout(flag, most).map_or_else(|e| e.into_inner().0, |(flag, _)| flag);
        }
        std::mem::take(&mut *flag)
    }

    /// Sends what was done here and not yet taken in. Nothing while not connected. A log this computer does not know
    /// refuses it: the next round finds out why (`exchange`).
    pub fn push(&self) -> Result<(), String> {
        let Some(transport) = self.transport.locked().clone() else { return Ok(()) };
        self.send(transport.as_ref()).map_err(|f| f.words())
    }

    fn send(&self, transport: &dyn Transport) -> Result<(), Failure> {
        let (after, last) = (self.replica.confirmed(), self.replica.last());
        for chunk in self.replica.outbox().chunks(MAX_POST) {
            transport.post(Posted { entries: chunk.iter().map(Entry::text).collect(), after, last: last.clone() })?;
        }
        Ok(())
    }

    /// Joins the server at `url` with `token`. A new server, whose log never had anything, takes what is
    /// here: the first computer sets everything up. Otherwise this computer takes the server's settings
    /// and sessions (applied here under this computer's rules, as anything from elsewhere is: what they
    /// refuse waits), so one that joins later never sends its own over everyone's. A computer that was
    /// connected disconnects first.
    pub fn connect(&self, context: &Context, url: &str, token: &str) -> Result<(), String> {
        let transport = (self.connector)(url, token)?;
        let snapshot = transport.snapshot().map_err(|failure| failure.words())?;
        self.disconnect();
        let settings = Settings { url: url.trim().to_string(), token: token.trim().to_string(), disconnect_asked: None };
        self.save_settings(&settings)?;
        self.replica.connect(snapshot.head, snapshot.last.clone(), snapshot.newest)?;
        *self.settings.locked() = settings;
        *self.transport.locked() = Some(transport);
        if snapshot.head == 0 {
            self.send_mine(context);
        } else {
            self.adopt(context, &snapshot);
        }
        let _ = self.round(context, Duration::ZERO);
        Ok(())
    }

    /// Sends everything here to the others, as it is: what is defined, the unlock settings and what runs
    /// (each applies it under its own rules). Nothing here changes.
    fn send_mine(&self, context: &Context) {
        for action in actions_of(&handler::snapshot(context, 0)) {
            self.record(action, context.now());
        }
    }

    /// Takes over a snapshot of the server's, under this computer's rules. Nothing once disconnected,
    /// also when that happens while a round is joining again: disconnecting is the way out.
    fn adopt(&self, context: &Context, snapshot: &Snapshot) {
        let _turn = self.turn();
        let now = context.now();
        for action in actions_of(snapshot) {
            if !self.connected() {
                return;
            }
            let import = matches!(action, Action::Import { .. });
            if matches!(self.replica.adopt(context, action, now), Outcome::Held(_)) && import {
                // Not all at once while a session here is locked (it waits): each target, condition and
                // session on its own, under the same rules, so what blocks more is made now and what
                // blocks less waits, as for any change from elsewhere.
                for item in items_of(&snapshot.library) {
                    self.replica.adopt(context, item, now);
                }
            }
        }
    }

    fn rejoin(&self, context: &Context, snapshot: &Snapshot) {
        self.replica.rejoin(snapshot.head, snapshot.last.clone(), snapshot.newest);
        self.adopt(context, snapshot);
    }

    /// Stops syncing. Always works: what was not sent and what waits is forgotten, the settings here stay
    /// as they are, and the sessions that run go on by their own rules.
    pub fn disconnect(&self) {
        self.replica.disconnect();
        *self.transport.locked() = None;
        *self.settings.locked() = Settings::default();
        *self.runtime.locked() = Runtime::default();
        if let Some(path) = &self.path {
            if path.exists() && fs::remove_file(path).is_err() {
                // A file that cannot be removed is emptied: unreadable is not connected.
                let _ = fs::write(path, b"");
            }
        }
    }

    fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_vec(settings).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Cannot save: {e}"))?;
        }
        fs::write(path, json).map_err(|e| format!("Cannot save: {e}"))
    }

    /// Disconnecting without the server, as unlocking (`handler::disconnect`): `None` when its countdown
    /// of `delay` seconds is over (or there is none); otherwise the seconds left and whether it was asked
    /// for just now, at `now`. Kept in `sync.json`, so a restart does not start it again.
    pub fn disconnect_wait(&self, now: SystemTime, delay: u64) -> Option<(u64, bool)> {
        let now = state::to_unix(now).unwrap_or(0);
        let mut settings = self.settings.locked();
        let asked = settings.disconnect_asked;
        let left = |asked: u64| (delay * 1000).saturating_sub(now.saturating_sub(asked)).div_ceil(1000);
        match asked {
            _ if delay == 0 => None,
            Some(asked) if left(asked) == 0 => None,
            Some(asked) => Some((left(asked), false)),
            None => {
                settings.disconnect_asked = Some(now);
                if let Err(why) = self.save_settings(&settings) {
                    note("sync", &format!("disconnecting was asked for, but cannot be kept: {why}"));
                }
                Some((delay, true))
            }
        }
    }

    /// One round with the server: what was done here goes out, the log comes in (waiting up to `wait` for
    /// something new), what waits is tried again, the use counted here is shared and this computer says how
    /// it is. Nothing while not connected.
    pub fn round(&self, context: &Context, wait: Duration) -> Result<(), String> {
        let Some(transport) = self.transport.locked().clone() else { return Ok(()) };
        let result = self.exchange(context, transport.as_ref(), wait);
        {
            let _turn = self.turn();
            self.replica.retry(context);
        }
        // The server is there: a disconnect asked for without it is no longer waited for.
        if result.is_ok() {
            let mut settings = self.settings.locked();
            if settings.disconnect_asked.take().is_some() {
                let _ = self.save_settings(&settings);
            }
        }
        let mut runtime = self.runtime.locked();
        match &result {
            Ok(()) => {
                runtime.last_contact = state::to_unix(context.now());
                runtime.error = None;
            }
            Err(why) => runtime.error = Some(why.clone()),
        }
        result
    }

    /// A round at once, sharing the use counted here and hearing of the other computers without waiting
    /// for their times (Sync now).
    pub fn now(&self, context: &Context) -> Result<(), String> {
        {
            let mut runtime = self.runtime.locked();
            runtime.shared_use = None;
            runtime.reported = None;
        }
        self.round(context, Duration::ZERO)
    }

    fn exchange(&self, context: &Context, transport: &dyn Transport, wait: Duration) -> Result<(), String> {
        self.share_use(context);
        match self.send(transport) {
            Ok(()) => {}
            // Not posted to a log this computer does not know: it is told so as by the log, below.
            Err(Failure::Diverged) => {
                self.diverged(context, transport)?;
                self.report(context, transport);
                return Ok(());
            }
            Err(failure) => return Err(failure.words()),
        }
        // Nothing to wait for while something of this computer's is still to come back.
        let wait = if self.replica.outbox().is_empty() { wait.min(Duration::from_secs(MAX_WAIT)) } else { Duration::ZERO };
        let after = self.replica.confirmed().unwrap_or(0);
        match transport.log(after, self.replica.last().as_deref(), wait) {
            Ok(page) => {
                let _turn = self.turn();
                self.replica.receive(context, &page.entries);
            }
            // Away too long: joined again from what the server has now, keeping what is still to send.
            Err(Failure::Gone) => {
                let snapshot = transport.snapshot().map_err(|f| f.words())?;
                self.rejoin(context, &snapshot);
            }
            // The server is not the one synced with (wiped, or restored from a backup). Whichever is newer
            // wins: what the server has now, if it changed after what this computer saw last (another
            // computer filled it again since: this one was away), taken over as after a long absence; or
            // what is here, sent to it again as when connecting with this computer's own settings (it is
            // empty, or older). Either way under each computer's rules: no lock gets looser.
            Err(Failure::Diverged) => self.diverged(context, transport)?,
            Err(failure) => return Err(failure.words()),
        }
        self.report(context, transport);
        Ok(())
    }

    /// What is done once the server is found not to be the one synced with (`exchange`).
    fn diverged(&self, context: &Context, transport: &dyn Transport) -> Result<(), String> {
        let snapshot = transport.snapshot().map_err(|f| f.words())?;
        if newer(snapshot.newest, self.replica.seen()) {
            self.rejoin(context, &snapshot);
        } else {
            let _turn = self.turn();
            self.replica.rejoin(snapshot.head, snapshot.last, snapshot.newest);
            self.send_mine(context);
        }
        Ok(())
    }

    /// The use counted here since the last time, for the others (`Action::Used`), every `SHARE_USE`.
    fn share_use(&self, context: &Context) {
        let mut runtime = self.runtime.locked();
        if runtime.shared_use.is_some_and(|at| at.elapsed() < SHARE_USE) {
            return;
        }
        runtime.shared_use = Some(Instant::now());
        drop(runtime);
        for (id, used) in context.sessions().take_unshared() {
            self.replica.record(Action::Used { id, used }, context.now());
        }
    }

    /// Says how this computer is, and hears of the others, every `REPORT`.
    fn report(&self, context: &Context, transport: &dyn Transport) {
        if self.runtime.locked().reported.is_some_and(|at| at.elapsed() < REPORT) {
            return;
        }
        let report = DeviceReport {
            name: self.name.clone(),
            confirmed: self.replica.confirmed().unwrap_or(0),
            held: self.replica.held().into_iter().map(|held| held.why).collect(),
            running: context.sessions().status().running.into_iter().map(|running| running.name).collect(),
            zone: Some(context.sessions().zone().name().to_string()),
        };
        let reported = transport.report(&self.computer, report).is_ok();
        let devices = transport.devices().unwrap_or_default();
        let mut runtime = self.runtime.locked();
        if reported {
            runtime.reported = Some(Instant::now());
        }
        runtime.devices = devices;
    }

    /// Where syncing stands, for the window and the CLI.
    pub fn status(&self, context: &Context) -> SyncStatus {
        let runtime = self.runtime.locked();
        let library = context.library();
        let now = state::to_unix(context.now()).unwrap_or(0);
        let url = self.settings.locked().url.clone();
        let outbox = self.replica.outbox().len();
        let device = self.computer.clone();
        let connected = self.connected();
        let summary = match (&runtime.error, runtime.last_contact) {
            _ if !connected => strings::t("sync.notConnected", &[]),
            (Some(why), _) => strings::t("sync.failing", &[("url", &url), ("why", why)]),
            (None, None) => strings::t("sync.connecting", &[("url", &url)]),
            (None, Some(at)) if outbox > 0 => {
                strings::t("sync.sending", &[("url", &url), ("n", &outbox.to_string()), ("ago", &ago(now, at))])
            }
            (None, Some(at)) => strings::t("sync.inSync", &[("url", &url), ("ago", &ago(now, at))]),
        };
        let devices = device_views(&runtime.devices, &device, now);
        SyncStatus {
            connected,
            summary,
            url,
            confirmed: self.replica.confirmed().unwrap_or(0),
            last_contact: runtime.last_contact,
            error: runtime.error.clone(),
            outbox,
            held: self
                .replica
                .held()
                .into_iter()
                .map(|held| HeldView { what: describe(&held.entry.action, &library), why: held.why })
                .collect(),
            devices,
            device,
            disconnect: self.settings.locked().disconnect_asked.map(|asked| {
                let left = (context.unlock().delay() * 1000).saturating_sub(now.saturating_sub(asked)).div_ceil(1000);
                match left {
                    0 => strings::t("sync.disconnectReady", &[]),
                    left => strings::t("sync.disconnectPending", &[("left", &crate::duration::left(left))]),
                }
            }),
        }
    }
}

/// How long ago `at` was at `now` (both in milliseconds since the Unix epoch), in words: one unit, rounded.
/// The computers that sync as people see them, at `now`; `this` is the id of the one asking, if any.
pub fn device_views(devices: &[crate::sync_api::Device], this: &str, now: u64) -> Vec<DeviceView> {
    devices
        .iter()
        .map(|known| DeviceView {
            id: known.id.clone(),
            name: known.report.name.clone(),
            this: known.id == this,
            seen: ago(now, known.seen),
            running: known.report.running.clone(),
            held: known.report.held.len(),
        })
        .collect()
}

/// This computer's id among those that sync, kept in `path` (`computer`): made the first time, the same
/// ever after. One that cannot be kept is new each time the service starts: then the server lists it
/// again, which is all it costs.
fn computer_id(path: &std::path::Path) -> String {
    let kept = fs::read_to_string(path).ok().map(|text| text.trim().to_string());
    if let Some(id) = kept.filter(|id| !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        return id;
    }
    let id = crate::replica::new_id();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(path, &id) {
        note("sync", &format!("this computer's id cannot be kept: {e}"));
    }
    id
}

fn ago(now: u64, at: u64) -> String {
    let seconds = now.saturating_sub(at) / 1000;
    match seconds < 60 {
        true => strings::t("sync.justNow", &[]),
        false => strings::t("sync.ago", &[("time", &crate::duration::left(seconds))]),
    }
}

/// What a snapshot comes to as entries: everything defined, the unlock settings, and each start of each
/// running session.
/// What `library` holds, one save each: what it uses before what uses it.
fn items_of(library: &Library) -> Vec<Action> {
    let conditions = library.conditions.iter().map(|condition| Action::SaveCondition { condition: condition.clone() });
    let targets = library.targets.iter().map(|target| Action::SaveTarget { target: target.clone() });
    let sessions = library.sessions.iter().map(|session| Action::SaveSession { session: session.clone() });
    conditions.chain(targets).chain(sessions).collect()
}

pub fn actions_of(snapshot: &Snapshot) -> Vec<Action> {
    let mut actions = vec![
        Action::Import { library: snapshot.library.clone() },
        Action::SetUnlockDelay { delay: snapshot.unlock_delay.clone() },
        Action::SetUnlockWay { way: snapshot.unlock_way.clone() },
    ];
    for run in &snapshot.runs {
        for start in &run.starts {
            let (ends, text, zone) = (run.ends, run.text.clone(), run.zone.clone());
            actions.push(Action::Start { id: run.id.clone(), start: start.clone(), ends, text, zone });
        }
    }
    actions
}

/// Whether the server's newest entry (`newest`) was done after what this computer saw last (`seen`). One
/// that never saw anything takes what the server has.
fn newer(newest: Option<u64>, seen: Option<u64>) -> bool {
    match (newest, seen) {
        (Some(newest), Some(seen)) => newest > seen,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// An entry that waits, in a few words: what it would do.
pub fn describe(action: &Action, library: &crate::definitions::Library) -> String {
    use crate::definitions::Kind;
    // A name quoted, or the id as it is when this computer does not have it.
    let name = |found: Option<&str>, id: &str| found.map_or_else(|| id.to_string(), strings::quoted);
    let session = |id: &str| name(library.session(id).map(|s| s.name.as_str()), id);
    let target = |id: &str| name(library.target(id).map(|t| t.name.as_str()), id);
    let condition = |id: &str| name(library.condition(id).map(|c| c.name.as_str()), id);
    let (key, named) = match action {
        Action::SaveSession { session } => ("saveSession", strings::quoted(&session.name)),
        Action::DeleteSession { id } => ("deleteSession", session(id)),
        Action::SaveTarget { target } => ("saveTarget", strings::quoted(&target.name)),
        Action::DeleteTarget { id } => ("deleteTarget", target(id)),
        Action::SaveCondition { condition } => ("saveCondition", strings::quoted(&condition.name)),
        Action::DeleteCondition { id } => ("deleteCondition", condition(id)),
        Action::Duplicate { kind, id, .. } => (
            "duplicate",
            match kind {
                Kind::Session => session(id),
                Kind::Target => target(id),
                Kind::Condition => condition(id),
            },
        ),
        Action::Import { .. } => ("import", String::new()),
        Action::Start { id, .. } => ("start", session(id)),
        Action::Lock { id, .. } => ("lock", session(id)),
        Action::Stop { id, .. } => ("stop", session(id)),
        Action::Used { id, .. } => ("used", session(id)),
        Action::AskToStop { id } => ("askToStop", session(id)),
        Action::AbortStop { id } => ("abortStop", session(id)),
        Action::TakeBreak { id } => ("takeBreak", session(id)),
        Action::EndBreak { id } => ("endBreak", session(id)),
        Action::AskForUnlock => ("askForUnlock", String::new()),
        Action::AbortUnlock => ("abortUnlock", String::new()),
        Action::Unlock { .. } => ("unlock", String::new()),
        Action::SetUnlockDelay { .. } => ("setUnlockDelay", String::new()),
        Action::SetUnlockWay { .. } => ("setUnlockWay", String::new()),
    };
    strings::t(&format!("sync.action.{key}"), &[("name", &named)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tests::{session, target};
    use crate::definitions::{Definition, Kind};
    use crate::handler::handle;
    use crate::hub::{self, Hub};
    use crate::lock::Lock;
    use crate::proto::{Request, Response};
    use crate::strings::t;
    use crate::sync_api::{Appended, Device, LogPage};
    use crate::unlock;
    use std::sync::atomic::{AtomicBool, Ordering};

    const TOKEN: &str = "0123456789abcdef";
    const URL: &str = "https://sync.example.com";

    /// The server, as the computers reach it: it can be down, say a computer was away too long, or be
    /// replaced (wiped, restored from a backup: `replace`).
    struct Server {
        hub: Mutex<Arc<Hub>>,
        down: AtomicBool,
        gone: AtomicBool,
    }

    impl Server {
        fn hub(&self) -> Arc<Hub> {
            self.hub.locked().clone()
        }

        /// Another server at the same address: over the files in `dir` (empty: wiped; a copy: a backup).
        fn replace(&self, dir: PathBuf) {
            *self.hub.locked() = Arc::new(Hub::new(hub::tests::context(&dir), Some(dir)));
        }

        fn up(&self) -> Result<(), Failure> {
            match self.down.load(Ordering::SeqCst) {
                true => Err(Failure::Other("The server cannot be reached".into())),
                false => Ok(()),
            }
        }
    }

    impl Transport for Server {
        fn snapshot(&self) -> Result<Snapshot, Failure> {
            self.up()?;
            self.hub().snapshot()
        }
        fn log(&self, after: u64, last: Option<&str>, wait: Duration) -> Result<LogPage, Failure> {
            self.up()?;
            if self.gone.swap(false, Ordering::SeqCst) {
                return Err(Failure::Gone);
            }
            self.hub().log(after, last, wait)
        }
        fn post(&self, posted: Posted) -> Result<Appended, Failure> {
            self.up()?;
            self.hub().post(posted)
        }
        fn report(&self, device: &str, report: DeviceReport) -> Result<(), Failure> {
            self.up()?;
            self.hub().report(device, report)
        }
        fn devices(&self) -> Result<Vec<Device>, Failure> {
            self.up()?;
            self.hub().devices()
        }
        fn request(&self, request: &Request) -> Result<Response, Failure> {
            self.up()?;
            self.hub().request(request)
        }
    }

    fn server(name: &str) -> Arc<Server> {
        let dir = hub::tests::dir(&format!("syncing-{name}"));
        Arc::new(Server {
            hub: Mutex::new(Arc::new(Hub::new(hub::tests::context(&dir), Some(dir)))),
            down: AtomicBool::new(false),
            gone: AtomicBool::new(false),
        })
    }

    /// A computer: its service, syncing through `server` once connected.
    struct Computer {
        context: Arc<Context>,
        syncer: Arc<Syncer>,
        dir: PathBuf,
    }

    fn connector(server: &Arc<Server>) -> Connector {
        let server = server.clone();
        Box::new(move |url, token| match token == TOKEN {
            true => Http::new(url, token).map(|_| server.clone() as Arc<dyn Transport>),
            false => Err(Failure::Unauthorized.words()),
        })
    }

    fn computer(name: &str, server: &Arc<Server>) -> Computer {
        computer_in(name, server, crate::localtime::Zone::fixed(0))
    }

    /// As `computer`, in Windows' time zone `zone`.
    fn computer_in(name: &str, server: &Arc<Server>, zone: crate::localtime::Zone) -> Computer {
        let dir = hub::tests::dir(&format!("syncing-{name}"));
        let syncer = Arc::new(Syncer::at(Some(dir.clone()), connector(server), name.into()));
        let context = Arc::new(hub::tests::context_in(&dir, zone).with_syncer(syncer.clone()));
        Computer { context, syncer, dir }
    }

    impl Computer {
        fn ask(&self, request: Request) -> Response {
            handle(&self.context, 0, request).0
        }

        fn ok(&self, request: Request) {
            assert_eq!(self.ask(request), Response::Done);
        }

        fn connect(&self) -> Response {
            self.ask(Request::SyncConnect { url: URL.into(), token: TOKEN.into() })
        }

        fn status(&self) -> SyncStatus {
            match self.ask(Request::SyncStatus) {
                Response::Sync(status) => *status,
                other => panic!("no status: {other:?}"),
            }
        }

        fn now(&self) -> SystemTime {
            self.context.sessions().now()
        }

        fn later(&self, by: Duration) {
            self.context.sessions().move_to(self.now() + by);
        }

        fn runs(&self, id: &str) -> bool {
            self.context.sessions().running_ids().iter().any(|r| r == id)
        }

        fn locked(&self) -> bool {
            self.context.sessions().locked().is_some()
        }

        /// The way out at this computer's keyboard: request, wait out the longest delay, unlock.
        fn unlocks(&self) -> bool {
            self.ok(Request::AskForUnlock);
            self.later(Duration::from_secs(unlock::MAX_DELAY + 1));
            self.ask(Request::Unlock) == Response::Done && self.context.sessions().running_ids().is_empty()
        }
    }

    /// Two rounds for every computer: what one sends, every other has taken in after.
    fn sync(computers: &[&Computer]) {
        for _ in 0..2 {
            for computer in computers {
                computer.syncer.round(&computer.context, Duration::ZERO).unwrap();
            }
        }
    }

    /// A target "social" and a session "work" of it with `lock`, made through the server's API.
    fn defined(server: &Server, lock: Lock) {
        let ok = |request| assert_eq!(server.hub().request(&request).unwrap(), Response::Done);
        ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        ok(Request::SaveSession { session: Definition { lock, ..session("work", &["social"]) } });
        ok(Request::SetUnlockDelay { delay: "1h".into() });
    }

    /// A computer in UTC+1 with a lock schedule of 9:00-17:00: the server reads the schedule in its own
    /// time zone (UTC, the sync server's), not the computer's.
    #[test]
    fn the_server_reads_a_lock_schedule_in_the_computers_time_zone() {
        let server = server("zone");
        let ok = |request| assert_eq!(server.hub().request(&request).unwrap(), Response::Done);
        ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        ok(Request::SaveCondition { condition: crate::definitions::tests::weekly("office", "daily 9-17") });
        ok(Request::SaveSession {
            session: Definition { lock: Lock::Condition { condition: "office".into() }, ..session("work", &["social"]) },
        });
        let a = computer_in("zone-a", &server, crate::localtime::Zone::fixed(60));
        assert_eq!(a.connect(), Response::Done);
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a]);
        assert_eq!(server.hub().snapshot().unwrap().runs.len(), 1);
        let at = |hours: u64, minutes: u64| {
            // 2030-01-08, at this time UTC.
            let time = std::time::UNIX_EPOCH + Duration::from_secs(1_894_060_800 + hours * 3600 + minutes * 60);
            a.context.sessions().move_to(time);
            server.hub().view().sessions().move_to(time);
        };
        let stop = || server.hub().request(&Request::Stop { id: "work".into(), password: None, text: None }).unwrap();
        // 16:30 UTC is 17:30 at the computer: free there, so free at the server too.
        at(16, 30);
        assert!(!a.locked(), "free at the computer at 17:30 its time");
        assert_eq!(stop(), Response::Done, "the server refuses a stop the computer would take");
        sync(&[&a]);
        assert!(!a.runs("work"));
        // Started again: at 8:30 UTC the next day it is 9:30 at the computer, locked there, so at the server too.
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a]);
        at(24 + 8, 30);
        assert!(a.locked(), "locked at the computer at 9:30 its time");
        assert!(matches!(stop(), Response::Failed(_)), "the server takes a stop the computer refuses");
    }

    /// The server's page starts sessions and sets the way to unlock in the zone of the computer it heard
    /// from last; a new server takes a running session's zone from the first computer; and a computer
    /// reads a session another one started in its own zone.
    #[test]
    fn the_servers_page_and_a_new_server_read_in_the_zone_of_the_computers() {
        let server = server("zones");
        let a = computer_in("zones-a", &server, crate::localtime::Zone::fixed(60));
        // 2030-01-08 05:00 UTC, 6:00 at A: nothing is locked by the office hours yet.
        let free = std::time::UNIX_EPOCH + Duration::from_secs(1_894_060_800 + 5 * 3600);
        a.context.sessions().move_to(free);
        server.hub().view().sessions().move_to(free);
        // Before the server has anything: A's settings and what runs there become the server's.
        a.ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        a.ok(Request::SaveCondition { condition: crate::definitions::tests::weekly("office", "daily 9-17") });
        let locked = Definition { lock: Lock::Condition { condition: "office".into() }, ..session("work", &["social"]) };
        a.ok(Request::SaveSession { session: locked.clone() });
        a.ok(Request::Start { id: "work".into() });
        assert_eq!(a.connect(), Response::Done);
        sync(&[&a]);
        let runs = server.hub().snapshot().unwrap().runs;
        assert_eq!(runs[0].zone.as_deref(), Some("fixed:+60"));
        // From the server's page, after A said its zone.
        let ok = |request| assert_eq!(server.hub().request(&request).unwrap(), Response::Done);
        ok(Request::SaveSession { session: Definition { id: "home".into(), name: "Home".into(), ..locked } });
        ok(Request::Start { id: "home".into() });
        assert_eq!(server.hub().view().sessions().zone_of("home").unwrap().name(), "fixed:+60");
        ok(Request::SetUnlockWay { way: Some(Lock::Condition { condition: "office".into() }) });
        let opening = server.hub().view().unlock().way().and_then(|way| way.opening);
        assert!(matches!(opening, Some(unlock::Opening::While { zone, .. }) if zone == "fixed:+60"));
        // B, in UTC, reads them in its own zone.
        let b = computer("zones-b", &server);
        assert_eq!(b.connect(), Response::Done);
        sync(&[&a, &b]);
        assert_eq!(b.context.sessions().zone_of("work").unwrap().name(), "fixed:+0");
        assert_eq!(b.context.sessions().zone_of("home").unwrap().name(), "fixed:+0");
    }

    /// Unlocking open 8:00-12:00, unlocked at the server at 10:00, a computer away until 13:00: it holds
    /// the unlock then, and does it when unlocking opens there again, at 8:00 the next day.
    #[test]
    fn an_unlock_reaches_a_computer_away_while_it_was_open_when_it_opens_there() {
        let server = server("late");
        let ok = |request| assert_eq!(server.hub().request(&request).unwrap(), Response::Done);
        ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        ok(Request::SaveCondition { condition: crate::definitions::tests::weekly("closed", "daily 12-8") });
        ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        ok(Request::SetUnlockWay { way: Some(Lock::Condition { condition: "closed".into() }) });
        ok(Request::SetUnlockDelay { delay: "1h".into() });
        let a = computer("late-a", &server);
        assert_eq!(a.connect(), Response::Done);
        let at = |hours: u64| {
            // 2030-01-08, at this hour UTC.
            let time = std::time::UNIX_EPOCH + Duration::from_secs(1_894_060_800 + hours * 3600);
            a.context.sessions().move_to(time);
            server.hub().view().sessions().move_to(time);
        };
        at(7);
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a]);
        at(10);
        ok(Request::Unlock);
        assert!(server.hub().snapshot().unwrap().runs.is_empty());
        // Away until 13:00: closed there by then, so it waits.
        at(13);
        sync(&[&a]);
        assert!(a.runs("work"));
        assert_eq!(a.status().held.len(), 1);
        assert!(a.status().held[0].why.contains("not ready"), "{}", a.status().held[0].why);
        // Open again at 8:00 the next day: done then.
        at(24 + 8);
        sync(&[&a]);
        assert!(!a.runs("work"));
        assert!(a.status().held.is_empty());
    }

    /// The same with a server that timestamps what comes in (`hub::Stamper`): the computer away does the
    /// unlock when it is back, as of when it was made, and does not wait for unlocking to open again.
    #[test]
    fn with_timestamps_an_unlock_reaches_a_computer_away_when_it_is_back() {
        let dir = hub::tests::dir("syncing-stamped");
        let time = Arc::new(Mutex::new(std::time::UNIX_EPOCH));
        let stamp_time = time.clone();
        let hub = Hub::new(hub::tests::context(&dir), Some(dir))
            .with_stamper(Box::new(move |digest| Some(crate::timestamp::tests::make(digest, *stamp_time.lock().unwrap()))));
        let server = Arc::new(Server { hub: Mutex::new(Arc::new(hub)), down: AtomicBool::new(false), gone: AtomicBool::new(false) });
        let ok = |request| assert_eq!(server.hub().request(&request).unwrap(), Response::Done);
        ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        ok(Request::SaveCondition { condition: crate::definitions::tests::weekly("closed", "daily 12-8") });
        ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        ok(Request::SetUnlockWay { way: Some(Lock::Condition { condition: "closed".into() }) });
        ok(Request::SetUnlockDelay { delay: "1h".into() });
        let a = computer("stamped-a", &server);
        assert_eq!(a.connect(), Response::Done);
        let at = |hours: u64| {
            let moment = std::time::UNIX_EPOCH + Duration::from_secs(1_894_060_800 + hours * 3600);
            a.context.sessions().move_to(moment);
            server.hub().view().sessions().move_to(moment);
            *time.lock().unwrap() = moment;
        };
        at(7);
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a]);
        at(10);
        ok(Request::Unlock);
        at(13);
        sync(&[&a]);
        assert!(!a.runs("work"));
        assert!(a.status().held.is_empty());
    }

    /// As above, with the countdown asked for too: the computer away counts it down from when it comes
    /// back, and does the unlock when it is over, before unlocking opens there again.
    #[test]
    fn a_countdown_asked_for_with_the_unlock_lets_a_computer_away_unlock_after_the_delay() {
        let server = server("late-countdown");
        let ok = |request| assert_eq!(server.hub().request(&request).unwrap(), Response::Done);
        ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        ok(Request::SaveCondition { condition: crate::definitions::tests::weekly("closed", "daily 12-8") });
        ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        ok(Request::SetUnlockWay { way: Some(Lock::Condition { condition: "closed".into() }) });
        ok(Request::SetUnlockDelay { delay: "1h".into() });
        let a = computer("late-countdown-a", &server);
        assert_eq!(a.connect(), Response::Done);
        let at = |minutes: u64| {
            // 2030-01-08, this many minutes after midnight UTC.
            let time = std::time::UNIX_EPOCH + Duration::from_secs(1_894_060_800 + minutes * 60);
            a.context.sessions().move_to(time);
            server.hub().view().sessions().move_to(time);
        };
        at(7 * 60);
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a]);
        at(10 * 60);
        ok(Request::AskForUnlock);
        ok(Request::Unlock);
        at(13 * 60);
        sync(&[&a]);
        assert!(a.runs("work"));
        at(13 * 60 + 30);
        sync(&[&a]);
        assert!(a.runs("work"), "the delay counts from when the computer came back");
        at(14 * 60 + 1);
        sync(&[&a]);
        assert!(!a.runs("work"));
        assert!(a.status().held.is_empty());
    }

    #[test]
    fn a_computer_that_joins_takes_the_servers_sessions_and_a_start_reaches_every_computer() {
        let server = server("join");
        defined(&server, Lock::None);
        let (a, b) = (computer("join-a", &server), computer("join-b", &server));
        // B had something of its own: taking the server's replaces it.
        b.ok(Request::SaveTarget { target: target("mine", &["mine.com"]) });
        assert_eq!(a.connect(), Response::Done);
        assert_eq!(b.connect(), Response::Done);
        assert_eq!(a.context.library(), server.hub().snapshot().unwrap().library);
        assert_eq!(b.context.library(), a.context.library());
        assert_eq!(b.context.unlock().delay(), 3600);
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a, &b]);
        assert!(b.runs("work"));
        assert_eq!(server.hub().snapshot().unwrap().runs.len(), 1);
        b.ok(Request::Stop { id: "work".into(), password: None, text: None });
        sync(&[&a, &b]);
        assert!(!a.runs("work") && !b.runs("work"));
        assert!(server.hub().snapshot().unwrap().runs.is_empty());
        // A computer that joins while a session runs has it running too.
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a]);
        let c = computer("join-c", &server);
        assert_eq!(c.connect(), Response::Done);
        assert!(c.runs("work"));
        assert_eq!(c.context.sessions().starts_of("work"), a.context.sessions().starts_of("work"));
    }

    /// The first computer on a new server sets everything up: what it has defined, its unlock settings and
    /// what runs. One that joins later, a freshly installed one with nothing defined too, takes the
    /// server's and overwrites nobody.
    #[test]
    fn the_first_computer_fills_a_new_server_and_the_next_ones_take_it() {
        let server = server("first");
        let (a, b) = (computer("first-a", &server), computer("first-b", &server));
        a.ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        a.ok(Request::SaveSession { session: session("work", &["social"]) });
        a.ok(Request::SetUnlockDelay { delay: "2h".into() });
        a.ok(Request::Start { id: "work".into() });
        assert_eq!(a.connect(), Response::Done);
        sync(&[&a]);
        let snapshot = server.hub().snapshot().unwrap();
        assert_eq!(snapshot.library, a.context.library());
        assert_eq!(snapshot.unlock_delay, "7200s");
        assert_eq!(snapshot.runs.len(), 1);
        // B has nothing: it takes A's, and A keeps everything.
        assert_eq!(b.connect(), Response::Done);
        sync(&[&a, &b]);
        assert_eq!(b.context.library(), a.context.library());
        assert!(a.context.library().session("work").is_some());
        assert_eq!(b.context.unlock().delay(), 7200);
        assert!(b.runs("work") && a.runs("work"));
    }

    /// A copy of a server's folder, as a backup is.
    fn backup(from: &std::path::Path, name: &str) -> PathBuf {
        let to = hub::tests::dir(&format!("syncing-{name}"));
        fs::create_dir_all(&to).unwrap();
        for file in fs::read_dir(from).unwrap().flatten().filter(|f| f.path().is_file()) {
            fs::copy(file.path(), to.join(file.file_name())).unwrap();
        }
        to
    }

    #[test]
    fn a_wiped_server_is_given_everything_again_and_takes_no_lock_away() {
        let server = server("wiped");
        defined(&server, Lock::timed_for("2h"));
        let (a, b) = (computer("wiped-a", &server), computer("wiped-b", &server));
        assert_eq!(a.connect(), Response::Done);
        assert_eq!(b.connect(), Response::Done);
        a.ok(Request::SetUnlockDelay { delay: "3h".into() });
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a, &b]);
        assert!(b.runs("work") && b.locked());
        let library = a.context.library();

        // The server comes back empty (a container without its volume).
        server.replace(hub::tests::dir("syncing-wiped-empty"));
        sync(&[&a, &b]);
        sync(&[&a, &b]);
        // Nothing here was taken from it: what is defined, the delay and the lock all stay.
        for computer in [&a, &b] {
            assert_eq!(computer.context.library(), library);
            assert_eq!(computer.context.unlock().delay(), 3 * 3600);
            assert!(computer.runs("work") && computer.locked());
            assert!(computer.status().error.is_none(), "{:?}", computer.status().error);
        }
        // The server has it all again, and what is done now reaches the others (it is not left behind).
        let snapshot = server.hub().snapshot().unwrap();
        assert_eq!((snapshot.library, snapshot.runs.len(), snapshot.unlock_delay.as_str()), (library.clone(), 1, "10800s"));
        a.ok(Request::SaveTarget { target: target("news", &["news.com"]) });
        sync(&[&a, &b]);
        assert!(b.context.library().target("news").is_some());
        // And the way out: once unlocked, nothing that waited takes what is defined away.
        assert!(b.unlocks());
        sync(&[&a, &b]);
        // (On A, B's unlock waits for A's own countdown, as any unlock from elsewhere does.)
        assert!(b.context.library().session("work").is_some() && b.context.library().target("news").is_some());
        assert_eq!(a.context.library(), b.context.library());
    }

    #[test]
    fn a_computer_away_since_before_a_wipe_takes_what_the_others_gave_the_server_since() {
        let server = server("stale");
        defined(&server, Lock::timed_for("3h"));
        let (a, b) = (computer("stale-a", &server), computer("stale-b", &server));
        assert_eq!(a.connect(), Response::Done);
        assert_eq!(b.connect(), Response::Done);
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a, &b]);
        assert!(b.locked());

        // B is away. An hour later A blocks more and adds a target; the server is wiped and A fills it again.
        a.later(Duration::from_secs(3600));
        b.later(Duration::from_secs(3600));
        a.ok(Request::SaveTarget { target: target("social", &["youtube.com", "x.com"]) });
        a.ok(Request::SaveTarget { target: target("news", &["news.com"]) });
        a.ok(Request::SetUnlockDelay { delay: "2h".into() });
        server.replace(hub::tests::dir("syncing-stale-empty"));
        sync(&[&a]);
        sync(&[&a]);
        assert!(server.hub().snapshot().unwrap().library.target("news").is_some());
        // B did something of its own while away.
        b.ok(Request::SaveTarget { target: target("mine", &["mine.com"]) });

        // B comes back: what it had (social without x.com, no news, a 1 hour delay) does not undo A's.
        sync(&[&a, &b]);
        sync(&[&a, &b]);
        for computer in [&a, &b] {
            let library = computer.context.library();
            let mut filters = library.target("social").unwrap().entries.filters.clone();
            filters.sort();
            assert_eq!(filters, vec!["x.com", "youtube.com"]);
            assert!(library.target("news").is_some() && library.target("mine").is_some(), "{library:?}");
            assert_eq!(computer.context.unlock().delay(), 2 * 3600);
            assert!(computer.runs("work") && computer.locked());
            assert!(computer.status().error.is_none(), "{:?}", computer.status().error);
        }
        assert_eq!(a.context.library(), b.context.library());
        assert_eq!(server.hub().snapshot().unwrap().library, a.context.library());
    }

    #[test]
    fn the_newer_of_the_server_and_what_a_computer_saw_last_wins() {
        assert!(newer(Some(2), Some(1)));
        assert!(!newer(Some(1), Some(1)), "a backup of the very moment seen: what is here goes to it");
        assert!(!newer(Some(1), Some(2)));
        assert!(newer(Some(1), None), "a computer that saw nothing takes what the server has");
        assert!(!newer(None, Some(1)) && !newer(None, None), "an empty server is given what is here");
    }

    #[test]
    fn a_server_restored_from_an_older_backup_loses_nothing_done_since() {
        let server = server("restored");
        defined(&server, Lock::None);
        let (a, b) = (computer("restored-a", &server), computer("restored-b", &server));
        assert_eq!(a.connect(), Response::Done);
        assert_eq!(b.connect(), Response::Done);
        sync(&[&a, &b]);
        // The folder `server("restored")` keeps its files in.
        let folder = std::env::temp_dir().join(format!("hub-{}-syncing-restored", std::process::id()));
        let copy = backup(&folder, "restored-backup");
        // Done after the backup: the server will not have it.
        a.ok(Request::SaveTarget { target: target("one", &["one.com"]) });
        a.ok(Request::SaveTarget { target: target("two", &["two.com"]) });
        a.ok(Request::SaveTarget { target: target("three", &["three.com"]) });
        sync(&[&a, &b]);
        assert!(b.context.library().target("three").is_some());

        server.replace(copy);
        // B does something before A hears of the restore: its number on the restored log is one A had read
        // past already. A must not skip it.
        b.ok(Request::SaveTarget { target: target("four", &["four.com"]) });
        sync(&[&a, &b]);
        sync(&[&a, &b]);
        assert!(a.context.library().target("four").is_some(), "skipped what B did after the restore");
        for name in ["one", "two", "three", "four"] {
            assert!(server.hub().snapshot().unwrap().library.target(name).is_some(), "the server lost {name}");
        }
        assert_eq!(a.context.library(), b.context.library());
    }

    #[test]
    fn a_wrong_token_or_no_server_connects_nothing() {
        let server = server("refused");
        let a = computer("refused-a", &server);
        let refused = a.ask(Request::SyncConnect { url: URL.into(), token: "wrong token, long enough".into() });
        assert_eq!(refused, Response::Failed("The server refused the token".into()));
        server.down.store(true, Ordering::SeqCst);
        assert!(matches!(a.connect(), Response::Failed(why) if why.contains("cannot be reached")));
        assert!(matches!(a.ask(Request::SyncConnect { url: "sync.example.com".into(), token: TOKEN.into() }), Response::Failed(_)));
        assert!(!a.status().connected);
        assert!(!a.dir.join("sync.json").exists());
        // Not connected: a round does nothing, and what is done here is not kept for the server.
        a.ok(Request::SaveTarget { target: target("social", &["a.com"]) });
        assert_eq!(a.syncer.round(&a.context, Duration::ZERO), Ok(()));
        assert_eq!(a.status().outbox, 0);
        assert_eq!(a.ask(Request::SyncNow), Response::Done);
    }

    #[test]
    fn a_lock_from_elsewhere_is_unlocked_here_and_the_unlock_reaches_every_computer() {
        let server = server("unlock");
        defined(&server, Lock::UntilRestart);
        let (a, b) = (computer("unlock-a", &server), computer("unlock-b", &server));
        a.connect();
        b.connect();
        assert_eq!(server.hub().request(&Request::Start { id: "work".into() }).unwrap(), Response::Done);
        sync(&[&a, &b]);
        assert!(a.locked() && b.locked());
        // Refused here as at the keyboard, and nothing of it goes out.
        assert!(matches!(a.ask(Request::Stop { id: "work".into(), password: None, text: None }), Response::Failed(_)));
        assert_eq!(a.status().outbox, 0);
        assert!(a.unlocks());
        sync(&[&a, &b]);
        // B counts the request from when it heard of it: its own countdown, then the unlock.
        assert!(b.locked());
        b.later(Duration::from_secs(unlock::MAX_DELAY + 1));
        sync(&[&a, &b]);
        assert!(!b.locked() && !b.runs("work"));
        assert!(server.hub().snapshot().unwrap().runs.is_empty());
        // Nothing brings it back.
        sync(&[&a, &b]);
        assert!(!a.runs("work") && !b.runs("work"));
        assert!(a.status().held.is_empty() && b.status().held.is_empty());
    }

    /// The same computer after Windows restarted: its service started again over the same folder.
    fn restarted(computer: &Computer, server: &Arc<Server>) -> Computer {
        let dir = computer.dir.clone();
        let syncer = Arc::new(Syncer::at(Some(dir.clone()), connector(server), "again".into()));
        let sessions = computer.context.sessions().restarted("after");
        let context = Context::new(
            sessions,
            crate::definitions::Definitions::at(Some(dir.join("sessions.json"))),
            Arc::new(crate::watchdog::AppSlot::new(false)),
        )
        .with_unlock(unlock::UnlockDelay::at(Some(dir.join("unlock.json"))));
        Computer { context: Arc::new(context.with_syncer(syncer.clone())), syncer, dir }
    }

    #[test]
    fn a_run_that_ends_with_a_restart_ends_on_the_server_and_waits_for_each_computers_own() {
        let server = server("restarted");
        defined(&server, Lock::UntilRestart);
        let (a, b) = (computer("restarted-a", &server), computer("restarted-b", &server));
        a.connect();
        b.connect();
        a.ok(Request::Start { id: "work".into() });
        sync(&[&a, &b]);
        assert!(a.locked() && b.locked());
        assert_eq!(server.hub().snapshot().unwrap().runs.len(), 1);
        // A restarts: its run ended with it, and the others are told, once.
        let a = restarted(&a, &server);
        assert!(!a.runs("work"));
        a.syncer.restarted(&a.context);
        assert_eq!(a.status().outbox, 1);
        a.syncer.restarted(&a.context);
        assert_eq!(a.status().outbox, 1);
        sync(&[&a, &b]);
        // The server's view has no restart of its own: it ends there, so nobody who joins later takes it.
        assert!(server.hub().snapshot().unwrap().runs.is_empty());
        let c = computer("restarted-c", &server);
        c.connect();
        assert!(!c.runs("work"));
        // B is locked until its own restart: the stop waits for that, and then has nothing to do.
        assert!(b.locked());
        assert_eq!(b.status().held.len(), 1);
        assert!(b.status().held[0].why.contains("restarts"));
        let b = restarted(&b, &server);
        assert!(!b.runs("work"));
        b.syncer.restarted(&b.context);
        sync(&[&a, &b]);
        assert!(b.status().held.is_empty());
        assert!(!a.runs("work") && !b.runs("work"));
    }

    #[test]
    fn nothing_of_the_server_is_taken_over_once_disconnected() {
        let server = server("adopt-gone");
        defined(&server, Lock::UntilRestart);
        assert_eq!(server.hub().request(&Request::Start { id: "work".into() }).unwrap(), Response::Done);
        let a = computer("adopt-gone-a", &server);
        // A round that was joining again when Disconnect came: what it read is not applied.
        a.syncer.adopt(&a.context, &server.hub().snapshot().unwrap());
        assert!(a.context.library().sessions.is_empty() && !a.runs("work"));
    }

    #[test]
    fn a_restart_while_not_connected_tells_nobody() {
        let server = server("restarted-alone");
        defined(&server, Lock::None);
        let a = computer("restarted-alone-a", &server);
        a.ok(Request::SaveTarget { target: target("social", &["youtube.com"]) });
        a.ok(Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } });
        a.ok(Request::Start { id: "work".into() });
        let a = restarted(&a, &server);
        a.syncer.restarted(&a.context);
        assert!(!a.runs("work") && !a.syncer.connected() && a.status().outbox == 0);
        // Nor does a run that ended by its own time, or that is still running.
        assert!(a.context.sessions().take_restarted().is_empty());
    }

    fn disconnect(computer: &Computer, password: Option<&str>, text: Option<&str>) -> Response {
        computer.ask(Request::SyncDisconnect { password: password.map(str::to_string), text: text.map(str::to_string) })
    }

    /// With the server reached, disconnecting is at once, after one more round: a session started
    /// elsewhere and not yet here comes in first, and runs on here, locked, by its own rules.
    #[test]
    fn disconnecting_with_the_server_takes_in_what_waits_there_first() {
        let server = server("disconnect-reached");
        defined(&server, Lock::UntilRestart);
        let a = computer("disconnect-reached-a", &server);
        a.connect();
        assert_eq!(server.hub().request(&Request::Start { id: "work".into() }).unwrap(), Response::Done);
        assert!(!a.runs("work"), "not taken in yet");
        assert_eq!(disconnect(&a, None, None), Response::Done);
        assert!(!a.status().connected && !a.dir.join("sync.json").exists());
        assert!(a.runs("work") && a.locked());
        // Not connected: nothing to do.
        assert_eq!(disconnect(&a, None, None), Response::Done);
    }

    /// Without the server, for whatever reason, disconnecting is as hard as unlocking, locked or not: a
    /// countdown of the unlock delay, said in the status, kept over a restart, and over once the server is
    /// reached again. The sessions go on by their own rules either way, and the way out stays.
    #[test]
    fn disconnecting_without_the_server_takes_the_unlock_countdown() {
        let server = server("disconnect-away");
        defined(&server, Lock::UntilRestart);
        let a = computer("disconnect-away-a", &server);
        a.connect();
        assert_eq!(server.hub().request(&Request::Start { id: "work".into() }).unwrap(), Response::Done);
        sync(&[&a]);
        server.down.store(true, Ordering::SeqCst);
        let waits = t("sync.disconnectWaits", &[("delay", "1 hour")]);
        assert_eq!(disconnect(&a, None, None), Response::Failed(waits.clone()));
        assert!(a.status().connected);
        assert_eq!(a.status().disconnect, Some(t("sync.disconnectPending", &[("left", "60 minutes")])));
        a.later(Duration::from_secs(1800));
        assert_eq!(disconnect(&a, None, None), Response::Failed(t("sync.disconnectLeft", &[("left", "30 minutes")])));
        // A service started again keeps it.
        let again = Syncer::at(Some(a.dir.clone()), connector(&server), "again".into());
        assert!(again.settings.locked().disconnect_asked.is_some());
        // The server back: over, and asked for again it starts from the beginning.
        server.down.store(false, Ordering::SeqCst);
        sync(&[&a]);
        assert_eq!(a.status().disconnect, None);
        server.down.store(true, Ordering::SeqCst);
        assert_eq!(disconnect(&a, None, None), Response::Failed(waits));
        a.later(Duration::from_secs(3601));
        assert_eq!(a.status().disconnect, Some(t("sync.disconnectReady", &[])));
        assert_eq!(disconnect(&a, None, None), Response::Done);
        let status = a.status();
        assert!(!status.connected && status.url.is_empty() && status.outbox == 0 && status.disconnect.is_none());
        assert!(!a.dir.join("sync.json").exists());
        assert!(a.locked(), "running sessions are not ended by it");
        assert!(a.unlocks());
        assert!(!Syncer::at(Some(a.dir.clone()), connector(&server), "again".into()).connected());
        // With no unlock delay, at once, as unlocking is.
        let free = server_and_computer("disconnect-free", "0");
        free.1.down.store(true, Ordering::SeqCst);
        assert_eq!(disconnect(&free.0, None, None), Response::Done);
    }

    /// A server with "work" defined, its unlock delay `delay`, and a computer connected to it.
    fn server_and_computer(name: &str, delay: &str) -> (Computer, Arc<Server>) {
        let server = server(name);
        defined(&server, Lock::UntilRestart);
        assert_eq!(server.hub().request(&Request::SetUnlockDelay { delay: delay.into() }).unwrap(), Response::Done);
        let a = computer(&format!("{name}-a"), &server);
        a.connect();
        (a, server)
    }

    /// Without the server, the other way to unlock also disconnects: its text or password at once (a
    /// random text drawn again after), an unlock window while it is open. A wrong answer starts the
    /// countdown, as no answer does.
    #[test]
    fn disconnecting_without_the_server_takes_the_other_way_to_unlock() {
        let way = |server: &Arc<Server>, lock: Lock| {
            assert_eq!(server.hub().request(&Request::SetUnlockWay { way: Some(lock) }).unwrap(), Response::Done);
        };
        let (a, server) = server_and_computer("disconnect-text", "1h");
        way(&server, Lock::Text { text: "let me go".into() });
        sync(&[&a]);
        server.down.store(true, Ordering::SeqCst);
        assert!(matches!(disconnect(&a, None, Some("wrong")), Response::Failed(_)));
        assert_eq!(disconnect(&a, None, Some("let me go")), Response::Done);

        let (a, server) = server_and_computer("disconnect-random", "1h");
        way(&server, Lock::RandomText { length: 8, text: None });
        sync(&[&a]);
        let drawn = |a: &Computer| match a.context.unlock().way().map(|way| way.lock) {
            Some(Lock::RandomText { text: Some(text), .. }) => text,
            other => panic!("{other:?}"),
        };
        let before = drawn(&a);
        server.down.store(true, Ordering::SeqCst);
        assert_eq!(disconnect(&a, None, Some(&before)), Response::Done);
        assert_ne!(drawn(&a), before, "used up: drawn again");

        let (a, server) = server_and_computer("disconnect-password", "1h");
        way(&server, Lock::Password { password: Some("secret".into()), hash: None });
        sync(&[&a]);
        server.down.store(true, Ordering::SeqCst);
        assert!(matches!(disconnect(&a, Some("nope"), None), Response::Failed(_)));
        assert_eq!(disconnect(&a, Some("secret"), None), Response::Done);

        // An unlock window: open now (it is closed only from 9:00 to 17:00 on a day that never comes).
        let (a, server) = server_and_computer("disconnect-window", "1h");
        assert_eq!(
            server
                .hub()
                .request(&Request::SaveCondition { condition: crate::definitions::tests::weekly("never", "daily 0-0:01") })
                .unwrap(),
            Response::Done
        );
        way(&server, Lock::Condition { condition: "never".into() });
        sync(&[&a]);
        server.down.store(true, Ordering::SeqCst);
        a.context.sessions().move_to(std::time::UNIX_EPOCH + Duration::from_secs(1_894_060_800 + 12 * 3600));
        assert_eq!(disconnect(&a, None, None), Response::Done);
    }

    #[test]
    fn a_service_started_again_stays_connected_and_keeps_what_it_did() {
        let server = server("restart");
        defined(&server, Lock::None);
        let a = computer("restart-a", &server);
        a.connect();
        let device = a.status().device;
        server.down.store(true, Ordering::SeqCst);
        a.ok(Request::Start { id: "work".into() });
        assert!(a.syncer.round(&a.context, Duration::ZERO).is_err());
        let status = a.status();
        assert_eq!(status.outbox, 1);
        assert!(status.error.unwrap().contains("cannot be reached"));
        let again = Syncer::at(Some(a.dir.clone()), connector(&server), "restart-a".into());
        assert!(again.connected());
        assert_eq!(again.status(&a.context).device, device);
        assert_eq!(again.status(&a.context).url, URL);
        server.down.store(false, Ordering::SeqCst);
        again.round(&a.context, Duration::ZERO).unwrap();
        assert_eq!(again.status(&a.context).outbox, 0);
        assert_eq!(server.hub().snapshot().unwrap().runs.len(), 1);
        // Without its settings, or with them unreadable, it is not connected.
        fs::write(a.dir.join("sync.json"), b"{").unwrap();
        assert!(!Syncer::at(Some(a.dir.clone()), connector(&server), "x".into()).connected());
        a.connect();
        fs::write(
            a.dir.join("sync.json"),
            serde_json::to_vec(&Settings { url: URL.into(), token: " ".into(), disconnect_asked: None }).unwrap(),
        )
        .unwrap();
        assert!(!Syncer::at(Some(a.dir.clone()), connector(&server), "x".into()).connected());
    }

    #[test]
    fn a_computer_away_too_long_joins_again_and_keeps_what_it_did_since() {
        let server = server("gone");
        defined(&server, Lock::None);
        let (a, b) = (computer("gone-a", &server), computer("gone-b", &server));
        a.connect();
        b.connect();
        server.hub().request(&Request::SaveTarget { target: target("news", &["news.com"]) }).unwrap();
        b.ok(Request::Start { id: "work".into() });
        server.gone.store(true, Ordering::SeqCst);
        sync(&[&b, &a]);
        assert!(a.context.library().target("news").is_some());
        assert!(a.runs("work") && b.runs("work"));
        assert_eq!(a.status().confirmed, server.hub().head());
    }

    /// Connected again, a computer is the same one to the server: listed once, as this computer. Its
    /// id is kept in its own file; one that cannot be read is made anew and kept.
    #[test]
    fn a_computer_connected_again_is_listed_once() {
        let server = server("again");
        defined(&server, Lock::None);
        let a = computer("again-a", &server);
        assert_eq!(a.connect(), Response::Done);
        sync(&[&a]);
        let first = a.status().device;
        a.ok(Request::SyncDisconnect { password: None, text: None });
        assert_eq!(a.connect(), Response::Done);
        sync(&[&a]);
        let status = a.status();
        assert_eq!(status.device, first);
        assert_eq!(server.hub().devices().unwrap().len(), 1);
        assert!(status.devices.iter().all(|d| d.this && d.id == first));
        // The same after the service starts again.
        let again = Syncer::at(Some(a.dir.clone()), connector(&server), "again-a".into());
        assert_eq!(again.computer, first);
        // An id that is not one (a file changed by hand) is made anew and kept.
        fs::write(a.dir.join("computer"), "not an id!").unwrap();
        let fresh = Syncer::at(Some(a.dir.clone()), connector(&server), "again-a".into()).computer;
        assert_ne!(fresh, first);
        assert_eq!(fs::read_to_string(a.dir.join("computer")).unwrap(), fresh);
        // Where nothing can be kept, a new one each time.
        assert_ne!(Syncer::at(None, connector(&server), "x".into()).computer, Syncer::at(None, connector(&server), "x".into()).computer);
        let blocked = a.dir.join("blocked");
        fs::write(&blocked, "a file, not a folder").unwrap();
        assert!(!computer_id(&blocked.join("computer")).is_empty());
    }

    #[test]
    fn the_status_says_where_syncing_stands_and_names_what_waits() {
        let server = server("status");
        defined(&server, Lock::None);
        let (a, b) = (computer("status-a", &server), computer("status-b", &server));
        a.connect();
        b.connect();
        sync(&[&a, &b]);
        a.ok(Request::SyncNow);
        let status = a.status();
        assert!(status.connected && status.error.is_none() && status.last_contact.is_some());
        assert_eq!((status.url.as_str(), status.confirmed), (URL, server.hub().head()));
        let names: Vec<String> = status.devices.iter().map(|d| d.name.clone()).collect();
        assert!(names.contains(&"status-a".to_string()) && names.contains(&"status-b".to_string()));
        assert!(status.devices.iter().any(|d| d.id == status.device && d.this && d.seen == "just now"));
        assert!(status.summary.starts_with("Connected to https://sync.example.com, last synced just now"), "{}", status.summary);
        // B's unlock waits on A until A's own countdown is over: said in the status.
        b.ok(Request::Start { id: "work".into() });
        server
            .hub()
            .request(&Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } })
            .unwrap();
        sync(&[&a, &b]);
        assert!(b.unlocks());
        sync(&[&a, &b]);
        let held = a.status().held;
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].what, "Unlock");
        assert!(!held[0].why.is_empty());
    }

    #[test]
    fn the_sync_threads_send_at_once_and_a_crash_disconnects() {
        let server = server("threads");
        defined(&server, Lock::None);
        let (a, b) = (computer("threads-a", &server), computer("threads-b", &server));
        a.connect();
        b.connect();
        a.syncer.spawn(a.context.clone());
        b.syncer.spawn(b.context.clone());
        a.ok(Request::Start { id: "work".into() });
        let started = Instant::now();
        while !b.runs("work") && started.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(b.runs("work"), "the start did not reach the other computer");
        assert!(started.elapsed() < Duration::from_secs(MAX_WAIT));
        assert_eq!(a.syncer.guarded(|| -> () { panic!("a bug") }), None);
        assert!(!a.syncer.connected());
        assert_eq!(b.syncer.guarded(|| 1), Some(1));
    }

    #[test]
    fn what_waits_is_named_for_people() {
        let mut library = Library::default();
        library.targets.push(target("social", &["a.com"]));
        library.sessions.push(session("work", &["social"]));
        let say = |action: Action| describe(&action, &library);
        assert_eq!(say(Action::Start { id: "work".into(), start: "s".into(), ends: None, text: None, zone: None }), "Start “Session work”");
        assert_eq!(say(Action::Stop { id: "gone".into(), starts: vec![], text: None, proof: None }), "Stop gone");
        assert_eq!(say(Action::DeleteTarget { id: "social".into() }), "Delete target “List social”");
        assert_eq!(say(Action::Duplicate { kind: Kind::Target, id: "social".into(), new_id: "copy".into() }), "Duplicate “List social”");
        assert_eq!(say(Action::Import { library: Library::default() }), "Replace all sessions, targets and conditions");
        let every = [
            Action::SaveSession { session: session("work", &[]) },
            Action::DeleteSession { id: "work".into() },
            Action::SaveTarget { target: target("social", &[]) },
            Action::SaveCondition {
                condition: crate::definitions::Condition {
                    id: "c".into(),
                    name: "C".into(),
                    rule: crate::condition::Rule::Schedule { week: Default::default() },
                },
            },
            Action::DeleteCondition { id: "c".into() },
            Action::Duplicate { kind: Kind::Session, id: "work".into(), new_id: "w".into() },
            Action::Duplicate { kind: Kind::Condition, id: "c".into(), new_id: "d".into() },
            Action::Lock { id: "work".into(), lock: Lock::None, ends: None, text: None },
            Action::Used { id: "work".into(), used: Default::default() },
            Action::AskToStop { id: "work".into() },
            Action::AbortStop { id: "work".into() },
            Action::TakeBreak { id: "work".into() },
            Action::EndBreak { id: "work".into() },
            Action::AskForUnlock,
            Action::AbortUnlock,
            Action::Unlock { text: None, next: None, proof: None },
            Action::SetUnlockDelay { delay: "1h".into() },
            Action::SetUnlockWay { way: None },
        ];
        for action in every {
            let said = say(action);
            assert!(!said.is_empty() && !said.contains("sync.action") && !said.contains('{'), "{said}");
        }
    }
}
