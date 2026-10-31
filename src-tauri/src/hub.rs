//! The sync server's logic (`sync_api.rs` has its API): the log every computer sends its entries to and
//! takes the others' from, numbered in the order they came, and the server's own view of what that
//! comes to (a `Context`, as a computer has, kept by `handler::mirror`: what a computer did is taken as
//! done). Requests to the server's API go through that view's `handler::handle_shared`, under the same
//! rules as at a computer's keyboard, and what they do goes into the log as the server's own entries.
//!
//! The server enforces nothing and decides nothing for a computer: each one applies the log under its
//! own rules (`replica.rs`). The HTTP side is the `bullseye-sync` binary (`server/`), kept thin.
//!
//! The log is kept in `log.jsonl` (one `[number, entry]` per line) and the last `KEEP` entries are
//! served; a computer that missed more joins again from a snapshot (`Failure::Gone`).

use std::collections::{HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::definitions::Definitions;
use crate::handler::{self, Context};
use crate::proto::{Request, Response};
use crate::replica::{self, Entry};
use crate::session::Sessions;
use crate::state::{self, Store};
use crate::sync::LockExt;
use crate::sync_api::{Appended, Device, DeviceReport, Failure, LogPage, Posted, Snapshot, Transport, MAX_POST, MAX_WAIT, PAGE};
use crate::unlock::UnlockDelay;
use crate::watchdog::AppSlot;

/// How many entries the log serves.
pub const KEEP: usize = 20_000;
/// The server's own id in the log, for what its API does.
pub const SERVER: &str = "server";
/// The most computers the server keeps track of.
const MAX_DEVICES: usize = 100;

struct Log {
    entries: VecDeque<(u64, String)>,
    /// The ids of the entries in `entries`, so one sent twice is taken once.
    ids: HashSet<String>,
    head: u64,
    /// Lines in the file, for when to write it anew with only what is kept.
    lines: usize,
}

pub struct Hub {
    context: Context,
    log: Mutex<Log>,
    changed: Condvar,
    /// `log.jsonl`; `None`: kept for this run only (tests).
    path: Option<PathBuf>,
    devices: Mutex<Vec<Device>>,
    /// `devices.json`.
    devices_path: Option<PathBuf>,
}

impl Hub {
    /// Over the view `context` and the log in `dir` (`None`: in memory), read back from the last run.
    /// The view keeps its own files: it is what the log came to, up to the entry it had last.
    pub fn new(context: Context, dir: Option<PathBuf>) -> Hub {
        let path = dir.as_ref().map(|dir| dir.join("log.jsonl"));
        let devices_path = dir.as_ref().map(|dir| dir.join("devices.json"));
        let mut log = Log { entries: VecDeque::new(), ids: HashSet::new(), head: 0, lines: 0 };
        if let Some(text) = path.as_ref().and_then(|path| fs::read_to_string(path).ok()) {
            for line in text.lines() {
                // A line cut short by a crash, or anything else unreadable, is skipped.
                let Ok((seq, entry)) = serde_json::from_str::<(u64, String)>(line) else { continue };
                if seq <= log.head {
                    continue;
                }
                log.lines += 1;
                log.head = seq;
                if let Ok(parsed) = Entry::parse(&entry) {
                    log.ids.insert(parsed.id);
                }
                log.entries.push_back((seq, entry));
                if log.entries.len() > KEEP {
                    forget_oldest(&mut log);
                }
            }
        }
        let devices = devices_path
            .as_ref()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Hub { context, log: Mutex::new(log), changed: Condvar::new(), path, devices: Mutex::new(devices), devices_path }
    }

    /// The number of the last entry.
    pub fn head(&self) -> u64 {
        self.log.locked().head
    }

    /// Appends `entry` (taken as it is) and writes it down, under the lock of the log.
    fn append(&self, log: &mut Log, id: String, text: String) {
        log.head += 1;
        let seq = log.head;
        if let Some(path) = &self.path {
            if let Err(e) = append_line(path, seq, &text) {
                eprintln!("hub: cannot write the log: {e}");
            }
            log.lines += 1;
        }
        log.ids.insert(id);
        log.entries.push_back((seq, text));
        if log.entries.len() > KEEP {
            forget_oldest(log);
        }
        // The file is written anew with what is kept once it holds a quarter more.
        if log.lines > KEEP + KEEP / 4 {
            if let Some(path) = &self.path {
                if rewrite(path, &log.entries).is_ok() {
                    log.lines = log.entries.len();
                }
            }
        }
        self.changed.notify_all();
    }
}

/// The server's view, kept in `dir` (`state.json`, `sessions.json`, `unlock.json`) and read back from
/// the last run, with the clock everything else has.
pub fn view(dir: &Path) -> Context {
    let sessions = Sessions::at(Store::new(dir.join("state.json")));
    sessions.restore();
    Context::new(sessions, Definitions::at(Some(dir.join("sessions.json"))), std::sync::Arc::new(AppSlot::new(false)))
        .with_unlock(UnlockDelay::at(Some(dir.join("unlock.json"))))
}

fn forget_oldest(log: &mut Log) {
    if let Some((_, text)) = log.entries.pop_front() {
        if let Ok(entry) = Entry::parse(&text) {
            log.ids.remove(&entry.id);
        }
    }
}

fn append_line(path: &PathBuf, seq: u64, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
    let line = serde_json::to_string(&(seq, text)).map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| e.to_string())
}

/// Through a temporary file, so a crash leaves the old file or the new.
fn rewrite(path: &PathBuf, entries: &VecDeque<(u64, String)>) -> Result<(), String> {
    let mut text = String::new();
    for entry in entries {
        text.push_str(&serde_json::to_string(entry).map_err(|e| e.to_string())?);
        text.push('\n');
    }
    let temporary = path.with_extension("new");
    fs::write(&temporary, text).map_err(|e| e.to_string())?;
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}

fn millis(time: SystemTime) -> u64 {
    state::to_unix(time).unwrap_or(0)
}

impl Transport for Hub {
    fn snapshot(&self) -> Result<Snapshot, Failure> {
        // Under the lock of the log, so the view is what the log comes to up to its head.
        let log = self.log.locked();
        Ok(handler::snapshot(&self.context, log.head))
    }

    fn log(&self, after: u64, wait: Duration) -> Result<LogPage, Failure> {
        let deadline = Instant::now() + wait.min(Duration::from_secs(MAX_WAIT));
        let mut log = self.log.locked();
        loop {
            let oldest = log.entries.front().map_or(log.head + 1, |(seq, _)| *seq);
            if after + 1 < oldest && after < log.head {
                return Err(Failure::Gone);
            }
            if log.head > after {
                let entries = log.entries.iter().filter(|(seq, _)| *seq > after).take(PAGE).cloned().collect();
                return Ok(LogPage { head: log.head, entries });
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(LogPage { head: log.head, entries: Vec::new() });
            }
            log = self.changed.wait_timeout(log, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// A computer's entries, each taken once and as it came (it did them under its own rules), and
    /// mirrored into the view. Entries that cannot be read are left out.
    fn post(&self, posted: Posted) -> Result<Appended, Failure> {
        if posted.entries.len() > MAX_POST {
            return Err(Failure::Other(format!("At most {MAX_POST} entries at once")));
        }
        let mut log = self.log.locked();
        for text in posted.entries {
            let Ok(entry) = Entry::parse(&text) else { continue };
            if log.ids.contains(&entry.id) || entry.device == SERVER {
                continue;
            }
            handler::mirror(&self.context, &entry.action, state::from_unix(entry.at));
            // Kept as it is now, re-written: what was read is what is served.
            self.append(&mut log, entry.id.clone(), entry.text());
        }
        Ok(Appended { head: log.head })
    }

    fn report(&self, device: &str, report: DeviceReport) -> Result<(), Failure> {
        if device.is_empty() || device.len() > 64 {
            return Err(Failure::Other("A computer's id is empty or too long".into()));
        }
        let mut devices = self.devices.locked();
        devices.retain(|known| known.id != device);
        if devices.len() >= MAX_DEVICES {
            // The one not heard from the longest goes.
            if let Some(oldest) = devices.iter().enumerate().min_by_key(|(_, d)| d.seen).map(|(at, _)| at) {
                devices.remove(oldest);
            }
        }
        let report =
            DeviceReport { name: report.name.chars().take(100).collect(), held: report.held.into_iter().take(50).collect(), ..report };
        devices.push(Device { id: device.to_string(), seen: millis(self.context.now()), report });
        if let Some(path) = &self.devices_path {
            if let Err(e) =
                serde_json::to_vec(&*devices).map_err(|e| e.to_string()).and_then(|json| fs::write(path, json).map_err(|e| e.to_string()))
            {
                eprintln!("hub: cannot write the devices: {e}");
            }
        }
        Ok(())
    }

    fn devices(&self) -> Result<Vec<Device>, Failure> {
        Ok(self.devices.locked().clone())
    }

    /// A request to the server's API, under the same rules as at a computer: what it does goes into
    /// the log as the server's own entry.
    fn request(&self, request: &Request) -> Result<Response, Failure> {
        let mut log = self.log.locked();
        let (response, _, action) = handler::handle_shared(&self.context, 0, request.clone());
        if let Some(action) = action {
            let entry = Entry { id: replica::new_id(), device: SERVER.into(), at: millis(self.context.now()), action };
            self.append(&mut log, entry.id.clone(), entry.text());
        }
        Ok(response)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::definitions::tests::{session, target};
    use crate::definitions::Definition;
    use crate::lock::Lock;
    use crate::replica::Action;
    use std::sync::Arc;

    pub fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hub-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    pub fn context(dir: &std::path::Path) -> Context {
        let sessions = Sessions::with_store(Store::new(dir.join("state.json")));
        Context::new(sessions, Definitions::at(Some(dir.join("sessions.json"))), Arc::new(AppSlot::new(false)))
            .with_unlock(UnlockDelay::at(Some(dir.join("unlock.json"))))
    }

    fn entry(device: &str, action: Action) -> String {
        Entry { id: replica::new_id(), device: device.into(), at: millis(SystemTime::now()), action }.text()
    }

    #[test]
    fn entries_are_numbered_once_and_kept_across_runs() {
        let dir = dir("kept");
        let hub = Hub::new(context(&dir), Some(dir.clone()));
        let one = entry("a", Action::SaveTarget { target: target("social", &["a.com"]) });
        let posted =
            hub.post(Posted { entries: vec![one.clone(), one.clone(), "garbage".into(), entry(SERVER, Action::AskForUnlock)] }).unwrap();
        assert_eq!(posted.head, 1);
        assert_eq!(hub.post(Posted { entries: vec![one] }).unwrap().head, 1);
        // Mirrored into the view.
        assert!(hub.snapshot().unwrap().library.target("social").is_some());
        let page = hub.log(0, Duration::ZERO).unwrap();
        assert_eq!((page.head, page.entries.len()), (1, 1));
        // A line cut short by a crash is skipped.
        fs::write(dir.join("log.jsonl"), format!("{}\n[2, \"cut", fs::read_to_string(dir.join("log.jsonl")).unwrap().trim())).unwrap();
        let again = Hub::new(context(&dir), Some(dir.clone()));
        assert_eq!(again.head(), 1);
        assert_eq!(again.log(0, Duration::ZERO).unwrap(), page);
        assert!(again.snapshot().unwrap().library.target("social").is_some());
        assert!(hub.post(Posted { entries: vec![String::new(); MAX_POST + 1] }).is_err());
    }

    #[test]
    fn a_log_request_waits_for_an_entry_and_no_longer_than_asked() {
        let dir = dir("wait");
        let hub = Arc::new(Hub::new(context(&dir), None));
        let started = Instant::now();
        assert!(hub.log(0, Duration::from_millis(50)).unwrap().entries.is_empty());
        assert!(started.elapsed() >= Duration::from_millis(50));
        let waiting = {
            let hub = hub.clone();
            std::thread::spawn(move || hub.log(0, Duration::from_secs(10)).unwrap())
        };
        std::thread::sleep(Duration::from_millis(50));
        hub.post(Posted { entries: vec![entry("a", Action::AskForUnlock)] }).unwrap();
        assert_eq!(waiting.join().unwrap().entries.len(), 1);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_computer_that_missed_more_than_is_kept_joins_again() {
        let dir = dir("gone");
        let hub = Hub::new(context(&dir), Some(dir.clone()));
        let entries: Vec<String> = (0..KEEP + KEEP / 4 + 10).map(|_| entry("a", Action::AskToStop { id: "x".into() })).collect();
        for chunk in entries.chunks(MAX_POST) {
            hub.post(Posted { entries: chunk.to_vec() }).unwrap();
        }
        assert_eq!(hub.log(0, Duration::ZERO), Err(Failure::Gone));
        let head = hub.head();
        assert_eq!(hub.log(head, Duration::ZERO).unwrap().entries.len(), 0);
        let page = hub.log(head - 10, Duration::ZERO).unwrap();
        assert_eq!(page.entries.len(), 10);
        assert_eq!(hub.log(head - KEEP as u64, Duration::ZERO).unwrap().entries.len(), PAGE);
        // The file was written anew with what is kept, and reads back the same.
        let lines = fs::read_to_string(dir.join("log.jsonl")).unwrap().lines().count();
        assert!(lines < KEEP + KEEP / 4);
        assert_eq!(Hub::new(context(&dir), Some(dir)).log(head - 10, Duration::ZERO).unwrap(), page);
    }

    #[test]
    fn the_api_has_the_rules_of_a_computer_and_shares_what_it_does() {
        let dir = dir("api");
        let hub = Hub::new(context(&dir), None);
        assert_eq!(hub.request(&Request::SaveTarget { target: target("social", &["a.com"]) }).unwrap(), Response::Done);
        let session = Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) };
        assert_eq!(hub.request(&Request::SaveSession { session }).unwrap(), Response::Done);
        assert_eq!(hub.request(&Request::Start { id: "work".into() }).unwrap(), Response::Done);
        assert_eq!(hub.request(&Request::SetUnlockDelay { delay: "1h".into() }).unwrap(), Response::Done);
        // Refused as at a computer, and nothing goes into the log for it.
        let head = hub.head();
        assert!(matches!(hub.request(&Request::Stop { id: "work".into(), password: None, text: None }).unwrap(), Response::Failed(_)));
        assert!(matches!(hub.request(&Request::Status).unwrap(), Response::Status(_)));
        assert_eq!(hub.head(), head);
        let page = hub.log(0, Duration::ZERO).unwrap();
        assert!(page.entries.iter().all(|(_, text)| Entry::parse(text).unwrap().device == SERVER));
        let snapshot = hub.snapshot().unwrap();
        assert_eq!(snapshot.head, head);
        assert_eq!(snapshot.unlock_delay, "3600s");
        assert_eq!(snapshot.runs.len(), 1);
        assert_eq!(snapshot.runs[0].id, "work");
    }

    #[test]
    fn the_server_started_again_has_what_runs_and_what_is_defined() {
        let dir = dir("view");
        let hub = Hub::new(view(&dir.join("view")), Some(dir.clone()));
        hub.request(&Request::SaveTarget { target: target("social", &["a.com"]) }).unwrap();
        let session = Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) };
        hub.request(&Request::SaveSession { session }).unwrap();
        assert_eq!(hub.request(&Request::Start { id: "work".into() }).unwrap(), Response::Done);
        let before = hub.snapshot().unwrap();
        let again = Hub::new(view(&dir.join("view")), Some(dir));
        assert_eq!(again.snapshot().unwrap(), before);
        assert_eq!(before.runs.len(), 1);
    }

    #[test]
    fn computers_report_themselves() {
        let dir = dir("devices");
        let hub = Hub::new(context(&dir), Some(dir.clone()));
        fs::create_dir_all(&dir).unwrap();
        let report = DeviceReport { name: "Desk".into(), confirmed: 3, held: vec!["why".into()], running: vec!["Work".into()] };
        hub.report("d1", report.clone()).unwrap();
        hub.report("d1", DeviceReport { name: "x".repeat(500), ..report.clone() }).unwrap();
        assert!(hub.report("", report.clone()).is_err());
        let devices = hub.devices().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].report.name.len(), 100);
        assert_eq!(Hub::new(context(&dir), Some(dir.clone())).devices().unwrap(), devices);
        for n in 0..MAX_DEVICES + 3 {
            hub.report(&format!("d{n}"), report.clone()).unwrap();
        }
        assert_eq!(hub.devices().unwrap().len(), MAX_DEVICES);
    }
}
