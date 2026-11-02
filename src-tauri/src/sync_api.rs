//! The sync server's API, as the computers (`syncing.rs`), the CLI (`client.rs`) and the server
//! itself (`hub.rs`, and the `bullseye-sync` binary in `server/`) know it: one set of types, and the
//! `Transport` trait that every client side talks through. Every path but `/api/health` needs the
//! server's token as `Authorization: Bearer <token>`.
//!
//! | Method and path | Body | Answer |
//! |---|---|---|
//! | `GET /api/health` | | `ok` |
//! | `GET /api/snapshot` | | `Snapshot`: what is defined, the unlock settings, what runs, the log's head |
//! | `GET /api/log?after=N&last=ID&wait=S` | | `LogPage`: the entries after `N`, waiting up to `S` seconds for one; `410` if `N` is older than the log keeps, `409` if the log has no `N`, or another entry than `ID` at `N` (it was wiped, or restored from a backup) |
//! | `POST /api/log` | `Posted` | `Appended`: entries of a computer's, each once (by id); `409` as for `GET` if its `after` and `last` do not match |
//! | `POST /api/request` | a `proto::Request` | its `proto::Response`: the server's own view, under the same rules as a computer, and shared if done |
//! | `PUT /api/devices/<id>` | `DeviceReport` | `204` |
//! | `GET /api/devices` | | `[Device]` |

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::definitions::Library;
use crate::lock::Lock;
use crate::proto::{Request, Response};

/// The longest a log request waits for an entry, in seconds.
pub const MAX_WAIT: u64 = 25;
/// The most entries one log page holds.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
pub const PAGE: usize = 500;
/// The most entries one post takes.
pub const MAX_POST: usize = 200;
/// A running session as the server's view has it, for a computer that joins: started there as it was.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RunShare {
    pub id: String,
    pub starts: Vec<String>,
    /// When it started (milliseconds since the Unix epoch).
    pub started: u64,
    /// A duration lock's end, a drawn text and the time zone it is read in (`replica::Action::Start`).
    pub ends: Option<u64>,
    pub text: Option<String>,
    #[serde(default)]
    pub zone: Option<String>,
}

/// Everything a computer that joins takes over, as of the entry `head` of the log.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub head: u64,
    /// The id of the entry at `head`, which a computer joining from here sends back with its next log
    /// request (`last`), so the server can tell its log is still the one it joined; none when the log is
    /// empty.
    #[serde(default)]
    pub last: Option<String>,
    /// When the entry at `head` was done (`replica::Entry::at`): a computer that finds the log is not the
    /// one it synced with takes this snapshot over if it is newer than what it saw last, and sends its
    /// own otherwise (`syncing.rs`).
    #[serde(default)]
    pub newest: Option<u64>,
    /// With password hashes: a computer needs them.
    pub library: Library,
    pub unlock_delay: String,
    pub unlock_way: Option<Lock>,
    pub runs: Vec<RunShare>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LogPage {
    /// The number of the last entry there is.
    pub head: u64,
    /// The entries after the one asked for, at most `PAGE`, each with its number, as `replica::Entry` text.
    pub entries: Vec<(u64, String)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Posted {
    pub entries: Vec<String>,
    /// Where the computer posting stands in the log, as for `log`: a post to a log that is not the one it
    /// synced with (wiped, or restored from a backup) is refused (`Failure::Diverged`), so nothing goes
    /// into it before the computer has noticed. Nothing given: not checked.
    #[serde(default)]
    pub after: Option<u64>,
    #[serde(default)]
    pub last: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Appended {
    pub head: u64,
}

/// What a computer says of itself, for the others to see.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DeviceReport {
    /// Its name (Windows' computer name).
    pub name: String,
    /// The last entry of the log it applied.
    pub confirmed: u64,
    /// What waits there, in words (`replica::Held::why`).
    pub held: Vec<String>,
    /// The names of the sessions running there.
    pub running: Vec<String>,
    /// Its time zone (`localtime::Zone::name`): the server reads the sessions started on its page in
    /// the zone of the computer it heard from last.
    #[serde(default)]
    pub zone: Option<String>,
}

/// A computer as the server knows it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Device {
    pub id: String,
    /// When it last reported (milliseconds since the Unix epoch).
    pub seen: u64,
    #[serde(flatten)]
    pub report: DeviceReport,
}

/// A refusal from the server, or no answer.
#[derive(Debug, PartialEq, Clone)]
pub enum Failure {
    /// The log no longer has what was asked for (this computer was away too long): join again.
    Gone,
    /// The log is not the one this computer synced with: it has no entry as far as the computer read, or
    /// another one there (the server was wiped, or restored from a backup). The computer sends what it
    /// has again, as when it connects with its own settings (`syncing.rs`).
    Diverged,
    /// Wrong token.
    Unauthorized,
    /// No answer, or one that cannot be read.
    Other(String),
}

impl Failure {
    pub fn words(&self) -> String {
        match self {
            Failure::Gone => "The server no longer has what this computer missed".into(),
            Failure::Diverged => "The server's log is not the one this computer synced with".into(),
            Failure::Unauthorized => "The server refused the token".into(),
            Failure::Other(why) => why.clone(),
        }
    }
}

/// The server, as every client side talks to it: over HTTP (`Http`), or in this process (`hub::Hub`,
/// in the server itself and in tests).
pub trait Transport: Send + Sync {
    fn snapshot(&self) -> Result<Snapshot, Failure>;
    /// The entries after `after`, whose entry has the id `last` (none: not known, not checked).
    fn log(&self, after: u64, last: Option<&str>, wait: Duration) -> Result<LogPage, Failure>;
    fn post(&self, posted: Posted) -> Result<Appended, Failure>;
    fn report(&self, device: &str, report: DeviceReport) -> Result<(), Failure>;
    fn devices(&self) -> Result<Vec<Device>, Failure>;
    fn request(&self, request: &Request) -> Result<Response, Failure>;
}

/// The server over HTTP(S): its address (`https://sync.example.com`, a path after it allowed) and token.
pub struct Http {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Http {
    pub fn new(url: &str, token: &str) -> Result<Http, String> {
        let base = url.trim().trim_end_matches('/').to_string();
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err("The server's address starts with https:// (or http://)".into());
        }
        if token.trim().is_empty() {
            return Err("Give the server's token".into());
        }
        // Long enough for a log request that waits.
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(MAX_WAIT + 15)).build();
        Ok(Http { base, token: token.trim().to_string(), agent })
    }

    fn call<T: for<'de> Deserialize<'de>>(&self, method: &str, path: &str, body: Option<String>) -> Result<T, Failure> {
        let request = self.agent.request(method, &format!("{}{path}", self.base)).set("Authorization", &format!("Bearer {}", self.token));
        let answer = match body {
            Some(body) => request.set("Content-Type", "application/json").send_string(&body),
            None => request.call(),
        };
        match answer {
            Ok(answer) if answer.status() == 204 => serde_json::from_str("null").map_err(|e| Failure::Other(e.to_string())),
            Ok(answer) => {
                let text = answer.into_string().map_err(|e| Failure::Other(format!("Cannot read the server's answer: {e}")))?;
                serde_json::from_str(&text).map_err(|e| Failure::Other(format!("The server's answer cannot be read: {e}")))
            }
            Err(ureq::Error::Status(401, _)) => Err(Failure::Unauthorized),
            Err(ureq::Error::Status(410, _)) => Err(Failure::Gone),
            Err(ureq::Error::Status(409, _)) => Err(Failure::Diverged),
            Err(ureq::Error::Status(code, answer)) => {
                let text = answer.into_string().unwrap_or_default();
                Err(Failure::Other(format!("The server answered {code}: {}", text.chars().take(200).collect::<String>())))
            }
            Err(e) => Err(Failure::Other(format!("The server cannot be reached: {e}"))),
        }
    }
}

fn json<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_string(value).ok()
}

impl Transport for Http {
    fn snapshot(&self) -> Result<Snapshot, Failure> {
        self.call("GET", "/api/snapshot", None)
    }

    fn log(&self, after: u64, last: Option<&str>, wait: Duration) -> Result<LogPage, Failure> {
        // An id is hex (`replica::new_id`): nothing in it needs escaping; anything else is not sent.
        let last = last.filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric())).map(|id| format!("&last={id}")).unwrap_or_default();
        self.call("GET", &format!("/api/log?after={after}{last}&wait={}", wait.as_secs().min(MAX_WAIT)), None)
    }

    fn post(&self, posted: Posted) -> Result<Appended, Failure> {
        self.call("POST", "/api/log", json(&posted))
    }

    fn report(&self, device: &str, report: DeviceReport) -> Result<(), Failure> {
        let _: Option<()> = self.call("PUT", &format!("/api/devices/{device}"), json(&report))?;
        Ok(())
    }

    fn devices(&self) -> Result<Vec<Device>, Failure> {
        self.call("GET", "/api/devices", None)
    }

    fn request(&self, request: &Request) -> Result<Response, Failure> {
        self.call("POST", "/api/request", json(request))
    }
}

/// Where syncing stands on this computer, for the window and the CLI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SyncStatus {
    pub connected: bool,
    /// Where it stands, in words (the window and the CLI say the same).
    pub summary: String,
    /// The server's address, while connected.
    pub url: String,
    /// The last entry of the log applied here.
    pub confirmed: u64,
    /// When the server last answered (milliseconds since the Unix epoch).
    pub last_contact: Option<u64>,
    /// Why the last attempt failed, if it did.
    pub error: Option<String>,
    /// Entries done here and not yet taken in by the server.
    pub outbox: usize,
    /// What waits here, in words.
    pub held: Vec<HeldView>,
    /// The computers that sync, as the server last said (this one too).
    pub devices: Vec<DeviceView>,
    /// This computer's id among them.
    pub device: String,
}

/// A computer that syncs, for people.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    /// Whether it is this computer.
    pub this: bool,
    /// When it was last heard from, in words.
    pub seen: String,
    /// The sessions running there, by name.
    pub running: Vec<String>,
    /// How many changes wait there.
    pub held: usize,
}

/// An entry that waits here, for people: what it is and why it waits.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct HeldView {
    pub what: String,
    pub why: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_and_a_token_are_needed() {
        assert!(Http::new("sync.example.com", "t").is_err());
        assert_eq!(Http::new("https://sync.example.com", "  ").err().as_deref(), Some("Give the server's token"));
        // Any length will do.
        assert!(Http::new("https://sync.example.com", "t").is_ok());
        let http = Http::new(" https://sync.example.com/base/ ", " 0123456789abcdef ").unwrap();
        assert_eq!((http.base.as_str(), http.token.as_str()), ("https://sync.example.com/base", "0123456789abcdef"));
    }

    #[test]
    fn no_server_is_an_answer_too() {
        // Nothing listens on port 9 of this computer: refused at once, no internet involved.
        let http = Http::new("http://127.0.0.1:9", "0123456789abcdef").unwrap();
        assert!(matches!(http.snapshot(), Err(Failure::Other(why)) if why.contains("cannot be reached")));
        assert!(matches!(http.devices(), Err(Failure::Other(_))));
        assert_eq!(Failure::Gone.words(), "The server no longer has what this computer missed");
        assert_eq!(Failure::Unauthorized.words(), "The server refused the token");
    }
}
