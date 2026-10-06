//! Closes Chromium-browser tabs (Chrome, Edge, Brave, ...) whose URL is blocked by the filter
//! list, using Windows UI Automation.
//!
//! Only the visible tab(s) of each browser window expose their URL (via each page's root
//! document, or the address bar), so a blocked tab is closed once it is shown in its window,
//! including as either page of a split view.
//!
//! Firefox exposes every tab's page, shown or not, and which are halves of a split view
//! (`Closer::tick_firefox`).
//!
//! Obsidian's web viewer exposes neither its URL nor its tabs, so while a block is running
//! Obsidian is closed whenever it hosts any web content.
//!
//! App windows (`chrome --app=<url>`, popups) have no tab strip and an address bar with no room
//! on screen, but their page's document still carries its URL; the window is closed whole.
//!
//! Each of these is an enforcer a block can turn on or off (`state::Enforcers`): blocked
//! tabs, blocked pages in a split view (a window showing two pages), blocked app windows, and Obsidian. With several
//! blocks running, each goes by its own enforcers and lists (`block::Active`).

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::block::Active;
use crate::client::Remote;
use crate::programs;
use crate::store_apps;
use crate::sync::spawn_critical;
use crate::wake;
use crate::watchdog::note;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use uiautomation::core::UICondition;
use uiautomation::patterns::{UIInvokePattern, UIValuePattern, UIWindowPattern};
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::{UIAutomation, UIElement};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};

const INTERVAL: Duration = Duration::from_secs(1);
/// The least time between two passes, when events ask for one sooner.
const MIN_GAP: Duration = Duration::from_millis(100);
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
        let mut closer = Closer::new(automation);
        loop {
            let start = Instant::now();
            if let Some(active) = remote.active() {
                // With no entries there is nothing for these to match, so skip the UI Automation work.
                if active.watches_sites() {
                    if let Err(e) = closer.tick(&active) {
                        eprintln!("tab_closer: {e}");
                    }
                }
                // Obsidian, the programs, the Store apps and the window titles all look at the running
                // programs: one scan serves them all.
                let obsidian = active.closes_obsidian();
                let programs = active.closes_programs();
                let store_apps = active.closes_store_apps();
                let titles = active.closes_titles();
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
                    closer.close_windows_by_title(&active);
                }
            }
            // Until the next second, or sooner when something worth a look happened (`wake.rs`);
            // a burst of those is one pass, after a short pause.
            thread::sleep(MIN_GAP.saturating_sub(start.elapsed()));
            closer.woken = wake::wait(INTERVAL.saturating_sub(start.elapsed()));
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
    /// What started this pass, for the log (`wake::Woken`).
    woken: wake::Woken,
}

/// Which family a browser window is of, by its window class.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Chromium,
    Firefox,
}

enum Window {
    Browser(Browser),
    /// A Firefox window (its address bar found), whose pages are looked up each pass.
    Firefox(UIElement),
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
    fn new(automation: UIAutomation) -> Closer {
        Closer {
            automation,
            windows: HashMap::new(),
            system: System::new(),
            asked: HashMap::new(),
            titles_asked: HashMap::new(),
            woken: wake::Woken::Second,
        }
    }

    fn tick(&mut self, active: &Active) -> uiautomation::Result<()> {
        // The browser windows are found by a plain window enumeration, which takes a fraction of a
        // millisecond. Asking UI Automation to search the desktop for them took about 90.
        let handles = browser_windows();
        let mut seen = Vec::with_capacity(handles.len());
        for (handle, kind) in handles {
            seen.push(handle);
            if kind == Kind::Firefox {
                self.tick_firefox(handle, active);
                continue;
            }
            let Some(browser) = self.browser(handle) else { continue };
            let Ok(pages) = self.pages(&browser) else {
                // Cached elements went stale; look them up again next tick.
                self.windows.remove(&handle);
                continue;
            };
            // An app window has no tabs to close: the page goes with its window.
            if self.is_app_window(&browser, &pages) {
                if let Some(page) = pages.iter().find(|p| active.blocks_app_window(&p.url)) {
                    match close_window(&browser.window) {
                        Ok(()) => note("app", &format!("closed the app window of {} ({})", page.url, self.woken.describe())),
                        Err(e) => eprintln!("tab_closer: failed to close the app window of {}: {e}", page.url),
                    }
                }
                continue;
            }
            // Two pages at once is a split view; one is an ordinary tab.
            let split_view = pages.len() > 1;
            if let Some(page) = pages.iter().find(|p| active.blocks_page(&p.url, split_view)) {
                // The pages were read a moment ago, and what is shown changes under us: the user
                // switches tabs, and the browser shows a neighbour as soon as a tab closes, while
                // its old page may still be exposed for a few milliseconds. So the page is looked
                // at again just before, and only closed if it is still there as it was.
                let unchanged = self.pages(&browser).is_ok_and(|now| now.iter().any(|p| p.url == page.url && p.title == page.title));
                if !unchanged {
                    continue;
                }
                match close_tab(&self.automation, &browser.window, &page.title) {
                    Ok(()) => note("app", &format!("closed {} ({})", page.url, self.woken.describe())),
                    Err(e) => eprintln!("tab_closer: failed to close {}: {e}", page.url),
                }
            }
        }
        self.windows.retain(|handle, _| seen.contains(handle));
        Ok(())
    }

    /// Closes the Firefox tabs whose page is blocked. Unlike Chromium, Firefox exposes the page of
    /// every tab, background ones too: each tab's root document is named like the tab and carries
    /// its URL, so a blocked page is closed whether or not it is shown.
    fn tick_firefox(&mut self, handle: isize, active: &Active) {
        let Some(window) = self.firefox(handle) else { return };
        match self.close_blocked_firefox_tab(&window, active) {
            Ok(Some(url)) => note("app", &format!("closed {url} in Firefox ({})", self.woken.describe())),
            Ok(None) => {}
            Err(e) => {
                // Cached elements went stale, or the tab would not close; look again next tick.
                eprintln!("tab_closer: Firefox: {e}");
                self.windows.remove(&handle);
            }
        }
    }

    fn close_blocked_firefox_tab(&self, window: &UIElement, active: &Active) -> uiautomation::Result<Option<String>> {
        let a = &self.automation;
        let walker = a.get_control_view_walker()?;
        let is_document = a.create_property_condition(UIProperty::ControlType, (ControlType::Document as i32).into(), None)?;
        // Each document in order, with whether it is in a split view and where (pages inside a page
        // have titles no tab has).
        let mut documents = Vec::new();
        for doc in window.find_all(TreeScope::Descendants, &is_document)? {
            documents.push(FirefoxPage {
                url: doc.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()).unwrap_or_default(),
                title: doc.get_name().unwrap_or_default(),
                split: in_split_view(&walker, &doc),
                left: doc.get_bounding_rectangle().map(|r| r.get_left()).unwrap_or_default(),
            });
        }
        if !documents.iter().any(|d| active.blocks_firefox_page(&d.url, d.split)) {
            return Ok(None);
        }
        let is_tab = a.create_property_condition(UIProperty::ControlType, (ControlType::TabItem as i32).into(), None)?;
        let tabs = window.find_all(TreeScope::Descendants, &is_tab)?;
        let names: Vec<String> = tabs.iter().map(|t| t.get_name().unwrap_or_default()).collect();
        let button = a.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None)?;
        for (index, page) in documents.iter().enumerate() {
            if page.title.is_empty() || !active.blocks_firefox_page(&page.url, page.split) {
                continue;
            }
            let Some(tab) = firefox_tab_of(&documents, index, &names) else { continue };
            tabs[tab].find_first(TreeScope::Children, &button)?.get_pattern::<UIInvokePattern>()?.invoke()?;
            return Ok(Some(page.url.clone()));
        }
        Ok(None)
    }

    /// The window's cached element if it is a Firefox window (Thunderbird has the same window
    /// class but no address bar), or None.
    fn firefox(&mut self, handle: isize) -> Option<UIElement> {
        match self.windows.get(&handle) {
            Some(Window::Firefox(w)) => return Some(w.clone()),
            Some(Window::Other(since)) if since.elapsed() < RECHECK => return None,
            _ => {}
        }
        let window = self.automation.element_from_handle(Handle::from(handle)).ok();
        let address_bar = window.as_ref().and_then(|w| {
            let cond = self.automation.create_property_condition(UIProperty::AutomationId, "urlbar-input".into(), None).ok()?;
            w.find_first(TreeScope::Descendants, &cond).ok()
        });
        match (window, address_bar) {
            (Some(window), Some(_)) => {
                self.windows.insert(handle, Window::Firefox(window.clone()));
                Some(window)
            }
            _ => {
                self.windows.insert(handle, Window::Other(Instant::now()));
                None
            }
        }
    }

    /// Looks at the running programs, once, for the checks below (`close_obsidian_with_web_content`,
    /// `close_processes`, `close_windows_by_title`). Command lines are only fetched when Obsidian's
    /// check needs them, and where each program is only when Store apps are to be closed.
    fn scan_processes(&mut self, command_lines: bool, locations: bool) {
        let mut kind = ProcessRefreshKind::nothing();
        if command_lines {
            kind = kind.with_cmd(UpdateKind::OnlyIfNotSet);
        }
        if locations {
            kind = kind.with_exe(UpdateKind::OnlyIfNotSet);
        }
        self.system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    }

    /// Closes Obsidian (like clicking its X) when it hosts web content. Obsidian's own UI runs in
    /// an unsandboxed renderer (it needs Node); web viewer pages run in sandboxed ones. Ending
    /// those renderers instead makes Obsidian quit on the next navigation, so close it cleanly.
    fn close_obsidian_with_web_content(&mut self) {
        let obsidian = |p: &&sysinfo::Process| p.name().eq_ignore_ascii_case("obsidian.exe");
        let has = |p: &sysinfo::Process, flag: &str| p.cmd().iter().any(|a| a.to_string_lossy() == flag);
        let processes = self.system.processes();
        let web_content = processes.values().filter(obsidian).any(|p| has(p, "--type=renderer") && has(p, "--enable-sandbox"));
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
                        note("app", &format!("closed Obsidian, which showed web content ({})", self.woken.describe()));
                    }
                }
            }
            Ok(())
        };
        if let Err(e) = close() {
            eprintln!("tab_closer: failed to close Obsidian: {e}");
        }
    }

    /// Closes the programs on the running blocks' lists, and the programs of the Store apps on
    /// them (a packaged app's program runs from its own folder, which names the app:
    /// `store_apps::family_of`): asks each to close like the X of its windows does, and ends the
    /// ones still running `KILL_AFTER` later (or that have no window to ask). Programs Windows
    /// needs are never touched (`ProgramList::is_blocked`), nor Windows' own apps, nor is this app.
    fn close_processes(&mut self, active: &Active) {
        let me = std::process::id();
        let mut running = Vec::new();
        for (pid, process) in self.system.processes() {
            let pid = pid.as_u32();
            let in_a_blocked_app = || process.exe().and_then(store_apps::family_of).is_some_and(|family| active.blocks_store_app(&family));
            if pid == me || !(active.blocks_program(&process.name().to_string_lossy()) || in_a_blocked_app()) {
                continue;
            }
            running.push(pid);
            match self.asked.get(&pid) {
                None => {
                    ask_to_close(pid);
                    self.asked.insert(pid, Instant::now());
                    note("app", &format!("asked {} ({pid}) to close ({})", process.name().to_string_lossy(), self.woken.describe()));
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

    /// Closes the windows whose title is on the running blocks' lists, as the X does, and again
    /// every `KILL_AFTER` for one that stays. Only the window, never its program, so a browser
    /// window goes with all its tabs and nothing else is ended. Not windows of programs Windows
    /// needs (`programs::is_protected`; File Explorer's windows can go, but not the desktop and the
    /// taskbar, `SHELL_CLASSES`), nor this app's.
    fn close_windows_by_title(&mut self, active: &Active) {
        let me = std::process::id();
        let mut open = Vec::new();
        for window in top_level_windows() {
            if window.pid == me || SHELL_CLASSES.contains(&window.class.as_str()) || !active.blocks_title(&window.title) {
                continue;
            }
            // Without knowing the program (it may have just started) the window is left for the next pass.
            let Some(process) = self.system.process(sysinfo::Pid::from_u32(window.pid)) else { continue };
            let program = process.name().to_string_lossy();
            if programs::is_protected(&program) && !program.eq_ignore_ascii_case("explorer.exe") {
                continue;
            }
            open.push(window.handle);
            let due = match self.titles_asked.get(&window.handle) {
                Some(since) => since.elapsed() >= KILL_AFTER,
                None => true,
            };
            if due {
                // SAFETY: a window handle Windows handed over a moment ago; a stale one only fails.
                unsafe { PostMessageW(window.handle as HWND, WM_CLOSE, 0, 0) };
                self.titles_asked.insert(window.handle, Instant::now());
                note("app", &format!("asked the window \"{}\" of {program} to close ({})", window.title, self.woken.describe()));
            }
        }
        // A window that has gone is not remembered.
        self.titles_asked.retain(|handle, _| open.contains(handle));
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

    /// Whether the window is an app window (`--app=<url>`, or a popup): it has no tab strip. It
    /// must also show a page, so a normal window whose tabs are not exposed yet is not taken for
    /// one: the whole window is closed.
    fn is_app_window(&self, browser: &Browser, pages: &[Page]) -> bool {
        !pages.is_empty() && has_tabs(&self.automation, &browser.window).is_ok_and(|tabs| !tabs)
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

/// The visible top-level windows of the classes browsers have: all Chromium-based ones share one
/// (Electron apps have it too; they have no address bar and are skipped in `Closer::browser`),
/// and Firefox has its own (Thunderbird too: `Closer::firefox`). Visible ones only, which is what
/// UI Automation lists as the desktop's children too.
fn browser_windows() -> Vec<(isize, Kind)> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
        let mut name = [0u16; 32];
        // SAFETY: the buffer is as long as the length says; `window` is one Windows just handed over.
        let (length, visible) = unsafe { (GetClassNameW(window, name.as_mut_ptr(), 32), IsWindowVisible(window) != 0) };
        let class = if length > 0 { String::from_utf16_lossy(&name[..length as usize]) } else { String::new() };
        let kind = match class.as_str() {
            "Chrome_WidgetWin_1" => Some(Kind::Chromium),
            "MozillaWindowClass" => Some(Kind::Firefox),
            _ => None,
        };
        if let (true, Some(kind)) = (visible, kind) {
            // SAFETY: `found` is the vector below, which outlives the enumeration.
            unsafe { (*(found as *mut Vec<(isize, Kind)>)).push((window as isize, kind)) };
        }
        1
    }
    let mut found: Vec<(isize, Kind)> = Vec::new();
    // SAFETY: the callback is the function above, and `found` outlives the call.
    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<(isize, Kind)> as LPARAM) };
    found
}

/// Windows that are the desktop and the taskbar, whatever their title.
const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// A visible top-level window with a title.
struct TopWindow {
    handle: isize,
    pid: u32,
    title: String,
    class: String,
}

/// The visible top-level windows that have a title, with the process each belongs to.
fn top_level_windows() -> Vec<TopWindow> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
        // SAFETY: `window` is one Windows just handed over, and the buffers are as long as their lengths say.
        unsafe {
            if IsWindowVisible(window) != 0 {
                let length = GetWindowTextLengthW(window);
                if length > 0 {
                    let mut title = vec![0u16; length as usize + 1];
                    let copied = GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32);
                    let mut class = [0u16; 64];
                    let class_length = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(window, &mut pid);
                    if copied > 0 {
                        // SAFETY: `found` is the vector below, which outlives the enumeration.
                        (*(found as *mut Vec<TopWindow>)).push(TopWindow {
                            handle: window as isize,
                            pid,
                            title: String::from_utf16_lossy(&title[..copied as usize]),
                            class: String::from_utf16_lossy(&class[..class_length.max(0) as usize]),
                        });
                    }
                }
            }
        }
        1
    }
    let mut found: Vec<TopWindow> = Vec::new();
    // SAFETY: the callback is the function above, and `found` outlives the call.
    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<TopWindow> as LPARAM) };
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

/// The condition for an element that is a tab of the tab strip (`TAB_CLASSES`).
fn is_a_tab(a: &UIAutomation) -> uiautomation::Result<UICondition> {
    a.create_or_condition(
        a.create_property_condition(UIProperty::ClassName, TAB_CLASSES[0].into(), None)?,
        a.create_property_condition(UIProperty::ClassName, TAB_CLASSES[1].into(), None)?,
    )
}

/// Whether the browser window has a tab strip: any tab, selected or not.
fn has_tabs(a: &UIAutomation, window: &UIElement) -> uiautomation::Result<bool> {
    Ok(window.find_first(TreeScope::Descendants, &is_a_tab(a)?).is_ok())
}

/// A page of a Firefox window: the root document of a tab, or of one half of a split view.
struct FirefoxPage {
    url: String,
    title: String,
    split: bool,
    /// Its left edge on screen, which tells the two halves of a split view apart.
    left: i32,
}

/// Whether the document is in a split view's panel (`split-view-panel` is among the classes of
/// the pane around it).
fn in_split_view(walker: &uiautomation::UITreeWalker, doc: &UIElement) -> bool {
    let mut node = doc.clone();
    for _ in 0..3 {
        let Ok(parent) = walker.get_parent(&node) else { return false };
        if parent.get_classname().is_ok_and(|classes| classes.split(' ').any(|class| class == "split-view-panel")) {
            return true;
        }
        node = parent;
    }
    false
}

/// The tab (an index into `names`) that shows `pages[index]`. A tab is named like its page, and
/// pages with one title go with the tabs with it in order. The two tabs of a split view add which
/// half they are (`<title>, split view left` and `... right`, in the language of Firefox), and
/// their titles have been seen to lag behind their pages (named for the page before, or for the
/// page that picks the second half), so a split view's halves are found by that: two neighbouring
/// tabs whose names end alike but for the last word, the left half first. Where this is not clear
/// (two split views in one window), none: closing the wrong tab is worse.
fn firefox_tab_of(pages: &[FirefoxPage], index: usize, names: &[String]) -> Option<usize> {
    let page = &pages[index];
    if page.split {
        if let Some(first) = split_tabs(names) {
            let leftmost = pages.iter().filter(|p| p.split).map(|p| p.left).min()?;
            return Some(if page.left == leftmost { first } else { first + 1 });
        }
    }
    let rank = pages[..index].iter().filter(|other| other.title == page.title).count();
    (0..names.len()).filter(|&t| names[t] == page.title || names[t].starts_with(&format!("{}, ", page.title))).nth(rank)
}

/// The first of the two neighbouring tabs of a split view, when there is exactly one such pair:
/// their names end (after the last `, `) with the same words but for the last.
fn split_tabs(names: &[String]) -> Option<usize> {
    let ending = |name: &str| name.rsplit_once(", ").map(|(_, end)| end.split(' ').map(str::to_owned).collect::<Vec<_>>());
    let pairs: Vec<usize> = (0..names.len().saturating_sub(1))
        .filter(|&i| match (ending(&names[i]), ending(&names[i + 1])) {
            (Some(left), Some(right)) => {
                let shared = left.len().saturating_sub(1);
                shared >= 1 && left.len() == right.len() && left[..shared] == right[..shared] && left.last() != right.last()
            }
            _ => false,
        })
        .collect();
    match pairs[..] {
        [first] => Some(first),
        _ => None,
    }
}

/// Closes the window, like clicking its X.
fn close_window(window: &UIElement) -> uiautomation::Result<()> {
    window.get_pattern::<UIWindowPattern>()?.close()
}

/// Presses the Close button of the selected tab showing `title`. In split view both tabs are
/// selected and named "<title> - Left view" / "<title> - Right view", so the title picks one.
/// Only with no title to go by (a page read from the address bar) is the first selected tab
/// closed; with one, a selected tab that is not named like it is not the page that was read (the
/// browser has moved on to another tab), and closing it would take down an allowed tab.
fn close_tab(a: &UIAutomation, window: &UIElement, title: &str) -> uiautomation::Result<()> {
    let cond = a.create_and_condition(
        is_a_tab(a)?,
        a.create_and_condition(
            a.create_property_condition(UIProperty::ControlType, (ControlType::TabItem as i32).into(), None)?,
            a.create_property_condition(UIProperty::SelectionItemIsSelected, true.into(), None)?,
        )?,
    )?;
    let selected = window.find_all(TreeScope::Descendants, &cond)?;
    let tab = selected
        .iter()
        .find(|t| !title.is_empty() && t.get_name().is_ok_and(|n| n.starts_with(title)))
        .or_else(|| if title.is_empty() { selected.first() } else { None })
        .ok_or_else(|| uiautomation::Error::new(-1, "no selected tab with that title"))?;
    let button = a.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None)?;
    tab.find_first(TreeScope::Children, &button)?.get_pattern::<UIInvokePattern>()?.invoke()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Enforced;
    use crate::filters::FilterList;
    use crate::lists::Lists;
    use crate::programs::ProgramList;
    use crate::state::Enforcers;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;

    /// One block with these sites and programs, and every enforcer on.
    fn active(sites: &[&str], programs: &[&str]) -> Active {
        Active {
            blocks: vec![Enforced {
                lists: Arc::new(Lists {
                    sites: FilterList::parse(sites).unwrap(),
                    programs: ProgramList::parse(programs).unwrap(),
                    ..Lists::default()
                }),
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
            let app = closer.is_app_window(&browser, &pages);
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
