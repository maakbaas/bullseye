//! VS Code's built-in browser (Browser, Ctrl+Alt+/): its pages are editor tabs of VS Code's own window,
//! which has Chromium's window class but no address bar, so `chromium.rs` would leave it alone.
//!
//! Every editor tab is in UI Automation, background tabs too, with a label that names a browser tab's
//! page: `Title (https://nos.nl/artikel/…)`. A blocked one is closed with its own Close button, which is
//! there without hovering. The tab strips (`tabs-container`, one per editor group) are searched for once
//! (that takes ~150 ms in a big window) and kept; reading their tabs then takes a few milliseconds.

use super::ledger::tab_key;
use super::win::program_file;
use super::{explain, Closer, Window, RECHECK};
use crate::session::Active;
use std::time::Instant;
use uiautomation::patterns::UIInvokePattern;
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::UIElement;

/// VS Code's programs, by lower case file name: the release, Insiders, and VSCodium (the same code).
const PROGRAMS: [&str; 3] = ["code.exe", "code - insiders.exe", "vscodium.exe"];

/// Whether a program (its lower case file name) is VS Code (`PROGRAMS`).
fn is_vscode_program(file: &str) -> bool {
    PROGRAMS.contains(&file)
}

/// The page a VS Code editor tab shows, from its label (`Title (https://…)`): the address in the last
/// brackets, if it is a web page. Other tabs (a file, `README.md (Working Tree)`, a preview) have none.
pub(super) fn page_of_label(label: &str) -> Option<&str> {
    let inner = label.strip_suffix(')')?;
    let start = inner.rfind(" (http").map(|at| at + 2).or_else(|| inner.starts_with("(http").then_some(1))?;
    let url = &inner[start..];
    (url.starts_with("https://") || url.starts_with("http://")).then_some(url)
}

impl Closer {
    /// The editor tab strips of a VS Code window, or None when the window is not VS Code's. Kept, and
    /// searched for again after `RECHECK` (a new editor group has a strip of its own).
    pub(super) fn vscode_strips(&mut self, handle: isize) -> Option<Vec<UIElement>> {
        match self.windows.get(&handle) {
            Some(Window::VsCode(strips, since)) if since.elapsed() < RECHECK => return Some(strips.clone()),
            Some(Window::VsCode(..)) => {}
            Some(Window::Browser(_) | Window::Firefox(_)) => return None,
            Some(Window::Other(since)) if since.elapsed() < RECHECK => return None,
            _ if !program_file(handle).is_some_and(|file| is_vscode_program(&file)) => return None,
            _ => {}
        }
        let started = Instant::now();
        let found = (|| {
            let window = self.automation.element_from_handle(Handle::from(handle))?;
            let tab = self.automation.create_property_condition(UIProperty::ControlType, (ControlType::Tab as i32).into(), None)?;
            let class = self.automation.create_property_condition(UIProperty::ClassName, "tabs-container".into(), None)?;
            window.find_all(TreeScope::Descendants, &self.automation.create_and_condition(tab, class)?)
        })();
        self.slow("looking for VS Code's tab strips", started);
        // None found (yet) is no error: a window with no editor open has none.
        let strips = found.unwrap_or_default();
        self.windows.insert(handle, Window::VsCode(strips.clone(), Instant::now()));
        Some(strips)
    }

    /// Closes the browser tabs of a VS Code window whose page is blocked.
    pub(super) fn tick_vscode(&mut self, handle: isize, strips: &[UIElement], active: &Active) {
        let started = Instant::now();
        let mut blocked = Vec::new();
        let Ok(all) = self.automation.create_true_condition() else { return };
        for strip in strips {
            let Ok(tabs) = strip.find_all(TreeScope::Children, &all) else {
                // The strip went (its editor group was closed): look again from the start next pass.
                self.forget(handle);
                return;
            };
            for tab in tabs {
                // A browser tab's name ends with its site in brackets; the label in it has the whole address.
                if !tab.get_name().is_ok_and(|name| name.ends_with(')')) {
                    continue;
                }
                let label = self
                    .automation
                    .create_property_condition(UIProperty::ControlType, (ControlType::Group as i32).into(), None)
                    .and_then(|group| tab.find_first(TreeScope::Children, &group))
                    .and_then(|label| label.get_name());
                if let Some(url) = label.ok().as_deref().and_then(page_of_label).filter(|url| active.blocks_page(url)) {
                    blocked.push((tab, url.to_string()));
                }
            }
        }
        self.slow("reading VS Code's tabs", started);
        if blocked.is_empty() {
            self.ledger.clear_window(handle);
        }
        for (tab, url) in blocked {
            let key = tab_key(&tab.get_runtime_id().unwrap_or_default());
            let what = format!("{url} in VS Code's browser");
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_tab_is_told_by_the_address_in_its_label() {
        let label = "Rusland gaat diesel leveren (https://nos.nl/artikel/2634362-rusland)";
        assert_eq!(page_of_label(label), Some("https://nos.nl/artikel/2634362-rusland"));
        // Brackets in the title are not the address; the last ones are.
        assert_eq!(page_of_label("A (small) title (http://example.com/a)"), Some("http://example.com/a"));
        // A page with no title is its address alone.
        assert_eq!(page_of_label("(https://example.com/)"), Some("https://example.com/"));
        // Files, diffs, previews and a local page in the browser are no web page.
        for other in [
            "README.md",
            "AGENTS.md (Working Tree) (AGENTS.md), preview",
            "Preview README.md",
            "Strings (/c%3A/Users/x/review.html)",
            "x (httpd.conf)",
        ] {
            assert_eq!(page_of_label(other), None, "{other}");
        }
    }

    #[test]
    fn vs_code_is_known_by_its_programs() {
        assert!(PROGRAMS.iter().all(|file| is_vscode_program(file)));
        assert!(!is_vscode_program("chrome.exe") && !is_vscode_program("spotify.exe"));
    }

    /// The tabs named after a site, in VS Code's windows, by name.
    fn vscode_tabs_on(closer: &mut Closer, site: &str) -> Vec<String> {
        let mut names = Vec::new();
        for (handle, _) in super::super::win::browser_windows() {
            for strip in closer.vscode_strips(handle).unwrap_or_default() {
                let all = closer.automation.create_true_condition().unwrap();
                for tab in strip.find_all(TreeScope::Children, &all).unwrap_or_default() {
                    names.extend(tab.get_name().ok().filter(|name| name.ends_with(&format!("({site})"))));
                }
            }
        }
        names
    }

    /// A manual check against a real VS Code: open a page of nos.nl in its browser (Ctrl+Alt+/), then
    /// `cargo test -- --ignored a_real_vs_code_browser_tab_is_closed --nocapture`. The tab must be closed
    /// by the passes, which also say how long one takes.
    #[test]
    #[ignore]
    fn a_real_vs_code_browser_tab_is_closed() {
        use crate::filters::FilterList;
        use crate::lists::Lists;
        use crate::session::Enforced;
        use crate::state::Actions;
        use std::sync::Arc;
        use std::time::Duration;
        let mut closer = Closer::new(uiautomation::UIAutomation::new().unwrap());
        let tabs = vscode_tabs_on(&mut closer, "nos.nl");
        println!("VS Code tabs on nos.nl: {tabs:?}");
        assert!(!tabs.is_empty(), "open nos.nl in VS Code's browser first");
        let quiet = Active {
            sessions: vec![Enforced::new(
                Arc::new(Lists { sites: FilterList::parse(&["nothing.invalid"]).unwrap(), ..Lists::default() }),
                Actions::default(),
            )],
            used: Default::default(),
        };
        let started = Instant::now();
        closer.tick(&quiet, None);
        println!("a pass with nothing to close: {:?}", started.elapsed());
        let session = Active {
            sessions: vec![Enforced::new(
                Arc::new(Lists { sites: FilterList::parse(&["nos.nl"]).unwrap(), ..Lists::default() }),
                Actions::default(),
            )],
            used: Default::default(),
        };
        let started = Instant::now();
        let closed = (0..20).any(|_| {
            closer.tick(&session, None);
            std::thread::sleep(Duration::from_millis(250));
            vscode_tabs_on(&mut closer, "nos.nl").is_empty()
        });
        println!("closed: {closed} after {:?}", started.elapsed());
        assert!(closed, "the nos.nl tab should have been closed");
    }
}
