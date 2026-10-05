//! What the app and the service say to each other over the named pipe (`pipe.rs`).
//!
//! One request and one answer per connection, each a 4-byte little-endian length followed by
//! that many bytes of JSON. The service treats every request as untrusted: any local user can
//! open the pipe and send whatever they like (`service::handle` checks it all again).

use std::io::{Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::block::Running;
use crate::definitions::{BlockList, Definition, Kind, Library, NamedSchedule};
use crate::state::Enforcers;

/// The pipe the service listens on.
pub const PIPE: &str = r"\\.\pipe\bullseye";

/// Larger messages are refused, so a request cannot make the service allocate without limit.
/// Room for the status of everything defined (`definitions::MAX_ITEMS` of each) with long lists.
const MAX_MESSAGE: usize = 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
// A saved block is larger than the rest (its weekly schedule); a request lives only as long as it is
// answered.
#[allow(clippy::large_enum_variant)]
pub enum Request {
    /// What the UI shows: the block lists, schedules and blocks, and which blocks run.
    Status,
    /// What the app enforces: asked every second, from the thread that closes tabs, which is how
    /// the service knows the closing is going on (`watchdog::AppSlot`).
    Policy,
    /// The app, when it starts: asks the service to take it in as the app. Refused if another app
    /// is running or this is not the installed exe, and then the app ends itself.
    Register,
    /// Adds a block, or replaces the one with its id. Refused while that block runs.
    SaveBlock { block: Definition },
    /// Removes a block. Refused while it runs.
    DeleteBlock { id: String },
    /// Adds a block list, or replaces the one with its id. Refused while a running block uses it.
    SaveList { list: BlockList },
    /// Removes a block list, and takes it out of the blocks that use it. Refused while a running
    /// block uses it.
    DeleteList { id: String },
    /// Adds a schedule, or replaces the one with its id. Refused while a running block uses it.
    SaveSchedule { schedule: NamedSchedule },
    /// Removes a schedule, and takes it out of the blocks that use it. Refused while a running
    /// block uses it.
    DeleteSchedule { id: String },
    /// Copies a block, block list or schedule as `new_id`, named after it with a number.
    Duplicate { kind: Kind, id: String, new_id: String },
    /// Replaces every block list, schedule and block with these, from a settings file (Import).
    /// Refused while a block runs.
    Import { library: Library },
    /// Starts the block defined as `id`, as it is defined now.
    Start { id: String },
    /// Stops the running block `id`, unless it is locked right now. `password`: the password of its
    /// password lock, which stops that one.
    Stop { id: String, password: Option<String> },
    /// Ends every block, whatever its time.
    Unlock { password: String },
    /// From the installer: end the service and remove it. Refused while a block is locked.
    PrepareUninstall,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Response {
    Done,
    Failed(String),
    /// A block is running that does not allow this (uninstall, update).
    Refused(String),
    Status(Overview),
    /// One per running block; none when nothing runs.
    Policy(Vec<Policy>),
}

/// What the UI shows.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Overview {
    /// Everything the user has defined, each kind in the order it was made (`definitions.rs`).
    pub lists: Vec<BlockList>,
    pub schedules: Vec<NamedSchedule>,
    pub blocks: Vec<Definition>,
    /// The blocks that run, as they were started (`Running::id` is the definition's).
    pub running: Vec<Running>,
    /// Whether the time comes from the internet yet; until then, time with the service not
    /// running does not count.
    pub clock_synced: bool,
}

impl Overview {
    /// What the user has defined, without what runs.
    pub fn library(self) -> Library {
        Library { lists: self.lists, schedules: self.schedules, blocks: self.blocks }
    }
}

/// What the app has to enforce for one running block.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Policy {
    pub filters: Vec<String>,
    pub allowed: Vec<String>,
    pub programs: Vec<String>,
    pub enforcers: Enforcers,
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
        use crate::definitions::tests::{block, block_list, weekly};
        let mut list = block_list("l", &["tweakers.net"]);
        list.allowed = vec!["gathering.tweakers.net".into()];
        list.programs = vec!["steam*".into()];
        let requests = [
            Request::SaveList { list },
            Request::SaveSchedule { schedule: weekly("s", "mon-fri 8-17") },
            Request::SaveBlock { block: block("a", &["l"]) },
            Request::Start { id: "a".into() },
            Request::DeleteBlock { id: "a".into() },
            Request::DeleteList { id: "l".into() },
            Request::DeleteSchedule { id: "s".into() },
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
            filters: vec!["a.com".into()],
            allowed: vec!["b.com".into()],
            programs: vec!["game.exe".into()],
            enforcers: Enforcers::default(),
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
}
