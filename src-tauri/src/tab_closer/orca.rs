//! Orca's built-in browser: its pages are tabs of Orca's own window (an Electron app, Chromium's window
//! class, no address bar `chromium.rs` knows), side by side with its terminal tabs.
//!
//! Orca's interface is a web page of its own (a `RootWebArea` document); a browser tab that is shown
//! is a page nested in it, a `RootWebArea` of its own whose value is its address. A tab in the
//! background has no page in UI Automation, only its title in the tab strip, so it is closed once it
//! is shown, as in Chromium. A pane (one per group of a split view) holds a tab group, whose strip has
//! one active tab (its class has `bg-tab-active`), and the page it shows: a blocked page's tab is the
//! active one of the nearest pane around it. It is closed with its own Close button, which is there
//! without hovering. Searching for the pages inside Orca's own page takes a few milliseconds.

use super::ledger::tab_key;
use super::win::program_file;
use super::{explain, Closer, Window};
use crate::session::Active;
use std::time::{Duration, Instant};
use uiautomation::patterns::{UIInvokePattern, UIValuePattern};
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::UIElement;

/// Orca's program, by lower case file name.
const PROGRAM: &str = "orca.exe";

/// How long an Orca window whose own page is not found (Electron builds its tree only once UI
/// Automation asks, or it is Orca's developer tools) is left before it is looked at again.
const NO_PAGE_AGAIN: Duration = Duration::from_secs(5);

/// How far up from a page its pane is looked for: the page is about ten levels below it.
const PANE_DEPTH: usize = 16;

/// Whether a program (its lower case file name) is Orca.
fn is_orca_program(file: &str) -> bool {
    file == PROGRAM
}

/// Whether an element's classes (space separated, as Orca's web page has them) include this one.
fn has_class(classes: &str, class: &str) -> bool {
    classes.split_whitespace().any(|c| c == class)
}

/// Whether an element is a pane's tab group (Orca's `group/tab-group`).
fn is_tab_group(classes: &str) -> bool {
    has_class(classes, "group/tab-group")
}

/// Whether a tab of Orca's strip is the one shown in its pane (`bg-tab-active`; the others have
/// `bg-tab-inactive`).
fn is_active_tab(classes: &str) -> bool {
    has_class(classes, "bg-tab-active")
}

impl Closer {
    /// Orca's own page in an Orca window, or None when the window is not Orca's. Some(None): Orca's,
    /// with nothing to look at for now (`NO_PAGE_AGAIN`).
    pub(super) fn orca_app(&mut self, handle: isize) -> Option<Option<UIElement>> {
        match self.windows.get(&handle) {
            Some(Window::Orca(Some(app), _)) => Some(Some(app.clone())),
            Some(Window::Orca(None, since)) if since.elapsed() < NO_PAGE_AGAIN => Some(None),
            Some(Window::Orca(None, _)) => Some(self.orca_page(handle)),
            Some(_) => None,
            None if program_file(handle).is_some_and(|file| is_orca_program(&file)) => Some(self.orca_page(handle)),
            None => None,
        }
    }

    /// Finds and keeps Orca's own page in its window; a window without one is noted as Orca's, with
    /// nothing to look at, until `NO_PAGE_AGAIN`.
    fn orca_page(&mut self, handle: isize) -> Option<UIElement> {
        let started = Instant::now();
        let page = (|| {
            let window = self.automation.element_from_handle(Handle::from(handle))?;
            let id = self.automation.create_property_condition(UIProperty::AutomationId, "RootWebArea".into(), None)?;
            window.find_first(TreeScope::Descendants, &id)
        })()
        .ok();
        self.slow("looking for Orca's page", started);
        self.windows.insert(handle, Window::Orca(page.clone(), Instant::now()));
        page
    }

    /// Closes the browser tabs of an Orca window whose page is shown and blocked.
    pub(super) fn tick_orca(&mut self, handle: isize, app: &UIElement, active: &Active) {
        let started = Instant::now();
        let found = self
            .automation
            .create_property_condition(UIProperty::AutomationId, "RootWebArea".into(), None)
            .and_then(|id| app.find_all(TreeScope::Descendants, &id));
        self.slow("reading Orca's pages", started);
        let pages = match found {
            Ok(pages) => pages,
            // None at all is no error (no browser tab shown); an error is Orca's page that went stale.
            Err(e) if e.code() == 0 => Vec::new(),
            Err(_) => return self.forget(handle),
        };
        let blocked: Vec<(UIElement, String)> = pages
            .into_iter()
            .filter_map(|page| {
                let url = page.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()).ok()?;
                active.blocks_page(&url).then_some((page, url))
            })
            .collect();
        if blocked.is_empty() {
            self.ledger.clear_window(handle);
        }
        for (page, url) in blocked {
            let what = format!("{url} in Orca's browser");
            let Some(tab) = self.orca_tab_of(&page, app) else {
                self.fail(&format!("could not close {what}: its tab was not found"));
                continue;
            };
            let key = tab_key(&tab.get_runtime_id().unwrap_or_default());
            if !self.may_try(handle, &key, &what) {
                continue;
            }
            let closed = self
                .automation
                .create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None)
                .and_then(|button| tab.find_first(TreeScope::Descendants, &button))
                .and_then(|button| button.get_pattern::<UIInvokePattern>()?.invoke());
            match closed {
                Ok(()) => {
                    self.tried(handle, &key);
                    self.closed_one(handle);
                }
                Err(e) => self.fail(&format!("could not close {what}: {}", explain(&e))),
            }
        }
    }

    /// The tab that shows a page: the active tab of the nearest pane around it, the first element up
    /// from the page with a tab group among its children.
    fn orca_tab_of(&self, page: &UIElement, app: &UIElement) -> Option<UIElement> {
        let walker = self.automation.get_control_view_walker().ok()?;
        let all = self.automation.create_true_condition().ok()?;
        let button = self.automation.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None).ok()?;
        let mut node = walker.get_parent(page).ok()?;
        for _ in 0..PANE_DEPTH {
            if self.automation.compare_elements(&node, app).unwrap_or(false) {
                return None;
            }
            let groups = node.find_all(TreeScope::Children, &all).unwrap_or_default();
            if let Some(group) = groups.iter().find(|child| child.get_classname().is_ok_and(|c| is_tab_group(&c))) {
                let tabs = group.find_all(TreeScope::Descendants, &button).unwrap_or_default();
                return tabs.into_iter().find(|tab| tab.get_classname().is_ok_and(|c| is_active_tab(&c)));
            }
            node = walker.get_parent(&node).ok()?;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orca_is_known_by_its_program() {
        assert!(is_orca_program("orca.exe"));
        assert!(!is_orca_program("code.exe") && !is_orca_program("orca-helper.exe") && !is_orca_program("chrome.exe"));
    }

    #[test]
    fn the_shown_tab_is_told_by_its_class() {
        // As Orca has them: the shown tab, and one in the background.
        let shown = "w-full group relative flex items-center h-full px-1.5 text-xs border-t border-r border-border  bg-tab-active text-tab-active-foreground";
        let background = "w-full group relative flex items-center h-full border-border  bg-tab-inactive text-tab-inactive-foreground hover:text-tab-active-foreground";
        assert!(is_active_tab(shown));
        assert!(!is_active_tab(background), "bg-tab-inactive and hover:text-tab-active-foreground are not it");
        assert!(!is_active_tab(""));
    }

    #[test]
    fn a_pane_is_told_by_its_tab_group() {
        assert!(is_tab_group("group/tab-group relative flex flex-col flex-1 min-w-0 min-h-0"));
        // The strip inside it, and the group of its tabs' hover, are not.
        assert!(!is_tab_group("group/tab-strip relative flex min-h-0 w-max"));
        assert!(!is_tab_group("group-hover/tab-group:opacity-100 flex"));
        assert!(!is_tab_group("absolute inset-0 flex"));
    }

    /// The tabs of Orca's windows, by name.
    fn orca_tabs(closer: &mut Closer) -> Vec<String> {
        let mut names = Vec::new();
        for (handle, _) in super::super::win::browser_windows() {
            let Some(Some(app)) = closer.orca_app(handle) else { continue };
            let button =
                closer.automation.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None).unwrap();
            for tab in app.find_all(TreeScope::Descendants, &button).unwrap_or_default() {
                if tab.get_classname().is_ok_and(|c| has_class(&c, "bg-tab-active") || has_class(&c, "bg-tab-inactive")) {
                    names.extend(tab.get_name().ok());
                }
            }
        }
        names
    }

    /// A manual check against a real Orca: `cargo test -- --ignored a_real_orca_browser_tab_is_closed
    /// --nocapture`, and show example.com in a browser tab of Orca within five minutes. The tab must be
    /// closed by the passes, which also say how long one takes.
    #[test]
    #[ignore]
    fn a_real_orca_browser_tab_is_closed() {
        use crate::filters::FilterList;
        use crate::lists::Lists;
        use crate::session::Enforced;
        use crate::state::Actions;
        use std::sync::Arc;
        let mut closer = Closer::new(uiautomation::UIAutomation::new().unwrap());
        let session = |site: &str| Active {
            sessions: vec![Enforced::new(
                Arc::new(Lists { sites: FilterList::parse(&[site]).unwrap(), ..Lists::default() }),
                Actions::default(),
            )],
            used: Default::default(),
        };
        // Electron builds its tree once asked; the tab may come later.
        let shown = |closer: &mut Closer| orca_tabs(closer).iter().any(|name| name.contains("Example Domain"));
        let waited = Instant::now();
        while !shown(&mut closer) {
            assert!(waited.elapsed() < Duration::from_secs(300), "show example.com in Orca's browser");
            std::thread::sleep(Duration::from_secs(1));
            closer.windows.clear();
        }
        let before = orca_tabs(&mut closer);
        println!("Orca's tabs: {before:?}");
        let quiet = session("nothing.invalid");
        closer.tick(&quiet, None);
        let started = Instant::now();
        for _ in 0..20 {
            closer.tick(&quiet, None);
        }
        println!("each pass with nothing to close: {:?}", started.elapsed() / 20);
        let blocking = session("example.com");
        let started = Instant::now();
        let closed = (0..20).any(|_| {
            closer.tick(&blocking, None);
            std::thread::sleep(Duration::from_millis(250));
            !shown(&mut closer)
        });
        println!("closed: {closed} after {:?}", started.elapsed());
        assert!(closed, "the example.com tab should have been closed");
        assert_eq!(orca_tabs(&mut closer).len(), before.len() - 1, "only that tab was closed");
    }
}
