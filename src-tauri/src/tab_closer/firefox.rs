//! Firefox and the browsers made of it (Zen, LibreWolf, Waterfox, Floorp: all have its window class).
//!
//! Unlike Chromium, Firefox exposes the page of every tab, background ones too: each tab's root
//! document is named like the tab and carries its URL, so a blocked page is closed whether or not it is
//! shown, and all the blocked ones in one pass.

use super::ledger::tab_key;
use super::win::{may_middle_click, middle_click, program_file};
use super::{explain, Closer, Window, RECHECK};
use crate::session::Active;
use crate::watchdog::note;
use std::time::Instant;
use uiautomation::patterns::{UIInvokePattern, UIValuePattern};
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::UIElement;

/// A blocked page of a Firefox window, and the tab that shows it.
struct Blocked {
    url: String,
    /// The tab's name when it was found: a tab no longer named so is not the one that was meant.
    name: String,
    element: UIElement,
    /// The tab's own id, which a later pass can tell it by (empty if it cannot be read).
    id: Vec<i32>,
}

impl Closer {
    /// Closes the Firefox tabs whose page is blocked.
    pub(super) fn tick_firefox(&mut self, handle: isize, active: &Active) {
        let Some(window) = self.firefox(handle) else { return };
        let blocked = match self.blocked_firefox_tabs(&window, active) {
            Ok(blocked) => blocked,
            Err(e) => {
                // The cached elements went stale; look again next tick.
                self.fail(&format!("could not read the pages of a Firefox window: {}", explain(&e)));
                self.windows.remove(&handle);
                return;
            }
        };
        if blocked.is_empty() {
            self.ledger.clear_window(handle);
            return;
        }
        let program = program_file(handle).unwrap_or_default();
        let Ok(button) = self.automation.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None)
        else {
            return;
        };
        // Every blocked tab in one pass: all were found in the one look above, so closing one does not
        // move the others (each is its own element). A tab that cannot be closed (pinned, or gone with
        // its window) is skipped, and one no longer named as it was is left alone.
        let mut closed = false;
        for tab in blocked {
            if tab.element.get_name().ok().as_deref() != Some(tab.name.as_str()) {
                continue;
            }
            let key = tab_key(&tab.id);
            if !tab.id.is_empty() && !self.may_try(handle, &key, &format!("{} in {program}", tab.url)) {
                continue;
            }
            // A tab's Close button is not always there (Zen shows it on hover only): a click closes it in
            // the browsers where that is so (`may_middle_click`), elsewhere it is said.
            let invoked = match tab.element.find_first(TreeScope::Children, &button) {
                Ok(button) => button.get_pattern::<UIInvokePattern>().and_then(|invoke| invoke.invoke()),
                Err(_) if may_middle_click(handle) => middle_click(handle, &tab.element),
                Err(_) => Err(uiautomation::Error::new(-1, "the tab has no Close button to press")),
            };
            if !tab.id.is_empty() {
                self.tried(handle, &key);
            }
            match invoked {
                Ok(()) => {
                    note("app", &format!("closed {} in {program} ({})", tab.url, self.woken.describe()));
                    closed = true;
                }
                Err(e) => self.fail(&format!("could not close {} in {program}: {}", tab.url, explain(&e))),
            }
        }
        if closed {
            self.closed_one(handle);
        }
    }

    /// The tabs of a Firefox window whose page is blocked.
    fn blocked_firefox_tabs(&self, window: &UIElement, active: &Active) -> uiautomation::Result<Vec<Blocked>> {
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
        let blocked: Vec<bool> = read.iter().map(|(_, url, _)| active.blocks_page(url)).collect();
        if !blocked.contains(&true) {
            return Ok(Vec::new());
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
        let mut found = Vec::new();
        for (index, page) in documents.iter().enumerate() {
            if page.title.is_empty() || !blocked[index] {
                continue;
            }
            let Some(tab) = firefox_tab_of(&documents, index, &names) else { continue };
            found.push(Blocked {
                url: page.url.clone(),
                name: names[tab].clone(),
                element: tabs[tab].clone(),
                id: tabs[tab].get_runtime_id().unwrap_or_default(),
            });
        }
        Ok(found)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn page(title: &str, split: bool, left: i32) -> FirefoxPage {
        FirefoxPage { url: format!("https://{}.example/", title.replace(' ', "-")), title: title.to_string(), split, left }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_page_goes_with_the_tab_that_is_named_like_it() {
        let pages = [page("Alpha", false, 0), page("Beta", false, 0)];
        let tabs = names(&["Beta", "Alpha"]);
        assert_eq!(firefox_tab_of(&pages, 0, &tabs), Some(1));
        assert_eq!(firefox_tab_of(&pages, 1, &tabs), Some(0));
        // A tab that says more than the title (a playing tab: "<title>, Playing") is still its tab.
        assert_eq!(firefox_tab_of(&pages, 0, &names(&["Alpha, Playing"])), Some(0));
        // No tab of that name: none, never another.
        assert_eq!(firefox_tab_of(&pages, 0, &names(&["Gamma", "Alphabet"])), None);
        assert_eq!(firefox_tab_of(&pages, 0, &[]), None);
    }

    #[test]
    fn pages_with_one_title_go_with_the_tabs_with_it_in_order() {
        let pages = [page("Same", false, 0), page("Other", false, 0), page("Same", false, 0)];
        let tabs = names(&["Same", "Other", "Same"]);
        assert_eq!(firefox_tab_of(&pages, 0, &tabs), Some(0));
        assert_eq!(firefox_tab_of(&pages, 2, &tabs), Some(2), "the second of the same title is the second tab of it");
        // One tab of that title less than there are pages of it: the last page has none.
        assert_eq!(firefox_tab_of(&pages, 2, &names(&["Same", "Other"])), None);
    }

    #[test]
    fn the_two_tabs_of_a_split_view_are_found_by_their_names_ending_alike() {
        // The halves are named "<title>, split view left" and "... right", next to each other.
        let tabs = names(&["Home", "News, split view left", "Video, split view right", "Mail"]);
        assert_eq!(split_tabs(&tabs), Some(1));
        let pages = [page("News", true, 0), page("Video", true, 960)];
        assert_eq!(firefox_tab_of(&pages, 0, &tabs), Some(1), "the left half is the first of the pair");
        assert_eq!(firefox_tab_of(&pages, 1, &tabs), Some(2), "the right half the next");
        // A page that is not in the split view goes by its title, as ever.
        let mixed = [page("Home", false, 0), page("News", true, 0), page("Video", true, 960)];
        assert_eq!(firefox_tab_of(&mixed, 0, &tabs), Some(0));
        assert_eq!(firefox_tab_of(&mixed, 2, &tabs), Some(2));
    }

    #[test]
    fn a_split_view_is_found_only_when_there_is_exactly_one_pair() {
        // None: names without the shared ending, or all alike.
        assert_eq!(split_tabs(&names(&["A", "B", "C"])), None);
        assert_eq!(split_tabs(&names(&["A, one", "B, one"])), None, "the same last word is not a pair of halves");
        assert_eq!(split_tabs(&names(&["A, left", "B, right"])), None, "nothing shared before the last word");
        assert_eq!(split_tabs(&names(&[])), None);
        assert_eq!(split_tabs(&names(&["Only, split view left"])), None);
        // Two pairs (two split views in one window): not told apart, so none.
        let two = names(&["A, split view left", "B, split view right", "C, split view left", "D, split view right"]);
        assert_eq!(split_tabs(&two), None);
        // Then a page of a split view goes by its title, never by a guess between the pairs.
        let pages = [page("A", true, 0), page("B", true, 960)];
        assert_eq!(firefox_tab_of(&pages, 0, &names(&["A, split view left", "B, split view right", "C, split view left"])), Some(0));
        assert_eq!(firefox_tab_of(&pages, 1, &two), Some(1));
        // A different last word on both sides of a pair, in another language, is still a pair.
        assert_eq!(split_tabs(&names(&["X, vue partagée gauche", "Y, vue partagée droite"])), Some(0));
    }
}
