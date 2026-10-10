//! Closes the browser pages (Chromium browsers and Firefox) whose URL is blocked by a target's
//! sites, using Windows UI Automation.
//!
//! Only the visible tab(s) of each browser window expose their URL (via each page's root
//! document, or the address bar), so a blocked tab is closed once it is shown in its window,
//! including as either page of a split view.
//!
//! Firefox exposes every tab's page, shown or not, and which are halves of a split view
//! (`Closer::tick_firefox`).
//!
//! Obsidian's web viewer exposes neither its URL nor its tabs, so while a session is running
//! Obsidian is closed whenever it hosts any web content.
//!
//! App windows (`chrome --app=<url>`, popups) have no tab strip and an address bar with no room
//! on screen, but their page's document still carries its URL; the window is closed whole.
//!
//! A target's sites are closed in all of these ways whenever it has any; only Obsidian's is an
//! action a target turns on or off (`state::Actions`). With several sessions running, each goes by
//! its own actions and lists (`session::Active`).
//!
//! Where things are: this file is the loop and what the passes share (`Closer`); `chromium.rs`,
//! `firefox.rs` read and close a browser's pages; `vscode.rs` VS Code's built-in browser; `others.rs` closes programs, Obsidian and windows by
//! title; `win.rs` is the plain Windows calls; `ledger.rs` remembers what was tried, so a close that
//! did not work is noticed (and not repeated at once).
//!
//! Passes: every `INTERVAL` all windows are looked at. In between, a window event looks only at the window that raised
//! it (`wake::take_changed`), and a window that asked to be looked at again (`Closer::watch`: a page
//! to be seen twice, or something just closed) is looked at every `QUICK` for a while.

mod chromium;
mod firefox;
mod ledger;
mod others;
mod vscode;
mod win;

use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

use crate::client::Remote;
use crate::session::Active;
use crate::sync::spawn_critical;
use crate::wake;
use crate::watchdog::note;
use chromium::{Browser, Doubt, Layout, Pending};
use ledger::{Check, Ledger};
use sysinfo::System;
use uiautomation::{UIAutomation, UIElement};
use win::Kind;

const INTERVAL: Duration = Duration::from_secs(1);

/// The least time between two passes, when events ask for one sooner.
const MIN_GAP: Duration = Duration::from_millis(100);

const RECHECK: Duration = Duration::from_secs(10);

/// The wait after a pass when a window asked to be looked at again soon: one not yet known to be an
/// app window, or a page read from the address bar that must be seen twice before it is closed.
const QUICK: Duration = Duration::from_millis(150);

/// How long a window that asked to be looked at again (`Closer::watch`) is looked at every `QUICK`.
const LOOK_AGAIN: Duration = Duration::from_secs(1);

/// After something was closed in a window it is looked at every `QUICK` for this long: the browser
/// shows the next tab at once, but exposes its page a moment later and raises no event for it (and
/// Firefox raises none for tabs in the background at all), so a flood of blocked tabs would otherwise
/// go one per `INTERVAL`.
const BURST: Duration = Duration::from_secs(3);

/// A step that takes longer than this is written to `app.log` (at most once in `SLOW_EVERY`).
const SLOW: Duration = Duration::from_millis(50);

const SLOW_EVERY: Duration = Duration::from_secs(10);

/// The same failure is written to `app.log` at most this often.
const FAIL_EVERY: Duration = Duration::from_secs(30);

/// How long a window is given to go down when asked to minimize, before it is taken as one that
/// cannot be minimized and left alone.
const MINIMIZE_AFTER: Duration = Duration::from_secs(1);

/// How long a program is given to close when asked to, before it is ended.
const KILL_AFTER: Duration = Duration::from_secs(3);

pub fn spawn(remote: Remote) {
    spawn_critical("tab closer", move || {
        let automation = match UIAutomation::new() {
            Ok(a) => a,
            // Without it nothing is enforced, so say so loudly: the process ends and is started again.
            Err(e) => {
                note("app", &format!("UI Automation is unavailable: {e}"));
                std::process::exit(1);
            }
        };
        let mut closer = Closer::new(automation);
        let mut last_full: Option<Instant> = None;
        loop {
            let start = Instant::now();
            // All windows once in `INTERVAL`; otherwise only those that raised an event or asked to be
            // looked at again.
            let changed = wake::take_changed();
            let full = last_full.is_none_or(|at| at.elapsed() >= INTERVAL);
            let active = remote.active();
            // While windows are minimized, any window coming up is worth a look at once.
            wake::watch_every_window(active.as_ref().is_some_and(|active| active.minimizes_windows()));
            if let Some(active) = active {
                // With no entries there is nothing for these to match, so skip the UI Automation work.
                if active.watches_sites() {
                    let only = (!full).then(|| closer.windows_to_look_at(&changed));
                    closer.tick(&active, only.as_ref());
                }
                // Obsidian, the programs, the Store apps and the window titles all look at the running
                // programs: one scan serves them all.
                let obsidian = active.closes_obsidian();
                let programs = active.closes_programs();
                let store_apps = active.closes_store_apps();
                // Steam's browser is a window found like one by title, in the same pass.
                let titles = active.watches_titles() || active.closes_steam_browser();
                if obsidian || programs || store_apps || titles {
                    closer.scan_processes(obsidian, store_apps);
                }
                if obsidian {
                    closer.close_obsidian_with_web_content();
                }
                if programs || store_apps {
                    closer.close_processes(&active);
                }
                if titles {
                    closer.windows_by_title(&active);
                }
            }
            if full {
                last_full = Some(start);
            }
            // Until the next whole pass, or sooner when something worth a look happened (`wake.rs`) or a
            // window asked for another look; a burst of events is one pass, after a short pause.
            thread::sleep(MIN_GAP.saturating_sub(start.elapsed()));
            let to_full = last_full.map_or(Duration::ZERO, |at| INTERVAL.saturating_sub(at.elapsed()));
            let wait = if closer.looking_again() { QUICK.min(to_full) } else { to_full };
            closer.woken = wake::wait(wait);
        }
    });
}

struct Closer {
    automation: UIAutomation,
    /// Window handle -> its cached UI elements, so they are not searched for every second.
    windows: HashMap<isize, Window>,
    system: System,
    /// Programs that were asked to close, by process id and when, so the ones that do not are ended.
    asked: HashMap<u32, Instant>,
    /// Windows that were asked to close because of their title, by handle and when, so one that
    /// stays (it asks to save first) is asked again later, not every pass.
    titles_asked: HashMap<isize, Instant>,
    /// Windows that were asked to minimize because of their title, by handle and when, and those
    /// that did not go down (left alone from then on, while they are open).
    minimize_asked: HashMap<isize, Instant>,
    unminimizable: HashSet<isize>,
    /// What started this pass, for the log (`wake::Woken`).
    woken: wake::Woken,
    /// How each Chromium window is laid out, found once (`Closer::layout`): never searched for again.
    layouts: HashMap<isize, Layout>,
    /// Windows that showed no tab strip so far, and for how long (`judge`).
    doubts: HashMap<isize, Doubt>,
    /// A blocked page read from a window's address bar, waiting to be seen again before it is closed.
    pending: HashMap<isize, Pending>,
    /// Windows that asked to be looked at again, until when (`Closer::watch`).
    hot: HashMap<isize, Instant>,
    /// What was tried to close, so it can be told whether it worked (`ledger.rs`).
    ledger: Ledger,
    /// When each slow step was last written to the log (`Closer::slow`).
    slow_logged: HashMap<&'static str, Instant>,
    /// When each failure was last written to the log (`Closer::fail`).
    fail_logged: HashMap<String, Instant>,
    /// A blocked page of a window and when it was first seen there, so a close can say how long it took
    /// from seeing it (`close_blocked_tab`).
    first_seen: HashMap<isize, (String, Instant)>,
}

enum Window {
    Browser(Browser),
    /// A Firefox window (its address bar found), whose pages are looked up each pass.
    Firefox(UIElement),
    /// A VS Code window (`vscode.rs`): its editor area (none: it has none, as its developer tools), and
    /// when that was looked for.
    VsCode(Option<UIElement>, Instant),
    /// Not a browser window (Electron app, devtools, popup); searched again after `RECHECK`.
    Other(Instant),
}

/// A readable text for a UI Automation error. "Nothing found" comes as an error with no code, whose own
/// text reads as a success.
fn explain(e: &uiautomation::Error) -> String {
    if e.code() == 0 {
        "nothing was found there".to_string()
    } else {
        e.to_string()
    }
}

impl Closer {
    fn new(automation: UIAutomation) -> Closer {
        Closer {
            automation,
            windows: HashMap::new(),
            system: System::new(),
            asked: HashMap::new(),
            titles_asked: HashMap::new(),
            minimize_asked: HashMap::new(),
            unminimizable: HashSet::new(),
            woken: wake::Woken::Second,
            layouts: HashMap::new(),
            doubts: HashMap::new(),
            pending: HashMap::new(),
            hot: HashMap::new(),
            ledger: Ledger::default(),
            slow_logged: HashMap::new(),
            fail_logged: HashMap::new(),
            first_seen: HashMap::new(),
        }
    }

    /// Writes a step that took long to `app.log`, now and then, so slowness can be seen.
    fn slow(&mut self, what: &'static str, since: Instant) {
        let took = since.elapsed();
        if took > SLOW && self.slow_logged.get(what).is_none_or(|last| last.elapsed() >= SLOW_EVERY) {
            self.slow_logged.insert(what, Instant::now());
            note("app", &format!("slow: {what} took {} ms", took.as_millis()));
        }
    }

    /// Writes a failure to `app.log` (the same one at most once in `FAIL_EVERY`): what could not be
    /// done, in words. Nothing the closer cannot do goes unsaid.
    fn fail(&mut self, text: &str) {
        if self.fail_logged.get(text).is_none_or(|at| at.elapsed() >= FAIL_EVERY) {
            self.fail_logged.insert(text.to_string(), Instant::now());
            note("app", text);
        }
    }

    /// Asks for a window to be looked at every `QUICK`, for this long.
    fn watch(&mut self, handle: isize, for_how_long: Duration) {
        let until = Instant::now() + for_how_long;
        let entry = self.hot.entry(handle).or_insert(until);
        if *entry < until {
            *entry = until;
        }
    }

    /// Something was just closed in a window: look at it again soon, and keep doing so for a while (`BURST`).
    fn closed_one(&mut self, handle: isize) {
        self.watch(handle, BURST);
    }

    /// Whether a window asked to be looked at again soon.
    fn looking_again(&mut self) -> bool {
        let now = Instant::now();
        self.hot.retain(|_, until| *until > now);
        !self.hot.is_empty()
    }

    /// The windows a pass between two whole ones looks at: those that raised an event, and those that
    /// asked for another look.
    fn windows_to_look_at(&mut self, changed: &[isize]) -> HashSet<isize> {
        self.looking_again();
        changed.iter().copied().chain(self.hot.keys().copied()).collect()
    }

    /// Whether a close may be tried now. Not while one asked a moment ago is waited for, and not at once
    /// after one that did not work (the wait grows with each failure). Says so, in `app.log`, when the
    /// last try did not work.
    fn may_try(&mut self, handle: isize, key: &str, what: &str) -> bool {
        match self.ledger.check(handle, key, Instant::now()) {
            Check::Fresh => true,
            Check::Waiting => {
                self.watch(handle, LOOK_AGAIN);
                false
            }
            Check::Backoff => false,
            Check::Failed(tries) => {
                self.fail(&format!("closing {what} did not work (tried {tries} times); trying again"));
                true
            }
        }
    }

    /// A close was tried now.
    fn tried(&mut self, handle: isize, key: &str) {
        self.ledger.record(handle, key, Instant::now());
    }

    /// Forgets what is cached of a window, so it is looked at again from the start.
    fn forget(&mut self, handle: isize) {
        self.windows.remove(&handle);
        self.layouts.remove(&handle);
        self.doubts.remove(&handle);
        self.pending.remove(&handle);
        self.first_seen.remove(&handle);
        self.ledger.clear_window(handle);
    }

    /// Looks at the browser windows: all of them, or only these (`only`). The windows are found by a
    /// plain window enumeration, which takes a fraction of a millisecond. Asking UI Automation to
    /// search the desktop for them took about 90.
    fn tick(&mut self, active: &Active, only: Option<&HashSet<isize>>) {
        let handles = win::browser_windows();
        let seen: Vec<isize> = handles.iter().map(|(handle, _)| *handle).collect();
        for (handle, kind) in handles {
            if only.is_some_and(|only| !only.contains(&handle)) {
                continue;
            }
            match kind {
                Kind::Chromium => self.tick_chromium(handle, active),
                Kind::Firefox => self.tick_firefox(handle, active),
            }
        }
        self.layouts.retain(|handle, _| seen.contains(handle));
        self.doubts.retain(|handle, _| seen.contains(handle));
        self.pending.retain(|handle, _| seen.contains(handle));
        self.first_seen.retain(|handle, _| seen.contains(handle));
        self.windows.retain(|handle, _| seen.contains(handle));
        self.hot.retain(|handle, _| seen.contains(handle));
        self.ledger.prune(Instant::now(), &seen);
        self.fail_logged.retain(|_, at| at.elapsed() < FAIL_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::chromium::{agrees, judge, Doubt, Pending, Seen, Verdict};
    use super::win::browser_windows;
    use super::*;
    use crate::filters::FilterList;
    use crate::lists::Lists;
    use crate::programs::ProgramList;
    use crate::session::Enforced;
    use crate::state::Actions;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;

    /// One session with these sites and programs, and every action on.
    fn active(sites: &[&str], programs: &[&str]) -> Active {
        Active {
            sessions: vec![Enforced::new(
                Arc::new(Lists {
                    sites: FilterList::parse(sites).unwrap(),
                    programs: ProgramList::parse(programs).unwrap(),
                    ..Lists::default()
                }),
                Actions::default(),
            )],
            used: Default::default(),
        }
    }

    /// A copy of a stock Windows program under a name of its own, so a test never touches a
    /// program the person running it is using.
    fn copy_of_ping(name: &str) -> Child {
        let dir = std::env::temp_dir().join(format!("closer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.exe"));
        let ping = std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join("System32").join("ping.exe");
        std::fs::copy(ping, &path).unwrap();
        Command::new(path).args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap()
    }

    /// What the loop does for the programs: one scan, then the check.
    fn pass(closer: &mut Closer, programs: &Active) {
        closer.scan_processes(false, false);
        closer.close_processes(programs);
    }

    #[test]
    fn a_listed_program_is_asked_to_close_then_ended_and_others_are_left_alone() {
        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
        let mut closer = Closer::new(automation);
        let (mut listed, mut other) = (copy_of_ping("bullseye-test-listed"), copy_of_ping("bullseye-test-other"));
        let programs = active(&[], &["bullseye-test-l*"]);

        // The first pass asks. This program has no window to ask, so it carries on for now.
        pass(&mut closer, &programs);
        assert!(closer.asked.contains_key(&listed.id()));
        assert!(!closer.asked.contains_key(&other.id()));
        assert!(listed.try_wait().unwrap().is_none());

        // Once it has had its time, the next pass ends it.
        thread::sleep(KILL_AFTER + Duration::from_millis(500));
        pass(&mut closer, &programs);
        let ended = (0..50).any(|_| {
            thread::sleep(Duration::from_millis(100));
            listed.try_wait().unwrap().is_some()
        });
        assert!(ended, "the listed program should have been ended");

        // A program that has gone is forgotten, and the one that was not listed never touched.
        pass(&mut closer, &programs);
        assert!(closer.asked.is_empty());
        assert!(other.try_wait().unwrap().is_none());
        let _ = other.kill();
        let _ = other.wait();
    }

    #[test]
    fn a_window_is_an_app_window_only_after_no_tabs_were_seen_for_a_while() {
        let start = Instant::now();
        let later = |ms: u64| start + Duration::from_millis(ms);
        // Tabs seen: a normal window, whatever was doubted before.
        assert!(matches!(judge(None, Seen::Tabs, start), Verdict::Tabs));
        // Not being able to look says nothing, and keeps what was doubted.
        let doubt = Doubt { passes: 1, since: start };
        assert!(matches!(judge(None, Seen::Unknown, start), Verdict::Unsure(None)));
        assert!(matches!(judge(Some(doubt), Seen::Unknown, later(5000)), Verdict::Unsure(Some(d)) if d.passes == 1));
        // No tabs once, or twice within the first second: still not known.
        assert!(matches!(judge(None, Seen::NoTabs, start), Verdict::Unsure(Some(d)) if d.passes == 1));
        assert!(matches!(judge(Some(doubt), Seen::NoTabs, later(200)), Verdict::Unsure(Some(d)) if d.passes == 2));
        // Twice, and for the whole of `APP_AFTER`: an app window.
        assert!(matches!(judge(Some(doubt), Seen::NoTabs, later(1000)), Verdict::App));
        // A first sighting is never enough, however late.
        assert!(matches!(judge(None, Seen::NoTabs, later(60_000)), Verdict::Unsure(Some(_))));
    }

    #[test]
    fn a_page_from_the_address_bar_is_closed_only_when_seen_again_in_the_same_tab() {
        let start = Instant::now();
        let pending = Pending { url: "https://blocked.example/".into(), tab: vec![42, 7], since: start };
        let after = |ms: u64| start + Duration::from_millis(ms);
        let (url, tab) = ("https://blocked.example/", [42, 7]);
        assert!(agrees(Some(&pending), url, &tab, after(100)));
        // Nothing seen before, or too soon to be a second look.
        assert!(!agrees(None, url, &tab, after(100)));
        assert!(!agrees(Some(&pending), url, &tab, after(10)));
        // Another page, or the selection moved to another tab: not the same.
        assert!(!agrees(Some(&pending), "https://other.example/", &tab, after(100)));
        assert!(!agrees(Some(&pending), url, &[42, 8], after(100)));
    }

    #[test]
    fn a_window_that_asks_to_be_looked_at_again_is_looked_at_until_its_time_is_up() {
        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
        let mut closer = Closer::new(automation);
        assert!(!closer.looking_again());
        assert!(closer.windows_to_look_at(&[]).is_empty());
        closer.watch(7, Duration::from_millis(80));
        closer.watch(9, Duration::from_secs(60));
        assert!(closer.looking_again());
        // Those that raised an event, and those that asked: each once.
        assert_eq!(closer.windows_to_look_at(&[3, 7, 7]), HashSet::from([3, 7, 9]));
        // A shorter ask does not shorten a longer one.
        closer.watch(9, Duration::from_millis(1));
        thread::sleep(Duration::from_millis(150));
        assert_eq!(closer.windows_to_look_at(&[]), HashSet::from([9]), "7 ran out of time, 9 has its own");
        assert!(closer.looking_again());
        // Closing something in a window keeps it in view for `BURST`.
        closer.closed_one(4);
        assert!(closer.hot[&4] > Instant::now() + Duration::from_secs(2));
    }

    #[test]
    fn a_close_that_was_tried_is_waited_for_and_not_tried_again_at_once() {
        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
        let mut closer = Closer::new(automation);
        assert!(closer.may_try(1, "tab [1]", "a page"), "nothing tried yet");
        closer.tried(1, "tab [1]");
        // Just tried: wait for it, and look at the window again soon to see whether it worked.
        assert!(!closer.hot.contains_key(&1));
        assert!(!closer.may_try(1, "tab [1]", "a page"));
        assert!(closer.hot.contains_key(&1), "a look again is asked for");
        // Another tab, another window, another kind of close are not held up.
        assert!(closer.may_try(1, "tab [2]", "a page") && closer.may_try(2, "tab [1]", "a page") && closer.may_try(1, "window", "a page"));
        // A window that is forgotten forgets what was tried in it.
        closer.forget(1);
        assert!(closer.may_try(1, "tab [1]", "a page"));
    }

    #[test]
    fn a_close_that_did_not_work_is_said_and_tried_again_after_the_wait() {
        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
        let mut closer = Closer::new(automation);
        // A try made long enough ago for a second wait to be up too.
        closer.ledger.record(1, "tab [1]", Instant::now() - Duration::from_secs(3));
        assert!(closer.may_try(1, "tab [1]", "https://blocked.example/ in a browser"));
        assert!(closer.fail_logged.keys().any(|text| text.contains("did not work") && text.contains("blocked.example")));
    }

    #[test]
    fn the_same_failure_is_said_once_in_a_while() {
        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
        let mut closer = Closer::new(automation);
        closer.fail("could not close a page");
        let said = closer.fail_logged["could not close a page"];
        closer.fail("could not close a page");
        assert_eq!(closer.fail_logged["could not close a page"], said, "not said again at once");
        closer.fail("could not close another");
        assert_eq!(closer.fail_logged.len(), 2);
        // Said again once `FAIL_EVERY` has passed.
        closer.fail_logged.insert("could not close a page".into(), Instant::now() - FAIL_EVERY);
        closer.fail("could not close a page");
        assert!(closer.fail_logged["could not close a page"] > said);
    }

    #[test]
    fn a_pass_between_two_whole_ones_looks_only_at_the_windows_asked_for() {
        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
        let mut closer = Closer::new(automation);
        let session = active(&["real-check-nothing.invalid"], &[]);
        // Nothing asked for: no window is looked at, so nothing is cached for any.
        closer.tick(&session, Some(&HashSet::new()));
        assert!(closer.windows.is_empty() && closer.layouts.is_empty());
        // A whole pass looks at what is open (if a browser is) and leaves nothing for windows that are gone.
        closer.hot.insert(-1, Instant::now() + Duration::from_secs(60));
        closer.tick(&session, None);
        assert!(!closer.hot.contains_key(&-1), "a window that is not there is not looked at again");
    }

    #[test]
    fn a_ui_automation_error_with_no_code_reads_as_nothing_found() {
        // What a search that finds nothing gives: an error whose own text says it succeeded.
        assert_eq!(explain(&uiautomation::Error::new(0, "The operation completed successfully.")), "nothing was found there");
        // Any other keeps its own words.
        assert_eq!(explain(&uiautomation::Error::new(-2147024809, "The parameter is incorrect.")), "The parameter is incorrect.");
        assert_eq!(explain(&uiautomation::Error::new(-1, "no selected tab with that title")), "no selected tab with that title");
    }

    /// A manual check against a real browser: have a Chromium browser window open on
    /// `https://example.com` (`msedge --new-window https://example.com`), then
    /// `cargo test -- --ignored a_real_browser_tab_is_closed --nocapture`. The tab must be closed by the
    /// sweep, which also prints how long a pass takes. (Needs a window that shows that page itself: a
    /// browser extension that replaces the page would hide it.)
    #[test]
    #[ignore]
    fn a_real_browser_tab_is_closed() {
        let automation = UIAutomation::new().unwrap();
        let mut closer = Closer::new(automation);
        let active = |entry: &str| active(&[entry], &[]);
        println!("chromium-class visible windows: {}", browser_windows().len());
        let quiet = active("real-check-nothing.invalid");
        closer.tick(&quiet, None);
        let runs = 20;
        let start = Instant::now();
        for _ in 0..runs {
            closer.tick(&quiet, None);
        }
        println!("tick, nothing to close: {:.2} ms per pass", start.elapsed().as_secs_f64() * 1000.0 / runs as f64);
        let browsers = closer.windows.values().filter(|w| matches!(w, Window::Browser(_))).count();
        println!("browser windows recognised: {browsers}");
        assert!(browsers >= 1, "open a Chromium browser window first");

        let session = active("example.com");
        let before = browser_windows().len();
        let start = Instant::now();
        let closed = (0..60).any(|_| {
            closer.tick(&session, None);
            thread::sleep(Duration::from_millis(250));
            browser_windows().len() < before
        });
        println!("blocked tab closed: {closed} (after {:?})", start.elapsed());
        assert!(closed, "the tab on example.com should have been closed");
    }

    #[test]
    #[ignore = "needs a Chromium app window open, e.g. chrome --app=https://example.com"]
    fn a_real_app_window_is_recognised_and_normal_windows_are_not() {
        let automation = UIAutomation::new().unwrap();
        let mut closer = Closer::new(automation);
        let (mut apps, mut normal) = (0, 0);
        for (handle, _) in browser_windows() {
            let Some(browser) = closer.browser(handle) else { continue };
            let pages = closer.pages(&browser).unwrap();
            let urls: Vec<_> = pages.iter().map(|p| p.url.as_str()).collect();
            let app = !pages.is_empty() && matches!(closer.probe(&browser.window, false).0, Seen::NoTabs);
            println!("{} window showing {urls:?}", if app { "app" } else { "browser" });
            if app {
                apps += 1
            } else {
                normal += 1
            }
        }
        assert!(apps >= 1, "open an app window first");
        println!("{apps} app windows, {normal} normal browser windows");
    }
}
