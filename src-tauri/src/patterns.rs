//! A list of text patterns with `*` as wildcard, matched against a whole text and ignoring letter
//! case. Used for window titles (`*YouTube*`), and for the names of Store apps, which are given
//! exactly, without `*` (`lists::store_apps`).
//!
//! The whole text has to match, so `YouTube` is a window with exactly that title and `*YouTube*`
//! one with it anywhere in it, and a lone `*` every one (the user chose to allow it: refusing it let
//! `*e*` through all the same). What must never be closed is spared where the closing is done:
//! Bullseye's own windows and Windows' shell (`tab_closer.rs`), Windows' own Store apps
//! (`store_apps::is_protected`).

use regex::RegexSet;

pub struct Patterns {
    /// The entries as typed (trimmed), kept so a block can be written to disk and resumed.
    entries: Vec<String>,
    matchers: RegexSet,
}

impl Default for Patterns {
    fn default() -> Patterns {
        Patterns { entries: Vec::new(), matchers: RegexSet::empty() }
    }
}

impl Patterns {
    /// Parses one entry per line, ignoring empty lines. The error says in words what is wrong.
    pub fn parse<S: AsRef<str>>(lines: &[S]) -> Result<Patterns, String> {
        let entries: Vec<String> = lines.iter().map(|l| l.as_ref().trim().to_string()).filter(|l| !l.is_empty()).collect();
        let matchers = RegexSet::new(entries.iter().map(|e| to_regex(e))).map_err(|e| e.to_string())?;
        Ok(Patterns { entries, matchers })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> Vec<String> {
        self.entries.clone()
    }

    /// Whether the whole text matches any entry.
    pub fn is_match(&self, text: &str) -> bool {
        self.matchers.is_match(text)
    }
}

/// `*` becomes "anything", everything else is literal, and the whole text has to match; case is
/// ignored, and `*` also covers line breaks.
fn to_regex(entry: &str) -> String {
    let parts: Vec<String> = entry.split('*').map(regex::escape).collect();
    format!("(?is)^{}$", parts.join(".*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(entries: &[&str]) -> Patterns {
        Patterns::parse(entries).unwrap()
    }

    #[test]
    fn the_whole_text_matches_and_case_is_ignored() {
        let titles = list(&["YouTube", "  Inbox - *  "]);
        assert!(titles.is_match("youtube"));
        assert!(titles.is_match("INBOX - Mail"));
        assert!(!titles.is_match("YouTube - Chrome"));
        assert!(!titles.is_match("My YouTube"));
        assert_eq!(titles.entries(), vec!["YouTube", "Inbox - *"]);
    }

    #[test]
    fn wildcards_stand_for_any_characters() {
        let titles = list(&["*YouTube*", "a*b*c"]);
        assert!(titles.is_match("Cats - YouTube - Google Chrome"));
        assert!(titles.is_match("YouTube"));
        assert!(titles.is_match("a--b--c"));
        assert!(!titles.is_match("a--c"));
        // Everything else is literal.
        assert!(!list(&["a.c"]).is_match("abc"));
        assert!(list(&["(x)"]).is_match("(x)"));
    }

    #[test]
    fn empty_lines_are_ignored_and_a_lone_wildcard_matches_everything() {
        assert!(list(&["", "  "]).is_empty());
        assert!(!list(&["x"]).is_empty());
        assert!(list(&["*"]).is_match("Any window at all") && list(&["*"]).is_match(""));
        assert!(list(&["a", "**"]).is_match("Notes"));
        assert!(!Patterns::default().is_match("anything"));
    }
}
