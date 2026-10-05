//! The app's side of the service: asks the service what to show and what to enforce.
//!
//! The app holds no block of its own. `tab_closer.rs` asks `Remote::active` every second, which
//! has the same shape as `Blocklist::active`: every running block's part.
//! If the service cannot be reached (it is restarting, or being updated), the last answer keeps
//! being enforced: a gap in the service must not open the block.
//!
//! In a debug build (`npm run dev`) the service's logic can run inside the app instead
//! (`Remote::local`): no service, no pipe, no installer, and nothing that has to stay alive. Release
//! builds do not contain that.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::block::{Active, Enforced};
use crate::lists::Lists;
use crate::pipe;
use crate::proto::{self, Overview, Policy, Request, Response};
use crate::registration;
use crate::sync::LockExt;
use crate::watchdog::note;

/// The service's logic called directly, in place of the pipe (`service::local_handler`).
pub type Local = Arc<dyn Fn(Request) -> Response + Send + Sync>;

/// One request and its answer.
pub fn call(request: &Request) -> Result<Response, String> {
    let mut pipe = pipe::connect()?;
    proto::write_message(&mut pipe, request)?;
    proto::read_message(&mut pipe)
}

#[derive(Clone, Default)]
pub struct Remote {
    /// The last policy the service gave, parsed once, not every second.
    cache: Arc<Mutex<Option<Cached>>>,
    /// Whether the last call failed, so a failing service is logged once rather than every second.
    failing: Arc<AtomicBool>,
    /// When the service was last asked to start, so that is not tried every second.
    last_start: Arc<Mutex<Option<Instant>>>,
    /// Set in development: the service's logic, in this process.
    local: Option<Local>,
    /// For the CLI: a request that finds the service registered but not running starts it and
    /// waits for it, rather than failing (the app tries again a second later anyway).
    patient: bool,
}

/// The least time between two attempts to start the service.
const START_EVERY: Duration = Duration::from_secs(5);

struct Cached {
    policies: Vec<Policy>,
    active: Active,
}

impl Remote {
    pub fn new() -> Remote {
        Remote::default()
    }

    /// The CLI's: the same requests and answers as the app's, but it waits for a service it had to
    /// start.
    pub fn for_cli() -> Remote {
        Remote { patient: true, ..Remote::default() }
    }

    /// A remote that is its own service: for `npm run dev`, and for tests.
    #[cfg(any(debug_assertions, test))]
    pub fn local(handler: Local) -> Remote {
        Remote { local: Some(handler), ..Remote::default() }
    }

    /// Whether this is its own service, so there is nobody to register with or to start.
    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    /// One request, to the service over the pipe or to the local one.
    fn send(&self, request: Request) -> Result<Response, String> {
        let first = match &self.local {
            Some(local) => return Ok(local(request)),
            None => call(&request),
        };
        match first {
            Err(e) if self.patient => self.after_starting_the_service(&request).ok_or(e),
            answer => answer,
        }
    }

    /// The answer to `request` once the service, registered but not running, has been started;
    /// `None` if it was running already (so something else is wrong) or did not come up.
    fn after_starting_the_service(&self, request: &Request) -> Option<Response> {
        if !matches!(registration::start_registered(), Ok(true)) {
            return None;
        }
        (0..40).find_map(|_| {
            std::thread::sleep(Duration::from_millis(250));
            call(request).ok()
        })
    }

    /// A command that answers yes or no.
    pub fn command(&self, request: Request) -> Result<(), String> {
        match self.send(request)? {
            Response::Done => Ok(()),
            Response::Failed(why) | Response::Refused(why) => Err(why),
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

    /// What to enforce right now, if anything.
    pub fn active(&self) -> Option<Active> {
        match self.send(Request::Policy) {
            Ok(Response::Policy(policy)) => {
                self.failing.store(false, Ordering::Relaxed);
                self.apply(policy);
            }
            Ok(other) => self.fail(&format!("unexpected answer: {other:?}")),
            Err(e) => {
                self.fail(&e);
                self.revive_service();
            }
        }
        self.cache.locked().as_ref().map(|c| c.active.clone())
    }

    /// Asks Windows to start the service if it is registered but not running (it only stays down
    /// if something ended it with exit code 0, or an update did not finish). Starting is the only
    /// thing the app may do to the service: the access list gives interactive users no more
    /// (`registration::SDDL`). A service that is not registered at all, as in development, just fails
    /// quietly.
    fn revive_service(&self) {
        if self.is_local() {
            return;
        }
        let mut last = self.last_start.locked();
        if last.is_some_and(|at| at.elapsed() < START_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        if let Ok(true) = registration::start_registered() {
            note("app", "the service was not running; started it");
        }
    }

    fn fail(&self, why: &str) {
        if !self.failing.swap(true, Ordering::Relaxed) {
            note("app", &format!("the service did not answer, going on with the last block: {why}"));
        }
    }

    /// Takes in what the service said; the lists are only parsed again when they changed.
    fn apply(&self, policies: Vec<Policy>) {
        let mut cache = self.cache.locked();
        if policies.is_empty() {
            *cache = None;
            return;
        }
        if cache.as_ref().is_some_and(|c| c.policies == policies) {
            return;
        }
        match policies.iter().map(parse).collect::<Option<Vec<Enforced>>>() {
            Some(blocks) => *cache = Some(Cached { policies, active: Active { blocks } }),
            // Cannot happen with a service of this version; keep what is being enforced.
            None => note("app", "the service sent a block list that cannot be read"),
        }
    }
}

fn parse(policy: &Policy) -> Option<Enforced> {
    Some(Enforced { lists: Arc::new(Lists::parse(&policy.entries).ok()?), enforcers: policy.enforcers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::Entries;
    use crate::service;
    use crate::state::Enforcers;

    fn policy(filters: &[&str]) -> Policy {
        Policy {
            entries: Entries { filters: filters.iter().map(|f| f.to_string()).collect(), ..Entries::default() },
            enforcers: Enforcers::default(),
        }
    }

    fn current(remote: &Remote) -> Option<Active> {
        remote.cache.locked().as_ref().map(|c| c.active.clone())
    }

    #[test]
    fn the_policy_follows_the_service_and_is_parsed_once() {
        let remote = Remote::new();
        assert!(current(&remote).is_none());

        remote.apply(vec![policy(&["tweakers.net"])]);
        let list = current(&remote).unwrap().blocks[0].lists.clone();
        assert!(list.sites.is_blocked("https://tweakers.net/nieuws"));

        // The same policy again keeps the same parsed list.
        remote.apply(vec![policy(&["tweakers.net"])]);
        assert!(Arc::ptr_eq(&list, &current(&remote).unwrap().blocks[0].lists));

        // A different one replaces it, a second block comes along; none at all ends the blocks.
        remote.apply(vec![policy(&["b.com"]), policy(&["c.com"])]);
        let active = current(&remote).unwrap();
        assert!(active.blocks_page("https://b.com/x", false) && active.blocks_page("https://c.com/", false));
        assert!(!active.blocks_page("https://tweakers.net/", false));
        remote.apply(Vec::new());
        assert!(current(&remote).is_none());
    }

    #[test]
    fn the_programs_come_along_and_are_parsed_with_the_policy() {
        let remote = Remote::new();
        let with_programs =
            Policy { entries: Entries { programs: vec!["Steam*".into()], ..Entries::default() }, enforcers: Enforcers::default() };
        remote.apply(vec![with_programs]);
        let active = current(&remote).unwrap();
        assert!(active.blocks_program("steamwebhelper.exe") && !active.blocks_program("chrome.exe"));
        // A list that cannot be read leaves what is being enforced as it was.
        let wrong = Policy { entries: Entries { programs: vec!["*".into()], ..Entries::default() }, enforcers: Enforcers::default() };
        remote.apply(vec![wrong]);
        assert!(current(&remote).unwrap().blocks_program("steamwebhelper.exe"));
    }

    #[test]
    fn a_local_remote_is_a_service_of_its_own_without_a_pipe() {
        use crate::block::Blocklist;
        use crate::definitions::{tests::block, tests::block_list, Definition, Definitions};
        use crate::lock::Lock;
        use crate::state::Store;
        let dir = std::env::temp_dir().join(format!("client-{}-local", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocklist = Blocklist::with_store(Store::new(dir.join("state.json")));
        let remote = Remote::local(service::local_handler(blocklist, Definitions::at(Some(dir.join("blocks.json")))));
        assert!(remote.is_local());
        assert!(remote.active().is_none());

        remote.command(Request::SaveList { list: block_list("a", &["a.com"]) }).unwrap();
        let block = Definition { lock: Lock::Duration { duration: "1h".into() }, ..block("a", &["a"]) };
        remote.command(Request::SaveBlock { block }).unwrap();
        remote.command(Request::Start { id: "a".into() }).unwrap();
        assert!(remote.active().unwrap().blocks_page("https://a.com/x", false));
        assert!(remote.status().unwrap().running[0].remaining.is_some());
        // The same rules as with the real service: a timed block cannot be stopped early, unlocking
        // (free until a delay is set) ends it.
        assert!(remote.command(Request::Stop { id: "a".into(), password: None, text: None }).is_err());
        remote.command(Request::Unlock).unwrap();
        // The app stays, with nothing to enforce.
        assert!(remote.active().is_none());
    }
}
