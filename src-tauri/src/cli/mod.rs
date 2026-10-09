//! The command line: everything the window can do, from a terminal or a script
//! (`bullseye.exe`, explained in `CLI.md`). It talks to the service through the same
//! `client::Remote` as the window, so the service checks everything done here exactly as it checks
//! the window, and texts such as schedules and durations come from the same Rust functions.
//!
//! **Keep it level with the window.** Every command the window can invoke (the list in
//! `generate_handler!` in `lib.rs`) is in `GUI_COMMANDS` with the CLI commands that do the same, and
//! a test fails when one is missing, or missing from `HELP` or `CLI.md`. Action names come from
//! `Actions` itself, so a new action is here without any change. A new list, condition kind or
//! lock method needs an option here, a line in `HELP` and in `CLI.md`.
//!
//! Here are the commands at work; `parse` reads a command line into a `Command`, `output` writes what
//! the window shows.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::allowance::{Anchor, DelayKind, Reset, Scope, Terms};
use crate::client::Remote;
use crate::condition::{Mode, Rule};
use crate::definitions::{self, Condition, Definition, Kind, Library, Target};
use crate::duration;
use crate::lists::{Entries, Lists};
use crate::localtime;
use crate::lock::{Lock, TimedKind};
use crate::proto::{Overview, Request, UnlockStatus};
use crate::schedule::Week;
use crate::state::Actions;
use crate::status::{Line, Status};
use crate::strings::{joined, quoted, t};
use crate::webhook::{self, Hook, Match, Matcher, Method};

mod output;
mod parse;
#[cfg(test)]
mod tests;

use output::*;
use parse::*;

/// Each command the window can invoke, and the CLI commands that do the same. Empty: it is about
/// the window itself.
pub const GUI_COMMANDS: [(&str, &[&str]); 34] = [
    ("status", &["status", "show", "target show", "condition show"]),
    ("save_session", &["new", "edit"]),
    ("delete_session", &["delete"]),
    ("save_target", &["target new", "target edit"]),
    ("delete_target", &["target delete"]),
    // The installed Store apps the editor offers to tick.
    ("installed_apps", &["target apps"]),
    // The flash drives the editor of a flash drive condition offers.
    ("connected_drives", &["drives"]),
    ("save_condition", &["condition new", "condition edit"]),
    ("delete_condition", &["condition delete"]),
    ("duplicate", &["duplicate", "target duplicate", "condition duplicate"]),
    ("start_session", &["start"]),
    ("lock_session", &["lock"]),
    ("stop_session", &["stop"]),
    ("request_stop", &["stop"]),
    ("abort_stop", &["stop"]),
    ("take_break", &["break"]),
    ("end_break", &["break"]),
    ("unlock_all", &["unlock"]),
    ("unlock_with", &["unlock"]),
    ("set_unlock_way", &["unlock-way"]),
    ("request_unlock", &["unlock"]),
    ("abort_unlock", &["unlock"]),
    ("set_unlock_delay", &["unlock-delay"]),
    ("describe_schedule", &["condition check"]),
    // A webhook condition asked once, as the editor Test button does.
    ("test_webhook", &["condition test"]),
    // A session's allowance in words: what `show` says of it.
    ("describe_allowance", &["show"]),
    // A lock's length or end in words: what `show` and `start` say of a duration or until lock.
    ("describe_duration", &["show"]),
    ("describe_until", &["show", "start"]),
    // What the question before a deletion says.
    ("deletion_fallout", &["target delete", "condition delete"]),
    ("export_settings", &["export"]),
    // A settings file's contents: what the question before importing it says.
    ("open_settings", &["import"]),
    ("import_settings", &["import"]),
    ("window_ready", &[]),
    // Asked before a confirmation in the window; the CLI asks, and the service answers when it does it.
    ("check_stricter", &[]),
];

/// Why a command did not work: the exit code says which.
enum Failure {
    /// Not understood (exit code 2).
    Usage(String),
    /// Understood, but it failed, was refused or was cancelled (exit code 1).
    Failed(String),
}

fn failed(why: impl Into<String>) -> Failure {
    Failure::Failed(why.into())
}

/// A line of what a command says (`out`).
macro_rules! say {
    ($($arg:tt)*) => {
        out(&format!("{}\n", format_args!($($arg)*)))
    };
}

/// The CLI; returns the exit code.
pub fn main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args).map_err(Failure::Usage).and_then(execute) {
        Ok(()) => 0,
        Err(Failure::Usage(why)) => {
            eprintln!("{why}\n\nRun `bullseye help` for the commands.");
            2
        }
        Err(Failure::Failed(why)) => {
            eprintln!("{why}");
            1
        }
    }
}

fn execute(command: Command) -> Result<(), Failure> {
    match command {
        Command::Help => say!("{}", help()),
        Command::Version => say!("bullseye {}", env!("CARGO_PKG_VERSION")),
        Command::Status { json } => {
            let overview = overview()?;
            if json {
                say!("{}", serde_json::to_string_pretty(&overview).map_err(|e| failed(e.to_string()))?);
            } else {
                out(&describe_overview(&overview));
            }
        }
        Command::Show { session, json } => {
            let overview = overview()?;
            let session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?;
            let running = overview.running.iter().find(|r| r.id == session.id);
            if json {
                let both = serde_json::json!({ "session": session, "running": running, "status": status_of(&overview, session) });
                say!("{}", serde_json::to_string_pretty(&both).map_err(|e| failed(e.to_string()))?);
            } else {
                out(&describe_session(&overview, session));
            }
        }
        Command::New { name, changes } => {
            let overview = overview()?;
            let mut session = Definition {
                id: new_id(),
                name,
                targets: Vec::new(),
                condition: None,
                allowance: None,
                lock: Lock::None,
                kept_locks: Vec::new(),
            };
            apply_session_changes(&overview, &mut session, changes)?;
            let (id, name) = (session.id.clone(), session.name.clone());
            send(Request::SaveSession { session })?;
            say!("Made the session “{name}” (id {id}). Start it with: bullseye start \"{name}\"");
        }
        Command::Edit { session, changes } => {
            let overview = overview()?;
            let mut session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?.clone();
            apply_session_changes(&overview, &mut session, changes)?;
            let name = session.name.clone();
            send(Request::SaveSession { session })?;
            say!("Saved the session “{name}”.");
        }
        Command::Delete { session, force } => {
            let overview = overview()?;
            let session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?;
            if !confirm(&delete_question(&overview, "sessions", &session.name, &session.id), force)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteSession { id: session.id.clone() })?;
            say!("Deleted the session “{}”.", session.name);
        }
        Command::Start { session, lock, force } => {
            let overview = overview()?;
            let mut session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?.clone();
            // As the lock on the session's tile: set on the session, then started with it.
            let lock = lock.map(|lock| resolve_lock(&overview, lock)).transpose()?;
            if let Some(lock) = lock.filter(|lock| *lock != session.lock) {
                session.lock = lock;
                send(Request::SaveSession { session: session.clone() })?;
            }
            // As in the window: a session that cannot be stopped is only started after a second yes.
            if let Some((heading, body)) = lock_question(&overview, &session) {
                let question = format!("{heading} {body}");
                if !confirm(&question, force)? {
                    return Err(failed("Not started."));
                }
            }
            send(Request::Start { id: session.id.clone() })?;
            say!("Started “{}”.", session.name);
        }
        Command::Lock { session, lock, force } => {
            let overview = overview()?;
            let mut session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?.clone();
            session.lock = resolve_lock(&overview, lock)?;
            // As in the window: a lock that holds is only given after a second yes.
            if let Some((_, body)) = lock_question(&overview, &session) {
                let question = format!("{} {body}", t("sessions.lockNow.heading", &[("name", &quoted(&session.name))]));
                if !confirm(&question, force)? {
                    return Err(failed("Not locked."));
                }
            }
            send(Request::Lock { id: session.id.clone(), lock: session.lock.clone() })?;
            if session.lock == Lock::None {
                say!("“{}” runs on without a lock.", session.name);
            } else {
                say!("Locked “{}” now.", session.name);
            }
        }
        Command::Stop { session, password, abort } => {
            let overview = overview()?;
            let session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?;
            // A session locked with a delay: stopping is asked for, waited out, and then done.
            if let Some(run) = overview.running.iter().find(|r| r.id == session.id && r.stop_delay.is_some()) {
                if abort {
                    send(Request::AbortStop { id: session.id.clone() })?;
                    say!("Stopping “{}” aborted: back to locked.", session.name);
                    return Ok(());
                }
                match (run.stop_remaining, &run.stop_remaining_words) {
                    (Some(0), _) => {}
                    (Some(_), Some(left)) => {
                        say!(
                            "“{}” can be stopped in {left}. Run `bullseye stop` again then, or `stop --abort` to end the wait.",
                            session.name
                        );
                        return Ok(());
                    }
                    _ => {
                        send(Request::AskToStop { id: session.id.clone() })?;
                        say!(
                            "Stopping “{}” was asked for: ready in {}. Run `bullseye stop` again then, or `stop --abort` to end the wait.",
                            session.name,
                            run.stop_delay_words.clone().unwrap_or_default()
                        );
                        return Ok(());
                    }
                }
            }
            // A session locked with a text stops in the window, by typing the text: typing it is the
            // point, and a command line cannot stop a paste.
            if overview.running.iter().any(|r| r.id == session.id && r.lock_text.is_some()) {
                return Err(failed(format!("“{}” is locked until its text is typed in: stop it in the window", session.name)));
            }
            // A session locked with a password stops with it: asked for when it was not given.
            let password_locked = overview.running.iter().any(|r| r.id == session.id && r.password);
            let password = match password {
                None if password_locked => Some(read_password(&format!("Password of “{}”: ", session.name)).map_err(Failure::Failed)?),
                given => given,
            };
            send(Request::Stop { id: session.id.clone(), password, text: None })?;
            say!("Stopped “{}”.", session.name);
        }
        Command::Break { session, end } => {
            let overview = overview()?;
            let session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?;
            if end {
                send(Request::EndBreak { id: session.id.clone() })?;
                say!("Ended the break of “{}”.", session.name);
            } else {
                send(Request::TakeBreak { id: session.id.clone() })?;
                let length = match &session.allowance {
                    Some(Terms::Breaks { length, .. }) => duration::parse(length).ok().flatten().map(|d| duration::countdown(d.as_secs())),
                    _ => None,
                };
                say!("“{}” is on a break for {}.", session.name, length.unwrap_or_default());
            }
        }
        Command::TargetShow { target: list, json } => {
            let overview = overview()?;
            let list = find(&overview.targets, &list, "target", |l| (&l.id, &l.name))?;
            if json {
                say!("{}", serde_json::to_string_pretty(list).map_err(|e| failed(e.to_string()))?);
            } else {
                out(&describe_target(&overview, list));
            }
        }
        Command::Drives { json } => {
            let drives = crate::drives::connected().map_err(Failure::Failed)?;
            if json {
                say!("{}", serde_json::to_string_pretty(&drives).map_err(|e| failed(e.to_string()))?);
            } else if drives.is_empty() {
                say!("No flash drive with a serial number is connected.");
            } else {
                for drive in drives {
                    say!("{:<40} {}", drive.name, drive.serial);
                }
            }
        }
        Command::TargetApps { json } => {
            let apps = crate::store_apps::installed().map_err(Failure::Failed)?;
            if json {
                say!("{}", serde_json::to_string_pretty(&apps).map_err(|e| failed(e.to_string()))?);
            } else {
                for app in apps {
                    say!("{:<40} {}", app.name, app.family);
                }
            }
        }
        Command::TargetNew { name, changes } => {
            let mut list = Target { id: new_id(), name, entries: Entries::default(), actions: Actions::default() };
            changes.apply(&mut list);
            let name = list.name.clone();
            send(Request::SaveTarget { target: list })?;
            say!("Made the target “{name}”. Use it with: bullseye new <session> --targets \"{name}\"");
        }
        Command::TargetEdit { target: list, changes } => {
            let overview = overview()?;
            let mut list = find(&overview.targets, &list, "target", |l| (&l.id, &l.name))?.clone();
            changes.apply(&mut list);
            let name = list.name.clone();
            send(Request::SaveTarget { target: list })?;
            say!("Saved the target “{name}”.");
        }
        Command::TargetDelete { target: list, force } => {
            let overview = overview()?;
            let list = find(&overview.targets, &list, "target", |l| (&l.id, &l.name))?;
            if !confirm(&delete_question(&overview, "targets", &list.name, &list.id), force)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteTarget { id: list.id.clone() })?;
            say!("Deleted the target “{}”.", list.name);
        }
        Command::ConditionTest { condition } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            let Rule::Webhook(hook) = &condition.rule else {
                return Err(failed(format!("“{}” is not a webhook condition", condition.name)));
            };
            say!("{}", webhook::test(hook).map_err(failed)?);
        }
        Command::ConditionShow { condition } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            say!("{}", condition.name);
            say!("{}", condition_words(&overview, &condition.rule));
            match &condition.rule {
                Rule::Schedule { week } => out(&per_day(week)),
                Rule::Webhook(hook) => say!("{}", describe_webhook(hook)),
                _ => {}
            }
            say!("{}: {}", t("tiles.usedBy", &[]), users(&overview, &condition.id));
        }
        Command::ConditionNew { name, rule } => {
            let rule = resolve_rule(&overview()?, rule)?;
            send(Request::SaveCondition { condition: Condition { id: new_id(), name: name.clone(), rule } })?;
            say!("Made the condition “{name}”. Use it with: bullseye new <session> --condition \"{name}\"");
        }
        Command::ConditionEdit { condition, name, rule } => {
            let overview = overview()?;
            let mut condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?.clone();
            if let Some(name) = name {
                condition.name = name;
            }
            if let Some(rule) = rule {
                condition.rule = resolve_rule(&overview, rule)?;
            }
            let name = condition.name.clone();
            send(Request::SaveCondition { condition })?;
            say!("Saved the condition “{name}”.");
        }
        Command::SessionItems { session, targets: lists, add } => {
            let overview = overview()?;
            let mut session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?.clone();
            let ids = lists
                .iter()
                .map(|l| find(&overview.targets, l, "target", |l| (&l.id, &l.name)).map(|l| (l.id.clone(), l.name.clone())))
                .collect::<Result<Vec<_>, _>>()?;
            session.targets = changed(&session.targets, &ids, add, |name| format!("“{name}” is not a target of “{}”", session.name))?;
            let name = session.name.clone();
            send(Request::SaveSession { session })?;
            say!("Saved the session “{name}”.");
        }
        Command::TargetItems { target: list, entries, actions, add } => {
            let overview = overview()?;
            let mut list = find(&overview.targets, &list, "target", |l| (&l.id, &l.name))?.clone();
            for (key, given) in entries {
                let what =
                    LIST_OPTIONS.iter().find(|(_, k)| *k == key).map(|(option, _)| option[2..].replace('-', " ")).unwrap_or_default();
                let Some(field) = list.entries.field(key) else { continue };
                let keyed: Vec<(String, String)> = given.iter().map(|entry| (entry_key(key, entry), entry.clone())).collect();
                let now: Vec<String> = field.iter().map(|entry| entry_key(key, entry)).collect();
                let kept = changed(&now, &keyed, add, |entry| format!("“{entry}” is not in the {what} of “{}”", list.name))?;
                // The entries as they were typed, the new ones as given.
                let mut typed: Vec<String> = Vec::new();
                for entry_key in kept {
                    match field.iter().find(|entry| self::entry_key(key, entry) == entry_key) {
                        Some(entry) => typed.push(entry.clone()),
                        None => typed.extend(keyed.iter().find(|(k, _)| *k == entry_key).map(|(_, entry)| entry.clone())),
                    }
                }
                *field = typed;
            }
            if !actions.is_empty() {
                list.actions = switched(list.actions, &actions, add).map_err(Failure::Usage)?;
            }
            let name = list.name.clone();
            send(Request::SaveTarget { target: list })?;
            say!("Saved the target “{name}”.");
        }
        Command::ConditionItems { condition, members, add } => {
            let overview = overview()?;
            let mut condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?.clone();
            let Rule::Combined { members: now, .. } = &mut condition.rule else {
                return Err(failed(format!("“{}” is not a combination; add and remove change the conditions of one", condition.name)));
            };
            let ids = members
                .iter()
                .map(|c| find(&overview.conditions, c, "condition", |c| (&c.id, &c.name)).map(|c| (c.id.clone(), c.name.clone())))
                .collect::<Result<Vec<_>, _>>()?;
            *now = changed(now, &ids, add, |name| format!("“{name}” is not in “{}”", condition.name))?;
            let name = condition.name.clone();
            send(Request::SaveCondition { condition })?;
            say!("Saved the condition “{name}”.");
        }
        Command::ConditionDelete { condition, force } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            if !confirm(&delete_question(&overview, "conditions", &condition.name, &condition.id), force)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteCondition { id: condition.id.clone() })?;
            say!("Deleted the condition “{}”.", condition.name);
        }
        Command::ConditionCheck { times } => match Week::parse(&times).map_err(Failure::Usage)? {
            Some(week) => out(&format!("{}\n{}", week.describe(), per_day(&week))),
            None => say!("Always active: a session without a condition"),
        },
        Command::Export { file } => {
            definitions::write_settings(std::path::Path::new(&file), remote().library().map_err(failed)?).map_err(failed)?;
            say!("Wrote every target, condition and session to {file}.");
        }
        Command::Import { file, force } => {
            let library = definitions::read_settings(std::path::Path::new(&file)).map_err(failed)?;
            let contents = library.contents();
            // A Store app this computer has not installed is left out, as the window does.
            let installed = crate::store_apps::installed_families();
            let checked = installed.is_some() || !library.has_store_apps();
            let (library, left_out) = library.without_missing_apps(installed.as_deref());
            // The window's question, sentence by sentence.
            let count = |key: &str, n: usize| t(&format!("settings.import{key}"), &[("n", &n.to_string())]);
            let mut body = vec![t(
                "settings.importBody",
                &[
                    ("targets", &count("Targets", contents.targets)),
                    ("conditions", &count("Conditions", contents.conditions)),
                    ("sessions", &count("Sessions", contents.sessions)),
                ],
            )];
            if !overview()?.running.is_empty() {
                body.push(t("settings.importStops", &[]));
            }
            if !contents.unlocked.is_empty() {
                let names = joined(&contents.unlocked.iter().map(|n| quoted(n)).collect::<Vec<_>>());
                body.push(t("settings.importUnlocked", &[("n", &contents.unlocked.len().to_string()), ("names", &names)]));
            }
            let apps = left_out_words(&left_out, checked);
            if !apps.is_empty() {
                body.push(apps.clone());
            }
            if !confirm(&format!("{} {}", t("settings.importHeading", &[]), body.join(". ")), force)? {
                return Err(failed("Not imported."));
            }
            send(Request::Import { library })?;
            say!("Imported {file}.");
            if !apps.is_empty() {
                say!("{apps}");
            }
        }
        Command::Duplicate { kind, item } => {
            // The item's id and name, then the copy's name, which the service gives it.
            let names = |overview: &Overview| -> Vec<(String, String)> {
                match kind {
                    Kind::Session => overview.sessions.iter().map(|b| (b.id.clone(), b.name.clone())).collect(),
                    Kind::Target => overview.targets.iter().map(|l| (l.id.clone(), l.name.clone())).collect(),
                    Kind::Condition => overview.conditions.iter().map(|c| (c.id.clone(), c.name.clone())).collect(),
                }
            };
            let noun = match kind {
                Kind::Session => "session",
                Kind::Target => "target",
                Kind::Condition => "condition",
            };
            let items = names(&overview()?);
            let (id, name) = find(&items, &item, noun, |(id, name)| (id, name))?;
            let new_id = new_id();
            send(Request::Duplicate { kind, id: id.clone(), new_id: new_id.clone() })?;
            let copy = names(&overview()?).into_iter().find(|(id, _)| *id == new_id).map(|(_, name)| name).unwrap_or_default();
            say!("Made “{copy}”, a copy of the {noun} “{name}”.");
        }
        Command::Unlock { abort, password } => {
            let status = overview()?.unlock;
            let password_way = matches!(status.way, Some(Lock::Password { .. }));
            if abort {
                send(Request::AbortUnlock)?;
                say!("Unlocking aborted: back to locked.");
            } else if password.is_some() || (password_way && status.remaining != Some(0) && !status.way_open) {
                // The other way to unlock, with its password: at once, whatever the countdown.
                let password = match password {
                    Some(password) => password,
                    None => read_password("Password to unlock: ").map_err(Failure::Failed)?,
                };
                send(Request::UnlockWith { password: Some(password), text: None })?;
                say!("Unlocked: every session has ended.");
            } else if status.remaining == Some(0) || status.way_open {
                send(Request::Unlock)?;
                say!("Unlocked: every session has ended.");
            } else if let Some(left) = status.remaining_words {
                say!("Unlocking is not ready yet: {left} to go. Run `bullseye unlock` again then, or `bullseye unlock --abort` to stop.");
            } else {
                send(Request::AskForUnlock)?;
                say!(
                    "Unlocking was asked for: ready in {}. Run `bullseye unlock` again then, or `bullseye unlock --abort` to stop.",
                    status.delay_words
                );
            }
        }
        Command::UnlockWay { way } => match way {
            None => say!("{}", describe_unlock_way(&overview()?.unlock)),
            Some(way) => {
                let overview = overview()?;
                let way = way.map(|lock| resolve_lock(&overview, lock)).transpose()?;
                send(Request::SetUnlockWay { way })?;
                say!("{}", describe_unlock_way(&self::overview()?.unlock));
            }
        },
        Command::UnlockDelay { delay } => match delay {
            None => {
                let status = overview()?.unlock;
                if status.delay == 0 {
                    say!("Unlocking takes no time: it is free.");
                } else {
                    say!("Unlocking takes {}.", status.delay_words);
                }
            }
            Some(delay) => {
                send(Request::SetUnlockDelay { delay })?;
                let status = overview()?.unlock;
                say!(
                    "{}",
                    if status.delay == 0 {
                        "Unlocking is free again.".to_string()
                    } else {
                        format!("Unlocking now takes {}.", status.delay_words)
                    }
                );
            }
        },
    }
    Ok(())
}

/// Weekly times, as a schedule takes them; "always" is not a schedule (a session without a condition is
/// always active).
fn times_of(text: &str) -> Result<Week, Failure> {
    match Week::parse(text).map_err(failed)? {
        Some(week) => Ok(week),
        None => Err(failed("Give the times, e.g. \"Mon-Fri 08:00-17:00\"; a session that is always active needs no condition")),
    }
}

/// A condition's rule from what was typed: weekly times, or a flash drive looked up among the
/// connected ones by name or serial number.
fn resolve_rule(overview: &Overview, choice: RuleChoice) -> Result<Rule, Failure> {
    match choice {
        RuleChoice::Any(ref given) | RuleChoice::All(ref given) if given.len() < 2 => {
            Err(failed("A combination needs at least two conditions: give --any or --all once for each"))
        }
        RuleChoice::Any(given) => combination(overview, Mode::Any, &given),
        RuleChoice::All(given) => combination(overview, Mode::All, &given),
        RuleChoice::Times(text) => Ok(Rule::Schedule { week: times_of(&text)? }),
        RuleChoice::Webhook(hook) => Ok(Rule::Webhook(hook)),
        RuleChoice::Drive(given) => {
            let drives = crate::drives::connected().map_err(Failure::Failed)?;
            let wanted = given.trim().to_lowercase();
            let found: Vec<&crate::drives::Drive> =
                drives.iter().filter(|d| d.serial.to_lowercase() == wanted || d.name.to_lowercase() == wanted).collect();
            match found[..] {
                [drive] => Ok(Rule::FlashDrive { serial: drive.serial.clone(), drive: drive.name.clone() }),
                [] => Err(failed(format!("No flash drive “{given}” is connected; `bullseye drives` lists the ones that are"))),
                _ => Err(failed(format!("More than one flash drive is called “{given}”; give its serial number instead"))),
            }
        }
    }
}

/// A combination of the conditions given by name or id; they must be single conditions, not
/// combinations (the service refuses those too).
fn combination(overview: &Overview, mode: Mode, given: &[String]) -> Result<Rule, Failure> {
    let mut members = Vec::new();
    for wanted in given {
        let condition = find(&overview.conditions, wanted, "condition", |c| (&c.id, &c.name))?;
        members.push(condition.id.clone());
    }
    Ok(Rule::Combined { mode, members, rules: Vec::new() })
}

/// `new` and `edit`'s changes, with lists and the condition looked up by name or id, and the
/// allowance's terms over the ones the session has.
fn apply_session_changes(overview: &Overview, session: &mut Definition, changes: SessionChanges) -> Result<(), Failure> {
    if let Some(name) = changes.name {
        session.name = name;
    }
    if let Some(lists) = changes.targets {
        session.targets = lists
            .iter()
            .map(|l| find(&overview.targets, l, "target", |l| (&l.id, &l.name)).map(|l| l.id.clone()))
            .collect::<Result<_, _>>()?;
    }
    if let Some(condition) = changes.condition {
        session.condition = match condition {
            Some(c) => Some(find(&overview.conditions, &c, "condition", |c| (&c.id, &c.name))?.id.clone()),
            None => None,
        };
    }
    if let Some(allowance) = changes.allowance {
        session.allowance = match allowance.kind.as_deref() {
            Some("none") => None,
            _ => Some(build_terms(allowance, session.allowance.as_ref()).map_err(failed)?),
        };
    }
    if let Some(lock) = changes.lock {
        session.lock = resolve_lock(overview, lock)?;
    }
    Ok(())
}

/// `now` with the items `given` (each a key and its name) added at the end, or taken out. Adding
/// what is there already changes nothing; taking out what is not there is refused, as it is most
/// likely a typo (`missing` says so, by its name).
fn changed(now: &[String], given: &[(String, String)], add: bool, missing: impl Fn(&str) -> String) -> Result<Vec<String>, Failure> {
    let mut items = now.to_vec();
    for (key, name) in given {
        let at = items.iter().position(|item| item == key);
        match (add, at) {
            (true, None) => items.push(key.clone()),
            (true, Some(_)) => {}
            (false, Some(at)) => {
                items.remove(at);
            }
            (false, None) => return Err(failed(missing(name))),
        }
    }
    Ok(items)
}

/// An entry of a list as the list reads it, letter case aside, so `Steam` is `steam.exe` and
/// `YouTube.com` is `youtube.com`; as typed if it does not read.
fn entry_key(field: &str, entry: &str) -> String {
    let mut one = Entries::default();
    if let Some(list) = one.field(field) {
        list.push(entry.to_string());
    }
    let read = Lists::parse(&one).ok().and_then(|lists| lists.entries().field(field).and_then(|list| list.first().cloned()));
    read.unwrap_or_else(|| entry.trim().to_string()).to_lowercase()
}

/// `actions` with those named (or `all`) turned on, or off.
fn switched(actions: Actions, names: &[String], on: bool) -> Result<Actions, String> {
    let named = serde_json::to_value(parse_actions(names)?).map_err(|e| e.to_string())?;
    let mut fields = serde_json::to_value(actions).map_err(|e| e.to_string())?;
    if let (Value::Object(named), Value::Object(fields)) = (named, &mut fields) {
        for (name, _) in named.into_iter().filter(|(_, is)| *is == Value::Bool(true)) {
            fields.insert(name, Value::Bool(on));
        }
    }
    serde_json::from_value(fields).map_err(|e| e.to_string())
}

/// A lock with what it names looked up: its lock condition, given by name or id, as the
/// condition's id.
fn resolve_lock(overview: &Overview, lock: Lock) -> Result<Lock, Failure> {
    match lock {
        Lock::Condition { condition } => {
            Ok(Lock::Condition { condition: find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?.id.clone() })
        }
        other => Ok(other),
    }
}

/// The service, as the window reaches it: the same `Remote`, so the same requests and answers. In
/// tests, one in the test's own process (`tests::serve`).
fn remote() -> Remote {
    #[cfg(test)]
    if let Some(remote) = tests::REMOTE.with(|remote| remote.borrow().clone()) {
        return remote;
    }
    Remote::for_cli()
}

/// What a command says, on standard output; in tests, kept for the test to read (`tests::cli`).
fn out(text: &str) {
    #[cfg(test)]
    if tests::SAID.with(|said| said.borrow_mut().as_mut().map(|said| said.push_str(text)).is_some()) {
        return;
    }
    print!("{text}");
}

fn overview() -> Result<Overview, Failure> {
    remote().status().map_err(failed)
}

/// One command, as the window sends it (`Remote::command`).
fn send(request: Request) -> Result<(), Failure> {
    remote().command(request).map_err(failed)
}

/// An item by its id, or else by its name, ignoring case. `key` gives an item's id and name.
fn find<'a, T>(items: &'a [T], wanted: &str, kind: &str, key: impl Fn(&T) -> (&String, &String)) -> Result<&'a T, Failure> {
    if let Some(item) = items.iter().find(|i| key(i).0 == wanted) {
        return Ok(item);
    }
    let named: Vec<&T> = items.iter().filter(|i| key(i).1.trim().eq_ignore_ascii_case(wanted.trim())).collect();
    match named.as_slice() {
        [item] => Ok(item),
        [] => Err(failed(format!("No {kind} called “{wanted}”; `bullseye status` lists them"))),
        _ => Err(failed(format!(
            "Several of the {kind}s are called “{wanted}”; use the id instead: {}",
            named.iter().map(|i| key(i).0.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// An id for something new: letters, digits and `-`, as the service wants, and different from any
/// other.
fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("cli-{nanos:x}-{:x}", std::process::id())
}

/// Asks a force/no question in the terminal; `force` (from `--force`) answers it beforehand. Without a
/// terminal there is nobody to ask, so `--force` is needed.
fn confirm(question: &str, force: bool) -> Result<bool, Failure> {
    if force {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(failed(format!("{question}\nNo terminal to ask in; add --force to go ahead.")));
    }
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).map_err(|e| failed(e.to_string()))?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// A password, typed after `prompt` without showing it; from standard input as it is when that is
/// not a terminal (`echo pw | bullseye unlock`).
fn read_password(prompt: &str) -> Result<String, String> {
    use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_ECHO_INPUT, STD_INPUT_HANDLE};

    let terminal = std::io::stdin().is_terminal();
    let mut mode = 0;
    // SAFETY: plain console calls on this process's own input handle, with a valid place to write.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let hidden = terminal && unsafe { GetConsoleMode(handle, &mut mode) } != 0;
    if terminal {
        eprint!("{prompt}");
    }
    if hidden {
        // SAFETY: as above; the mode is put back below.
        unsafe { SetConsoleMode(handle, mode & !ENABLE_ECHO_INPUT) };
    }
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if hidden {
        // SAFETY: as above.
        unsafe { SetConsoleMode(handle, mode) };
        eprintln!();
    }
    read.map_err(|e| format!("Cannot read the password: {e}"))?;
    let password = line.trim_end_matches(['\r', '\n']).to_string();
    if password.is_empty() {
        return Err("No password given".into());
    }
    Ok(password)
}
