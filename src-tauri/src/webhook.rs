//! A webhook condition (`condition::Rule::Webhook`): active or not by what a URL answers. The
//! service asks it every `interval` (at least a minute) with GET or POST, and the answer is judged
//! by two matchers: if the "true" matcher matches, the condition is active; if not, the "false"
//! matcher is looked at, and if it matches the condition is not active; if neither does (or no
//! answer came, or the last one is too old: three intervals) the `fallback` says. There are no
//! headers or tokens: a single URL, which can carry GET parameters. Timeout, redirects (followed, at
//! most five) and TLS (certificates are checked) have fixed, sensible values.
//!
//! `Rule::active` is asked every second by running blocks, so it only reads what the poller thread
//! last found out (`answer`); nothing there waits for the network. The poller starts with the first
//! question and stops asking about a webhook nobody asks about any more (`IDLE`).

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::sync::LockExt;

/// The shortest time between two requests: a minute.
pub const MIN_INTERVAL: u64 = 60;
const TIMEOUT: Duration = Duration::from_secs(10);
/// How much of an answer is read: a body longer than this is cut (and so does not equal anything).
const MAX_BODY: u64 = 64 * 1024;
/// A webhook nobody has asked about for this long is not polled any more.
const IDLE: Duration = Duration::from_secs(600);
/// An answer older than this many intervals is as good as none.
const STALE_AFTER: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    #[default]
    Get,
    Post,
}

/// What a matcher looks at.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    /// The HTTP status code only.
    #[default]
    Status,
    /// The body only, which has to equal the text (surrounding white space aside).
    Body,
    /// Both.
    Both,
}

/// One of the two matchers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Matcher {
    pub on: Match,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub body: Option<String>,
}

impl Matcher {
    fn needs_status(&self) -> bool {
        matches!(self.on, Match::Status | Match::Both)
    }

    fn needs_body(&self) -> bool {
        matches!(self.on, Match::Body | Match::Both)
    }

    fn check(&self, what: &str) -> Result<(), String> {
        if self.needs_status() && !self.status.is_some_and(|code| (100..=599).contains(&code)) {
            return Err(format!("Give the HTTP status code (100 to 599) of the {what} matcher"));
        }
        if self.needs_body() && self.body.as_deref().is_none_or(|body| body.trim().is_empty()) {
            return Err(format!("Give the body text of the {what} matcher"));
        }
        Ok(())
    }

    fn matches(&self, status: u16, body: &str) -> bool {
        let status_fits = !self.needs_status() || self.status == Some(status);
        let body_fits = !self.needs_body() || self.body.as_deref().is_some_and(|expected| expected.trim() == body.trim());
        status_fits && body_fits
    }
}

/// What is asked and how it is judged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    pub url: String,
    #[serde(default)]
    pub method: Method,
    /// Seconds between requests, at least `MIN_INTERVAL`.
    pub interval: u64,
    /// Whether the condition is active when the answer cannot be told.
    pub fallback: bool,
    pub when_true: Matcher,
    pub when_false: Matcher,
}

/// What an answer makes of the condition.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    True,
    False,
    /// Neither matcher matched: the fallback.
    Neither,
}

impl Hook {
    /// Checked when it is saved: a web address, the interval at least a minute, what each matcher needs.
    pub fn check(&self) -> Result<(), String> {
        let url = self.url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() < 9 || url.contains(char::is_whitespace) {
            return Err("Give the web address, starting with http:// or https://".into());
        }
        if self.interval < MIN_INTERVAL {
            return Err("Ask at most once a minute: the interval is at least 1m".into());
        }
        self.when_true.check("true")?;
        self.when_false.check("false")
    }

    /// What an answer with this status and body means: the "true" matcher first, then the "false" one.
    pub fn judge(&self, status: u16, body: &str) -> Outcome {
        if self.when_true.matches(status, body) {
            Outcome::True
        } else if self.when_false.matches(status, body) {
            Outcome::False
        } else {
            Outcome::Neither
        }
    }

    fn active_by(&self, outcome: Option<Outcome>) -> bool {
        match outcome {
            Some(Outcome::True) => true,
            Some(Outcome::False) => false,
            _ => self.fallback,
        }
    }
}

/// One request: the status code and the (cut) body, or why there was no answer.
pub fn fetch(hook: &Hook) -> Result<(u16, String), String> {
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).redirects(5).user_agent("bullseye").build();
    let url = hook.url.trim();
    let result = match hook.method {
        Method::Get => agent.get(url).call(),
        Method::Post => agent.post(url).send_string(""),
    };
    let response = match result {
        Ok(response) => response,
        // An answer with an error status is still an answer.
        Err(ureq::Error::Status(_, response)) => response,
        Err(e) => return Err(e.to_string()),
    };
    let status = response.status();
    let mut body = String::new();
    response.into_reader().take(MAX_BODY).read_to_string(&mut body).map_err(|e| e.to_string())?;
    Ok((status, body))
}

/// What one test request found: what came back (the body cut short) and what the matchers make of it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Probe {
    pub status: u16,
    pub body: String,
    pub outcome: Outcome,
    /// Whether the condition is active by this answer, the fallback included.
    pub active: bool,
}

/// Asks once (the editor Test button): the answer and what it makes of the condition.
pub fn probe(hook: &Hook) -> Result<Probe, String> {
    hook.check()?;
    let (status, body) = fetch(hook)?;
    let outcome = hook.judge(status, &body);
    Ok(Probe { status, body: body.trim().chars().take(200).collect(), outcome, active: hook.active_by(Some(outcome)) })
}

/// What a test request found, in words (`condition test`).
pub fn test(hook: &Hook) -> Result<String, String> {
    let found = probe(hook)?;
    let verdict = match found.outcome {
        Outcome::True => "the condition is active (the true matcher matches)".to_string(),
        Outcome::False => "the condition is not active (the false matcher matches)".to_string(),
        Outcome::Neither => {
            format!("neither matcher matches, so the fallback applies: {}", if found.active { "active" } else { "not active" })
        }
    };
    Ok(format!("HTTP {}, body \"{}\": {verdict}", found.status, found.body))
}

// ---- The poller.

struct Entry {
    hook: Hook,
    asked: Instant,
    /// When it was last asked of the web, whatever came of it.
    polled: Option<Instant>,
    last: Option<(Instant, Outcome)>,
}

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

/// Whether the webhook's condition is active now, by the last answer; the fallback when there is
/// none yet, or it is too old. Starts asking the web about it if nobody has.
pub fn answer(hook: &Hook) -> bool {
    start_poller();
    let key = serde_json::to_string(hook).unwrap_or_default();
    let mut all = registry().locked();
    let entry = all.entry(key).or_insert_with(|| Entry { hook: hook.clone(), asked: Instant::now(), polled: None, last: None });
    entry.asked = Instant::now();
    let fresh = entry.last.filter(|(at, _)| at.elapsed() <= Duration::from_secs(hook.interval) * STALE_AFTER).map(|(_, outcome)| outcome);
    hook.active_by(fresh)
}

fn start_poller() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new().name("webhook".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_secs(1));
            poll_due();
        });
    });
}

/// Asks the web about every webhook that is due, one after another (each takes 10 s at most).
fn poll_due() {
    let due: Vec<(String, Hook)> = {
        let mut all = registry().locked();
        all.retain(|_, entry| entry.asked.elapsed() <= IDLE);
        all.iter()
            .filter(|(_, entry)| entry.polled.is_none_or(|at| at.elapsed() >= Duration::from_secs(entry.hook.interval)))
            .map(|(key, entry)| (key.clone(), entry.hook.clone()))
            .collect()
    };
    for (key, hook) in due {
        let outcome = fetch(&hook).ok().map(|(status, body)| hook.judge(status, &body));
        if let Some(entry) = registry().locked().get_mut(&key) {
            entry.polled = Some(Instant::now());
            // An answer that did not come leaves the last one to age.
            if let Some(outcome) = outcome {
                entry.last = Some((Instant::now(), outcome));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook() -> Hook {
        Hook {
            url: "https://example.com/state?room=1".into(),
            method: Method::Get,
            interval: 60,
            fallback: true,
            when_true: Matcher { on: Match::Status, status: Some(200), body: None },
            when_false: Matcher { on: Match::Both, status: Some(200), body: Some("off".into()) },
        }
    }

    #[test]
    fn it_is_checked_when_saved() {
        assert!(hook().check().is_ok());
        assert!(Hook { url: "ftp://x".into(), ..hook() }.check().is_err());
        assert!(Hook { url: "https://a b".into(), ..hook() }.check().is_err());
        assert!(Hook { interval: 59, ..hook() }.check().is_err());
        assert!(Hook { when_true: Matcher { on: Match::Status, status: None, body: None }, ..hook() }.check().is_err());
        assert!(Hook { when_true: Matcher { on: Match::Body, status: None, body: Some(" ".into()) }, ..hook() }.check().is_err());
        assert!(Hook { when_false: Matcher { on: Match::Status, status: Some(99), body: None }, ..hook() }.check().is_err());
    }

    #[test]
    fn the_true_matcher_goes_first_then_the_false_one_then_the_fallback() {
        let both = Hook {
            when_true: Matcher { on: Match::Both, status: Some(200), body: Some("on".into()) },
            when_false: Matcher { on: Match::Body, status: None, body: Some("off".into()) },
            ..hook()
        };
        assert_eq!(both.judge(200, "on\n"), Outcome::True);
        assert_eq!(both.judge(500, "off"), Outcome::False);
        assert_eq!(both.judge(200, "off"), Outcome::False);
        assert_eq!(both.judge(200, "maybe"), Outcome::Neither);
        assert_eq!(both.judge(404, "on"), Outcome::Neither);
        // Both match: the true one wins.
        assert_eq!(hook().judge(200, "off"), Outcome::True);
    }

    #[test]
    fn what_cannot_be_told_is_the_fallback() {
        let mut quiet = hook();
        assert!(quiet.active_by(None) && quiet.active_by(Some(Outcome::Neither)));
        quiet.fallback = false;
        assert!(!quiet.active_by(None) && !quiet.active_by(Some(Outcome::Neither)));
        assert!(quiet.active_by(Some(Outcome::True)) && !hook().active_by(Some(Outcome::False)));
    }

    /// A web server on 127.0.0.1 that answers every request with `status` and `body`, and says
    /// what each request's first line was. Its address, with a path and a parameter.
    fn serve(status: u16, body: String) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/state?room=1", listener.local_addr().unwrap());
        let (said, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                    request.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&request).lines().next().unwrap_or_default().to_string();
                let _ = said.send(line);
                let head = format!("HTTP/1.1 {status} Something\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes()));
            }
        });
        (url, heard)
    }

    /// An address where nothing answers.
    fn nobody() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        url
    }

    #[test]
    fn a_request_gives_the_status_and_the_body() {
        let (url, heard) = serve(200, "on\n".into());
        assert_eq!(fetch(&Hook { url: format!("  {url} "), ..hook() }), Ok((200, "on\n".to_string())));
        assert_eq!(heard.recv().unwrap(), "GET /state?room=1 HTTP/1.1");
        assert_eq!(fetch(&Hook { url: url.clone(), method: Method::Post, ..hook() }), Ok((200, "on\n".to_string())));
        assert!(heard.recv().unwrap().starts_with("POST /state?room=1 "));
        // An error status is an answer too.
        let (missing, _) = serve(404, "off".into());
        assert_eq!(fetch(&Hook { url: missing, ..hook() }), Ok((404, "off".to_string())));
        // A body longer than 64 KiB is cut, so it equals nothing.
        let (long, _) = serve(200, "x".repeat(70_000));
        assert_eq!(fetch(&Hook { url: long, ..hook() }).map(|(_, body)| body.len()), Ok(64 * 1024));
        // No answer at all is an error.
        assert!(fetch(&Hook { url: nobody(), ..hook() }).is_err());
    }

    #[test]
    fn a_test_says_what_the_matchers_make_of_the_answer() {
        let on = |status: u16, body: &str| {
            let (url, _) = serve(status, body.into());
            let hook = Hook {
                url,
                when_true: Matcher { on: Match::Body, status: None, body: Some("on".into()) },
                when_false: Matcher { on: Match::Body, status: None, body: Some("off".into()) },
                ..hook()
            };
            (probe(&hook), test(&hook))
        };
        let (found, words) = on(200, " on ");
        assert_eq!(found, Ok(Probe { status: 200, body: "on".into(), outcome: Outcome::True, active: true }));
        assert_eq!(words.unwrap(), "HTTP 200, body \"on\": the condition is active (the true matcher matches)");
        let (found, words) = on(200, "off");
        assert!(found.is_ok_and(|p| p.outcome == Outcome::False && !p.active));
        assert!(words.unwrap().ends_with("the condition is not active (the false matcher matches)"));
        let (found, words) = on(500, "maybe");
        assert!(found.is_ok_and(|p| p.outcome == Outcome::Neither && p.active));
        assert_eq!(words.unwrap(), "HTTP 500, body \"maybe\": neither matcher matches, so the fallback applies: active");
        let (url, _) = serve(500, "maybe".into());
        let quiet = Hook { url, fallback: false, ..hook() };
        assert!(test(&quiet).unwrap().ends_with("the fallback applies: not active"));
        // The body shown is cut to 200 characters.
        let (url, _) = serve(200, "y".repeat(300));
        assert_eq!(probe(&Hook { url, ..hook() }).map(|p| p.body.len()), Ok(200));
        // A hook that cannot be saved is not asked; one that gets no answer says so.
        assert!(probe(&Hook { interval: 1, ..hook() }).is_err());
        assert!(test(&Hook { url: nobody(), ..hook() }).is_err());
    }

    #[test]
    fn the_condition_goes_by_the_last_answer_and_else_its_fallback() {
        let (url, _) = serve(204, String::new());
        // The false matcher matches a 204; until the poller has asked, the fallback (active) holds.
        let asked = Hook { url, when_false: Matcher { on: Match::Status, status: Some(204), body: None }, ..hook() };
        assert!(answer(&asked));
        let waited = Instant::now();
        while answer(&asked) && waited.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!answer(&asked), "the poller's answer: not active");
        // An answer older than three intervals is as good as none.
        let key = serde_json::to_string(&asked).unwrap();
        if let Some(old) = Instant::now().checked_sub(Duration::from_secs(asked.interval * 3 + 1)) {
            registry().locked().get_mut(&key).unwrap().last = Some((old, Outcome::False));
            assert!(answer(&asked), "too old: the fallback");
        }
        // A webhook nobody asks about any more is dropped at the next round.
        if let Some(old) = Instant::now().checked_sub(IDLE + Duration::from_secs(1)) {
            registry().locked().get_mut(&key).unwrap().asked = old;
            poll_due();
            assert!(!registry().locked().contains_key(&key));
        }
        // No answer at all leaves the fallback, inactive here.
        let silent = Hook { url: nobody(), fallback: false, ..hook() };
        assert!(!answer(&silent));
        poll_due();
        assert!(!answer(&silent) && registry().locked().get(&serde_json::to_string(&silent).unwrap()).is_some_and(|e| e.last.is_none()));
    }

    #[test]
    fn it_travels_as_json() {
        let json = serde_json::to_string(&hook()).unwrap();
        assert_eq!(serde_json::from_str::<Hook>(&json).unwrap(), hook());
    }
}
