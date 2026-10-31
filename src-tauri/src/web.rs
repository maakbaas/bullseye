//! The window's commands for a browser: the sync server shows the window's own page (`src/`), and its
//! stand-in for Tauri (`src/js/web.js`) sends each command by name, with the arguments the window
//! passes to Tauri, to `POST /api/invoke` (`server/src/http.rs`). They are answered here as the app's
//! commands answer them (`lib.rs`): the ones that are requests go to the server's view under a
//! computer's rules (`Hub::request`), the ones that only put something in words are worded here. What
//! only a computer can do (its sync settings, its Store apps and drives, files) is refused with a
//! message (`COMPUTER_ONLY`; the page hides most of it).
//!
//! The server has no time zone of its own (it is not Windows): what is worded by the clock, a session's
//! rows and a date to lock until, is worded in the browser's offset from UTC, which comes with each
//! command (`Invoked::zone`).

use std::time::SystemTime;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::localtime::{self, Zone};
use crate::proto::{Request, Response};
use crate::strings::t;

/// The app's commands a browser cannot use: what is this computer's own (its syncing, its installed
/// Store apps and flash drives, files to export to and import from, its window).
pub const COMPUTER_ONLY: [&str; 10] = [
    "sync_status",
    "sync_connect",
    "sync_disconnect",
    "sync_now",
    "installed_apps",
    "connected_drives",
    "export_settings",
    "open_settings",
    "import_settings",
    "window_ready",
];

/// One command, as the page sends it.
#[derive(Deserialize, Debug)]
pub struct Invoked {
    /// The command's name, as the window invokes it (`save_target`).
    pub cmd: String,
    /// Its arguments, named as the window names them (`newId`).
    #[serde(default)]
    pub args: Map<String, Value>,
    /// The browser's offset from UTC, in minutes east.
    #[serde(default)]
    pub zone: i32,
}

/// The answer to `invoked`, as Tauri gives the window one: its value, or what went wrong (mostly
/// words; an allowance's problem names its field). `send` answers a request as the server's view.
pub fn invoke(invoked: Invoked, send: &dyn Fn(Request) -> Result<Response, String>) -> Result<Value, Value> {
    let Invoked { cmd, args, zone } = invoked;
    let zone = Zone::fixed(zone);
    // An argument, of whatever type it is taken as; one that cannot be read ends the command.
    macro_rules! arg {
        ($name:expr) => {
            read(&args, $name).map_err(Value::String)?
        };
        ($name:expr, $type:ty) => {
            read::<$type>(&args, $name).map_err(Value::String)?
        };
    }
    let words = |result: Result<String, String>| result.map(Value::String).map_err(Value::String);
    let request = match cmd.as_str() {
        "status" => return status(send(Request::Status).map_err(Value::String)?, &zone),
        "save_session" => Request::SaveSession { session: arg!("session") },
        "delete_session" => Request::DeleteSession { id: arg!("id") },
        "save_target" => Request::SaveTarget { target: arg!("target") },
        "delete_target" => Request::DeleteTarget { id: arg!("id") },
        "save_condition" => Request::SaveCondition { condition: arg!("condition") },
        "delete_condition" => Request::DeleteCondition { id: arg!("id") },
        "duplicate" => Request::Duplicate { kind: arg!("kind"), id: arg!("id"), new_id: arg!("new_id") },
        "start_session" => Request::Start { id: arg!("id") },
        "check_stricter" => Request::Check { check: arg!("check") },
        "lock_session" => Request::Lock { id: arg!("id"), lock: arg!("lock") },
        "stop_session" => Request::Stop { id: arg!("id"), password: arg!("password"), text: arg!("text") },
        "request_stop" => Request::AskToStop { id: arg!("id") },
        "abort_stop" => Request::AbortStop { id: arg!("id") },
        "take_break" => Request::TakeBreak { id: arg!("id") },
        "end_break" => Request::EndBreak { id: arg!("id") },
        "unlock_all" => Request::Unlock,
        "unlock_with" => Request::UnlockWith { password: arg!("password"), text: arg!("text") },
        "set_unlock_way" => Request::SetUnlockWay { way: arg!("way") },
        "request_unlock" => Request::AskForUnlock,
        "abort_unlock" => Request::AbortUnlock,
        "set_unlock_delay" => Request::SetUnlockDelay { delay: arg!("delay") },
        "describe_schedule" => return words(arg!("week", crate::schedule::Week).normalized().map(|week| week.describe())),
        "describe_allowance" => {
            return match arg!("terms", crate::allowance::Terms).described() {
                Ok(described) => value(&described),
                Err(problem) => Err(value(&problem)?),
            };
        }
        "describe_duration" => {
            return words(match crate::duration::parse(&arg!("text", String)) {
                Ok(Some(length)) => Ok(crate::duration::countdown(length.as_secs())),
                Ok(None) => Err("No duration given".into()),
                Err(why) => Err(why),
            });
        }
        "describe_until" => {
            return localtime::until_in(&arg!("text", String), &zone).map_err(Value::String).and_then(|until| value(&until))
        }
        "deletion_fallout" => {
            let library = crate::definitions::Library { sessions: arg!("sessions"), conditions: arg!("conditions"), ..Default::default() };
            return value(&library.fallout(&arg!("id", String)));
        }
        "test_webhook" => return crate::webhook::probe(&arg!("hook")).map_err(Value::String).and_then(|probe| value(&probe)),
        name if COMPUTER_ONLY.contains(&name) => return Err(Value::String(t("web.computerOnly", &[]))),
        name => return Err(Value::String(format!("No such command: {name}"))),
    };
    match send(request).map_err(Value::String)? {
        Response::Done => Ok(Value::Null),
        Response::Failed(why) | Response::Refused(why) => Err(Value::String(why)),
        other => Err(Value::String(format!("Unexpected answer: {other:?}"))),
    }
}

/// The overview, with each session's rows worded in the browser's time zone.
fn status(response: Response, zone: &Zone) -> Result<Value, Value> {
    let Response::Status(mut overview) = response else {
        return Err(Value::String(format!("Unexpected answer: {response:?}")));
    };
    let now = SystemTime::now();
    overview.status = overview
        .sessions
        .iter()
        .map(|session| crate::status::of(session, overview.running.iter().find(|r| r.id == session.id), &overview.conditions, zone, now))
        .collect();
    value(&overview)
}

/// The argument `name` (`new_id`), which the window names in camel case (`newId`), as Tauri takes it;
/// one not given is `null` (none, for what may be left out).
fn read<T: DeserializeOwned>(args: &Map<String, Value>, name: &str) -> Result<T, String> {
    let mut camel = String::new();
    let mut upper = false;
    for c in name.chars() {
        match c {
            '_' => upper = true,
            c if upper => {
                camel.extend(c.to_uppercase());
                upper = false;
            }
            c => camel.push(c),
        }
    }
    let given = args.get(&camel).cloned().unwrap_or(Value::Null);
    serde_json::from_value(given).map_err(|e| format!("{camel}: {e}"))
}

fn value<T: serde::Serialize>(answer: &T) -> Result<Value, Value> {
    serde_json::to_value(answer).map_err(|e| Value::String(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tests::{session, target};
    use crate::handler::{handle, Context};
    use crate::proto::Overview;
    use serde_json::json;
    use std::sync::Arc;

    fn context(name: &str) -> Context {
        let dir = std::env::temp_dir().join(format!("web-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions = crate::session::Sessions::with_store(crate::state::Store::new(dir.join("state.json")));
        Context::new(
            sessions,
            crate::definitions::Definitions::at(Some(dir.join("sessions.json"))),
            Arc::new(crate::watchdog::AppSlot::new(false)),
        )
    }

    fn call(context: &Context, cmd: &str, args: Value) -> Result<Value, Value> {
        let invoked = Invoked { cmd: cmd.into(), args: args.as_object().cloned().unwrap_or_default(), zone: 60 };
        invoke(invoked, &|request| Ok(handle(context, 0, request).0))
    }

    /// Every command the window can invoke is answered here, or refused as a computer's own: when
    /// this fails, the window has learned a command the sync server's page does not know yet.
    #[test]
    fn every_command_of_the_window_is_answered_or_said_to_be_a_computers() {
        let lib = include_str!("lib.rs");
        let start = lib.find("generate_handler![").expect("the command list") + "generate_handler![".len();
        let list = &lib[start..start + lib[start..].find(']').unwrap()];
        let commands: Vec<&str> = list.split(',').map(|c| c.trim().trim_start_matches("ui::")).filter(|c| !c.is_empty()).collect();
        assert!(commands.len() > 30);
        let context = context("all");
        for command in commands {
            let answer = call(&context, command, json!({}));
            if COMPUTER_ONLY.contains(&command) {
                assert_eq!(answer, Err(Value::String(t("web.computerOnly", &[]))), "{command}");
            } else {
                assert!(!matches!(&answer, Err(Value::String(why)) if why.starts_with("No such command")), "{command}: {answer:?}");
            }
        }
        assert_eq!(call(&context, "lasers", json!({})), Err(json!("No such command: lasers")));
    }

    #[test]
    fn requests_go_to_the_view_under_its_rules_and_come_back_as_the_app_gives_them() {
        let context = context("requests");
        assert_eq!(call(&context, "save_target", json!({ "target": target("social", &["youtube.com"]) })), Ok(Value::Null));
        assert_eq!(call(&context, "save_session", json!({ "session": session("work", &["social"]) })), Ok(Value::Null));
        // Arguments named as the window names them.
        assert_eq!(call(&context, "duplicate", json!({ "kind": "target", "id": "social", "newId": "copy" })), Ok(Value::Null));
        assert_eq!(call(&context, "start_session", json!({ "id": "work" })), Ok(Value::Null));
        let overview: Overview = serde_json::from_value(call(&context, "status", json!({})).unwrap()).unwrap();
        assert_eq!(overview.targets.len(), 2);
        assert_eq!(overview.running.len(), 1);
        assert_eq!(overview.status.len(), 1);
        // A refusal is its words; arguments that cannot be read say which.
        assert_eq!(call(&context, "delete_target", json!({ "id": "nope" })), Err(json!("No such target")));
        assert!(call(&context, "start_session", json!({})).unwrap_err().as_str().unwrap().starts_with("id: "));
        assert_eq!(call(&context, "stop_session", json!({ "id": "work" })), Ok(Value::Null), "password and text may be left out");
        assert_eq!(call(&context, "set_unlock_delay", json!({ "delay": "1h" })), Ok(Value::Null));
        assert_eq!(call(&context, "request_unlock", json!({})), Ok(Value::Null));
        assert_eq!(call(&context, "abort_unlock", json!({})), Ok(Value::Null));
        // What the view cannot answer is said.
        let failing = invoke(Invoked { cmd: "status".into(), args: Map::new(), zone: 0 }, &|_| Err("away".into()));
        assert_eq!(failing, Err(json!("away")));
        let odd = invoke(Invoked { cmd: "start_session".into(), args: json!({"id": "x"}).as_object().cloned().unwrap(), zone: 0 }, &|_| {
            Ok(Response::Status(serde_json::from_value(call(&context, "status", json!({})).unwrap()).unwrap()))
        });
        assert!(odd.unwrap_err().as_str().unwrap().starts_with("Unexpected answer"));
        assert!(status(Response::Done, &Zone::fixed(0)).unwrap_err().as_str().unwrap().starts_with("Unexpected answer"));
    }

    /// The local date and time two hours from now, at `minutes` east of UTC, as typed (`2026-10-31 18:05`).
    fn local_in_two_hours(minutes: i64) -> String {
        let unix = SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let seconds = unix + 7200 + 60 + minutes * 60;
        let (days, of_day) = (seconds.div_euclid(86400), seconds.rem_euclid(86400));
        // Days since 1970 as a civil date (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", of_day / 3600, of_day % 3600 / 60)
    }

    #[test]
    fn what_is_only_worded_is_worded_here_in_the_browsers_time_zone() {
        let context = context("words");
        assert_eq!(
            call(&context, "describe_schedule", json!({ "week": crate::schedule::Week::parse("mon 9-17").unwrap() })),
            Ok(json!("Mon 09:00-17:00"))
        );
        assert_eq!(call(&context, "describe_duration", json!({ "text": "90m" })), Ok(json!("1 hour and 30 minutes")));
        assert_eq!(call(&context, "describe_duration", json!({ "text": "" })), Err(json!("No duration given")));
        assert!(call(&context, "describe_duration", json!({ "text": "soon" })).is_err());
        let terms = json!({ "type": "budget", "allowed": "30m", "scope": "total", "reset": "never" });
        assert!(call(&context, "describe_allowance", json!({ "terms": terms })).is_ok());
        let wrong = json!({ "type": "budget", "allowed": "", "scope": "total", "reset": "never" });
        assert!(call(&context, "describe_allowance", json!({ "terms": wrong })).unwrap_err().is_object(), "a problem names its field");
        // A date to lock until is in the browser's zone (here UTC+1).
        let until = call(&context, "describe_until", json!({ "text": local_in_two_hours(60) })).unwrap();
        assert_eq!(until["left"], json!("2 hours"));
        assert!(call(&context, "describe_until", json!({ "text": "2000-01-01 00:00" })).is_err());
        let fallout = call(&context, "deletion_fallout", json!({ "sessions": [session("w", &["t"])], "conditions": [], "id": "t" }));
        assert!(fallout.unwrap().is_object());
        assert!(call(&context, "test_webhook", json!({ "hook": { "url": "not a url" } })).is_err());
        assert_eq!(call(&context, "connected_drives", json!({})), Err(Value::String(t("web.computerOnly", &[]))));
    }
}
