//! The window's texts (`src/strings.js`), for the service and the CLI: one file of words, so the window,
//! a session's status (`status.rs`) and the command line say the same. The file is built into the
//! program and read once; `t` looks a text up as the page's `t` does (`core.js`).
//!
//! The file is a JavaScript object of objects and double-quoted strings with unquoted keys and `//`
//! comments, nothing else (`read` refuses anything more, and a test reads the real file).

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// A text, or a group of them by key (a text by number is a group with `one` and `other`).
#[derive(Debug, PartialEq)]
enum Node {
    Text(String),
    Group(BTreeMap<String, Node>),
}

const FILE: &str = include_str!("../../src/strings.js");

fn texts() -> &'static Node {
    static TEXTS: OnceLock<Node> = OnceLock::new();
    // A file that cannot be read is a build that must not ship: the test below reads this one.
    TEXTS.get_or_init(|| read(FILE).unwrap_or_else(|_| Node::Group(BTreeMap::new())))
}

/// The text at `path` (`sessions.now.active`) with `{name}` placeholders filled from `values`; one with
/// `one` and `other` is chosen by the value `n` ("1" or not). A missing text is its path, as in the
/// window; a placeholder without a value stays as it is (`{condition}`, made a link by the window).
pub fn t(path: &str, values: &[(&str, &str)]) -> String {
    let found = path.split('.').try_fold(texts(), |node, key| match node {
        Node::Group(group) => group.get(key),
        Node::Text(_) => None,
    });
    let text = match found {
        Some(Node::Text(text)) => text,
        Some(Node::Group(forms)) => {
            let one = values.iter().any(|&(key, value)| key == "n" && value == "1");
            match forms.get(if one { "one" } else { "other" }) {
                Some(Node::Text(text)) => text,
                _ => return path.to_string(),
            }
        }
        None => return path.to_string(),
    };
    values.iter().fold(text.clone(), |text, (key, value)| text.replace(&format!("{{{key}}}"), value))
}

/// The names joined as the window joins them: `A`, `A and B`, `A, B and C`.
pub fn joined(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{}{}{last}", rest.join(&t("common.separator", &[])), t("common.and", &[])),
    }
}

/// A name in quotes, as the window quotes one.
pub fn quoted(name: &str) -> String {
    t("common.quoted", &[("name", name)])
}

/// The object after `window.STRINGS =` in the file.
fn read(file: &str) -> Result<Node, String> {
    let start = file.find("window.STRINGS =").ok_or("no window.STRINGS")?;
    let mut reader = Reader { chars: file[start + "window.STRINGS =".len()..].chars().collect(), at: 0 };
    reader.value()
}

struct Reader {
    chars: Vec<char>,
    at: usize,
}

impl Reader {
    fn peek(&mut self) -> Option<char> {
        // Space and `//` comments between the parts.
        loop {
            match self.chars.get(self.at) {
                Some(c) if c.is_whitespace() => self.at += 1,
                Some('/') if self.chars.get(self.at + 1) == Some(&'/') => {
                    while self.chars.get(self.at).is_some_and(|&c| c != '\n') {
                        self.at += 1;
                    }
                }
                other => return other.copied(),
            }
        }
    }

    fn expect(&mut self, wanted: char) -> Result<(), String> {
        match self.peek() {
            Some(c) if c == wanted => {
                self.at += 1;
                Ok(())
            }
            other => Err(format!("expected {wanted:?} at {}, found {other:?}", self.at)),
        }
    }

    fn value(&mut self) -> Result<Node, String> {
        match self.peek() {
            Some('{') => self.group(),
            Some('"') => self.text().map(Node::Text),
            other => Err(format!("expected a text or a group at {}, found {other:?}", self.at)),
        }
    }

    fn group(&mut self) -> Result<Node, String> {
        self.expect('{')?;
        let mut group = BTreeMap::new();
        while self.peek() != Some('}') {
            let key = self.key()?;
            self.expect(':')?;
            group.insert(key, self.value()?);
            if self.peek() == Some(',') {
                self.at += 1;
            }
        }
        self.expect('}')?;
        Ok(Node::Group(group))
    }

    fn key(&mut self) -> Result<String, String> {
        let first = self.at;
        while self.chars.get(self.at).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') {
            self.at += 1;
        }
        if first == self.at {
            return Err(format!("expected a key at {}", self.at));
        }
        Ok(self.chars[first..self.at].iter().collect())
    }

    /// A double-quoted text: the file has no escapes, so a `\` is refused rather than misread.
    fn text(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut text = String::new();
        loop {
            match self.chars.get(self.at) {
                Some('"') => {
                    self.at += 1;
                    return Ok(text);
                }
                Some('\\') => return Err(format!("an escape at {}: not read", self.at)),
                Some(&c) => {
                    text.push(c);
                    self.at += 1;
                }
                None => return Err("a text without its end".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_file_is_read() {
        assert!(read(FILE).is_ok(), "{:?}", read(FILE).err());
        assert_eq!(t("app.name", &[]), "Bullseye");
    }

    #[test]
    fn texts_are_filled_in_and_counted() {
        let file = r#"// A comment, and one after a value.
            window.STRINGS = {
              a: { b: "Hi {name}, see http://x", // the address is no comment
                n: { one: "{n} item", other: "{n} items" } },
              c: "C",
            };"#;
        let node = read(file).unwrap();
        let Node::Group(top) = &node else { panic!("not a group") };
        let Node::Group(a) = &top["a"] else { panic!("not a group") };
        assert_eq!(a["b"], Node::Text("Hi {name}, see http://x".into()));
        // The real file's lookups: placeholders, one and other, a missing text, one left open.
        assert_eq!(t("common.quoted", &[("name", "Work")]), "“Work”");
        assert_eq!(t("sessions.now.itemsUsedUp", &[("n", "1")]), "1 item used up");
        assert_eq!(t("sessions.now.itemsUsedUp", &[("n", "2")]), "2 items used up");
        assert_eq!(t("no.such.text", &[]), "no.such.text");
        assert_eq!(t("app", &[]), "app");
        assert_eq!(t("app.name.deeper", &[]), "app.name.deeper");
        assert_eq!(t("sessions.now.by", &[]), "By {condition}");
    }

    #[test]
    fn names_are_joined_and_quoted_as_the_window_does() {
        let names = |list: &[&str]| list.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(joined(&[]), "");
        assert_eq!(joined(&names(&["A"])), "A");
        assert_eq!(joined(&names(&["A", "B"])), "A and B");
        assert_eq!(joined(&names(&["A", "B", "C"])), "A, B and C");
        assert_eq!(quoted("A"), "“A”");
    }

    #[test]
    fn what_is_not_this_files_kind_is_refused() {
        for bad in [
            "",
            "window.STRINGS = [];",
            "window.STRINGS = { a: 1 };",
            "window.STRINGS = { : \"x\" };",
            "window.STRINGS = { a \"x\" };",
            "window.STRINGS = { a: \"x\\\"y\" };",
            "window.STRINGS = { a: \"x",
            "window.STRINGS = { a: \"x\"",
        ] {
            assert!(read(bad).is_err(), "{bad:?}");
        }
    }

    /// Every text the Rust code asks for is in the file (a missing one would show as its path).
    #[test]
    fn every_text_asked_for_is_there() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(std::path::Path::new(file!()).parent().unwrap());
        let mut asked = 0;
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()).collect();
        files.extend(std::fs::read_dir(dir.join("cli")).unwrap().map(|entry| entry.unwrap().path()));
        // The code, not its tests (which ask for missing texts on purpose).
        for file in files.iter().filter(|file| !file.ends_with("tests.rs")) {
            let source = std::fs::read_to_string(file).unwrap_or_default();
            let source = source.split("#[cfg(test)]").next().unwrap_or_default();
            for (at, _) in source.match_indices("t(\"") {
                let path: String = source[at + 3..].chars().take_while(|&c| c != '"').collect();
                // Paths only: `t("…")` with a dot in it (not `format!("…")` and the like).
                if !path.contains('.')
                    || path.contains(' ')
                    || path.contains('{')
                    || source[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_')
                {
                    continue;
                }
                asked += 1;
                let found = path.split('.').try_fold(texts(), |node, key| match node {
                    Node::Group(group) => group.get(key),
                    Node::Text(_) => None,
                });
                assert!(found.is_some(), "{path} is not in strings.js");
            }
        }
        assert!(asked > 10, "only {asked} found in {dir:?}");
    }
}
