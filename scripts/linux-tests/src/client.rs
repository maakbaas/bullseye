//! Stand-in for the pipe client: no pipe, only a service in this process (`Remote::local`), which
//! is what the CLI's tests talk to, or a sync server (`Remote::server`, as in `client.rs`); without
//! one, every request fails.
use std::sync::Arc;

use crate::definitions::Library;
use crate::proto::{Overview, Request, Response};
use crate::sync_api::Transport;

pub type Local = Arc<dyn Fn(Request) -> Response + Send + Sync>;

#[derive(Clone, Default)]
pub struct Remote {
    local: Option<Local>,
    server: Option<Arc<dyn Transport>>,
}

impl Remote {
    pub fn for_cli() -> Remote {
        Remote::default()
    }

    pub fn local(handler: Local) -> Remote {
        Remote { local: Some(handler), server: None }
    }

    pub fn server(server: Arc<dyn Transport>) -> Remote {
        Remote { local: None, server: Some(server) }
    }

    fn send(&self, request: Request) -> Result<Response, String> {
        if let Some(server) = &self.server {
            return server.request(&request).map_err(|failure| failure.words());
        }
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

    /// Where syncing stands on this computer.
    pub fn sync_status(&self) -> Result<crate::sync_api::SyncStatus, String> {
        match self.send(Request::SyncStatus)? {
            Response::Sync(status) => Ok(*status),
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
