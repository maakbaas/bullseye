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
//! Each of these is an enforcer the block can turn on or off (`state::Enforcers`): blocked
//! tabs, blocked pages in a split view (a window showing two pages), and Obsidian.

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::client::Remote;
use crate::filters::FilterList;
use crate::state::Enforcers;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use uiautomation::patterns::{UIInvokePattern, UIValuePattern, UIWindowPattern};
use uiautomation::types::{ControlType, TreeScope, UIProperty};
use uiautomation::{UIAutomation, UIElement};

const INTERVAL: Duration = Duration::from_secs(1);
const RECHECK: Duration = Duration::from_secs(10);

pub fn spawn(remote: Remote) {
    thread::spawn(move || {
        let automation = match UIAutomation::new() {
            Ok(a) => a,
            Err(e) => return eprintln!("tab_closer: UI Automation unavailable: {e}"),
        };
        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new() };
        loop {
            let start = Instant::now();
            if let Some((filters, enforcers)) = remote.active() {
                if enforcers.use_block_list() {
                    if let Err(e) = closer.tick(&filters, enforcers) {
                        eprintln!("tab_closer: {e}");
                    }
                }
                if enforcers.obsidian_web_viewer {
                    closer.close_obsidian_with_web_content();
                }
            }
            thread::sleep(INTERVAL.saturating_sub(start.elapsed()));
        }
    });
}

struct Closer {
    automation: UIAutomation,
    /// Window runtime id -> its cached UI elements, so they are not searched for every second.
    windows: HashMap<Vec<i32>, Window>,
    system: System,
}

enum Window {
    Browser(Browser),
    /// Not a browser window (Electron app, devtools, popup); searched again after `RECHECK`.
    Other(Instant),
}

#[derive(Clone)]
struct Browser {
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
    fn tick(&mut self, filters: &FilterList, enforcers: Enforcers) -> uiautomation::Result<()> {
        // Window class shared by all Chromium-based browsers (and Electron apps, which have
        // no address bar and are skipped in `browser`).
        let cond = self.automation.create_property_condition(UIProperty::ClassName, "Chrome_WidgetWin_1".into(), None)?;
        let windows = self.automation.get_root_element()?.find_all(TreeScope::Children, &cond)?;

        let mut seen = Vec::new();
        for window in windows {
            let Ok(id) = window.get_runtime_id() else { continue };
            seen.push(id.clone());
            let Some(browser) = self.browser(&window, &id) else { continue };
            let Ok(pages) = self.pages(&browser) else {
                // Cached elements went stale; look them up again next tick.
                self.windows.remove(&id);
                continue;
            };
            // Two pages at once is a split view; one is an ordinary tab.
            let enforced = if pages.len() > 1 { enforcers.chromium_split_view } else { enforcers.chromium_tabs };
            if !enforced {
                continue;
            }
            if let Some(page) = pages.iter().find(|p| filters.is_blocked(&p.url)) {
                match close_tab(&self.automation, &window, &page.title) {
                    Ok(()) => eprintln!("tab_closer: closed {}", page.url),
                    Err(e) => eprintln!("tab_closer: failed to close {}: {e}", page.url),
                }
            }
        }
        self.windows.retain(|id, _| seen.contains(id));
        Ok(())
    }

    /// Closes Obsidian (like clicking its X) when it hosts web content. Obsidian's own UI runs in
    /// an unsandboxed renderer (it needs Node); web viewer pages run in sandboxed ones. Ending
    /// those renderers instead makes Obsidian quit on the next navigation, so close it cleanly.
    fn close_obsidian_with_web_content(&mut self) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet),
        );
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

    /// The window's cached browser elements, or None if it is not a browser window.
    fn browser(&mut self, window: &UIElement, id: &Vec<i32>) -> Option<Browser> {
        match self.windows.get(id) {
            Some(Window::Browser(b)) => return Some(b.clone()),
            Some(Window::Other(since)) if since.elapsed() < RECHECK => return None,
            _ => {}
        }
        let find = |class: &str| {
            let cond = self.automation.create_property_condition(UIProperty::ClassName, class.into(), None).ok()?;
            window.find_first(TreeScope::Descendants, &cond).ok()
        };
        let Some(omnibox) = find("OmniboxViewViews") else {
            self.windows.insert(id.clone(), Window::Other(Instant::now()));
            return None;
        };
        let browser = Browser { omnibox, contents: find("MultiContentsView") };
        self.windows.insert(id.clone(), Window::Browser(browser.clone()));
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

/// Presses the Close button of the selected tab showing `title`. In split view both tabs are
/// selected and named "<title> - Left view" / "<title> - Right view", so the title picks one;
/// with a single selected tab (or no title) the first selected tab is closed.
fn close_tab(a: &UIAutomation, window: &UIElement, title: &str) -> uiautomation::Result<()> {
    let cond = a.create_and_condition(
        a.create_property_condition(UIProperty::ClassName, "Tab".into(), None)?,
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
