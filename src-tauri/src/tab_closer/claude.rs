//! Claude's desktop app and its built-in browser: a pane beside the chat with tabs of its own, in
//! Claude's window (an Electron app, Chromium's window class, no address bar `chromium.rs` knows).
//!
//! Each page Claude shows is a document that carries its address: Claude's own shell (`file:`), the
//! chat (`https://claude.ai/…`) and the browser's pages, a few levels down its window. They are found
//! every pass by walking down to them, never into one (searching the whole window took 120-250 ms;
//! the hidden window Chromium keeps for each page is not there while Claude is behind another). Only
//! while a page that is not Claude's own is loaded are the browser's controls looked for, in the chat's
//! page (they are part of it: a toolbar named `Browser controls`, beside it the `Page URL` field and the
//! `Browser tabs` strip). The field names the page the pane shows; a page stays loaded when the pane is
//! closed, so a page alone is no reason to act.
//!
//! A tab has no Close button: it is closed by giving it the focus and pressing Delete, as Claude says,
//! and the focus is given back on the next pass. Giving it the focus brings Claude's window to the
//! front, which is where it is anyway when a tab is shown (the user's decision: one way, the tab's).

use super::ledger::tab_key;
use super::win::{is_foreground, press_delete, program_file};
use super::{explain, Closer, Window};
use crate::session::Active;
use std::time::{Duration, Instant};
use uiautomation::patterns::{UISelectionItemPattern, UIValuePattern};
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::UIElement;

/// Claude's program, by lower case file name.
const PROGRAM: &str = "claude.exe";

/// How long the browser's controls, once not found while a page is loaded (the pane is closed, or
/// Electron is still building its tree), are left before they are looked for again: the search is
/// tens of milliseconds in the chat's page.
const CONTROLS_AGAIN: Duration = Duration::from_secs(2);

/// How far down Claude's window its pages are looked for: they are about ten levels down.
const PAGE_DEPTH: usize = 16;

/// What is kept of a Claude window.
#[derive(Default)]
pub(super) struct Claude {
    /// The browser's controls, while its pane is open.
    controls: Option<Controls>,
    /// When the controls were last looked for and not found.
    missed: Option<Instant>,
    /// What had the focus before a tab was given it to be closed, to give it back on the next pass.
    refocus: Option<UIElement>,
}

/// The browser pane's controls.
struct Controls {
    /// The `Page URL` field: the page the pane shows.
    url: UIElement,
    /// The `Browser tabs` strip.
    strip: UIElement,
}

/// Whether a program (its lower case file name) is Claude's desktop app.
fn is_claude_program(file: &str) -> bool {
    file == PROGRAM
}

/// Whether a page is Claude's own (its shell, the chat, an empty tab), not one shown in its browser.
fn is_own_page(url: &str) -> bool {
    url.is_empty() || ["file:", "about:", "devtools:", "chrome:", "https://claude.ai/"].iter().any(|own| url.starts_with(own))
}

/// Whether a page is the chat, in which the browser's controls are.
fn is_chat(url: &str) -> bool {
    url.starts_with("https://claude.ai/")
}

impl Closer {
    /// Whether a window is Claude's; noted the first time, so its program is asked for once.
    pub(super) fn is_claude(&mut self, handle: isize) -> bool {
        match self.windows.get(&handle) {
            Some(Window::Claude(_)) => true,
            Some(_) => false,
            None if program_file(handle).is_some_and(|file| is_claude_program(&file)) => {
                self.windows.insert(handle, Window::Claude(Box::default()));
                true
            }
            None => false,
        }
    }

    /// Closes what Claude's browser shows of a blocked site.
    pub(super) fn tick_claude(&mut self, handle: isize, active: &Active) {
        let Some(Window::Claude(mut claude)) = self.windows.remove(&handle) else { return };
        if let Some(focus) = claude.refocus.take() {
            if is_foreground(handle) {
                let _ = focus.set_focus();
            }
        }
        self.claude_pass(handle, &mut claude, active);
        self.windows.insert(handle, Window::Claude(claude));
    }

    fn claude_pass(&mut self, handle: isize, claude: &mut Claude, active: &Active) {
        let started = Instant::now();
        let pages = self.claude_pages(handle);
        self.slow("reading Claude's pages", started);
        // Nothing but Claude's own pages: the browser shows nothing.
        if pages.iter().all(|(_, url)| is_own_page(url)) {
            claude.controls = None;
            self.ledger.clear_window(handle);
            return;
        }
        let shown = match claude.controls.as_ref().map(|c| (c.url.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()), c)) {
            Some((Ok(url), controls)) => Some((url, controls.url.has_keyboard_focus().unwrap_or(false))),
            // The pane was closed: its controls are gone.
            Some((Err(_), _)) => {
                claude.controls = None;
                None
            }
            None => None,
        };
        let (url, typing) = match shown {
            Some(shown) => shown,
            None => {
                if claude.missed.is_some_and(|at| at.elapsed() < CONTROLS_AGAIN) {
                    return;
                }
                let chats: Vec<UIElement> = pages.iter().filter(|(_, url)| is_chat(url)).map(|(page, _)| page.clone()).collect();
                let started = Instant::now();
                claude.controls = chats.iter().find_map(|chat| self.claude_controls(chat));
                self.slow("looking for Claude's browser", started);
                claude.missed = claude.controls.is_none().then(Instant::now);
                let Some(controls) = &claude.controls else { return };
                let Ok(url) = controls.url.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()) else { return };
                (url, controls.url.has_keyboard_focus().unwrap_or(false))
            }
        };
        // An address being typed is not a page yet.
        if typing || is_own_page(&url) || !active.blocks_page(&url) {
            self.ledger.clear_window(handle);
            return;
        }
        let Some(controls) = &claude.controls else { return };
        let what = format!("{url} in Claude's browser");
        let Some(tab) = self.claude_selected_tab(&controls.strip) else {
            return self.fail(&format!("could not close {what}: no tab is selected"));
        };
        let key = tab_key(&tab.get_runtime_id().unwrap_or_default());
        if !self.may_try(handle, &key, &what) {
            return;
        }
        let before = self.automation.get_focused_element().ok();
        match tab.set_focus() {
            Ok(()) => {
                press_delete(handle);
                // What had the focus in Claude gets it back next pass, once Delete has been taken.
                claude.refocus = before.filter(|focus| !self.automation.compare_elements(focus, &tab).unwrap_or(false));
                self.tried(handle, &key);
                self.closed_one(handle);
            }
            Err(e) => self.fail(&format!("could not close {what}: {}", explain(&e))),
        }
    }

    /// The pages of a Claude window, each with its address: its tree walked from the top, never into a
    /// page (the chat's alone is thousands of elements). The pages are under a few plain views.
    fn claude_pages(&self, handle: isize) -> Vec<(UIElement, String)> {
        let mut pages = Vec::new();
        let (Ok(window), Ok(all)) = (self.automation.element_from_handle(Handle::from(handle)), self.automation.create_true_condition())
        else {
            return pages;
        };
        let mut level = vec![window];
        for _ in 0..PAGE_DEPTH {
            let mut next = Vec::new();
            for node in level {
                for child in node.find_all(TreeScope::Children, &all).unwrap_or_default() {
                    if child.get_automation_id().is_ok_and(|id| id == "RootWebArea") {
                        if let Ok(url) = child.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()) {
                            pages.push((child, url));
                        }
                    } else {
                        next.push(child);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            level = next;
        }
        pages
    }

    /// The browser's controls in the chat's page, if its pane is open.
    fn claude_controls(&self, chat: &UIElement) -> Option<Controls> {
        let a = &self.automation;
        let condition = |name: &str, kind: ControlType| {
            let name = a.create_property_condition(UIProperty::Name, name.into(), None)?;
            let kind = a.create_property_condition(UIProperty::ControlType, (kind as i32).into(), None)?;
            a.create_and_condition(name, kind)
        };
        let toolbar = chat.find_first(TreeScope::Descendants, &condition("Browser controls", ControlType::ToolBar).ok()?).ok()?;
        // The address field is beside the toolbar, not in it.
        let pane = a.get_control_view_walker().ok()?.get_parent(&toolbar).ok()?;
        Some(Controls {
            url: pane.find_first(TreeScope::Descendants, &condition("Page URL", ControlType::Edit).ok()?).ok()?,
            strip: toolbar.find_first(TreeScope::Children, &condition("Browser tabs", ControlType::Tab).ok()?).ok()?,
        })
    }

    /// The tab the browser shows.
    fn claude_selected_tab(&self, strip: &UIElement) -> Option<UIElement> {
        let item = self.automation.create_property_condition(UIProperty::ControlType, (ControlType::TabItem as i32).into(), None).ok()?;
        let tabs = strip.find_all(TreeScope::Children, &item).ok()?;
        tabs.into_iter().find(|tab| tab.get_pattern::<UISelectionItemPattern>().and_then(|s| s.is_selected()).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_is_known_by_its_program() {
        assert!(is_claude_program("claude.exe"));
        assert!(!is_claude_program("orca.exe") && !is_claude_program("claude-helper.exe") && !is_claude_program(""));
    }

    #[test]
    fn claudes_own_pages_are_not_the_browsers() {
        // Its shell, the chat and an empty tab.
        for own in [
            "file:///C:/Program%20Files/WindowsApps/Claude_4.4212.0.0_x64__pzs8sxrjxfjjc/app/resources/app.asar/.vite/renderer/main_window/index.html",
            "https://claude.ai/chat/9e123887-93fd-4490-a0e6-f0c8b24f5ec7",
            "https://claude.ai/new",
            "about:blank",
            "devtools://devtools/bundled/devtools_app.html",
            "",
        ] {
            assert!(is_own_page(own), "{own}");
        }
        for shown in ["https://tweakers.net/", "https://www.google.com/", "http://claude.ai.example.com/", "https://claude.aix/"] {
            assert!(!is_own_page(shown), "{shown}");
        }
        assert!(is_chat("https://claude.ai/chat/1") && !is_chat("https://tweakers.net/") && !is_chat("file:///x"));
    }

    /// What Claude's browser pane shows, in Claude's windows: the address in its field, if it is open.
    fn shown(closer: &mut Closer) -> Vec<String> {
        let mut shown = Vec::new();
        for (handle, _) in super::super::win::browser_windows() {
            if !closer.is_claude(handle) {
                continue;
            }
            let pages = closer.claude_pages(handle);
            for (chat, _) in pages.iter().filter(|(_, url)| is_chat(url)) {
                if let Some(controls) = closer.claude_controls(chat) {
                    shown.extend(controls.url.get_pattern::<UIValuePattern>().and_then(|v| v.get_value()).ok());
                }
            }
        }
        shown
    }

    /// A manual check against Claude's desktop app: `cargo test -- --ignored a_real_claude_browser_page_is_closed
    /// --nocapture`, and show tweakers.net in Claude's browser within five minutes, beside another tab.
    /// Its tab must be closed, and only it.
    #[test]
    #[ignore]
    fn a_real_claude_browser_page_is_closed() {
        use crate::filters::FilterList;
        use crate::lists::Lists;
        use crate::session::Enforced;
        use crate::state::Actions;
        use std::sync::Arc;
        let mut closer = Closer::new(uiautomation::UIAutomation::new().unwrap());
        let on_tweakers = |closer: &mut Closer| shown(closer).iter().any(|url| url.contains("tweakers.net"));
        let waited = Instant::now();
        while !on_tweakers(&mut closer) {
            assert!(waited.elapsed() < Duration::from_secs(300), "show tweakers.net in Claude's browser");
            std::thread::sleep(Duration::from_secs(1));
        }
        let session = Active {
            sessions: vec![Enforced::new(
                Arc::new(Lists { sites: FilterList::parse(&["tweakers.net"]).unwrap(), ..Lists::default() }),
                Actions::default(),
            )],
            used: Default::default(),
        };
        let started = Instant::now();
        let closed = (0..20).any(|_| {
            closer.tick(&session, None);
            std::thread::sleep(Duration::from_millis(250));
            !on_tweakers(&mut closer)
        });
        println!("closed: {closed} after {:?}; the pane now shows {:?}", started.elapsed(), shown(&mut closer));
        assert!(closed, "tweakers.net should have been closed");
        assert!(!shown(&mut closer).is_empty(), "only its tab: the pane, with the other tab, stays");
        let started = Instant::now();
        for _ in 0..20 {
            closer.tick(&session, None);
        }
        println!("each later pass: {:?}", started.elapsed() / 20);
    }
}
