//! A list of sites: one address per line, `*` as wildcard. Used twice: for the sites to block, and
//! for the sites that are always allowed, which win over the block list (`block::Active::blocks_site`).
//!
//! - Only web pages count. The browser's own pages (`chrome://settings`, `edge://newtab`,
//!   `about:blank`), extensions and local files are never matched, so even a lone `*`, which
//!   matches every site, leaves the browser itself usable.
//! - `https://`, `www.` and letter case are ignored.
//! - An entry blocks everything that continues it at a break (`/ ? # & :`):
//!   `tweakers.net` blocks `tweakers.net/nieuws?x=1`, not `tweakers.network`.
//! - `*` matches any characters: within the domain it stops at `/` (`*.tweakers.net` covers
//!   subdomains), in the path it spans folders (`tweakers.net/*/a.html` covers `/x/y/a.html`).

use regex::RegexSet;

pub struct FilterList {
    /// The normalized entries, kept so a block can be written to disk and resumed (`state.rs`).
    entries: Vec<String>,
    matchers: RegexSet,
}

impl Default for FilterList {
    fn default() -> FilterList {
        FilterList { entries: Vec::new(), matchers: RegexSet::empty() }
    }
}

impl FilterList {
    /// Parses one entry per line, ignoring empty lines. Errors list the invalid lines.
    pub fn parse<S: AsRef<str>>(lines: &[S]) -> Result<FilterList, Vec<String>> {
        let trimmed: Vec<&str> = lines.iter().map(|l| l.as_ref().trim()).filter(|l| !l.is_empty()).collect();
        // URLs never contain whitespace, so such an entry could never match.
        let errors: Vec<String> = trimmed.iter().filter(|e| e.contains(char::is_whitespace)).map(|e| e.to_string()).collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        let entries: Vec<String> = trimmed.iter().map(|e| normalize(e)).collect();
        Ok(FilterList { matchers: RegexSet::new(entries.iter().map(|e| to_regex(e))).unwrap(), entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The blocked entries as written down, which are normalized (see `normalize`) and so match
    /// exactly what they matched before.
    pub fn entries(&self) -> Vec<String> {
        self.entries.clone()
    }

    /// Whether the address is a web page that an entry of this list matches.
    pub fn is_blocked(&self, url: &str) -> bool {
        is_website(url) && self.matchers.is_match(&normalize(url))
    }
}

/// Lowercases and strips scheme, `www.` and trailing slashes.
fn normalize(url: &str) -> String {
    let url = url.trim().to_lowercase();
    let rest = split_scheme(&url).1;
    rest.strip_prefix("www.").unwrap_or(rest).trim_end_matches('/').to_string()
}

/// The scheme of an address (`https`) and the rest of it, if it starts with a scheme. Only text that
/// looks like one counts: an address with none (the address bar leaves it off) can still have a
/// `://` further on, in its query, and must not be cut there.
fn split_scheme(url: &str) -> (Option<&str>, &str) {
    match url.split_once("://") {
        Some((scheme, rest)) if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) => {
            (Some(scheme), rest)
        }
        _ => (None, url),
    }
}

/// Whether the address is a page on the web, which is all these lists are about. Not the browser's
/// own pages, extensions, local files or an empty address. An address without a scheme counts: the
/// address bar leaves it off.
fn is_website(url: &str) -> bool {
    let url = url.trim().to_lowercase();
    if url.is_empty() {
        return false;
    }
    match split_scheme(&url) {
        (Some(scheme), _) => matches!(scheme, "http" | "https" | "ftp" | "ws" | "wss"),
        (None, rest) => !["about:", "data:", "javascript:", "blob:", "view-source:"].iter().any(|p| rest.starts_with(p)),
    }
}

fn to_regex(entry: &str) -> String {
    let (host, path) = entry.split_at(entry.find('/').unwrap_or(entry.len()));
    let wildcard = |part: &str, any: &str| part.split('*').map(regex::escape).collect::<Vec<_>>().join(any);
    format!("^{}{}(?:[/?#&:].*)?$", wildcard(host, "[^/]*"), wildcard(path, ".*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(entry: &str, url: &str) -> bool {
        FilterList::parse(&[entry]).unwrap().is_blocked(url)
    }

    #[test]
    fn site() {
        assert!(blocks("tweakers.net", "https://tweakers.net/"));
        assert!(blocks("https://www.Tweakers.net/", "https://www.tweakers.net/nieuws?x=1"));
        assert!(blocks("tweakers.net", "tweakers.net/nieuws"));
        assert!(!blocks("tweakers.net", "https://tweakers.network/"));
        assert!(!blocks("tweakers.net", "https://gathering.tweakers.net/"));
        assert!(!blocks("tweakers.net", "https://google.com/search?q=tweakers.net"));
    }

    #[test]
    fn folder_and_page() {
        assert!(blocks("tweakers.net/folder", "https://tweakers.net/folder/sub/page.html"));
        assert!(!blocks("tweakers.net/folder", "https://tweakers.net/folderx"));
        assert!(blocks("tweakers.net/index.html", "https://tweakers.net/index.html?a=1&b=2"));
        assert!(!blocks("tweakers.net/index.html", "https://tweakers.net/index.html5"));
    }

    #[test]
    fn wildcards() {
        assert!(blocks("*.tweakers.net", "https://gathering.tweakers.net/forum"));
        assert!(blocks("*.tweakers.net", "https://a.b.tweakers.net/"));
        assert!(!blocks("*.tweakers.net", "https://tweakers.net/"));
        assert!(!blocks("*.tweakers.net", "https://evil.com/x.tweakers.net"));

        assert!(blocks("tweakers.net/*/a.html", "https://tweakers.net/test/a.html?x=1"));
        assert!(blocks("tweakers.net/*/a.html", "https://tweakers.net/x/y/a.html"));
        assert!(!blocks("tweakers.net/*/a.html", "https://tweakers.net/test/b.html"));
        assert!(blocks("tweakers.net/test*/index.html", "https://tweakers.net/test123/index.html"));
        assert!(!blocks("tweakers.net/test*/index.html", "https://tweakers.net/other/index.html"));
    }

    #[test]
    fn a_lone_star_blocks_every_site_but_not_the_browser_itself() {
        let everything = FilterList::parse(&["*"]).unwrap();
        for url in [
            "https://youtube.com/watch?v=1",
            "http://example.org/",
            "https://sub.domain.example.co.uk/a/b?c=d#e",
            "https://www.tweakers.net/",
            "localhost:3000/app",
            "youtube.com/watch?v=1",
            "example.com",
            "ftp://files.example.com/pub",
        ] {
            assert!(everything.is_blocked(url), "{url}");
        }
        // The browser's own pages, extensions, local files and an empty address are not sites.
        for url in [
            "chrome://settings",
            "chrome://newtab/",
            "edge://newtab/",
            "brave://settings/",
            "about:blank",
            "chrome-extension://abcdef/popup.html",
            "devtools://devtools/bundled/inspector.html",
            "view-source:https://example.com/",
            "file:///C:/Users/me/page.html",
            "data:text/html,hello",
            "",
            "   ",
        ] {
            assert!(!everything.is_blocked(url), "{url:?}");
        }
    }

    #[test]
    fn only_a_real_scheme_is_cut_off() {
        // An address without a scheme can have a "://" in its query: that is not its scheme.
        assert!(!blocks("tweakers.net", "google.com/url?q=https://tweakers.net/nieuws"));
        assert!(blocks("google.com", "google.com/url?q=https://tweakers.net/nieuws"));
        assert!(!blocks("tweakers.net", "https://google.com/url?q=https://tweakers.net/nieuws"));
        // Real schemes still go, whatever they are written like.
        assert!(blocks("tweakers.net", "HTTPS://tweakers.net/"));
        assert!(blocks("tweakers.net", "http://tweakers.net/x"));
        assert!(blocks("tweakers.net", "wss://tweakers.net/socket"));
    }

    #[test]
    fn invalid_and_empty() {
        assert_eq!(FilterList::parse(&["tweakers .net"]).err(), Some(vec!["tweakers .net".to_string()]));
        assert!(FilterList::parse(&["", "  "]).unwrap().is_empty());
    }

    /// The entries are written down and parsed again on the next run (`block.rs`), so writing
    /// them down must not change what they block.
    #[test]
    fn entries_survive_being_written_down_and_read_again() {
        let urls = ["https://tweakers.net/nieuws?x=1", "https://gathering.tweakers.net/f", "https://google.com/"];
        let list = FilterList::parse(&["  HTTPS://WWW.Tweakers.NET/  ", "*.tweakers.net", "tweakers.net/nieuws"]).unwrap();
        assert_eq!(list.entries(), vec!["tweakers.net", "*.tweakers.net", "tweakers.net/nieuws"]);
        let again = FilterList::parse(&list.entries()).unwrap();
        assert_eq!(again.entries(), list.entries(), "normalizing must be a fixed point");
        for url in urls {
            assert_eq!(list.is_blocked(url), again.is_blocked(url), "{url}");
        }
        // A folder entry spans everything below it, so this one does block the sub-path.
        assert!(again.is_blocked("https://tweakers.net/nieuws/x"));
    }
}
