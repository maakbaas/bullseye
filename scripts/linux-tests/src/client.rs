//! Stand-in for the pipe client: no pipe, only a service in this process (`Remote::local`), which
//! is what the CLI's tests talk to; without one, every request fails.
use std::sync::Arc;

use crate::definitions::Library;
use crate::proto::{Overview, Request, Response};

pub type Local = Arc<dyn Fn(Request) -> Response + Send + Sync>;

#[derive(Clone, Default)]
pub struct Remote {
    local: Option<Local>,
}

impl Remote {
    pub fn for_cli() -> Remote {
        Remote::default()
    }

    pub fn local(handler: Local) -> Remote {
        Remote { local: Some(handler) }
    }

    fn send(&self, request: Request) -> Result<Response, String> {
        self.local.as_ref().map(|local| local(request)).ok_or_else(|| "no service in tests".to_string())
    }

    pub fn command(&self, request: Request) -> Result<(), String> {
        match self.send(request)? {
            Response::Done => Ok(()),
            Response::Failed(why) | Response::Refused(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    pub fn library(&self) -> Result<Library, String> {
        match self.send(Request::Export)? {
            Response::Library(library) => Ok(library),
            Response::Failed(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    pub fn status(&self) -> Result<Overview, String> {
        match self.send(Request::Status)? {
            Response::Status(status) => Ok(status),
            Response::Failed(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }
}
