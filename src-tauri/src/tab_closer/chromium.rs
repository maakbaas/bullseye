//! Chromium browsers: Chrome, Edge, Brave, Opera, Vivaldi and the rest (they share a window class, with
//! Electron apps, which have no address bar and are skipped).
//!
//! A window's visible page(s) are read from its documents, or its address bar; a blocked one is closed
//! as a tab (its Close button, or a click where there is none: `win::may_middle_click`) or, in a window
//! with no tab strip (`--app`, a popup), as the whole window. Opera's sidebar apps and Vivaldi's panels
//! are windows or panels of their own: `close_panel`, `close_vivaldi_panel`.

use super::ledger::tab_key;
use super::win::{close_window, is_browser, may_middle_click, middle_click, program_file};
use super::{explain, Closer, Window, LOOK_AGAIN, RECHECK};
use crate::session::Active;
use crate::watchdog::note;
use std::time::{Duration, Instant};
use uiautomation::core::UICondition;
use uiautomation::patterns::{UIInvokePattern, UIValuePattern};
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::{UIAutomation, UIElement};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindow, PostMessageW, GW_OWNER, WM_CLOSE};

/// How long two reads of an address-bar page must be apart to count as two.
const SETTLE: Duration = Duration::from_millis(80);

/// A window with no tab strip is an app window only when that has been so for this long, on two
/// passes at least (`judge`).
const APP_AFTER: Duration = Duration::from_secs(1);

/// How a Chromium window is laid out. A window never changes from one to the other, so it is
/// worked out once and kept.
#[derive(Clone)]
pub(super) enum Layout {
    /// Has a tab strip (the element that holds the tabs): tabs are closed one at a time.
    Tabs(UIElement),
    /// Has none (`chrome --app=<url>`, a popup): its page goes with the whole window.
    App,
    /// Has none, and belongs to a program that is no browser (`BROWSERS`): an app built on Chromium
    /// that has an address bar of its own (Spotify's desktop app), which is left alone.
    NotABrowser,
}

/// What a look for tabs in a window found.
pub(super) enum Seen {
    Tabs,
    NoTabs,
    /// Windows could not be asked (busy, a timeout): says nothing.
    Unknown,
}

/// A window that showed no tabs, how many passes in a row and since when.
#[derive(Clone, Copy)]
pub(super) struct Doubt {
    pub(super) passes: u8,
    pub(super) since: Instant,
}

pub(super) enum Verdict {
    Tabs,
    App,
    /// Not known yet: what to remember of the doubt.
    Unsure(Option<Doubt>),
}

/// Whether a window is an app window, from what was seen of it. A failure to look is no evidence
/// (it used to count as "no tabs" and closed whole windows), and no tabs counts only when it has
/// been so on two passes at least and for `APP_AFTER`, as a window being built shows none for a moment.
pub(super) fn judge(doubt: Option<Doubt>, seen: Seen, now: Instant) -> Verdict {
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
pub(super) struct Pending {
    pub(super) url: String,
    pub(super) tab: Vec<i32>,
    pub(super) since: Instant,
}

/// The site of an address, as the address bar and a page's own address both give it: no scheme, no `www.`,
/// no path, lower case, so the two can be compared.
fn host_of(url: &str) -> String {
    let url = url.trim().to_lowercase();
    let rest = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// Whether the address bar names a site that none of the pages exposed is on: then it is ahead of them
/// (a page not loaded yet, an error page, a browser's own page that comes along) and counts. A page of
/// the site it names, as either half of a split view, is trusted instead.
fn names_another_site(pages: &[Page], bar_url: &str) -> bool {
    !pages.iter().any(|page| host_of(&page.url) == host_of(bar_url))
}

/// Whether what is seen now is what was seen before, long enough ago to count as a second look.
pub(super) fn agrees(pending: Option<&Pending>, url: &str, tab: &[i32], now: Instant) -> bool {
    pending.is_some_and(|p| p.url == url && p.tab == tab && now.duration_since(p.since) >= SETTLE)
}

#[derive(Clone)]
pub(super) struct Browser {
    /// The browser window itself.
    pub(super) window: UIElement,
    omnibox: UIElement,
    /// Parent of the visible pages (`MultiContentsView`); holds two pages in split view.
    contents: Option<UIElement>,
    /// Vivaldi, whose whole interface is a web page: its address bar, tab strip and sidebar panels
    /// are found by the classes of that page (`Closer::close_vivaldi_panel`).
    vivaldi: bool,
}

/// A page shown in a browser window.
pub(super) struct Page {
    pub(super) url: String,
    pub(super) title: String,
}

/// Window classes of a browser's tab strip entries: Chrome and Brave call them `Tab`, Edge calls
/// them `EdgeTab`, Opera `TabView` and Brave `BraveTab` (Brave puts its name before others' too:
/// `BraveOmniboxViewViews`, `BraveMultiContentsView`). The class is what tells a browser tab from a
/// tab-like widget inside a web page (an ARIA tablist), whose close button must never be pressed.
const TAB_CLASSES: [&str; 4] = ["Tab", "EdgeTab", "TabView", "BraveTab"];

/// The condition for an element that is a tab of the tab strip (`TAB_CLASSES`).
fn is_a_tab(a: &UIAutomation) -> uiautomation::Result<UICondition> {
    let class = |name: &str| a.create_property_condition(UIProperty::ClassName, name.into(), None);
    let mut any = class(TAB_CLASSES[0])?;
    for name in &TAB_CLASSES[1..] {
        any = a.create_or_condition(any, class(name)?)?;
    }
    Ok(any)
}

/// The selected tabs of a window: in its tab strip, which is small, or if there are none there
/// (tabs outside the strip, a layout not seen before) in the whole window.
fn selected_tabs(a: &UIAutomation, strip: &UIElement, window: &UIElement) -> uiautomation::Result<Vec<UIElement>> {
    let selected = a.create_and_condition(
        a.create_property_condition(UIProperty::ControlType, (ControlType::TabItem as i32).into(), None)?,
        a.create_property_condition(UIProperty::SelectionItemIsSelected, true.into(), None)?,
    )?;
    // In the strip every tab item is a tab (Vivaldi's have classes of their own); elsewhere in the window
    // only those of a browser's tab class, not the tabs of a web page.
    let in_strip = strip.find_all(TreeScope::Descendants, &selected)?;
    if in_strip.is_empty() {
        window.find_all(TreeScope::Descendants, &a.create_and_condition(is_a_tab(a)?, selected)?)
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
fn close_tab(
    a: &UIAutomation,
    handle: isize,
    strip: &UIElement,
    window: &UIElement,
    title: &str,
    expected: Option<&[i32]>,
) -> uiautomation::Result<()> {
    let selected = selected_tabs(a, strip, window)?;
    let tab = match expected {
        Some(id) => selected.iter().find(|t| t.get_runtime_id().is_ok_and(|now| now == id)),
        None => selected.iter().find(|t| !title.is_empty() && t.get_name().is_ok_and(|n| n.starts_with(title))),
    }
    .ok_or_else(|| uiautomation::Error::new(-1, "no selected tab with that title"))?;
    let button = a.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None)?;
    match tab.find_first(TreeScope::Children, &button) {
        Ok(button) => button.get_pattern::<UIInvokePattern>()?.invoke(),
        // Opera's and Vivaldi's tabs have no Close button to press (Opera draws it on hover only): a
        // middle click closes the tab there. Elsewhere a tab without a button is an error, said in the log.
        Err(_) if may_middle_click(handle) => middle_click(handle, tab),
        Err(_) => Err(uiautomation::Error::new(-1, "the tab has no Close button to press")),
    }
}

/// The first element of this class under `root`.
fn find_by_class(a: &UIAutomation, root: &UIElement, class: &str) -> Option<UIElement> {
    let class = a.create_property_condition(UIProperty::ClassName, class.into(), None).ok()?;
    root.find_first(TreeScope::Descendants, &class).ok()
}

/// The address of the page in a window or a panel: its first document (an iframe's is another, further in).
fn first_document_url(a: &UIAutomation, root: &UIElement) -> Option<String> {
    let document = a.create_property_condition(UIProperty::ControlType, (ControlType::Document as i32).into(), None).ok()?;
    let page = root.find_first(TreeScope::Descendants, &document).ok()?;
    Some(page.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()).unwrap_or_default())
}

/// The id of the one tab that is selected in a window, if exactly one is.
fn selected_tab_id(a: &UIAutomation, strip: &UIElement, window: &UIElement) -> Option<Vec<i32>> {
    match selected_tabs(a, strip, window).ok()?.as_slice() {
        [only] => only.get_runtime_id().ok(),
        _ => None,
    }
}

impl Closer {
    /// Looks at a Chromium window: its pages, and what to close in them.
    pub(super) fn tick_chromium(&mut self, handle: isize, active: &Active) {
        let Some(browser) = self.browser(handle) else {
            self.close_panel(handle, active);
            return;
        };
        // What kind of window this is is settled when it is first seen, once (`layout`): the window of a
        // program that is not a browser (Spotify's, an Electron app's with an address bar) is then left
        // alone for as long as it is open, and not read every pass. Until it is settled the pages are read
        // as before (a window being built shows no tabs for a moment).
        let layout = self.layout(handle, &browser);
        if matches!(layout, Some(Layout::NotABrowser)) {
            return;
        }
        if browser.vivaldi {
            self.close_vivaldi_panel(handle, &browser, active);
        }
        let started = Instant::now();
        let pages = self.pages(&browser);
        self.slow("reading the pages of a window", started);
        let Ok(pages) = pages else {
            // Cached elements went stale; look them up again next tick.
            self.forget(handle);
            return;
        };
        // An app window's page goes with its window; a normal window's blocked page (either half of
        // a split view, or an ordinary tab) is a tab to close.
        // The address bar is a quicker witness than a heavy page: the browser shows where a tab is
        // going before the page's accessibility tree is built (which takes long on a big page, and
        // is why such a page closed late). It counts only when it names another site than the page
        // that is exposed (an exposed page of the same site is trusted: the bar may show less of it),
        // and then goes the same way as a page read from the bar alone: seen twice, in the same
        // selected tab (`close_blocked_tab`).
        // (A page that is itself the bar's, or a document not loaded yet with no title, is no reason to
        // wait: the bar names the same host or another one, whichever it is. More than one page does not
        // matter either: a split view shows two, and a browser's own pages come along (an error page
        // shows with the new tab page's footer, `chrome://newtab-footer/`): the bar counts when none of
        // the pages is on the site it names.)
        let bar = self.address_bar_page(&browser).ok().flatten().filter(|bar| names_another_site(&pages, &bar.url));
        // Each page judged once: the same page is the app window's or the tab's, the bar only the tab's.
        let app_page = pages.iter().find(|p| active.blocks_page(&p.url));
        let tab_page = app_page.or_else(|| bar.as_ref().filter(|p| active.blocks_page(&p.url)));
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
            // Nothing blocked here: whatever was tried to close worked.
            self.pending.remove(&handle);
            self.ledger.clear_window(handle);
            return;
        }
        // Only now is it worth knowing what the window is.
        let Some(layout) = layout else {
            self.watch(handle, LOOK_AGAIN);
            return;
        };
        match layout {
            Layout::App => {
                if let Some(page) = app_page {
                    self.close_app_window(handle, &browser, &page.url);
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

    /// An app window's page goes with its window, as a click on its X does it.
    fn close_app_window(&mut self, handle: isize, browser: &Browser, url: &str) {
        if !self.may_try(handle, "window", &format!("the app window of {url}")) {
            return;
        }
        let closed = close_window(&browser.window);
        self.tried(handle, "window");
        match closed {
            Ok(()) => {
                note("app", &format!("closed the app window of {url} ({})", self.woken.describe()));
                self.closed_one(handle);
            }
            Err(e) => self.fail(&format!("could not close the app window of {url}: {}", explain(&e))),
        }
    }

    /// A site shown in a panel of a browser (Opera's sidebar apps: Instagram, WhatsApp): a window of
    /// its own, owned by the browser's, with no address bar and no tabs, whose page is the panel. It
    /// is closed whole, as a click on its X would. Only windows with an owner, of a browser's program;
    /// the page is the first document in it (an iframe's is another, further in).
    pub(super) fn close_panel(&mut self, handle: isize, active: &Active) {
        // SAFETY: a window handle Windows handed over a moment ago; a stale one only gives no owner.
        if unsafe { GetWindow(handle as HWND, GW_OWNER) }.is_null() || !is_browser(handle) {
            return;
        }
        let Ok(window) = self.automation.element_from_handle(Handle::from(handle)) else { return };
        let Some(url) = first_document_url(&self.automation, &window) else { return };
        if active.blocks_page(&url) && self.may_try(handle, "panel", &format!("the panel of {url}")) {
            // SAFETY: as above; posted, so a window that does not answer holds nothing up.
            unsafe { PostMessageW(handle as HWND, WM_CLOSE, 0, 0) };
            self.tried(handle, "panel");
            note("app", &format!("closed the panel of {url} ({})", self.woken.describe()));
            self.closed_one(handle);
        }
    }

    /// A site shown in one of Vivaldi's sidebar panels (a web panel): its page is in the window beside
    /// the tabs', in a `panel-collapse-guard`. The panel is not closed (it would be gone for any other
    /// page too) but switched to Windows and Tabs, by pressing that panel's button in the panel bar. If
    /// that one is not found (the bar is in another language), the first panel of Vivaldi's own is
    /// shown (its button has no address in its help text, as a web panel's has).
    pub(super) fn close_vivaldi_panel(&mut self, handle: isize, browser: &Browser, active: &Active) {
        let a = &self.automation;
        let Some(guard) = find_by_class(a, &browser.window, "panel-collapse-guard") else { return };
        let Some(url) = first_document_url(a, &guard) else { return };
        if !active.blocks_page(&url) {
            return;
        }
        let Ok(panel_button) = a.create_property_condition(UIProperty::AutomationId, "ui-region-panel_button".into(), None) else { return };
        let Ok(buttons) = browser.window.find_all(TreeScope::Descendants, &panel_button) else { return };
        let own = |b: &UIElement| b.get_help_text().is_ok_and(|help| !help.contains("://"));
        let target =
            buttons.iter().find(|b| b.get_name().is_ok_and(|name| name == "Windows and Tabs")).or_else(|| buttons.iter().find(|b| own(b)));
        let Some(target) = target else {
            self.fail(&format!("could not find a panel to switch to away from {url} in Vivaldi"));
            return;
        };
        let target = target.clone();
        if !self.may_try(handle, "panel", &format!("switching the panel away from {url}")) {
            return;
        }
        let switched = target.get_pattern::<UIInvokePattern>().and_then(|invoke| invoke.invoke());
        self.tried(handle, "panel");
        match switched {
            Ok(()) => {
                note("app", &format!("switched the panel away from {url} ({})", self.woken.describe()));
                self.closed_one(handle);
            }
            Err(e) => self.fail(&format!("could not switch the panel away from {url}: {}", explain(&e))),
        }
    }

    /// What kind of window this is, found once and kept: with a tab strip, or an app window. None
    /// while that is not known yet (see `judge`); then the pass is repeated soon.
    pub(super) fn layout(&mut self, handle: isize, browser: &Browser) -> Option<Layout> {
        if let Some(layout) = self.layouts.get(&handle) {
            return Some(layout.clone());
        }
        let started = Instant::now();
        let (seen, strip) = self.probe(&browser.window, browser.vivaldi);
        self.slow("looking for the tab strip of a window", started);
        let layout = match judge(self.doubts.get(&handle).copied(), seen, Instant::now()) {
            Verdict::Tabs => strip.map(Layout::Tabs),
            // Only a browser's own windows are app windows. Tabs are proof enough of a browser, but a
            // window with none is also what an app made of Chromium looks like (Spotify's has an address
            // bar too), so the program decides.
            Verdict::App if is_browser(handle) => Some(Layout::App),
            Verdict::App => {
                // Said once, as the window is settled: it is left alone from now on, and not read again.
                let title = browser.window.get_name().unwrap_or_default();
                let program = program_file(handle).unwrap_or_else(|| "a program that could not be told".to_string());
                note("app", &format!("left alone: “{title}” of {program}, a Chromium window that is not a browser's"));
                Some(Layout::NotABrowser)
            }
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
    pub(super) fn probe(&self, window: &UIElement, vivaldi: bool) -> (Seen, Option<UIElement>) {
        let Ok(tab) = is_a_tab(&self.automation) else { return (Seen::Unknown, None) };
        match window.find_all(TreeScope::Descendants, &tab) {
            Err(_) => (Seen::Unknown, None),
            Ok(tabs) => match tabs.first() {
                // Vivaldi's tabs have classes of their own (`tab-wrapper active ...`); its strip is `tab-strip`.
                None if vivaldi => {
                    let strip = self.automation.create_property_condition(UIProperty::ClassName, "tab-strip".into(), None);
                    match strip.and_then(|strip| window.find_first(TreeScope::Descendants, &strip)) {
                        Ok(strip) => (Seen::Tabs, Some(strip)),
                        Err(_) => (Seen::NoTabs, None),
                    }
                }
                None => (Seen::NoTabs, None),
                Some(first) => match self.automation.get_control_view_walker().and_then(|walker| walker.get_parent(first)) {
                    Ok(strip) => (Seen::Tabs, Some(strip)),
                    Err(_) => (Seen::Unknown, None),
                },
            },
        }
    }

    /// Closes the tab of a blocked page, only if it is still that page, and only that tab.
    pub(super) fn close_blocked_tab(&mut self, handle: isize, browser: &Browser, strip: &UIElement, page: &Page) {
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
                self.watch(handle, LOOK_AGAIN);
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
        // Whether an earlier try at this tab worked (`ledger.rs`): it is the tab with this id that was
        // closed, so one that is still selected after a moment was not. (A split view's tabs are told by
        // their titles, and no id says which: those are not checked.)
        let program = program_file(handle).unwrap_or_default();
        let id = expected.clone().or_else(|| selected_tab_id(&self.automation, strip, &browser.window));
        let key = id.map(|id| tab_key(&id));
        if let Some(key) = &key {
            if !self.may_try(handle, key, &format!("{} in {program}", page.url)) {
                return;
            }
        }
        let started = Instant::now();
        let closed = close_tab(&self.automation, handle, strip, &browser.window, &page.title, expected.as_deref());
        self.slow("closing a tab", started);
        if let Some(key) = &key {
            self.tried(handle, key);
        }
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
                self.closed_one(handle);
            }
            Err(e) => {
                self.fail(&format!("could not close {} in {program}: {}", page.url, explain(&e)));
                // The cached strip may have gone stale: look for it again.
                self.layouts.remove(&handle);
            }
        }
    }

    /// The window's cached browser elements, or None if it is not a browser window.
    pub(super) fn browser(&mut self, handle: isize) -> Option<Browser> {
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
        let find = |class: &str| find_by_class(&self.automation, &window, class);
        // Opera calls its address bar `AddressTextfieldView`.
        let views = find("OmniboxViewViews").or_else(|| find("BraveOmniboxViewViews")).or_else(|| find("AddressTextfieldView"));
        // Vivaldi's address bar is a field of its web page, whose classes are those of its style sheet.
        let vivaldi = || {
            if program_file(handle).as_deref() != Some("vivaldi.exe") {
                return None;
            }
            let edit = self.automation.create_property_condition(UIProperty::ControlType, (ControlType::Edit as i32).into(), None).ok()?;
            let fields = window.find_all(TreeScope::Descendants, &edit).ok()?;
            fields.into_iter().find(|f| f.get_classname().is_ok_and(|c| c.split(' ').any(|class| class == "vivaldi-addressfield")))
        };
        let (omnibox, is_vivaldi) = match views {
            Some(omnibox) => (omnibox, false),
            None => match vivaldi() {
                Some(omnibox) => (omnibox, true),
                None => {
                    self.windows.insert(handle, Window::Other(Instant::now()));
                    return None;
                }
            },
        };
        let contents = find("MultiContentsView").or_else(|| find("BraveMultiContentsView"));
        let browser = Browser { window, omnibox, contents, vivaldi: is_vivaldi };
        self.windows.insert(handle, Window::Browser(browser.clone()));
        Some(browser)
    }

    /// URLs of the pages visible in the window. Each page's root document carries its URL, which
    /// also covers the second page in split view. Falls back to the address bar (active page only)
    /// when the documents are not exposed, skipping it while the user types in it.
    pub(super) fn pages(&self, browser: &Browser) -> uiautomation::Result<Vec<Page>> {
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
    pub(super) fn address_bar_page(&self, browser: &Browser) -> uiautomation::Result<Option<Page>> {
        if browser.omnibox.has_keyboard_focus()? {
            return Ok(None);
        }
        let url = browser.omnibox.get_pattern::<UIValuePattern>()?.get_value()?;
        Ok(Some(Page { url, title: String::new() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str) -> Page {
        Page { url: url.to_string(), title: String::new() }
    }

    #[test]
    fn a_site_is_the_host_without_scheme_www_path_or_case() {
        assert_eq!(host_of("https://www.NRC.nl/nieuws/2025?x=1#top"), "nrc.nl");
        assert_eq!(host_of("nrc.nl"), "nrc.nl", "the address bar leaves the scheme out");
        assert_eq!(host_of("  http://nos.nl  "), "nos.nl");
        assert_eq!(host_of("nos.nl/"), "nos.nl");
        assert_eq!(host_of("https://sub.nos.nl/"), "sub.nos.nl", "only www. goes");
        assert_eq!(host_of("chrome-error://chromewebdata/"), "chromewebdata");
        assert_eq!(host_of(""), "");
    }

    #[test]
    fn the_address_bar_counts_only_when_no_page_is_on_the_site_it_names() {
        // A page of the site it names: the page is trusted (the bar may show less of it).
        assert!(!names_another_site(&[page("https://www.nrc.nl/")], "nrc.nl"));
        // A page not loaded yet, or still the last one: the bar is ahead.
        assert!(names_another_site(&[page("https://nos.nl/")], "nrc.nl"));
        assert!(names_another_site(&[page("")], "nrc.nl"));
        // An error page with the new tab page's footer beside it (two documents, not a split view):
        // the bar names the site, and that is what is blocked.
        assert!(names_another_site(&[page("chrome-error://chromewebdata/"), page("chrome://newtab-footer/")], "nrc.nl"));
        // A split view: either half on the site it names is trusted, neither is not.
        assert!(!names_another_site(&[page("https://nos.nl/"), page("https://nrc.nl/x")], "nrc.nl"));
        assert!(names_another_site(&[page("https://nos.nl/"), page("https://tweakers.net/")], "nrc.nl"));
        // No page exposed at all: the bar is the one witness.
        assert!(names_another_site(&[], "nrc.nl"));
    }
}
