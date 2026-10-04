//! Blocked-site list: one address per line, `*` as wildcard.
//!
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

    pub fn is_blocked(&self, url: &str) -> bool {
        self.matchers.is_match(&normalize(url))
    }
}

/// Lowercases and strips scheme, `www.` and trailing slashes.
fn normalize(url: &str) -> String {
    let url = url.trim().to_lowercase();
    let url = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    url.strip_prefix("www.").unwrap_or(url).trim_end_matches('/').to_string()
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
