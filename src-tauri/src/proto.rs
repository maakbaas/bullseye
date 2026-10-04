//! What the app and the service say to each other over the named pipe (`pipe.rs`).
//!
//! One request and one answer per connection, each a 4-byte little-endian length followed by
//! that many bytes of JSON. The service treats every request as untrusted: any local user can
//! open the pipe and send whatever they like (`service::handle` checks it all again).

use std::io::{Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::block::Status;
use crate::state::Enforcers;

/// The pipe the service listens on.
pub const PIPE: &str = r"\\.\pipe\SiteBlocker";

/// Larger messages are refused, so a request cannot make the service allocate without limit.
const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
pub enum Request {
    /// What the UI shows: time left, entries, enforcers.
    Status,
    /// What the app enforces: asked every second.
    Policy,
    Start { filters: Vec<String>, duration: String, enforcers: Enforcers },
    Stop,
    Unlock { password: String },
    /// From the installer: end the service and remove it. Refused during a timed block.
    PrepareUninstall,
    /// From the installer: let go of the exe so it can be replaced. Refused during any block.
    PrepareUpdate,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Response {
    Done,
    Failed(String),
    /// A block is running that does not allow this (uninstall, update).
    Refused(String),
    Status(Status),
    Policy(Option<Policy>),
}

/// What the app has to enforce right now.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Policy {
    pub filters: Vec<String>,
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
        let request = Request::Start {
            filters: vec!["tweakers.net".into()],
            duration: "10m".into(),
            enforcers: Enforcers::default(),
        };
        let mut wire = Vec::new();
        write_message(&mut wire, &request).unwrap();
        assert_eq!(read_message::<Request>(&mut Cursor::new(wire)), Ok(request));

        let mut wire = Vec::new();
        write_message(&mut wire, &Request::Policy).unwrap();
        assert_eq!(read_message::<Request>(&mut Cursor::new(wire)), Ok(Request::Policy));
    }

    #[test]
    fn a_policy_survives_the_wire() {
        let response = Response::Policy(Some(Policy { filters: vec!["a.com".into()], enforcers: Enforcers::default() }));
        let mut wire = Vec::new();
        write_message(&mut wire, &response).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(response));
        let mut wire = Vec::new();
        write_message(&mut wire, &Response::Policy(None)).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(Response::Policy(None)));
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
