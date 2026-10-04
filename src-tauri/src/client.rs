//! The app's side of the service: asks the service what to show and what to enforce.
//!
//! The app holds no block of its own. `tab_closer.rs` asks `Remote::active` every second, which
//! has the same shape as `Blocklist::active` did, so the closing code did not have to change.
//! If the service cannot be reached (it is restarting, or being updated), the last answer keeps
//! being enforced: a gap in the service must not open the block.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::block::Status;
use crate::filters::FilterList;
use crate::pipe;
use crate::proto::{self, Policy, Request, Response};
use crate::state::Enforcers;
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
}

struct Cached {
    policy: Policy,
    list: Arc<FilterList>,
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

    /// The filters and enforcers to enforce right now, if any.
    pub fn active(&self) -> Option<(Arc<FilterList>, Enforcers)> {
        match call(&Request::Policy) {
            Ok(Response::Policy(policy)) => {
                self.failing.store(false, Ordering::Relaxed);
                self.apply(policy);
            }
            Ok(other) => self.fail(&format!("unexpected answer: {other:?}")),
            Err(e) => self.fail(&e),
        }
        self.cache.lock().unwrap().as_ref().map(|c| (c.list.clone(), c.policy.enforcers))
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
        match FilterList::parse(&policy.filters) {
            Ok(list) => *cache = Some(Cached { policy, list: Arc::new(list) }),
            // Cannot happen with a service of this version; keep what is being enforced.
            Err(_) => note("app", "the service sent a block list that cannot be read"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(filters: &[&str]) -> Policy {
        Policy { filters: filters.iter().map(|f| f.to_string()).collect(), enforcers: Enforcers::default() }
    }

    fn current(remote: &Remote) -> Option<(Arc<FilterList>, Enforcers)> {
        remote.cache.lock().unwrap().as_ref().map(|c| (c.list.clone(), c.policy.enforcers))
    }

    #[test]
    fn the_policy_follows_the_service_and_is_parsed_once() {
        let remote = Remote::new();
        assert!(current(&remote).is_none());

        remote.apply(Some(policy(&["tweakers.net"])));
        let (list, _) = current(&remote).unwrap();
        assert!(list.is_blocked("https://tweakers.net/nieuws"));

        // The same policy again keeps the same parsed list.
        remote.apply(Some(policy(&["tweakers.net"])));
        assert!(Arc::ptr_eq(&list, &current(&remote).unwrap().0));

        // A different one replaces it; none at all ends the block.
        remote.apply(Some(policy(&["b.com"])));
        assert!(current(&remote).unwrap().0.is_blocked("b.com/x"));
        remote.apply(None);
        assert!(current(&remote).is_none());
    }
}
