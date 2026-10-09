//! What the app and the service say to each other over the named pipe (`pipe.rs`).
//!
//! One request and one answer per connection, each a 4-byte little-endian length followed by
//! that many bytes of JSON. The service treats every request as untrusted: any local user can
//! open the pipe and send whatever they like (`service::handle` checks it all again).

use std::io::{Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::allowance::Terms;
use crate::definitions::{Condition, Definition, Kind, Library, Target};
use crate::lists::Entries;
use crate::lock::Lock;
use crate::session::Running;
use crate::state::Actions;
use crate::status::Status;

/// The pipe the service listens on.
pub const PIPE: &str = r"\\.\pipe\bullseye";

/// Larger messages are refused, so a request cannot make the service allocate without limit.
/// Room for the status of everything defined (`definitions::MAX_ITEMS` of each) with long lists.
const MAX_MESSAGE: usize = 1024 * 1024;

/// A change to check (`Request::Check`): the same rules as making it (`handler::check_stricter`).
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "what", rename_all = "snake_case")]
pub enum Check {
    /// The session's allowance set to this kind (`none`, `budget`, `pomodoro`, `delayed`).
    AllowanceKind { id: String, kind: String },
    /// The session's allowance set to these terms (none: taken away).
    Allowance { id: String, allowance: Option<Terms> },
    /// The session's lock set to this one (a timed lock or a delay, longer).
    Lock { id: String, lock: Lock },
    /// A target saved like this.
    Target { target: Target },
    /// A condition saved like this.
    Condition { condition: Condition },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
// A saved condition is larger than the rest (its weekly schedule); a request lives only as long as it is
// answered.
#[allow(clippy::large_enum_variant)]
pub enum Request {
    /// What the UI shows: the targets, conditions and sessions, and which sessions run.
    Status,
    /// What the app enforces: asked every second, from the thread that closes tabs, which is how
    /// the service knows the closing is going on (`watchdog::AppSlot`).
    Policy,
    /// The app, before it asks for the policy: what it has seen in use since, of what an allowance
    /// lets through (`Policy::free`), which the service counts against it (`Sessions::charge`).
    Report { used: Vec<Used> },
    /// The app, when it starts: asks the service to take it in as the app. Refused if another app
    /// is running or this is not the installed exe, and then the app ends itself.
    Register,
    /// Adds a session, or replaces the one with its id. Refused while that session runs.
    SaveSession { session: Definition },
    /// Removes a session. Refused while it runs.
    DeleteSession { id: String },
    /// Adds a target, or replaces the one with its id. While a locked session uses it, only to be
    /// stricter (`Target::tightened_by`).
    SaveTarget { target: Target },
    /// Removes a target, and takes it out of the sessions that use it. Refused while a running
    /// session uses it.
    DeleteTarget { id: String },
    /// Adds a condition, or replaces the one with its id. While a locked session uses it, only to be
    /// stricter (`Rule::tightened_by`).
    SaveCondition { condition: Condition },
    /// Removes a condition, and takes it out of the sessions that use it. Refused while a running
    /// session uses it.
    DeleteCondition { id: String },
    /// Copies a session, target or condition as `new_id`, named after it with a number.
    Duplicate { kind: Kind, id: String, new_id: String },
    /// Replaces every target, condition and session with these, from a settings file (Import).
    /// Refused while a session is locked; sessions that run unlocked end with it.
    Import { library: Library },
    /// Starts the session defined as `id`, as it is defined now.
    Start { id: String },
    /// Gives the running session `id` this lock, as if it started with it now, and saves it as the
    /// session's lock. Only while the session is not locked.
    Lock { id: String, lock: Lock },
    /// Whether a change to what a locked session holds would be accepted, asked before the window
    /// asks the person to confirm it: answered `Done`, or why not, and nothing is changed. A change that
    /// nothing locked holds is accepted.
    Check { check: Check },
    /// Stops the running session `id`, unless it is locked right now. `password`: the password of its
    /// password lock, which stops that one; `text`: the text of its text lock, likewise.
    Stop {
        id: String,
        password: Option<String>,
        #[serde(default)]
        text: Option<String>,
    },
    /// Starts the wait before the running session `id`, locked with a delay, can be stopped.
    AskToStop { id: String },
    /// Ends that wait: the session is locked again.
    AbortStop { id: String },
    /// Ends every session, whatever its time, once unlocking is ready: at once with no delay,
    /// otherwise when the countdown of `AskForUnlock` is over (`unlock::UnlockDelay`).
    Unlock,
    /// As `Unlock`, with an answer: the text or password of the other way to unlock
    /// (`unlock::Way`), which lets go without the countdown.
    UnlockWith {
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        text: Option<String>,
    },
    /// Sets the other way to unlock besides the delay, as a lock method (never the delay), or removes
    /// it (`None`). Setting one needs unlocking to be ready or no session to be locked.
    SetUnlockWay { way: Option<Lock> },
    /// Everything defined, password hashes included, for Export (the status has them left out).
    Export,
    /// Starts the countdown to unlocking. It can be aborted.
    AskForUnlock,
    /// Stops the countdown, or ends the readiness: back to locked.
    AbortUnlock,
    /// Sets how long unlocking takes, as typed (`2h 30m`, `0`; at most 24 hours). A longer delay is
    /// free; a shorter one needs unlocking to be ready, or no session to be locked.
    SetUnlockDelay { delay: String },
    /// From the installer: end the service and remove it. Refused while a session is locked.
    PrepareUninstall,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
// The status is the one large answer, built and sent once a second at most.
#[allow(clippy::large_enum_variant)]
pub enum Response {
    Done,
    Failed(String),
    /// A session is running that does not allow this (uninstall, update).
    Refused(String),
    Status(Overview),
    /// The whole library, for Export.
    Library(Library),
    /// One per running session; none when nothing runs.
    Policy(Vec<Policy>),
}

/// Where unlocking stands: how long it takes, and whether it is being waited for.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Default)]
pub struct UnlockStatus {
    /// How long unlocking takes, in seconds (0: no delay, so it is ready at any time), and in words.
    pub delay: u64,
    pub delay_words: String,
    /// Seconds until unlocking is ready: `None`: not asked for; `Some(0)`: ready now.
    pub remaining: Option<u64>,
    pub remaining_words: Option<String>,
    /// The other way to unlock, as chosen (a password without its hash; a text as it is: it is typed).
    #[serde(default)]
    pub way: Option<Lock>,
    /// Whether that way lets go now without an answer (a time that has come, a restart, a condition).
    #[serde(default)]
    pub way_open: bool,
    /// Seconds until it does, where that is known, and in words.
    #[serde(default)]
    pub way_left: Option<u64>,
    #[serde(default)]
    pub way_left_words: Option<String>,
}

/// What the UI shows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Overview {
    /// Everything the user has defined, each kind in the order it was made (`definitions.rs`).
    pub targets: Vec<Target>,
    pub conditions: Vec<Condition>,
    pub sessions: Vec<Definition>,
    /// The sessions that run, as they were started (`Running::id` is the definition's).
    pub running: Vec<Running>,
    /// Where unlocking stands (a clean install has no delay: unlock is free).
    pub unlock: UnlockStatus,
    /// Whether the time comes from the internet yet; until then, time with the service not
    /// running does not count.
    pub clock_synced: bool,
    /// What each session's Condition, Allowance and Lock rows say now (`status.rs`), for its tile.
    #[serde(default)]
    pub status: Vec<Status>,
}

impl Overview {
    /// What the user has defined, without what runs.
    #[cfg(test)]
    pub fn library(self) -> Library {
        Library { targets: self.targets, conditions: self.conditions, sessions: self.sessions }
    }
}

/// What the app has to enforce for one running session.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Policy {
    #[serde(flatten)]
    pub entries: Entries,
    pub actions: Actions,
    /// What the session's allowance lets through now: not enforced, but to be reported when it is in use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free: Vec<FreeEntries>,
}

impl Policy {
    /// A policy that lets nothing through.
    #[cfg(test)]
    pub fn new(entries: Entries, actions: Actions) -> Policy {
        Policy { entries, actions, free: Vec::new() }
    }
}

/// Something an allowance lets through for now (`session::Free`): the entries that are, without the
/// always-allowed sites, which the policy's own entries have.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct FreeEntries {
    pub session: String,
    pub key: String,
    #[serde(flatten)]
    pub entries: Entries,
}

/// One thing the app saw in use that an allowance counts (`Request::Report`).
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Used {
    pub session: String,
    pub key: String,
}

pub fn write_message<T: Serialize>(to: &mut impl Write, message: &T) -> Result<(), String> {
    let body = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    if body.len() > MAX_MESSAGE {
        return Err("message too large".into());
    }
    // One write, so the length and the body never arrive apart.
    let mut frame = (body.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&body);
    to.write_all(&frame).and_then(|_| to.flush()).map_err(|e| e.to_string())
}

pub fn read_message<T: DeserializeOwned>(from: &mut impl Read) -> Result<T, String> {
    let mut length = [0u8; 4];
    from.read_exact(&mut length).map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Err("message too large".into());
    }
    let mut body = vec![0u8; length];
    from.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_request_survives_the_wire() {
        use crate::definitions::tests::{session, target, weekly};
        let mut list = target("l", &["tweakers.net"]);
        list.entries.allowed = vec!["gathering.tweakers.net".into()];
        list.entries.programs = vec!["steam*".into()];
        list.entries.store_apps = vec!["Microsoft.XboxApp_8wekyb3d8bbwe".into()];
        list.entries.titles = vec!["*YouTube*".into()];
        let requests = [
            Request::SaveTarget { target: list },
            Request::SaveCondition { condition: weekly("s", "mon-fri 8-17") },
            Request::SaveSession { session: session("a", &["l"]) },
            Request::Start { id: "a".into() },
            Request::Lock { id: "a".into(), lock: Lock::timed_for("2h") },
            Request::Check { check: Check::AllowanceKind { id: "a".into(), kind: "budget".into() } },
            Request::Check { check: Check::Lock { id: "a".into(), lock: Lock::timed_for("2h") } },
            Request::DeleteSession { id: "a".into() },
            Request::DeleteTarget { id: "l".into() },
            Request::DeleteCondition { id: "s".into() },
            Request::Report { used: vec![Used { session: "a".into(), key: "site:d.com".into() }] },
        ];
        for request in requests {
            let mut wire = Vec::new();
            write_message(&mut wire, &request).unwrap();
            assert_eq!(read_message::<Request>(&mut Cursor::new(wire)), Ok(request));
        }

        let mut wire = Vec::new();
        write_message(&mut wire, &Request::Policy).unwrap();
        assert_eq!(read_message::<Request>(&mut Cursor::new(wire)), Ok(Request::Policy));
    }

    #[test]
    fn a_policy_survives_the_wire() {
        let response = Response::Policy(vec![Policy {
            entries: Entries {
                filters: vec!["a.com".into()],
                allowed: vec!["b.com".into()],
                programs: vec!["game.exe".into()],
                store_apps: vec!["A.B_1".into()],
                titles: vec!["*c*".into()],
                allowed_titles: vec!["*cd*".into()],
            },
            actions: Actions::default(),
            free: vec![FreeEntries {
                session: "a".into(),
                key: "site:d.com".into(),
                entries: Entries { filters: vec!["d.com".into()], ..Entries::default() },
            }],
        }]);
        let mut wire = Vec::new();
        write_message(&mut wire, &response).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(response));
        let mut wire = Vec::new();
        write_message(&mut wire, &Response::Policy(Vec::new())).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(Response::Policy(Vec::new())));
    }

    #[test]
    fn oversized_and_broken_messages_are_refused() {
        // A length far beyond the limit is refused before anything is allocated.
        let huge = u32::MAX.to_le_bytes().to_vec();
        assert!(read_message::<Request>(&mut Cursor::new(huge)).is_err());
        // Cut short, and not JSON.
        assert!(read_message::<Request>(&mut Cursor::new(vec![9, 0, 0, 0, b'{'])).is_err());
        let mut wire = 5u32.to_le_bytes().to_vec();
        wire.extend_from_slice(b"nope!");
        assert!(read_message::<Request>(&mut Cursor::new(wire)).is_err());
        // An unknown command is not a request.
        let body = br#"{"cmd":"format_c"}"#;
        let mut wire = (body.len() as u32).to_le_bytes().to_vec();
        wire.extend_from_slice(body);
        assert!(read_message::<Request>(&mut Cursor::new(wire)).is_err());
    }

    #[test]
    fn a_message_over_a_megabyte_is_neither_sent_nor_read() {
        let huge = "x".repeat(MAX_MESSAGE + 1);
        let mut wire = Vec::new();
        assert_eq!(write_message(&mut wire, &huge), Err("message too large".into()));
        assert!(wire.is_empty(), "nothing is written");
        let claimed = ((MAX_MESSAGE + 1) as u32).to_le_bytes().to_vec();
        assert_eq!(read_message::<String>(&mut Cursor::new(claimed)), Err("message too large".into()));
        // A message cut short is an error, not a wait.
        assert!(read_message::<String>(&mut Cursor::new(vec![9, 0, 0, 0, b'"'])).is_err());
    }
}
