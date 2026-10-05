//! Closes Chromium-browser tabs (Chrome, Edge, Brave, ...) whose URL is blocked by the filter
//! list, using Windows UI Automation.
//!
//! Only the visible tab(s) of each browser window expose their URL (via each page's root
//! document, or the address bar), so a blocked tab is closed once it is shown in its window,
//! including as either page of a split view.
//!
//! Obsidian's web viewer exposes neither its URL nor its tabs, so while a block is running
//! Obsidian is closed whenever it hosts any web content.
//!
//! Each of these is an enforcer a block can turn on or off (`state::Enforcers`): blocked
//! tabs, blocked pages in a split view (a window showing two pages), and Obsidian. With several
//! blocks running, each goes by its own enforcers and lists (`block::Active`).

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::client::Remote;
use crate::block::Active;
use crate::sync::spawn_critical;
use crate::watchdog::note;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use uiautomation::patterns::{UIInvokePattern, UIValuePattern, UIWindowPattern};
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::{UIAutomation, UIElement};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};

const INTERVAL: Duration = Duration::from_secs(1);
const RECHECK: Duration = Duration::from_secs(10);

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
        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new(), asked: HashMap::new() };
        loop {
            let start = Instant::now();
            if let Some(active) = remote.active() {
                // With no entries there is nothing for these to match, so skip the UI Automation work.
                if active.watches_sites() {
                    if let Err(e) = closer.tick(&active) {
                        eprintln!("tab_closer: {e}");
                    }
                }
                // Obsidian and the programs both look at the running programs: one scan serves both.
                let obsidian = active.closes_obsidian();
                let programs = active.closes_programs();
                if obsidian || programs {
                    closer.scan_processes(obsidian);
                }
                if obsidian {
                    closer.close_obsidian_with_web_content();
                }
                if programs {
                    closer.close_programs(&active);
                }
            }
            thread::sleep(INTERVAL.saturating_sub(start.elapsed()));
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
}

enum Window {
    Browser(Browser),
    /// Not a browser window (Electron app, devtools, popup); searched again after `RECHECK`.
    Other(Instant),
}

#[derive(Clone)]
struct Browser {
    /// The browser window itself.
    window: UIElement,
    omnibox: UIElement,
    /// Parent of the visible pages (`MultiContentsView`); holds two pages in split view.
    contents: Option<UIElement>,
}

/// A page shown in a browser window.
struct Page {
    url: String,
    title: String,
}

impl Closer {
    fn tick(&mut self, active: &Active) -> uiautomation::Result<()> {
        // The browser windows are found by a plain window enumeration, which takes a fraction of a
        // millisecond. Asking UI Automation to search the desktop for them took about 90.
        let handles = browser_windows();
        let mut seen = Vec::with_capacity(handles.len());
        for handle in handles {
            seen.push(handle);
            let Some(browser) = self.browser(handle) else { continue };
            let Ok(pages) = self.pages(&browser) else {
                // Cached elements went stale; look them up again next tick.
                self.windows.remove(&handle);
                continue;
            };
            // Two pages at once is a split view; one is an ordinary tab.
            let split_view = pages.len() > 1;
            if let Some(page) = pages.iter().find(|p| active.blocks_page(&p.url, split_view)) {
                match close_tab(&self.automation, &browser.window, &page.title) {
                    Ok(()) => eprintln!("tab_closer: closed {}", page.url),
                    Err(e) => eprintln!("tab_closer: failed to close {}: {e}", page.url),
                }
            }
        }
        self.windows.retain(|handle, _| seen.contains(handle));
        Ok(())
    }

    /// Looks at the running programs, once, for the checks below (`close_obsidian_with_web_content`,
    /// `close_programs`). Command lines are only fetched when Obsidian's check needs them.
    fn scan_processes(&mut self, command_lines: bool) {
        let kind = if command_lines {
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet)
        } else {
            ProcessRefreshKind::nothing()
        };
        self.system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    }

    /// Closes Obsidian (like clicking its X) when it hosts web content. Obsidian's own UI runs in
    /// an unsandboxed renderer (it needs Node); web viewer pages run in sandboxed ones. Ending
    /// those renderers instead makes Obsidian quit on the next navigation, so close it cleanly.
    fn close_obsidian_with_web_content(&mut self) {
        let obsidian = |p: &&sysinfo::Process| p.name().eq_ignore_ascii_case("obsidian.exe");
        let has = |p: &sysinfo::Process, flag: &str| p.cmd().iter().any(|a| a.to_string_lossy() == flag);
        let processes = self.system.processes();
        let web_content = processes
            .values()
            .filter(obsidian)
            .any(|p| has(p, "--type=renderer") && has(p, "--enable-sandbox"));
        if !web_content {
            return;
        }
        // Main process: the one without a --type= switch. Its top-level windows are the vault windows.
        let main_pids: Vec<u32> = processes
            .values()
            .filter(obsidian)
            .filter(|p| !p.cmd().iter().any(|a| a.to_string_lossy().starts_with("--type=")))
            .map(|p| p.pid().as_u32())
            .collect();
        let close = || -> uiautomation::Result<()> {
            for window in self.automation.get_root_element()?.find_all(TreeScope::Children, &self.automation.create_true_condition()?)? {
                if main_pids.contains(&window.get_process_id()?) {
                    if let Ok(pattern) = window.get_pattern::<UIWindowPattern>() {
                        pattern.close()?;
                        eprintln!("tab_closer: closed Obsidian (web content open)");
                    }
                }
            }
            Ok(())
        };
        if let Err(e) = close() {
            eprintln!("tab_closer: failed to close Obsidian: {e}");
        }
    }

    /// Closes the programs on the running blocks' lists: asks each to close like the X of its
    /// windows does, and ends the ones still running `KILL_AFTER` later (or that have no window to
    /// ask). Programs Windows needs are never touched (`ProgramList::is_blocked`), nor is this app.
    fn close_programs(&mut self, active: &Active) {
        let me = std::process::id();
        let mut running = Vec::new();
        for (pid, process) in self.system.processes() {
            let pid = pid.as_u32();
            if pid == me || !active.blocks_program(&process.name().to_string_lossy()) {
                continue;
            }
            running.push(pid);
            match self.asked.get(&pid) {
                None => {
                    ask_to_close(pid);
                    self.asked.insert(pid, Instant::now());
                    eprintln!("tab_closer: asked {} ({pid}) to close", process.name().to_string_lossy());
                }
                Some(since) if since.elapsed() >= KILL_AFTER => {
                    let ended = process.kill();
                    eprintln!("tab_closer: {} {} ({pid})", if ended { "ended" } else { "could not end" }, process.name().to_string_lossy());
                }
                Some(_) => {}
            }
        }
        // A program that has gone, whether it closed or was ended, is not remembered.
        self.asked.retain(|pid, _| running.contains(pid));
    }

    /// The window's cached browser elements, or None if it is not a browser window.
    fn browser(&mut self, handle: isize) -> Option<Browser> {
        match self.windows.get(&handle) {
            Some(Window::Browser(b)) => return Some(b.clone()),
            Some(Window::Other(since)) if since.elapsed() < RECHECK => return None,
            _ => {}
        }
        let Ok(window) = self.automation.element_from_handle(Handle::from(handle)) else {
            // Gone already, or not ours to look at.
            self.windows.insert(handle, Window::Other(Instant::now()));
            return None;
        };
        let find = |class: &str| {
            let cond = self.automation.create_property_condition(UIProperty::ClassName, class.into(), None).ok()?;
            window.find_first(TreeScope::Descendants, &cond).ok()
        };
        let Some(omnibox) = find("OmniboxViewViews") else {
            self.windows.insert(handle, Window::Other(Instant::now()));
            return None;
        };
        let contents = find("MultiContentsView");
        let browser = Browser { window, omnibox, contents };
        self.windows.insert(handle, Window::Browser(browser.clone()));
        Some(browser)
    }

    /// URLs of the pages visible in the window. Each page's root document carries its URL, which
    /// also covers the second page in split view. Falls back to the address bar (active page only)
    /// when the documents are not exposed, skipping it while the user types in it.
    fn pages(&self, browser: &Browser) -> uiautomation::Result<Vec<Page>> {
        let mut pages = Vec::new();
        if let Some(contents) = &browser.contents {
            let all = self.automation.create_true_condition()?;
            let document =
                self.automation.create_property_condition(UIProperty::ControlType, (ControlType::Document as i32).into(), None)?;
            for view in contents.find_all(TreeScope::Children, &all)? {
                for doc in view.find_all(TreeScope::Children, &document).unwrap_or_default() {
                    let url = doc.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()).unwrap_or_default();
                    pages.push(Page { url, title: doc.get_name().unwrap_or_default() });
                }
            }
        }
        if pages.is_empty() && !browser.omnibox.has_keyboard_focus()? {
            let url = browser.omnibox.get_pattern::<UIValuePattern>()?.get_value()?;
            pages.push(Page { url, title: String::new() });
        }
        Ok(pages)
    }
}

/// The visible top-level windows of the class all Chromium-based browsers share (Electron apps
/// have it too; they have no address bar and are skipped in `Closer::browser`). Visible ones only,
/// which is what UI Automation lists as the desktop's children too.
fn browser_windows() -> Vec<isize> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
        let mut name = [0u16; 32];
        // SAFETY: the buffer is as long as the length says; `window` is one Windows just handed over.
        let (length, visible) = unsafe { (GetClassNameW(window, name.as_mut_ptr(), 32), IsWindowVisible(window) != 0) };
        if visible && length > 0 && String::from_utf16_lossy(&name[..length as usize]) == "Chrome_WidgetWin_1" {
            // SAFETY: `found` is the vector below, which outlives the enumeration.
            unsafe { (*(found as *mut Vec<isize>)).push(window as isize) };
        }
        1
    }
    let mut found: Vec<isize> = Vec::new();
    // SAFETY: the callback is the function above, and `found` outlives the call.
    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<isize> as LPARAM) };
    found
}

/// Posts a close request to every top-level window of the process, as the X button does. The
/// program may ask to save first, or not close at all; that is what `KILL_AFTER` is for.
fn ask_to_close(pid: u32) {
    unsafe extern "system" fn each(window: HWND, pid: LPARAM) -> i32 {
        let mut owner = 0u32;
        // SAFETY: `window` is one Windows just handed over; `owner` is a valid place to write to.
        unsafe {
            GetWindowThreadProcessId(window, &mut owner);
            if owner == pid as u32 {
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
        }
        1
    }
    // SAFETY: the callback is the function above, and takes no state beyond the process id.
    unsafe { EnumWindows(Some(each), pid as LPARAM) };
}

/// Window classes of a browser's tab strip entries: Chrome and Brave call them `Tab`, Edge calls
/// them `EdgeTab`. The class is what tells a browser tab from a tab-like widget inside a web page
/// (an ARIA tablist), whose close button must never be pressed.
const TAB_CLASSES: [&str; 2] = ["Tab", "EdgeTab"];

/// Presses the Close button of the selected tab showing `title`. In split view both tabs are
/// selected and named "<title> - Left view" / "<title> - Right view", so the title picks one;
/// with a single selected tab (or no title) the first selected tab is closed.
fn close_tab(a: &UIAutomation, window: &UIElement, title: &str) -> uiautomation::Result<()> {
    let is_a_tab_class = a.create_or_condition(
        a.create_property_condition(UIProperty::ClassName, TAB_CLASSES[0].into(), None)?,
        a.create_property_condition(UIProperty::ClassName, TAB_CLASSES[1].into(), None)?,
    )?;
    let cond = a.create_and_condition(
        is_a_tab_class,
        a.create_and_condition(
            a.create_property_condition(UIProperty::ControlType, (ControlType::TabItem as i32).into(), None)?,
            a.create_property_condition(UIProperty::SelectionItemIsSelected, true.into(), None)?,
        )?,
    )?;
    let selected = window.find_all(TreeScope::Descendants, &cond)?;
    let tab = selected
        .iter()
        .find(|t| !title.is_empty() && t.get_name().is_ok_and(|n| n.starts_with(title)))
        .or(selected.first())
        .ok_or_else(|| uiautomation::Error::new(-1, "no selected tab"))?;
    let button = a.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None)?;
    tab.find_first(TreeScope::Children, &button)?.get_pattern::<UIInvokePattern>()?.invoke()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Enforced;
    use crate::filters::FilterList;
    use crate::programs::ProgramList;
    use crate::state::Enforcers;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;

    /// One block with these sites and programs, and every enforcer on.
    fn active(sites: &[&str], programs: &[&str]) -> Active {
        Active {
            blocks: vec![Enforced {
                filters: Arc::new(FilterList::parse(sites).unwrap()),
                allowed: Arc::new(FilterList::default()),
                programs: Arc::new(ProgramList::parse(programs).unwrap()),
                enforcers: Enforcers::default(),
            }],
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
        closer.scan_processes(false);
        closer.close_programs(programs);
    }

    #[test]
    fn a_listed_program_is_asked_to_close_then_ended_and_others_are_left_alone() {
        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new(), asked: HashMap::new() };
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

    /// A manual check against a real browser: have a Chromium browser window open on
    /// `https://example.com` (`msedge --new-window https://example.com`), then
    /// `cargo test -- --ignored a_real_browser_tab_is_closed --nocapture`. The tab must be closed by the
    /// sweep, which also prints how long a pass takes. (Needs a window that shows that page itself: a
    /// browser extension that replaces the page would hide it.)
    #[test]
    #[ignore]
    fn a_real_browser_tab_is_closed() {
        let automation = UIAutomation::new().unwrap();
        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new(), asked: HashMap::new() };
        let active = |entry: &str| active(&[entry], &[]);
        println!("chromium-class visible windows: {}", browser_windows().len());
        let quiet = active("real-check-nothing.invalid");
        let _ = closer.tick(&quiet);
        let runs = 20;
        let start = Instant::now();
        for _ in 0..runs {
            closer.tick(&quiet).unwrap();
        }
        println!("tick, nothing to close: {:.2} ms per pass", start.elapsed().as_secs_f64() * 1000.0 / runs as f64);
        let browsers = closer.windows.values().filter(|w| matches!(w, Window::Browser(_))).count();
        println!("browser windows recognised: {browsers}");
        assert!(browsers >= 1, "open a Chromium browser window first");

        let block = active("example.com");
        let before = browser_windows().len();
        let start = Instant::now();
        let closed = (0..60).any(|_| {
            let _ = closer.tick(&block);
            thread::sleep(Duration::from_millis(250));
            browser_windows().len() < before
        });
        println!("blocked tab closed: {closed} (after {:?})", start.elapsed());
        assert!(closed, "the tab on example.com should have been closed");
    }

}
