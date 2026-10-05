//! What the service does with each request from the app and the installer. Everything in a request
//! is checked here again: it comes from any local user's program, not necessarily ours. The same
//! `handle` answers over the pipe (`service.rs`) and, in a debug build, inside the app
//! (`service::local_handler`), so the rules cannot differ between the two.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::block::Blocklist;
use crate::definitions::{Definition, Definitions};
use crate::duration;
use crate::proto::{Overview, Policy, Request, Response};
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
    definitions: Definitions,
    lockout: Mutex<Lockout>,
    /// Which process is the app (`watchdog.rs`).
    slot: Arc<AppSlot>,
    /// Until when the app is told to quit when it asks for its policy: set by the unlock password,
    /// so the app quits however the blocks were unlocked (the window or the CLI). Only for a short
    /// while, so an app started later (at the next logon) is not sent away, and cleared when a block
    /// starts.
    quit_app_until: Mutex<Option<Instant>>,
}

/// How long after an unlock the app is told to quit; it asks every second.
const QUIT_WINDOW: Duration = Duration::from_secs(10);

impl Context {
    pub fn new(blocklist: Blocklist, definitions: Definitions, slot: Arc<AppSlot>) -> Context {
        Context { blocklist, definitions, lockout: Mutex::default(), slot, quit_app_until: Mutex::default() }
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
        Request::Status => {
            let status = blocklist.status();
            let overview = Overview { blocks: context.definitions.all(), running: status.running, clock_synced: status.clock_synced };
            (Response::Status(overview), None)
        }
        // The app asks to be taken in; anyone else is refused, and the app then ends itself.
        Request::Register => match context.slot.register(caller) {
            Ok(()) => (Response::Done, None),
            Err(why) => (Response::Refused(why), None),
        },
        Request::Policy => {
            // This request comes from the thread that closes tabs: hearing it is how the service
            // knows that is going on. It counts only from the process taken in as the app.
            context.slot.heard(caller);
            if context.quit_app_until.locked().is_some_and(|until| Instant::now() < until) {
                return (Response::Quit, None);
            }
            let policies = blocklist
                .active()
                .map(|active| active.blocks)
                .unwrap_or_default()
                .into_iter()
                .map(|block| Policy {
                    filters: block.filters.entries(),
                    allowed: block.allowed.entries(),
                    programs: block.programs.entries(),
                    enforcers: block.enforcers,
                })
                .collect();
            (Response::Policy(policies), None)
        }
        // A running block is a copy of its definition, so changing the definition would not change
        // the block; refusing it keeps what the UI shows and what runs the same.
        Request::Save { block } => (done(unless_running(blocklist, &block.id).and_then(|()| context.definitions.save(block))), None),
        Request::Delete { id } => (done(unless_running(blocklist, &id).and_then(|()| context.definitions.delete(&id))), None),
        Request::Start { id } => {
            let started = start(context, &id);
            // A block wants the app: it is not to be sent away by an unlock just before.
            if started.is_ok() {
                *context.quit_app_until.locked() = None;
            }
            (done(started), None)
        }
        Request::Stop { id } => (done(blocklist.stop(&id)), None),
        Request::Unlock { password } => (done(unlock(context, &password)), None),
        // Refused during a timed block, judged by internet time, like the block itself.
        Request::PrepareUninstall => match blocklist.longest_remaining() {
            Some(left) => (Response::Refused(format!("a locked block has {} left", duration::words(left))), None),
            None => (Response::Done, Some(Shutdown::Remove)),
        },
        // Stricter: replacing the exe is also a way to end the service, so no block at all.
        Request::PrepareUpdate => match blocklist.is_running() {
            true => (Response::Refused("a block is running".into()), None),
            false => (Response::Done, Some(Shutdown::Update)),
        },
    }
}

fn unless_running(blocklist: &Blocklist, id: &str) -> Result<(), String> {
    match blocklist.runs(id) {
        true => Err("A running block cannot be changed or deleted".into()),
        false => Ok(()),
    }
}

/// Starts a block as it is defined now, checked again where it counts.
fn start(context: &Context, id: &str) -> Result<(), String> {
    let definition: Definition = context.definitions.get(id).ok_or("No such block")?;
    definition.check()?;
    let schedule = definition.schedule.week()?;
    let locked_for = definition.lock.duration()?;
    let lists = definition.lists()?;
    let enforcers = definition.enforcers;
    if !enforcers.any() {
        return Err("Choose at least one enforcer".into());
    }
    // An enforcer has nothing to do with an empty list of its own kind, while one that needs no
    // list (Obsidian's) always does. What matters is that something can run.
    if !enforcers.can_run(|uses| lists.has(uses)) {
        return Err("Enter at least one site or program to block, or turn on an enforcer that needs no list".into());
    }
    context.blocklist.start(id, &definition.name, lists, enforcers, schedule, locked_for)
}

/// Ends every block for the unlock password. After three wrong guesses in a row, guesses are
/// refused for longer each time. The lock is held while the hash is computed, so guesses cannot
/// be made in parallel either.
fn unlock(context: &Context, password: &str) -> Result<(), String> {
    let mut lockout = context.lockout.locked();
    if let Some(left) = lockout.until.and_then(|until| until.checked_duration_since(Instant::now())) {
        return Err(format!("Too many wrong passwords; wait {}", duration::words(left.as_secs() + 1)));
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
    *context.quit_app_until.locked() = Some(Instant::now() + QUIT_WINDOW);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tests::definition;
    use crate::definitions::Schedule;
    use crate::lock::Lock;
    use crate::schedule::Week;
    use crate::state::{Enforcers, Store};
    use std::fs;

    fn context(name: &str) -> Context {
        let dir = std::env::temp_dir().join(format!("service-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let blocklist = Blocklist::with_store(Store::new(dir.join("state.json")));
        Context::new(blocklist, Definitions::at(Some(dir.join("blocks.json"))), Arc::new(AppSlot::new(false)))
    }

    fn timed(id: &str, filters: &[&str], duration: &str) -> Definition {
        Definition { lock: Lock::Timer { duration: duration.into() }, ..definition(id, filters) }
    }

    /// Saves the definition and starts it.
    fn run(context: &Context, block: Definition) -> Response {
        let id = block.id.clone();
        assert_eq!(handle(context, 0, Request::Save { block }).0, Response::Done);
        handle(context, 0, Request::Start { id }).0
    }

    fn policies(context: &Context) -> Vec<Policy> {
        let Response::Policy(policies) = handle(context, 0, Request::Policy).0 else { panic!("no policy") };
        policies
    }

    fn overview(context: &Context) -> Overview {
        let Response::Status(overview) = handle(context, 0, Request::Status).0 else { panic!("no status") };
        overview
    }

    #[test]
    fn requests_are_checked_again_by_the_service() {
        let context = context("checked");
        // An empty list with only enforcers that go by it, and nothing chosen.
        let only_the_list = Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let none = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: false, exe_names: false };
        for block in [Definition { enforcers: only_the_list, ..definition("a", &[]) }, Definition { enforcers: none, ..definition("b", &["a.com"]) }] {
            assert!(matches!(run(&context, block), Response::Failed(_)));
        }
        // A bad duration, and an address that cannot match, are not even saved.
        for block in [timed("c", &["a.com"], "soon"), definition("d", &["has space.com"])] {
            assert!(matches!(handle(&context, 0, Request::Save { block }).0, Response::Failed(_)));
        }
        // A block that is not defined.
        assert!(matches!(handle(&context, 0, Request::Start { id: "nope".into() }).0, Response::Failed(_)));
        assert!(policies(&context).is_empty());
        assert_eq!(overview(&context).blocks.len(), 2);
    }

    #[test]
    fn an_empty_list_starts_a_block_when_an_enforcer_without_the_list_is_on() {
        let context = context("empty-list");
        let obsidian_only = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true, exe_names: false };
        assert_eq!(run(&context, Definition { enforcers: obsidian_only, ..definition("a", &[]) }), Response::Done);
        // The block keeps what it was started with: no entries, and Obsidian's enforcer only.
        let policy = &policies(&context)[0];
        assert!(policy.filters.is_empty());
        assert_eq!(policy.enforcers, obsidian_only);

        // With all enforcers on (the default), an empty list is fine too: Obsidian's can run.
        assert_eq!(run(&context, definition("b", &[])), Response::Done);
    }

    #[test]
    fn blocks_are_defined_started_and_stopped_on_their_own() {
        let context = context("several");
        assert_eq!(run(&context, timed("work", &["a.com"], "1h")), Response::Done);
        assert_eq!(run(&context, definition("evening", &["b.com"])), Response::Done);
        assert_eq!(handle(&context, 0, Request::Save { block: definition("idle", &["c.com"]) }).0, Response::Done);
        let overview = overview(&context);
        assert_eq!(overview.blocks.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["work", "evening", "idle"]);
        assert_eq!(overview.running.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["work", "evening"]);
        assert_eq!(policies(&context).len(), 2);

        // A running block cannot be changed, deleted or started again; an idle one can.
        let changed = definition("work", &["z.com"]);
        assert!(matches!(handle(&context, 0, Request::Save { block: changed }).0, Response::Failed(_)));
        assert!(matches!(handle(&context, 0, Request::Delete { id: "evening".into() }).0, Response::Failed(_)));
        assert!(matches!(handle(&context, 0, Request::Start { id: "work".into() }).0, Response::Failed(_)));
        assert_eq!(handle(&context, 0, Request::Delete { id: "idle".into() }).0, Response::Done);

        // The untimed one stops, and can then be changed; the timed one cannot stop.
        assert_eq!(handle(&context, 0, Request::Stop { id: "evening".into() }).0, Response::Done);
        assert!(matches!(handle(&context, 0, Request::Stop { id: "work".into() }).0, Response::Failed(_)));
        assert_eq!(handle(&context, 0, Request::Save { block: definition("evening", &["d.com"]) }).0, Response::Done);
        assert_eq!(policies(&context).iter().map(|p| p.filters.clone()).collect::<Vec<_>>(), vec![vec!["a.com".to_string()]]);
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
        assert_eq!(handle(&context, stranger.id(), Request::Policy).0, Response::Policy(Vec::new()));
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
        let block = |programs: &[&str]| Definition {
            programs: programs.iter().map(|p| p.to_string()).collect(),
            enforcers: programs_only,
            ..timed("p", &[], "1h")
        };
        let context = context("programs");

        // The programs enforcer alone, with nothing listed, has nothing to do.
        assert!(matches!(run(&context, block(&[])), Response::Failed(_)));
        // Entries that would close what Windows or this app needs, or are not names.
        for bad in ["*", "explorer.exe", "s*", r"C:\Games\game.exe"] {
            let Response::Failed(why) = handle(&context, 0, Request::Save { block: block(&[bad]) }).0 else { panic!("{bad} was accepted") };
            assert!(why.contains("needs") || why.contains("Not a program name"), "{bad}: {why}");
        }
        assert!(policies(&context).is_empty());

        // A real one starts, and is what the app is told to close.
        assert_eq!(run(&context, block(&["Steam*", "game"])), Response::Done);
        let policy = &policies(&context)[0];
        assert_eq!(policy.programs, vec!["steam*.exe", "game.exe"]);
        assert!(policy.filters.is_empty());
        assert_eq!(policy.enforcers, programs_only);
    }

    #[test]
    fn allowed_sites_travel_with_the_block_and_must_be_valid() {
        let context = context("allowed");
        let block = |allowed: &[&str]| Definition { allowed: allowed.iter().map(|f| f.to_string()).collect(), ..timed("a", &["*"], "1h") };
        // An allowed entry that cannot match is refused like a bad block entry.
        assert!(matches!(handle(&context, 0, Request::Save { block: block(&["has space.com"]) }).0, Response::Failed(_)));

        // Everything blocked but one site: the app is told about both.
        assert_eq!(run(&context, block(&["Google.com"])), Response::Done);
        let policy = &policies(&context)[0];
        assert_eq!(policy.filters, vec!["*"]);
        assert_eq!(policy.allowed, vec!["google.com"]);
        assert_eq!(overview(&context).running[0].allowed, vec!["google.com"]);
    }

    #[test]
    fn allowed_sites_alone_are_not_something_to_block() {
        let context = context("allowed-alone");
        // Only the tab enforcers, no sites to block and only an allowed one: nothing to do.
        let tabs_only = Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let block = Definition { allowed: vec!["google.com".into()], enforcers: tabs_only, ..timed("a", &[], "1h") };
        assert!(matches!(run(&context, block), Response::Failed(_)));
        assert!(policies(&context).is_empty());
    }

    #[test]
    fn a_timed_block_cannot_be_shortened_stopped_or_uninstalled() {
        let context = context("timed");
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        // Not shortened through its definition, not stopped.
        assert!(matches!(handle(&context, 0, Request::Save { block: timed("a", &["a.com"], "1s") }).0, Response::Failed(_)));
        assert!(matches!(handle(&context, 0, Request::Stop { id: "a".into() }).0, Response::Failed(_)));
        // Neither uninstalling nor updating gets through, and the service keeps running.
        let (response, shutdown) = handle(&context, 0, Request::PrepareUninstall);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        let (response, shutdown) = handle(&context, 0, Request::PrepareUpdate);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        // The policy is what the app enforces.
        assert_eq!(policies(&context)[0].filters, vec!["a.com"]);
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
        assert_eq!(run(&context, definition("a", &["a.com"])), Response::Done);
        assert!(matches!(handle(&context, 0, Request::PrepareUninstall).1, Some(Shutdown::Remove)));
        assert!(matches!(handle(&context, 0, Request::PrepareUpdate).0, Response::Refused(_)));
    }

    #[test]
    fn the_password_ends_every_block_and_wrong_ones_are_slowed_down() {
        let context = context("unlock");
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        assert_eq!(run(&context, timed("b", &["b.com"], "2h")), Response::Done);
        for _ in 0..3 {
            let wrong = Request::Unlock { password: "nope".into() };
            assert_eq!(handle(&context, 0, wrong).0, Response::Failed("Wrong password".into()));
        }
        // After the third, even the right password waits.
        let right = Request::Unlock { password: "test1234".into() };
        assert!(matches!(handle(&context, 0, right).0, Response::Failed(why) if why.starts_with("Too many")));
        assert!(context.blocklist.is_running());
        // Once the wait is over, it works, and ends them all; the definitions stay.
        context.lockout.locked().until = None;
        let right = Request::Unlock { password: "test1234".into() };
        assert_eq!(handle(&context, 0, right).0, Response::Done);
        assert!(!context.blocklist.is_running());
        assert_eq!(overview(&context).blocks.len(), 2);
    }

    #[test]
    fn a_weekly_block_runs_by_its_schedule_and_messages_say_minutes_and_hours() {
        let context = context("weekly");
        // Every day all day: active now, whatever the day.
        let daily = Definition { schedule: Schedule::Weekly(Week::parse("daily").unwrap().unwrap()), ..timed("w", &["a.com"], "2h 5m") };
        assert_eq!(run(&context, daily), Response::Done);
        let running = &overview(&context).running[0];
        assert!(running.enforcing && running.schedule.is_some());
        assert_eq!(policies(&context).len(), 1);
        let Response::Failed(why) = handle(&context, 0, Request::Stop { id: "w".into() }).0 else { panic!("stopped") };
        assert!(why.contains("2 hours 4 minutes") || why.contains("2 hours 5 minutes"), "{why}");
        let Response::Refused(why) = handle(&context, 0, Request::PrepareUninstall).0 else { panic!("not refused") };
        assert!(why.contains("2 hours") && why.ends_with("seconds left"), "{why}");
        // A weekly schedule with no time in it is never active: not even saved.
        let never = Definition { schedule: Schedule::Weekly(Week::default()), ..definition("n", &["a.com"]) };
        assert!(matches!(handle(&context, 0, Request::Save { block: never }).0, Response::Failed(why) if why.contains("never be active")));
    }

    #[test]
    fn after_an_unlock_the_app_is_told_to_quit_until_a_block_starts() {
        let context = context("quit");
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        assert!(matches!(handle(&context, 0, Request::Policy).0, Response::Policy(_)));
        // Unlocked from anywhere (the CLI, say): the app's next policy request is answered with Quit.
        assert_eq!(handle(&context, 0, Request::Unlock { password: "test1234".into() }).0, Response::Done);
        assert_eq!(handle(&context, 0, Request::Policy).0, Response::Quit);
        assert_eq!(handle(&context, 0, Request::Policy).0, Response::Quit, "until it has quit");
        // A block started again wants the app, so it is not sent away.
        assert_eq!(handle(&context, 0, Request::Start { id: "a".into() }).0, Response::Done);
        assert!(matches!(handle(&context, 0, Request::Policy).0, Response::Policy(p) if p.len() == 1));
        // And only for a short while: an app started at the next logon gets its policy.
        assert_eq!(handle(&context, 0, Request::Unlock { password: "test1234".into() }).0, Response::Done);
        *context.quit_app_until.locked() = Some(Instant::now());
        assert_eq!(handle(&context, 0, Request::Policy).0, Response::Policy(Vec::new()));
    }
}
