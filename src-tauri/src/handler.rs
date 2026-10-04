//! What the service does with each request from the app and the installer. Everything in a request
//! is checked here again: it comes from any local user's program, not necessarily ours. The same
//! `handle` answers over the pipe (`service.rs`) and, in a debug build, inside the app
//! (`service::local_handler`), so the rules cannot differ between the two.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::block::{parse_duration, Blocklist, Lists};
use crate::filters::FilterList;
use crate::programs::ProgramList;
use crate::proto::{Policy, Request, Response};
use crate::state::Enforcers;
use crate::sync::LockExt;
use crate::unlock;
use crate::watchdog::AppSlot;

/// What the installer asked the service to end for.
pub enum Shutdown {
    /// Let go of the exe so it can be replaced; the installer starts the service again.
    Update,
    /// Remove the service and everything it keeps.
    Remove,
}

/// What `handle` works with.
pub struct Context {
    blocklist: Blocklist,
    lockout: Mutex<Lockout>,
    /// Which process is the app (`watchdog.rs`).
    slot: Arc<AppSlot>,
}

impl Context {
    pub fn new(blocklist: Blocklist, slot: Arc<AppSlot>) -> Context {
        Context { blocklist, lockout: Mutex::default(), slot }
    }
}

/// Wrong unlock passwords in a row, and until when the next guess is refused.
#[derive(Default)]
struct Lockout {
    failures: u32,
    until: Option<Instant>,
}

/// Answers one request from the process `caller`, and says whether the service should end
/// afterwards. Everything in the request is checked here again: it comes from any local user's
/// program, not necessarily ours.
pub fn handle(context: &Context, caller: u32, request: Request) -> (Response, Option<Shutdown>) {
    let blocklist = &context.blocklist;
    let done = |result: Result<(), String>| match result {
        Ok(()) => Response::Done,
        Err(why) => Response::Failed(why),
    };
    match request {
        Request::Status => (Response::Status(blocklist.status()), None),
        // The app asks to be taken in; anyone else is refused, and the app then ends itself.
        Request::Register => match context.slot.register(caller) {
            Ok(()) => (Response::Done, None),
            Err(why) => (Response::Refused(why), None),
        },
        Request::Policy => {
            // This request comes from the thread that closes tabs: hearing it is how the service
            // knows that is going on. It counts only from the process taken in as the app.
            context.slot.heard(caller);
            let policy = blocklist.active().map(|active| Policy {
                filters: active.filters.entries(),
                allowed: active.allowed.entries(),
                programs: active.programs.entries(),
                enforcers: active.enforcers,
            });
            (Response::Policy(policy), None)
        }
        Request::Start { filters, allowed, programs, duration, enforcers } => {
            (done(start(blocklist, filters, allowed, programs, &duration, enforcers)), None)
        }
        Request::Stop => (done(blocklist.stop().map_err(|_| "A timed block cannot be stopped early".to_string())), None),
        Request::Unlock { password } => (done(unlock(context, &password)), None),
        // Refused during a timed block, judged by internet time, like the block itself.
        Request::PrepareUninstall => match blocklist.status().remaining {
            Some(left) => (Response::Refused(format!("a timed block has {left}s left")), None),
            None => (Response::Done, Some(Shutdown::Remove)),
        },
        // Stricter: replacing the exe is also a way to end the service, so no block at all.
        Request::PrepareUpdate => match blocklist.filters() {
            Some(_) => (Response::Refused("a block is running".into()), None),
            None => (Response::Done, Some(Shutdown::Update)),
        },
    }
}

/// What `start_block` in the app used to check, now checked where it counts.
fn start(
    blocklist: &Blocklist,
    filters: Vec<String>,
    allowed: Vec<String>,
    programs: Vec<String>,
    duration: &str,
    enforcers: Enforcers,
) -> Result<(), String> {
    let duration = parse_duration(duration)?;
    let sites = FilterList::parse(&filters).map_err(|_| "Invalid block list syntax".to_string())?;
    let allowed = FilterList::parse(&allowed).map_err(|_| "Invalid syntax in the list of allowed sites".to_string())?;
    // Also refuses what would close programs Windows needs (`programs.rs`), with the reason.
    let programs = ProgramList::parse(&programs)?;
    if !enforcers.any() {
        return Err("Choose at least one enforcer".into());
    }
    // An enforcer has nothing to do with an empty list of its own kind, while one that needs no
    // list (Obsidian's) always does. What matters is that something can run.
    let lists = Lists { sites, allowed, programs };
    if !enforcers.can_run(|uses| lists.has(uses)) {
        return Err("Enter at least one site or program to block, or turn on an enforcer that needs no list".into());
    }
    blocklist.start(lists, duration, enforcers)
}

/// Ends the block for the unlock password. After three wrong guesses in a row, guesses are
/// refused for longer each time. The lock is held while the hash is computed, so guesses cannot
/// be made in parallel either.
fn unlock(context: &Context, password: &str) -> Result<(), String> {
    let mut lockout = context.lockout.locked();
    if let Some(left) = lockout.until.and_then(|until| until.checked_duration_since(Instant::now())) {
        return Err(format!("Too many wrong passwords; wait {}s", left.as_secs() + 1));
    }
    if !unlock::check(password) {
        lockout.failures += 1;
        if lockout.failures >= 3 {
            let seconds = 5u64 << (lockout.failures - 3).min(8);
            lockout.until = Some(Instant::now() + Duration::from_secs(seconds));
        }
        return Err("Wrong password".into());
    }
    *lockout = Lockout::default();
    context.blocklist.reset();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Store;
    use std::fs;

    fn context(name: &str) -> Context {
        let dir = std::env::temp_dir().join(format!("service-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let blocklist = Blocklist::with_store(Store::new(dir.join("state.json")));
        Context { blocklist, lockout: Mutex::default(), slot: Arc::new(AppSlot::new(false)) }
    }

    fn start_request(duration: &str) -> Request {
        Request::Start { programs: vec![], allowed: vec![], filters: vec!["a.com".into()], duration: duration.into(), enforcers: Enforcers::default() }
    }

    #[test]
    fn requests_are_checked_again_by_the_service() {
        let context = context("checked");
        // An empty list with only enforcers that go by it, nothing chosen, a bad duration, and an
        // address that cannot match.
        let only_the_list = Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let bad = [
            Request::Start { programs: vec![], allowed: vec![], filters: vec![], duration: "1h".into(), enforcers: only_the_list },
            Request::Start {
                filters: vec!["a.com".into()],
                programs: vec![],
                allowed: vec![],
                duration: "1h".into(),
                enforcers: Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: false, exe_names: false },
            },
            start_request("soon"),
            Request::Start { programs: vec![], allowed: vec![], filters: vec!["has space.com".into()], duration: "1h".into(), enforcers: Enforcers::default() },
        ];
        for request in bad {
            assert!(matches!(handle(&context, 0, request).0, Response::Failed(_)));
        }
        assert!(context.blocklist.filters().is_none());
    }

    #[test]
    fn an_empty_list_starts_a_block_when_an_enforcer_without_the_list_is_on() {
        let first = context("empty-list");
        let obsidian_only = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true, exe_names: false };
        let request = |enforcers| Request::Start { programs: vec![], allowed: vec![], filters: vec![], duration: "1h".into(), enforcers };
        assert_eq!(handle(&first, 0, request(obsidian_only)).0, Response::Done);
        // The block keeps what it was started with: no entries, and Obsidian's enforcer only.
        let Response::Policy(Some(policy)) = handle(&first, 0, Request::Policy).0 else { panic!("no policy") };
        assert!(policy.filters.is_empty());
        assert_eq!(policy.enforcers, obsidian_only);

        // With all enforcers on (the default), an empty list is fine too: Obsidian's can run.
        let other = context("empty-list-default");
        assert_eq!(handle(&other, 0, request(Enforcers::default())).0, Response::Done);
    }

    #[test]
    fn only_the_installed_app_is_taken_in_as_the_app() {
        use std::process::{Command, Stdio};
        let mut context = context("register");
        context.slot = Arc::new(AppSlot::new(true));
        // A process that is not the installed exe, whatever it calls itself.
        let mut stranger = Command::new("ping.exe").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();

        let (response, _) = handle(&context, stranger.id(), Request::Register);
        assert!(matches!(response, Response::Refused(why) if why.contains("not the installed")));
        assert!(matches!(handle(&context, 0, Request::Register).0, Response::Refused(_)));
        // It may ask for the policy all the same; that is just information, and counts for nothing.
        assert_eq!(handle(&context, stranger.id(), Request::Policy).0, Response::Policy(None));
        assert!(context.slot.supervise(true, false), "no app should have been taken in");

        // This test process is the installed exe as far as the service can tell. Only if it runs
        // in the console session is it taken in; elsewhere an app just runs unwatched.
        let (response, _) = handle(&context, std::process::id(), Request::Register);
        assert_eq!(response, Response::Done);
        // A second app is refused while the first one is there.
        let (response, _) = handle(&context, stranger.id(), Request::Register);
        assert!(matches!(response, Response::Refused(_)));

        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    #[test]
    fn programs_alone_can_start_a_block_and_what_windows_needs_is_refused() {
        let programs_only = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: false, exe_names: true };
        let request = |programs: &[&str]| Request::Start {
            filters: vec![],
            allowed: vec![],
            programs: programs.iter().map(|p| p.to_string()).collect(),
            duration: "1h".into(),
            enforcers: programs_only,
        };
        let context = context("programs");

        // The programs enforcer alone, with nothing listed, has nothing to do.
        assert!(matches!(handle(&context, 0, request(&[])).0, Response::Failed(_)));
        // Entries that would close what Windows or this app needs, or are not names.
        for bad in ["*", "explorer.exe", "s*", r"C:\Games\game.exe"] {
            let Response::Failed(why) = handle(&context, 0, request(&[bad])).0 else { panic!("{bad} was accepted") };
            assert!(why.contains("needs") || why.contains("Not a program name"), "{bad}: {why}");
        }
        assert!(context.blocklist.filters().is_none());

        // A real one starts, and is what the app is told to close.
        assert_eq!(handle(&context, 0, request(&["Steam*", "game"])).0, Response::Done);
        let Response::Policy(Some(policy)) = handle(&context, 0, Request::Policy).0 else { panic!("no policy") };
        assert_eq!(policy.programs, vec!["steam*.exe", "game.exe"]);
        assert!(policy.filters.is_empty());
        assert_eq!(policy.enforcers, programs_only);
    }

    #[test]
    fn allowed_sites_travel_with_the_block_and_must_be_valid() {
        let context = context("allowed");
        let request = |filters: &[&str], allowed: &[&str]| Request::Start {
            filters: filters.iter().map(|f| f.to_string()).collect(),
            allowed: allowed.iter().map(|f| f.to_string()).collect(),
            programs: vec![],
            duration: "1h".into(),
            enforcers: Enforcers::default(),
        };
        // An allowed entry that cannot match is refused like a bad block entry.
        assert!(matches!(handle(&context, 0, request(&["*"], &["has space.com"])).0, Response::Failed(_)));
        assert!(context.blocklist.filters().is_none());

        // Everything blocked but one site: the app is told about both.
        assert_eq!(handle(&context, 0, request(&["*"], &["Google.com"])).0, Response::Done);
        let Response::Policy(Some(policy)) = handle(&context, 0, Request::Policy).0 else { panic!("no policy") };
        assert_eq!(policy.filters, vec!["*"]);
        assert_eq!(policy.allowed, vec!["google.com"]);
        let Response::Status(status) = handle(&context, 0, Request::Status).0 else { panic!("no status") };
        assert_eq!(status.allowed, vec!["google.com"]);
    }

    #[test]
    fn allowed_sites_alone_are_not_something_to_block() {
        let context = context("allowed-alone");
        // Only the tab enforcers, no sites to block and only an allowed one: nothing to do.
        let tabs_only = Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let request = Request::Start {
            filters: vec![],
            allowed: vec!["google.com".into()],
            programs: vec![],
            duration: "1h".into(),
            enforcers: tabs_only,
        };
        assert!(matches!(handle(&context, 0, request).0, Response::Failed(_)));
        assert!(context.blocklist.filters().is_none());
    }

    #[test]
    fn a_timed_block_cannot_be_shortened_stopped_or_uninstalled() {
        let context = context("timed");
        assert_eq!(handle(&context, 0, start_request("1h")).0, Response::Done);
        // Not replaced by a shorter or an untimed one, not stopped.
        assert!(matches!(handle(&context, 0, start_request("1s")).0, Response::Failed(_)));
        assert!(matches!(handle(&context, 0, start_request("")).0, Response::Failed(_)));
        assert!(matches!(handle(&context, 0, Request::Stop).0, Response::Failed(_)));
        // Neither uninstalling nor updating gets through, and the service keeps running.
        let (response, shutdown) = handle(&context, 0, Request::PrepareUninstall);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        let (response, shutdown) = handle(&context, 0, Request::PrepareUpdate);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        // The policy is what the app enforces.
        let Response::Policy(Some(policy)) = handle(&context, 0, Request::Policy).0 else { panic!("no policy") };
        assert_eq!(policy.filters, vec!["a.com"]);
    }

    #[test]
    fn without_a_block_the_installer_gets_through() {
        let context = context("idle");
        let (response, shutdown) = handle(&context, 0, Request::PrepareUninstall);
        assert_eq!(response, Response::Done);
        assert!(matches!(shutdown, Some(Shutdown::Remove)));
        let (response, shutdown) = handle(&context, 0, Request::PrepareUpdate);
        assert_eq!(response, Response::Done);
        assert!(matches!(shutdown, Some(Shutdown::Update)));
        // An untimed block does not stop an uninstall, but does stop an update.
        assert_eq!(handle(&context, 0, start_request("")).0, Response::Done);
        assert!(matches!(handle(&context, 0, Request::PrepareUninstall).1, Some(Shutdown::Remove)));
        assert!(matches!(handle(&context, 0, Request::PrepareUpdate).0, Response::Refused(_)));
    }

    #[test]
    fn the_password_ends_a_timed_block_and_wrong_ones_are_slowed_down() {
        let context = context("unlock");
        assert_eq!(handle(&context, 0, start_request("1h")).0, Response::Done);
        for _ in 0..3 {
            let wrong = Request::Unlock { password: "nope".into() };
            assert_eq!(handle(&context, 0, wrong).0, Response::Failed("Wrong password".into()));
        }
        // After the third, even the right password waits.
        let right = Request::Unlock { password: "test1234".into() };
        assert!(matches!(handle(&context, 0, right).0, Response::Failed(why) if why.starts_with("Too many")));
        assert!(context.blocklist.filters().is_some());
        // Once the wait is over, it works.
        context.lockout.locked().until = None;
        let right = Request::Unlock { password: "test1234".into() };
        assert_eq!(handle(&context, 0, right).0, Response::Done);
        assert!(context.blocklist.filters().is_none());
    }
}
