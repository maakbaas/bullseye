//! The app's side of the service: asks the service what to show and what to enforce.
//!
//! The app holds no block of its own. `tab_closer.rs` asks `Remote::active` every second, which
//! has the same shape as `Blocklist::active` did, so the closing code did not have to change.
//! If the service cannot be reached (it is restarting, or being updated), the last answer keeps
//! being enforced: a gap in the service must not open the block.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::block::{Active, Status};
use crate::filters::FilterList;
use crate::pipe;
use crate::programs::ProgramList;
use crate::proto::{self, Policy, Request, Response};
use crate::service;
use crate::watchdog::note;

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
}

/// The least time between two attempts to start the service.
const START_EVERY: Duration = Duration::from_secs(5);

struct Cached {
    policy: Policy,
    active: Active,
}

impl Remote {
    pub fn new() -> Remote {
        Remote::default()
    }

    /// A command that answers yes or no.
    pub fn command(&self, request: Request) -> Result<(), String> {
        match call(&request)? {
            Response::Done => Ok(()),
            Response::Failed(why) | Response::Refused(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    pub fn status(&self) -> Result<Status, String> {
        match call(&Request::Status)? {
            Response::Status(status) => Ok(status),
            Response::Failed(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    /// What to enforce right now, if anything.
    pub fn active(&self) -> Option<Active> {
        match call(&Request::Policy) {
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
        self.cache.lock().unwrap().as_ref().map(|c| c.active.clone())
    }

    /// Asks Windows to start the service if it is registered but not running (it only stays down
    /// if something ended it with exit code 0, or an update did not finish). Starting is the only
    /// thing the app may do to the service: the access list gives interactive users no more
    /// (`service::SDDL`). A service that is not registered at all, as in development, just fails
    /// quietly.
    fn revive_service(&self) {
        let mut last = self.last_start.lock().unwrap();
        if last.is_some_and(|at| at.elapsed() < START_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        if let Ok(true) = service::start_registered() {
            note("app", "the service was not running; started it");
        }
    }

    fn fail(&self, why: &str) {
        if !self.failing.swap(true, Ordering::Relaxed) {
            note("app", &format!("the service did not answer, going on with the last block: {why}"));
        }
    }

    /// Takes in what the service said; the list is only parsed again when it changed.
    fn apply(&self, policy: Option<Policy>) {
        let mut cache = self.cache.lock().unwrap();
        let Some(policy) = policy else {
            *cache = None;
            return;
        };
        if cache.as_ref().is_some_and(|c| c.policy == policy) {
            return;
        }
        match (FilterList::parse(&policy.filters), FilterList::parse(&policy.allowed), ProgramList::parse(&policy.programs)) {
            (Ok(filters), Ok(allowed), Ok(programs)) => {
                let active = Active {
                    filters: Arc::new(filters),
                    allowed: Arc::new(allowed),
                    programs: Arc::new(programs),
                    enforcers: policy.enforcers,
                };
                *cache = Some(Cached { policy, active });
            }
            // Cannot happen with a service of this version; keep what is being enforced.
            _ => note("app", "the service sent a block list that cannot be read"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Enforcers;

    fn policy(filters: &[&str]) -> Policy {
        Policy {
            filters: filters.iter().map(|f| f.to_string()).collect(),
            allowed: Vec::new(),
            programs: Vec::new(),
            enforcers: Enforcers::default(),
        }
    }

    fn current(remote: &Remote) -> Option<Active> {
        remote.cache.lock().unwrap().as_ref().map(|c| c.active.clone())
    }

    #[test]
    fn the_policy_follows_the_service_and_is_parsed_once() {
        let remote = Remote::new();
        assert!(current(&remote).is_none());

        remote.apply(Some(policy(&["tweakers.net"])));
        let list = current(&remote).unwrap().filters;
        assert!(list.is_blocked("https://tweakers.net/nieuws"));

        // The same policy again keeps the same parsed list.
        remote.apply(Some(policy(&["tweakers.net"])));
        assert!(Arc::ptr_eq(&list, &current(&remote).unwrap().filters));

        // A different one replaces it; none at all ends the block.
        remote.apply(Some(policy(&["b.com"])));
        assert!(current(&remote).unwrap().filters.is_blocked("b.com/x"));
        remote.apply(None);
        assert!(current(&remote).is_none());
    }

    #[test]
    fn the_programs_come_along_and_are_parsed_with_the_policy() {
        let remote = Remote::new();
        let with_programs = Policy { filters: vec![], allowed: vec![], programs: vec!["Steam*".into()], enforcers: Enforcers::default() };
        remote.apply(Some(with_programs));
        let active = current(&remote).unwrap();
        assert!(active.programs.is_blocked("steamwebhelper.exe") && !active.programs.is_blocked("chrome.exe"));
        // A list that cannot be read leaves what is being enforced as it was.
        let wrong = Policy { filters: vec![], allowed: vec![], programs: vec!["*".into()], enforcers: Enforcers::default() };
        remote.apply(Some(wrong));
        assert!(current(&remote).unwrap().programs.is_blocked("steamwebhelper.exe"));
    }
}
