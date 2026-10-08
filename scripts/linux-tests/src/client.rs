//! Stand-in for the pipe client: the CLI's tests never send anything.
use crate::definitions::Library;
use crate::proto::{Overview, Request};
pub struct Remote;
impl Remote {
    pub fn for_cli() -> Remote { Remote }
    pub fn status(&self) -> Result<Overview, String> { Err("no service in tests".into()) }
    pub fn command(&self, _request: Request) -> Result<(), String> { Err("no service in tests".into()) }
    pub fn library(&self) -> Result<Library, String> { Err("no service in tests".into()) }
}
