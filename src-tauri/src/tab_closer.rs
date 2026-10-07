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
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};

const INTERVAL: Duration = Duration::from_secs(1);
/// The least time between two passes, when events ask for one sooner.
const MIN_GAP: Duration = Duration::from_millis(100);
const RECHECK: Duration = Duration::from_secs(10);
/// The wait after a pass that wants to look again soon: a window not yet known to be an app window,
/// or a page read from the address bar that must be seen twice before it is closed.
const QUICK: Duration = Duration::from_millis(150);
/// After something was closed the passes keep coming at `QUICK` for this long: the browser shows the
/// next tab at once, but exposes its page a moment later and raises no event for it (and Firefox
/// raises none for tabs in the background at all), so a flood of blocked tabs would otherwise go one
/// per `INTERVAL`.
const BURST: Duration = Duration::from_secs(3);
/// How long two reads of an address-bar page must be apart to count as two.
const SETTLE: Duration = Duration::from_millis(80);
/// A window with no tab strip is an app window only when that has been so for this long, on two
/// passes at least (`judge`).
const APP_AFTER: Duration = Duration::from_secs(1);
/// A step that takes longer than this is written to `app.log` (at most once in `SLOW_EVERY`).
const SLOW: Duration = Duration::from_millis(50);
const SLOW_EVERY: Duration = Duration::from_secs(10);

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
            closer.quick = false;
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
            let bursting = closer.burst_until.is_some_and(|until| Instant::now() < until);
            let wait = if closer.quick || bursting { QUICK } else { INTERVAL };
            closer.woken = wake::wait(wait.saturating_sub(start.elapsed()));
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
    /// How each Chromium window is laid out, found once (`Closer::layout`): never searched for again.
    layouts: HashMap<isize, Layout>,
    /// Windows that showed no tab strip so far, and for how long (`judge`).
    doubts: HashMap<isize, Doubt>,
    /// A blocked page read from a window's address bar, waiting to be seen again before it is closed.
    pending: HashMap<isize, Pending>,
    /// Whether the pass just made wants another soon (`QUICK`).
    quick: bool,
    /// Until when passes follow each other at `QUICK`, after a close (`BURST`).
    burst_until: Option<Instant>,
    /// When each slow step was last written to the log (`Closer::slow`).
    slow_logged: HashMap<&'static str, Instant>,
    /// A blocked page of a window and when it was first seen there, so a close can say how long it took
    /// from seeing it (`close_blocked_tab`).
    first_seen: HashMap<isize, (String, Instant)>,
}

/// How a Chromium window is laid out. A window never changes from one to the other, so it is
/// worked out once and kept.
#[derive(Clone)]
enum Layout {
    /// Has a tab strip (the element that holds the tabs): tabs are closed one at a time.
    Tabs(UIElement),
    /// Has none (`chrome --app=<url>`, a popup): its page goes with the whole window.
    App,
    /// Has none, and belongs to a program that is no browser (`BROWSERS`): an app built on Chromium
    /// that has an address bar of its own (Spotify's desktop app), which is left alone.
    NotABrowser,
}

/// What a look for tabs in a window found.
enum Seen {
    Tabs,
    NoTabs,
    /// Windows could not be asked (busy, a timeout): says nothing.
    Unknown,
}

/// A window that showed no tabs, how many passes in a row and since when.
#[derive(Clone, Copy)]
struct Doubt {
    passes: u8,
    since: Instant,
}

enum Verdict {
    Tabs,
    App,
    /// Not known yet: what to remember of the doubt.
    Unsure(Option<Doubt>),
}

/// Whether a window is an app window, from what was seen of it. A failure to look is no evidence
/// (it used to count as "no tabs" and closed whole windows), and no tabs counts only when it has
/// been so on two passes at least and for `APP_AFTER`, as a window being built shows none for a moment.
fn judge(doubt: Option<Doubt>, seen: Seen, now: Instant) -> Verdict {
    match seen {
        Seen::Tabs => Verdict::Tabs,
        Seen::Unknown => Verdict::Unsure(doubt),
        Seen::NoTabs => {
            let doubt = doubt.map_or(Doubt { passes: 1, since: now }, |d| Doubt { passes: d.passes.saturating_add(1), since: d.since });
            if doubt.passes >= 2 && now.duration_since(doubt.since) >= APP_AFTER {
                Verdict::App
            } else {
                Verdict::Unsure(Some(doubt))
            }
        }
    }
}

/// A page read from the address bar alone (no title to check it by), with the tab that was
/// selected when it was read.
struct Pending {
    url: String,
    tab: Vec<i32>,
    since: Instant,
}

/// The programs whose windows without tabs are app windows (`chrome --app=<url>`, installed web apps,
/// popups). Lower case file names. A Chromium window of any other program with no tab strip is some
/// app's own (Spotify, Electron and CEF apps with an address bar), and is never closed as a page.
const BROWSERS: [&str; 12] = [
    "chrome.exe",
    "msedge.exe",
    "brave.exe",
    "vivaldi.exe",
    "opera.exe",
    "chromium.exe",
    "browser.exe",
    "whale.exe",
    "thorium.exe",
    "iron.exe",
    "arc.exe",
    "comet.exe",
];

/// Whether the window belongs to a browser (`BROWSERS`), by the file name of its program. If that cannot be
/// told the window is left alone: closing an app's window by mistake costs more than missing an app window.
fn is_browser(handle: isize) -> bool {
    let mut pid = 0u32;
    // SAFETY: a window handle Windows handed over a moment ago; a stale one only gives no process.
    unsafe { GetWindowThreadProcessId(handle as HWND, &mut pid) };
    if pid == 0 {
        return false;
    }
    // SAFETY: the process handle is closed below, and the buffer is as long as its length says.
    let name = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut buffer = [0u16; 520];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if ok == 0 {
            return false;
        }
        String::from_utf16_lossy(&buffer[..length as usize])
    };
    let file = name.rsplit(['\\', '/']).next().unwrap_or_default().to_lowercase();
    BROWSERS.contains(&file.as_str())
}

/// The site of an address, as the address bar and a page's own address both give it: no scheme, no `www.`,
/// no path, lower case, so the two can be compared.
fn host_of(url: &str) -> String {
    let url = url.trim().to_lowercase();
    let rest = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// Whether what is seen now is what was seen before, long enough ago to count as a second look.
fn agrees(pending: Option<&Pending>, url: &str, tab: &[i32], now: Instant) -> bool {
    pending.is_some_and(|p| p.url == url && p.tab == tab && now.duration_since(p.since) >= SETTLE)
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
            layouts: HashMap::new(),
            doubts: HashMap::new(),
            pending: HashMap::new(),
            quick: false,
            burst_until: None,
            slow_logged: HashMap::new(),
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
            let started = Instant::now();
            let pages = self.pages(&browser);
            self.slow("reading the pages of a window", started);
            let Ok(pages) = pages else {
                // Cached elements went stale; look them up again next tick.
                self.forget(handle);
                continue;
            };
            // Two pages at once is a split view; one is an ordinary tab. An app window's page goes
            // with its window; a normal window's blocked page is a tab to close.
            let split_view = pages.len() > 1;
            // The address bar is a quicker witness than a heavy page: the browser shows where a tab is
            // going before the page's accessibility tree is built (which takes long on a big page, and
            // is why such a page closed late). It counts only when it names another site than the page
            // that is exposed (an exposed page of the same site is trusted: the bar may show less of it),
            // and then goes the same way as a page read from the bar alone: seen twice, in the same
            // selected tab (`close_blocked_tab`).
            let bar = match pages.as_slice() {
                [one] if !one.title.is_empty() => {
                    self.address_bar_page(&browser).ok().flatten().filter(|bar| host_of(&bar.url) != host_of(&one.url))
                }
                _ => None,
            };
            let app_page = pages.iter().find(|p| active.blocks_app_window(&p.url));
            let tab_page = pages
                .iter()
                .find(|p| active.blocks_page(&p.url, split_view))
                .or(bar.as_ref().filter(|p| active.blocks_page(&p.url, false)));
            match tab_page {
                Some(page) if self.first_seen.get(&handle).is_none_or(|(url, _)| *url != page.url) => {
                    self.first_seen.insert(handle, (page.url.clone(), Instant::now()));
                }
                Some(_) => {}
                None => {
                    self.first_seen.remove(&handle);
                }
            }
            if app_page.is_none() && tab_page.is_none() {
                self.pending.remove(&handle);
                continue;
            }
            // Only now is it worth knowing what the window is.
            let Some(layout) = self.layout(handle, &browser.window) else {
                self.quick = true;
                continue;
            };
            match layout {
                Layout::App => {
                    if let Some(page) = app_page {
                        match close_window(&browser.window) {
                            Ok(()) => {
                                note("app", &format!("closed the app window of {} ({})", page.url, self.woken.describe()));
                                self.closed_one();
                            }
                            Err(e) => eprintln!("tab_closer: failed to close the app window of {}: {e}", page.url),
                        }
                    }
                }
                Layout::NotABrowser => {}
                Layout::Tabs(strip) => {
                    if let Some(page) = tab_page {
                        self.close_blocked_tab(handle, &browser, &strip, page);
                    }
                }
            }
        }
        self.layouts.retain(|handle, _| seen.contains(handle));
        self.doubts.retain(|handle, _| seen.contains(handle));
        self.pending.retain(|handle, _| seen.contains(handle));
        self.first_seen.retain(|handle, _| seen.contains(handle));
        self.windows.retain(|handle, _| seen.contains(handle));
        Ok(())
    }

    /// Something was just closed: look again soon, and keep doing so for a while (`BURST`).
    fn closed_one(&mut self) {
        self.burst_until = Some(Instant::now() + BURST);
    }

    /// Forgets what is cached of a window, so it is looked at again from the start.
    fn forget(&mut self, handle: isize) {
        self.windows.remove(&handle);
        self.layouts.remove(&handle);
        self.doubts.remove(&handle);
        self.pending.remove(&handle);
        self.first_seen.remove(&handle);
    }

    /// What kind of window this is, found once and kept: with a tab strip, or an app window. None
    /// while that is not known yet (see `judge`); then the pass is repeated soon.
    fn layout(&mut self, handle: isize, window: &UIElement) -> Option<Layout> {
        if let Some(layout) = self.layouts.get(&handle) {
            return Some(layout.clone());
        }
        let started = Instant::now();
        let (seen, strip) = self.probe(window);
        self.slow("looking for the tab strip of a window", started);
        let layout = match judge(self.doubts.get(&handle).copied(), seen, Instant::now()) {
            Verdict::Tabs => strip.map(Layout::Tabs),
            // Only a browser's own windows are app windows. Tabs are proof enough of a browser, but a
            // window with none is also what an app made of Chromium looks like (Spotify's has an address
            // bar too), so the program decides.
            Verdict::App => Some(if is_browser(handle) { Layout::App } else { Layout::NotABrowser }),
            Verdict::Unsure(doubt) => {
                match doubt {
                    Some(doubt) => self.doubts.insert(handle, doubt),
                    None => self.doubts.remove(&handle),
                };
                None
            }
        };
        if let Some(layout) = &layout {
            self.doubts.remove(&handle);
            self.layouts.insert(handle, layout.clone());
        }
        layout
    }

    /// Looks for the tabs of a window, once: all of them are listed, so that "none" really is none,
    /// and the parent of the first is the tab strip. The only search of a whole window.
    fn probe(&self, window: &UIElement) -> (Seen, Option<UIElement>) {
        let Ok(tab) = is_a_tab(&self.automation) else { return (Seen::Unknown, None) };
        match window.find_all(TreeScope::Descendants, &tab) {
            Err(_) => (Seen::Unknown, None),
            Ok(tabs) => match tabs.first() {
                None => (Seen::NoTabs, None),
                Some(first) => match self.automation.get_control_view_walker().and_then(|walker| walker.get_parent(first)) {
                    Ok(strip) => (Seen::Tabs, Some(strip)),
                    Err(_) => (Seen::Unknown, None),
                },
            },
        }
    }

    /// Closes the tab of a blocked page, only if it is still that page, and only that tab.
    fn close_blocked_tab(&mut self, handle: isize, browser: &Browser, strip: &UIElement, page: &Page) {
        // A page read from the address bar has no title to check the tab by. So the tab that is
        // selected is noted with it, and the page must be seen again, in that tab, a moment later
        // (the address bar can lag behind a switch of tabs): then that tab, by its own id, is closed.
        let expected = if page.title.is_empty() {
            let started = Instant::now();
            let tabs = selected_tabs(&self.automation, strip, &browser.window);
            self.slow("looking for the selected tab", started);
            let id = match tabs.as_deref() {
                Ok([only]) => only.get_runtime_id().ok(),
                Ok(_) => None,
                Err(_) => {
                    self.forget(handle);
                    return;
                }
            };
            let Some(id) = id else {
                self.pending.remove(&handle);
                return;
            };
            if !agrees(self.pending.get(&handle), &page.url, &id, Instant::now()) {
                self.pending.insert(handle, Pending { url: page.url.clone(), tab: id, since: Instant::now() });
                self.quick = true;
                return;
            }
            Some(id)
        } else {
            None
        };
        // The pages were read a moment ago, and what is shown changes under us: the user switches
        // tabs, and the browser shows a neighbour as soon as a tab closes, while its old page may
        // still be exposed for a few milliseconds. So the page is looked at again just before, and
        // only closed if it is still there as it was.
        // (A page that came from the address bar is looked at again there.)
        let unchanged = self.pages(browser).is_ok_and(|now| now.iter().any(|p| p.url == page.url && p.title == page.title))
            || (page.title.is_empty() && self.address_bar_page(browser).ok().flatten().is_some_and(|bar| bar.url == page.url));
        if !unchanged {
            self.pending.remove(&handle);
            return;
        }
        let started = Instant::now();
        let closed = close_tab(&self.automation, strip, &browser.window, &page.title, expected.as_deref());
        self.slow("closing a tab", started);
        self.pending.remove(&handle);
        match closed {
            Ok(()) => {
                // How long it took from first seeing the page here, to see where the time goes.
                let seen = self
                    .first_seen
                    .remove(&handle)
                    .map_or(String::new(), |(_, at)| format!(", {} ms after it was first seen", at.elapsed().as_millis()));
                let how = if page.title.is_empty() { "from the address bar" } else { "from the page" };
                note("app", &format!("closed {} ({}, {how}{seen})", page.url, self.woken.describe()));
                self.closed_one();
            }
            Err(e) => {
                eprintln!("tab_closer: failed to close {}: {e}", page.url);
                // The cached strip may have gone stale: look for it again.
                self.layouts.remove(&handle);
            }
        }
    }

    /// Closes the Firefox tabs whose page is blocked. Unlike Chromium, Firefox exposes the page of
    /// every tab, background ones too: each tab's root document is named like the tab and carries
    /// its URL, so a blocked page is closed whether or not it is shown.
    fn tick_firefox(&mut self, handle: isize, active: &Active) {
        let Some(window) = self.firefox(handle) else { return };
        match self.close_blocked_firefox_tab(&window, active) {
            Ok(Some(url)) => {
                note("app", &format!("closed {url} in Firefox ({})", self.woken.describe()));
                self.closed_one();
            }
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
        let mut read = Vec::new();
        for doc in window.find_all(TreeScope::Descendants, &is_document)? {
            let url = doc.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()).unwrap_or_default();
            read.push((doc.get_name().unwrap_or_default(), url, doc));
        }
        // Where a page is on screen and whether it is in a split view costs a few calls each, so
        // only when some page is blocked, split or not.
        if !read.iter().any(|(_, url, _)| active.blocks_firefox_page(url, false) || active.blocks_firefox_page(url, true)) {
            return Ok(None);
        }
        let documents: Vec<FirefoxPage> = read
            .into_iter()
            .map(|(title, url, doc)| FirefoxPage {
                url,
                title,
                split: in_split_view(&walker, &doc),
                left: doc.get_bounding_rectangle().map(|r| r.get_left()).unwrap_or_default(),
            })
            .collect();
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
        if pages.is_empty() {
            pages.extend(self.address_bar_page(browser)?);
        }
        Ok(pages)
    }

    /// The page the address bar shows, unless the user is typing in it. It has no title to check a tab by.
    fn address_bar_page(&self, browser: &Browser) -> uiautomation::Result<Option<Page>> {
        if browser.omnibox.has_keyboard_focus()? {
            return Ok(None);
        }
        let url = browser.omnibox.get_pattern::<UIValuePattern>()?.get_value()?;
        Ok(Some(Page { url, title: String::new() }))
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

/// The selected tabs of a window: in its tab strip, which is small, or if there are none there
/// (tabs outside the strip, a layout not seen before) in the whole window.
fn selected_tabs(a: &UIAutomation, strip: &UIElement, window: &UIElement) -> uiautomation::Result<Vec<UIElement>> {
    let cond = a.create_and_condition(
        is_a_tab(a)?,
        a.create_and_condition(
            a.create_property_condition(UIProperty::ControlType, (ControlType::TabItem as i32).into(), None)?,
            a.create_property_condition(UIProperty::SelectionItemIsSelected, true.into(), None)?,
        )?,
    )?;
    let in_strip = strip.find_all(TreeScope::Descendants, &cond)?;
    if in_strip.is_empty() {
        window.find_all(TreeScope::Descendants, &cond)
    } else {
        Ok(in_strip)
    }
}

/// Presses the Close button of a selected tab. With its id (`expected`, the tab that was selected
/// when a page was read from the address bar) it is that tab or none. Otherwise it is the one
/// showing `title`: in split view both tabs are selected and named "<title> - Left view" and
/// "<title> - Right view", so the title picks one, and a selected tab that is not named like it is
/// not the page that was read (the browser has moved on to another tab), and closing it would take down an
/// allowed tab.
fn close_tab(a: &UIAutomation, strip: &UIElement, window: &UIElement, title: &str, expected: Option<&[i32]>) -> uiautomation::Result<()> {
    let selected = selected_tabs(a, strip, window)?;
    let tab = match expected {
        Some(id) => selected.iter().find(|t| t.get_runtime_id().is_ok_and(|now| now == id)),
        None => selected.iter().find(|t| !title.is_empty() && t.get_name().is_ok_and(|n| n.starts_with(title))),
    }
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
            blocks: vec![Enforced::new(
                Arc::new(Lists {
                    sites: FilterList::parse(sites).unwrap(),
                    programs: ProgramList::parse(programs).unwrap(),
                    ..Lists::default()
                }),
                Enforcers::default(),
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
            let app = !pages.is_empty() && matches!(closer.probe(&browser.window).0, Seen::NoTabs);
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
