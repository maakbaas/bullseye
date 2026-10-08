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

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::allowance::{Anchor, DelayKind, Reset, Scope, Status as AllowanceStatus, Terms};
use crate::client::Remote;
use crate::condition::{Mode, Rule};
use crate::definitions::{self, Condition, Definition, Kind, Library, Target};
use crate::duration;
use crate::lists::{Entries, Lists};
use crate::localtime;
use crate::lock::{Lock, TimedKind};
use crate::proto::{Overview, Request, UnlockStatus};
use crate::schedule::Week;
use crate::session::Running;
use crate::state::Actions;
use crate::webhook::{self, Hook, Match, Matcher, Method};

/// Each command the window can invoke, and the CLI commands that do the same. Empty: it is about
/// the window itself.
pub const GUI_COMMANDS: [(&str, &[&str]); 32] = [
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

/// Where the descriptions start in `HELP`, and how wide a line of it may be.
const HELP_COLUMN: usize = 36;
const HELP_WIDTH: usize = 100;

const HELP: &str = "\
Bullseye command line: everything the window can do.

Usage: bullseye <command> [options]
A <session>, <target> or <condition> is its name (any case) or its id. An option that takes several
items takes the words after it, up to the next option: --sites youtube.com reddit.com. An item
with a space in it goes in double quotes: --titles \"Team chat\" *YouTube*.

A session is what you start: one or more targets (what to block), a condition (when it is
active; 'always', a weekly schedule, or a flash drive that is not connected), at most one
allowance (what lets it go quiet for a while) and a lock (how it is kept from being stopped).

Sessions:
  status [--json]                   Every session (running or not, active or not, locked or not),
                                    every target and every condition.
  show <session> [--json]           One session: its targets, condition, lock and state.
  new <name> [options]              Make a session. Options:
      --targets <targets>           The targets it enforces.
      --condition <condition>       When it is active: a condition's name, or 'always' (the
                                    default).
      --allowance <budget|pomodoro|delayed|none>
                                    What lets it go quiet for a while (none by default): a budget of
                                    use, a Pomodoro cycle, or a delayed start; use is what the app
                                    sees: a page shown in a tab, a window, a program that runs.
                                    Given with the options below, it can be left out (they say
                                    which); edit changes only what they name.
      --allowed <duration>          Budget: the time allowed, e.g. 30m. Pomodoro: how long
                                    everything is allowed for each cycle.
      --scope <total|each>          Budget: for the whole session (the default) or for each site,
                                    program, Store app and window title on its own.
      --reset <never|condition|duration>
                                    Budget: when what was used is given back: never while the
                                    session runs (the default), each time the session's condition
                                    turns on, or every so long, e.g. 1d or 12h, counted by --anchor.
      --blocked <duration>          Pomodoro: how long it is blocked for before it allows.
      --after <duration>            Delayed start: nothing is enforced for this long after the
                                    session starts, e.g. 30m, and everything is from then on.
      --from <date time>            Delayed start: nothing is enforced until then, e.g.
                                    \"2026-10-06 18:00\" (local time).
      --anchor <start|clock>        What a Pomodoro, or a budget's reset every so long, is counted
                                    from: the session's start (the default), or midnight on the
                                    clock, so an hour starts at xx:00 (it must then fit a day
                                    evenly).
      --lock-duration <duration>    Locked for this long once started, e.g. 2h 30m: it cannot be
                                    stopped and ends by itself.
      --lock-until <date time>      Locked until then, e.g. \"2026-10-06 18:00\" (local time): it
                                    cannot be stopped and ends by itself.
      --lock-until-restart          Locked until Windows restarts, and ends then. A shut down with
                                    Fast Startup is not a restart.
      --lock-condition <condition>  Locked while this condition is active: it cannot be stopped
                                    then, and runs until stopped otherwise. With the session's own
                                    condition, it is locked exactly while it is active.
      --lock-password <pw>          Locked with a password: it runs until stopped with that password
                                    (or by unlocking). Only a hash of it is kept.
      --lock-delay <time>           Locked behind a wait, like unlocking: stopping is asked for,
                                    counts down this long (e.g. 30m), can be aborted, and then it
                                    can be stopped.
      --lock-text <text>            Locked until the text is typed in exactly: the window shows it
                                    above an input that cannot be pasted into, and that is the only
                                    place to stop such a session (or by unlocking).
      --no-lock                     Not locked: it can be stopped at any time (the default).
  edit <session> [options]          Change a session: the options of new, which replace what they
                                    name (--targets replaces all its targets), and --name <name>. A
                                    running session takes the change at once (not its lock); while
                                    it is locked, only its name and a stricter allowance: a budget
                                    shorter or reset less often, or none; a cycle cannot change.
  add <session> --targets <targets>
                                    Add targets to a session, keeping the ones it has.
  remove <session> --targets <targets>
                                    Take targets out of a session.
  delete <session> [--force]        Delete a session that is not running. Asks first, unless
                                    --force.
  duplicate <session>               Make a copy of a session, named after it with a number:
                                    \"Work (2)\".
  start <session> [--force]         Start a session. The lock options of new set its lock first, as
                                    the lock on its tile in the window does. A locked session asks
                                    first, unless --force.
  lock <session> <lock option> [--force]
                                    Lock a running session that is not locked right now, from now
                                    on, with one of the lock options of new (--no-lock takes its
                                    lock away). It is also the lock its next start has. Asks first,
                                    unless --force.
  stop <session> [--password <pw>]  Stop a running session that is not locked right now; one locked
                                    with a password with its password (asked for when not given).
                                    One locked with a text is stopped in the window, by typing the
                                    text. One locked with a delay: the first stop asks for it and
                                    starts the wait, the next one, once it is over, stops the
                                    session; `stop <session> --abort` ends the wait.

Targets:
  target show <target> [--json]     One target, and the sessions that use it.
  target apps [--json]              The Store apps installed, by name and package family name, to
                                    block with --store-apps.
  target new <name> [options]       Make a target. Options:
      --sites <sites>               The sites to block: each with its subdomains and everything
                                    under it, e.g. youtube.com. * as a wildcard.
      --allowed-sites <sites>       Sites that are always allowed, also when blocked.
      --programs <names>            Programs to close, by exe name, e.g. steam.exe.
      --store-apps <families>       Store apps to close, by their exact package family names, e.g.
                                    Microsoft.WindowsCalculator_8wekyb3d8bbwe, as 'target apps'
                                    shows them; no *.
      --titles <titles>             Windows to close, by their title, * as a wildcard, e.g.
                                    *YouTube*. The whole window closes, tabs and all.
      --allowed-titles <titles>     Window titles that are always left alone, also when one of the
                                    titles matches them.
      --sites-file <file>           Sites to block, one per line. Likewise --allowed-sites-file,
                                    --programs-file, --store-apps-file, --titles-file and
                                    --allowed-titles-file.
      --actions <names>             The actions, or 'all' (the default). Names:
                                    {actions}
  target edit <target> [options]    Change a target: the options of target new, which replace what
                                    they name, --name <name> and --clear <lists>, the lists to
                                    empty: sites, allowed-sites, programs, store-apps, titles,
                                    allowed-titles. While a locked session uses it, only stricter:
                                    more to block, fewer allowed, more actions.
  target add <target> [options]     Add entries to the lists of a target, keeping what it has: the
                                    list options of target new (--sites, --programs, …), and
                                    --actions to turn on. While a locked session uses it, adding
                                    entries to block is allowed.
  target remove <target> [options]  Take entries out of a target, however they are written (Steam is
                                    steam.exe), and turn --actions off. One that is not in it is
                                    refused, as a typo.
  target delete <target> [--force]  Delete a target that no running session uses; it is taken out of
                                    the sessions that use it (one left with none does
                                    nothing).
  target duplicate <target>         Make a copy of a target, named after it with a number.

Conditions:
  condition show <condition>        One condition, and the sessions that use it.
  condition new <name> (--times <times> | --drive <drive> | --webhook <url> [webhook options] | --any <conditions> | --all <conditions>)
                                    Make a condition: a weekly schedule, e.g. --times
                                    \"Mon-Fri 08:00-17:00; Sat 10-12\"; or a flash drive, by name or
                                    serial number ('drives' lists them): active while it is not
                                    connected; or a combination of two or more other conditions
                                    (--any or --all, separated by spaces): active when any of them
                                    is, or when all are. Not of other combinations. A webhook asks
                                    one web address (it can hold GET parameters) every --every
                                    <time> (at least 1m, the default), with --method get|post
                                    (default get). It is what --fallback active|inactive says
                                    (default active; also when no answer comes), unless the answer
                                    matches --match-status <code> or --match-body <text> (the whole
                                    body, white space around it aside): then it is the other.
  condition edit <condition> [--name <name>] [--times <times> | --drive <drive> | --webhook <url> [webhook options] | --any <conditions> | --all <conditions>]
                                    Change a condition. While a locked session uses it, a schedule
                                    can only get more times, a drive cannot change, and a
                                    combination can only get more active.
  condition add <condition> --conditions <conditions>
                                    Add conditions to a combination.
  condition remove <condition> --conditions <conditions>
                                    Take conditions out of a combination (at least two stay).
  condition delete <condition> [--force]
                                    Delete a condition that no running session uses, directly or in
                                    a combination; sessions it was the condition of are always
                                    active, and those it locked are not locked any more. It is taken
                                    out of its combinations, and one left with a single condition is
                                    deleted too.
  condition duplicate <condition>   Make a copy of a condition, named after it with a number.
  condition test <condition>        Ask a webhook condition once: the status, the start of the body,
                                    and what the match makes of it.
  condition check <times>           Check times and show them per day, as a schedule takes them.

Moving to another computer:
  export <file>                     Write every target, condition and session to a settings file, as
                                    Export in Settings does. Password locks are in it without their
                                    password.
  import <file> [--force]           Replace every target, condition and session with those in a
                                    settings file, as Import in Settings does; not while a session
                                    is locked, and sessions that run unlocked end with it. A session
                                    with a password lock comes without a lock. Asks first, unless
                                    --force.

Other:
  drives [--json]                   The flash drives connected, by name and serial number, for
                                    'condition new --drive'.
  unlock [--abort] [--password <pw>]
                                    End every running session, as the window's Unlock does. By
                                    default unlocking is free. With an unlock delay it is a
                                    countdown: the first 'unlock' starts it, a later one ends every
                                    session once it is over, and --abort stops it (back to locked).
                                    The other way to unlock (below) works instead of the countdown,
                                    never as well: with a password (asked for when not given)
                                    unlocking is at once; one set to a text is used in the window,
                                    by typing the text.
  unlock-way [options]              Show the other way to unlock besides the delay, or set it:
                                    --condition <condition> (locked while it is active, open when it
                                    is not), --password <pw>, --text <text> or --none to remove it.
                                    Setting one needs unlocking to be ready, or no session to be
                                    locked; removing it is free.
  unlock-delay [<time>]             Show how long unlocking takes, or set it: 0 to 24 hours, e.g.
                                    30m or 2h 30m, or 0 for none. A longer delay is free; a shorter
                                    one needs unlocking to be ready, or no session to be locked.
  help                              Show this.
  version                           Show the version.

Durations: 30s, 10m, 2h, 1d, or several together: 2h 30m, 1d 12h.
Exit codes: 0 done, 1 failed, refused or cancelled, 2 not understood.";

/// What the CLI was asked to do.
#[derive(Debug, PartialEq)]
enum Command {
    Help,
    Version,
    Status {
        json: bool,
    },
    Show {
        session: String,
        json: bool,
    },
    New {
        name: String,
        changes: SessionChanges,
    },
    Edit {
        session: String,
        changes: SessionChanges,
    },
    Delete {
        session: String,
        force: bool,
    },
    Start {
        session: String,
        lock: Option<Lock>,
        force: bool,
    },
    Lock {
        session: String,
        lock: Lock,
        force: bool,
    },
    Stop {
        session: String,
        password: Option<String>,
        abort: bool,
    },
    TargetShow {
        target: String,
        json: bool,
    },
    TargetApps {
        json: bool,
    },
    Drives {
        json: bool,
    },
    TargetNew {
        name: String,
        changes: TargetChanges,
    },
    TargetEdit {
        target: String,
        changes: TargetChanges,
    },
    TargetDelete {
        target: String,
        force: bool,
    },
    /// `add` / `remove`: targets of a session.
    SessionItems {
        session: String,
        targets: Vec<String>,
        add: bool,
    },
    /// `target add` / `target remove`: entries of a target's lists, by their field of `Entries`,
    /// and actions.
    TargetItems {
        target: String,
        entries: BTreeMap<&'static str, Vec<String>>,
        actions: Vec<String>,
        add: bool,
    },
    /// `condition add` / `condition remove`: the conditions of a combination.
    ConditionItems {
        condition: String,
        members: Vec<String>,
        add: bool,
    },
    ConditionShow {
        condition: String,
    },
    ConditionNew {
        name: String,
        rule: RuleChoice,
    },
    ConditionEdit {
        condition: String,
        name: Option<String>,
        rule: Option<RuleChoice>,
    },
    ConditionDelete {
        condition: String,
        force: bool,
    },
    ConditionCheck {
        times: String,
    },
    ConditionTest {
        condition: String,
    },
    Export {
        file: String,
    },
    Import {
        file: String,
        force: bool,
    },
    /// `item` of the kind, by name or id.
    Duplicate {
        kind: Kind,
        item: String,
    },
    Unlock {
        abort: bool,
        password: Option<String>,
    },
    /// `None`: show the other way to unlock; `Some(None)`: remove it; `Some(Some(lock))`: set it.
    UnlockWay {
        way: Option<Option<Lock>>,
    },
    UnlockDelay {
        delay: Option<String>,
    },
}

/// The rule a condition is given: weekly times as typed, or a flash drive by name or serial number;
/// read when the command runs (`resolve_rule`).
#[derive(Debug, PartialEq)]
enum RuleChoice {
    Times(String),
    Drive(String),
    /// Conditions by name or id, of which any one is enough (`--any`).
    Any(Vec<String>),
    /// Conditions by name or id, all of which are needed (`--all`).
    All(Vec<String>),
    /// A web address that is asked (`--webhook`, with its options).
    Webhook(Hook),
}

/// What `new` and `edit` set on a session; `None` leaves it. Lists and the condition by name or id,
/// looked up when the command runs.
#[derive(Debug, PartialEq, Default)]
struct SessionChanges {
    name: Option<String>,
    targets: Option<Vec<String>>,
    /// `Some(None)`: always.
    condition: Option<Option<String>>,
    /// What `--allowance` and its options set; `None`: they were not given.
    allowance: Option<AllowanceChanges>,
    lock: Option<Lock>,
}

/// What `--allowance` and its options of `new` and `edit` set on a session's allowance; `None` leaves
/// it (with no allowance yet, the default). Read when the command runs (`build_terms`).
#[derive(Debug, PartialEq, Default)]
struct AllowanceChanges {
    /// `budget`, `pomodoro`, `delayed` or `none`.
    kind: Option<String>,
    allowed: Option<String>,
    scope: Option<Scope>,
    reset: Option<Reset>,
    /// With `Reset::Every`: how often, as typed (`1d`).
    every: Option<String>,
    blocked: Option<String>,
    anchor: Option<Anchor>,
    /// A delayed start: how long after the session starts, or the date and time, enforcing begins.
    after: Option<String>,
    from: Option<String>,
}

/// What `target new` and `target edit` set on a target; `None` leaves it.
#[derive(Debug, PartialEq, Default)]
struct TargetChanges {
    name: Option<String>,
    /// The lists to set, by their field of `Entries`.
    lists: BTreeMap<&'static str, Vec<String>>,
    actions: Option<Actions>,
}

/// The options that set a list, one per list: the option (its items; `<option>-file` reads them
/// from a file, one per line, and `--clear` names it without the dashes) and its field of
/// `Entries`. **A new list is a line here.**
const LIST_OPTIONS: [(&str, &str); 6] = [
    ("--sites", "filters"),
    ("--allowed-sites", "allowed"),
    ("--programs", "programs"),
    ("--store-apps", "store_apps"),
    ("--titles", "titles"),
    ("--allowed-titles", "allowed_titles"),
];

impl TargetChanges {
    fn apply(self, list: &mut Target) {
        if let Some(name) = self.name {
            list.name = name;
        }
        for (key, entries) in self.lists {
            if let Some(field) = list.entries.field(key) {
                *field = entries;
            }
        }
        if let Some(actions) = self.actions {
            list.actions = actions;
        }
    }
}

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

fn parse(args: &[String]) -> Result<Command, String> {
    let Some((command, rest)) = args.split_first() else { return Ok(Command::Help) };
    // `target` and `condition` take a second word.
    let (command, rest) = match (command.as_str(), rest.split_first()) {
        ("target" | "condition", Some((verb, rest))) => (format!("{command} {verb}"), rest),
        ("target" | "condition", None) => {
            return Err(format!(
                "`{command}` needs show, new, edit, add, remove, delete, duplicate{}",
                if command == "condition" { " or check" } else { "" }
            ))
        }
        _ => (command.clone(), rest),
    };
    let mut options = Options::new(rest);
    let named = |options: &mut Options, what: &str| options.positional().ok_or_else(|| format!("Say which {what}: its name or its id"));
    let parsed = match command.as_str() {
        "help" | "--help" | "-h" | "/?" => Command::Help,
        "version" | "--version" | "-V" => Command::Version,
        "status" => Command::Status { json: options.flag("--json") },
        "show" => Command::Show { session: named(&mut options, "session")?, json: options.flag("--json") },
        "new" => {
            let name = options.positional().ok_or("Give the new session a name")?;
            Command::New { name, changes: parse_session_changes(&mut options, false)? }
        }
        "edit" => Command::Edit { session: named(&mut options, "session")?, changes: parse_session_changes(&mut options, true)? },
        "add" | "remove" => Command::SessionItems {
            session: named(&mut options, "session")?,
            targets: options.items("--targets")?.ok_or(format!("Give the targets to {command}: --targets <targets>"))?,
            add: command == "add",
        },
        "delete" => Command::Delete { session: named(&mut options, "session")?, force: options.flag("--force") },
        "export" => Command::Export { file: options.positional().ok_or("Give the file to write, e.g. bullseye.json")? },
        "import" => Command::Import { file: options.positional().ok_or("Give the settings file to read")?, force: options.flag("--force") },
        "duplicate" => Command::Duplicate { kind: Kind::Session, item: named(&mut options, "session")? },
        "target duplicate" => Command::Duplicate { kind: Kind::Target, item: named(&mut options, "target")? },
        "condition duplicate" => Command::Duplicate { kind: Kind::Condition, item: named(&mut options, "condition")? },
        "start" => {
            Command::Start { session: named(&mut options, "session")?, lock: parse_lock(&mut options)?, force: options.flag("--force") }
        }
        "lock" => Command::Lock {
            session: named(&mut options, "session")?,
            lock: parse_lock(&mut options)?.ok_or("Give the lock, with one of the lock options of new: --lock-duration 2h")?,
            force: options.flag("--force"),
        },
        "stop" => Command::Stop {
            session: named(&mut options, "session")?,
            password: options.value("--password")?,
            abort: options.flag("--abort"),
        },
        "target show" => Command::TargetShow { target: named(&mut options, "target")?, json: options.flag("--json") },
        "target apps" => Command::TargetApps { json: options.flag("--json") },
        "drives" => Command::Drives { json: options.flag("--json") },
        "target new" => {
            let name = options.positional().ok_or("Give the new target a name")?;
            Command::TargetNew { name, changes: parse_target_changes(&mut options, false)? }
        }
        "target edit" => Command::TargetEdit { target: named(&mut options, "target")?, changes: parse_target_changes(&mut options, true)? },
        "target add" | "target remove" => {
            let list = named(&mut options, "target")?;
            let entries = parse_list_entries(&mut options, &[])?;
            let actions = options.items("--actions")?.unwrap_or_default();
            if entries.is_empty() && actions.is_empty() {
                return Err(format!("Give what to {}: --sites, --programs, --titles, … or --actions", &command[7..]));
            }
            Command::TargetItems { target: list, entries, actions, add: command == "target add" }
        }
        "target delete" => Command::TargetDelete { target: named(&mut options, "target")?, force: options.flag("--force") },
        "condition show" => Command::ConditionShow { condition: named(&mut options, "condition")? },
        "condition new" => {
            let name = options.positional().ok_or("Give the new condition a name")?;
            let rule = parse_rule(&mut options)?.ok_or(
                "Give --times \"Mon-Fri 08:00-17:00\" for a weekly schedule, --drive <flash drive>, or --any / --all <condition> (twice or more) to combine conditions",
            )?;
            Command::ConditionNew { name, rule }
        }
        "condition edit" => Command::ConditionEdit {
            condition: named(&mut options, "condition")?,
            name: options.value("--name")?,
            rule: parse_rule(&mut options)?,
        },
        "condition add" | "condition remove" => Command::ConditionItems {
            condition: named(&mut options, "condition")?,
            members: options
                .items("--conditions")?
                .ok_or(format!("Give the conditions to {}: --conditions <conditions>", &command[10..]))?,
            add: command == "condition add",
        },
        "condition delete" => Command::ConditionDelete { condition: named(&mut options, "condition")?, force: options.flag("--force") },
        "condition test" => Command::ConditionTest { condition: named(&mut options, "condition")? },
        "condition check" => {
            Command::ConditionCheck { times: options.positional().ok_or("Give the times to check, e.g. \"Mon-Fri 08:00-17:00\"")? }
        }
        "unlock" => Command::Unlock { abort: options.flag("--abort"), password: options.value("--password")? },
        "unlock-way" => Command::UnlockWay { way: parse_way(&mut options)? },
        "unlock-delay" => Command::UnlockDelay { delay: options.positional() },
        other => return Err(format!("Unknown command \"{other}\"")),
    };
    options.finish()?;
    Ok(parsed)
}

/// `--lock-duration <duration>`, `--lock-until <date time>`, `--lock-until-restart`,
/// `--lock-condition <condition>`, `--lock-password <pw>`, `--lock-text <text>` or `--no-lock`, at most one. A lock
/// condition by name or id, looked up when the command runs (`resolve_lock`).
fn parse_lock(options: &mut Options) -> Result<Option<Lock>, String> {
    let duration = options.value("--lock-duration")?.map(Lock::timed_for);
    let until = options.value("--lock-until")?.map(Lock::timed_until);
    let restart = options.flag("--lock-until-restart").then_some(Lock::UntilRestart);
    let conditional = options.value("--lock-condition")?.map(|condition| Lock::Condition { condition });
    let password = options.value("--lock-password")?.map(|password| Lock::Password { password: Some(password), hash: None });
    let typed = options.value("--lock-text")?.map(|text| Lock::Text { text });
    let delayed = options.value("--lock-delay")?.map(|delay| Lock::Delay { delay });
    let unlocked = options.flag("--no-lock").then_some(Lock::None);
    let mut given: Vec<Lock> = [duration, until, restart, conditional, password, typed, delayed, unlocked].into_iter().flatten().collect();
    let lock = match given.len() {
        0 => return Ok(None),
        1 => given.remove(0),
        _ => return Err(
            "Give one of --lock-duration, --lock-until, --lock-until-restart, --lock-condition, --lock-password, --lock-text, --lock-delay and --no-lock"
                .into(),
        ),
    };
    // The service checks it again; this is for a clear message before anything is sent.
    lock.check()?;
    Ok(Some(lock))
}

/// `--condition <condition>`, `--password <pw>`, `--text <text>` or `--none` of `unlock-way`: the other way
/// to unlock as a lock method, at most one; nothing given is showing it.
fn parse_way(options: &mut Options) -> Result<Option<Option<Lock>>, String> {
    let mut given: Vec<Option<Lock>> = Vec::new();
    given.extend(options.value("--condition")?.map(|condition| Some(Lock::Condition { condition })));
    given.extend(options.value("--password")?.map(|password| Some(Lock::Password { password: Some(password), hash: None })));
    given.extend(options.value("--text")?.map(|text| Some(Lock::Text { text })));
    given.extend(options.flag("--none").then_some(None));
    match given.len() {
        0 => Ok(None),
        1 => {
            let way = given.remove(0);
            if let Some(lock) = &way {
                lock.check()?;
            }
            Ok(Some(way))
        }
        _ => Err("Give one of --condition, --password, --text and --none".into()),
    }
}

/// The options of `new` and `edit`. Only `edit` takes `--name`.
fn parse_session_changes(options: &mut Options, editing: bool) -> Result<SessionChanges, String> {
    Ok(SessionChanges {
        name: if editing { options.value("--name")? } else { None },
        targets: options.items("--targets")?,
        condition: options.value("--condition")?.map(|c| (!c.eq_ignore_ascii_case("always")).then_some(c)),
        allowance: parse_allowance_changes(options)?,
        lock: parse_lock(options)?,
    })
}

/// `--allowance`, `--allowed`, `--scope`, `--reset`, `--blocked` and `--anchor` of a session; `None` when
/// none of them is given. The words are checked here; whether they make terms is `build_terms`'s to
/// say when the command runs.
fn parse_allowance_changes(options: &mut Options) -> Result<Option<AllowanceChanges>, String> {
    let kind = options.value("--allowance")?.map(|kind| kind.to_lowercase());
    if let Some(kind) = &kind {
        if !["budget", "pomodoro", "delayed", "none"].contains(&kind.as_str()) {
            return Err(format!("--allowance takes budget, pomodoro, delayed or none, not \"{kind}\""));
        }
    }
    let allowed = options.value("--allowed")?;
    let scope = options
        .value("--scope")?
        .map(|scope| match scope.to_lowercase().as_str() {
            "total" => Ok(Scope::Total),
            "each" => Ok(Scope::Each),
            other => Err(format!("--scope takes total or each, not \"{other}\"")),
        })
        .transpose()?;
    // Never, each time the condition turns on, or every so long (a duration).
    let (reset, every) = match options.value("--reset")? {
        None => (None, None),
        Some(reset) => match reset.trim().to_lowercase().as_str() {
            "never" => (Some(Reset::Never), None),
            "condition" => (Some(Reset::Condition), None),
            _ => match duration::parse(&reset) {
                Ok(Some(_)) => (Some(Reset::Every), Some(reset.trim().to_string())),
                _ => return Err(format!("--reset takes never, condition or a duration such as 1d, not \"{reset}\"")),
            },
        },
    };
    let blocked = options.value("--blocked")?;
    let anchor = options
        .value("--anchor")?
        .map(|anchor| match anchor.to_lowercase().as_str() {
            "start" => Ok(Anchor::Start),
            "clock" => Ok(Anchor::Clock),
            other => Err(format!("--anchor takes start or clock, not \"{other}\"")),
        })
        .transpose()?;
    let after = options.value("--after")?;
    let from = options.value("--from")?;
    let changes = AllowanceChanges { kind, allowed, scope, reset, every, blocked, anchor, after, from };
    if changes == AllowanceChanges::default() {
        return Ok(None);
    }
    let delayed_given = changes.after.is_some() || changes.from.is_some();
    let terms_given = changes.allowed.is_some()
        || changes.scope.is_some()
        || changes.reset.is_some()
        || changes.blocked.is_some()
        || changes.anchor.is_some()
        || delayed_given;
    if changes.kind.as_deref() == Some("none") && terms_given {
        return Err("--allowance none takes no other allowance options".into());
    }
    if (delayed_given || changes.kind.as_deref() == Some("delayed"))
        && (changes.allowed.is_some()
            || changes.scope.is_some()
            || changes.reset.is_some()
            || changes.blocked.is_some()
            || changes.anchor.is_some())
    {
        return Err("A delayed start takes only --after or --from".into());
    }
    if delayed_given && changes.kind.as_deref().is_some_and(|kind| kind != "delayed") {
        return Err("--after and --from go with --allowance delayed".into());
    }
    if changes.after.is_some() && changes.from.is_some() {
        return Err("Give --after or --from, not both".into());
    }
    Ok(Some(changes))
}

/// An allowance's terms from what was given, over the ones it has (`old`, when editing): what is not
/// named stays, and a new one has defaults (the whole session, never reset, counted from the start).
/// `--blocked` makes a Pomodoro, `--scope` or `--reset` a budget, `--after` or `--from` a delayed
/// start, and `--allowed` or `--anchor` alone keep the kind there is. Checked, so a mistake is told
/// before anything is sent.
fn build_terms(changes: AllowanceChanges, old: Option<&Terms>) -> Result<Terms, String> {
    let delayed = match changes.kind.as_deref() {
        Some(kind) => kind == "delayed",
        None => changes.after.is_some() || changes.from.is_some(),
    };
    if delayed {
        let old = match old {
            Some(Terms::Delayed { kind, value }) => Some((*kind, value.clone())),
            _ => None,
        };
        let (kind, value) = match (changes.after, changes.from) {
            (Some(value), _) => (DelayKind::After, value),
            (None, Some(value)) => (DelayKind::From, value),
            (None, None) => old.ok_or("Give --after, how long to wait before enforcing, e.g. 30m, or --from, a date and time")?,
        };
        return Terms::Delayed { kind, value }.checked();
    }
    let pomodoro = match changes.kind.as_deref() {
        Some(kind) => kind == "pomodoro",
        None if changes.blocked.is_some() => true,
        None if changes.scope.is_some() || changes.reset.is_some() => false,
        None => matches!(old, Some(Terms::Pomodoro { .. })),
    };
    let terms = if pomodoro {
        let (old_blocked, old_allowed, old_anchor) = match old {
            Some(Terms::Pomodoro { blocked, allowed, anchor }) => (Some(blocked.clone()), Some(allowed.clone()), Some(*anchor)),
            _ => (None, None, None),
        };
        Terms::Pomodoro {
            blocked: changes.blocked.or(old_blocked).ok_or("Give --blocked, how long it is blocked for each cycle, e.g. 50m")?,
            allowed: changes.allowed.or(old_allowed).ok_or("Give --allowed, how long everything is allowed for each cycle, e.g. 10m")?,
            anchor: changes.anchor.or(old_anchor).unwrap_or(Anchor::Start),
        }
    } else {
        let (old_allowed, old_scope, old_reset, old_every, old_anchor) = match old {
            Some(Terms::Budget { allowed, scope, reset, every, anchor }) => {
                (Some(allowed.clone()), Some(*scope), Some(*reset), Some(every.clone()), Some(*anchor))
            }
            _ => (None, None, None, None, None),
        };
        Terms::Budget {
            allowed: changes.allowed.or(old_allowed).ok_or("Give --allowed, the time that is allowed, e.g. 30m")?,
            scope: changes.scope.or(old_scope).unwrap_or(Scope::Total),
            reset: changes.reset.or(old_reset).unwrap_or(Reset::Never),
            every: changes.every.or(old_every).unwrap_or_default(),
            anchor: changes.anchor.or(old_anchor).unwrap_or(Anchor::Start),
        }
    };
    terms.checked()
}

/// `--times <times>`, `--drive <drive>`, `--any <conditions>` or `--all <conditions>` of a
/// condition, at most one kind.
fn parse_rule(options: &mut Options) -> Result<Option<RuleChoice>, String> {
    let mut given: Vec<RuleChoice> = Vec::new();
    given.extend(options.value("--times")?.map(RuleChoice::Times));
    given.extend(options.value("--drive")?.map(RuleChoice::Drive));
    given.extend(parse_webhook(options)?.map(RuleChoice::Webhook));
    given.extend(options.items("--any")?.map(RuleChoice::Any));
    given.extend(options.items("--all")?.map(RuleChoice::All));
    match given.len() {
        0 | 1 => Ok(given.pop()),
        _ => Err("Give one of --times, --drive, --webhook, --any or --all".into()),
    }
}

/// `--webhook <url>` with `--method get|post`, `--every <time>` (at least 1m, default 1m), `--fallback
/// active|inactive` (default active: what the condition is unless the answer matches) and the one
/// match that makes it the other: `--match-status <code>` or `--match-body <text>`.
fn parse_webhook(options: &mut Options) -> Result<Option<Hook>, String> {
    let Some(url) = options.value("--webhook")? else { return Ok(None) };
    let method = match options.value("--method")?.map(|m| m.to_lowercase()).as_deref() {
        None | Some("get") => Method::Get,
        Some("post") => Method::Post,
        Some(other) => return Err(format!("--method is get or post, not \"{other}\"")),
    };
    let interval = match options.value("--every")? {
        Some(text) => duration::parse(&text)?.ok_or("Give the time between requests, e.g. 5m")?.as_secs(),
        None => 60,
    };
    let fallback = match options.value("--fallback")?.map(|f| f.to_lowercase()).as_deref() {
        None | Some("active") => true,
        Some("inactive") => false,
        Some(other) => return Err(format!("--fallback is active or inactive, not \"{other}\"")),
    };
    let status = options.value("--match-status")?;
    let body = options.value("--match-body")?;
    let matcher = match (status, body) {
        (Some(status), None) => Matcher {
            on: Match::Status,
            status: Some(status.trim().parse::<u16>().map_err(|_| "--match-status is a number, e.g. 200")?),
            body: None,
        },
        (None, Some(body)) => Matcher { on: Match::Body, status: None, body: Some(body) },
        _ => {
            return Err(format!(
                "Give one of --match-status and --match-body: the answer that makes the condition {}",
                if fallback { "not active" } else { "active" }
            ))
        }
    };
    let hook = Hook { url, method, interval, fallback, matcher };
    hook.check()?;
    Ok(Some(hook))
}

/// The options of `target new` and `target edit`. Only `target edit` takes `--name` and `--clear`.
fn parse_target_changes(options: &mut Options, editing: bool) -> Result<TargetChanges, String> {
    let mut changes = TargetChanges::default();
    let cleared = if editing { options.items("--clear")?.unwrap_or_default() } else { Vec::new() };
    let names: Vec<&str> = LIST_OPTIONS.iter().map(|(option, _)| &option[2..]).collect();
    for list in &cleared {
        if !names.contains(&list.as_str()) {
            return Err(format!("--clear takes {}, not \"{list}\"", names.join(", ")));
        }
    }
    changes.lists = parse_list_entries(options, &cleared)?;
    if editing {
        changes.name = options.value("--name")?;
    }
    changes.actions = match options.items("--actions")? {
        Some(names) => Some(parse_actions(&names)?),
        None if editing => None,
        None => Some(parse_actions(&["all".to_string()])?),
    };
    Ok(changes)
}

/// The lists given (`--sites`, `--sites-file`, …) by their field of `Entries`, and those named in
/// `cleared` as empty when not given.
fn parse_list_entries(options: &mut Options, cleared: &[String]) -> Result<BTreeMap<&'static str, Vec<String>>, String> {
    let mut lists = BTreeMap::new();
    for (option, key) in LIST_OPTIONS {
        let given = options.items(option)?;
        let from_file = options.value(&format!("{option}-file"))?;
        let mut entries = given.clone().unwrap_or_default();
        if let Some(path) = &from_file {
            let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
            entries.extend(text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string));
        }
        if given.is_some() || from_file.is_some() || cleared.iter().any(|c| c == &option[2..]) {
            lists.insert(key, entries);
        }
    }
    Ok(lists)
}

/// The names of the actions, as `Actions` calls its fields: a new action is known here
/// without a change.
/// The help, with the action names where `{actions}` stands: as many to a line as fit, under the
/// text above them (the descriptions start in column `HELP_COLUMN`).
fn help() -> String {
    let (indent, width) = (" ".repeat(HELP_COLUMN), HELP_WIDTH - HELP_COLUMN);
    let mut lines: Vec<String> = Vec::new();
    for name in action_names() {
        match lines.last_mut() {
            Some(line) if line.len() + 2 + name.len() < width => {
                line.push_str(", ");
                line.push_str(&name);
            }
            _ => lines.push(name),
        }
    }
    HELP.replace("{actions}", &lines.join(&format!(",\n{indent}")))
}

fn action_names() -> Vec<String> {
    match serde_json::to_value(Actions::default()) {
        Ok(Value::Object(fields)) => fields.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// The actions named (`chromium_tabs firefox_tabs`), or `all`: every action as a new target has them (windows by
/// title closed, not minimized: `window_titles_minimize` says to minimize them).
fn parse_actions(given: &[String]) -> Result<Actions, String> {
    let names = action_names();
    let mut chosen: Map<String, Value> = names.iter().map(|name| (name.clone(), Value::Bool(false))).collect();
    for name in given.iter().map(String::as_str) {
        if name == "all" {
            if let Ok(Value::Object(all)) = serde_json::to_value(Actions::default()) {
                for (name, on) in all.into_iter().filter(|(_, on)| *on == Value::Bool(true)) {
                    chosen.insert(name, on);
                }
            }
        } else if let Some(on) = chosen.get_mut(name) {
            *on = Value::Bool(true);
        } else {
            return Err(format!("Unknown action \"{name}\"; the actions are {}", names.join(", ")));
        }
    }
    serde_json::from_value(Value::Object(chosen)).map_err(|e| e.to_string())
}

/// The arguments after the command, taken out one kind at a time; whatever is left is an error.
struct Options {
    args: Vec<String>,
}

impl Options {
    fn new(args: &[String]) -> Options {
        Options { args: args.to_vec() }
    }

    fn flag(&mut self, name: &str) -> bool {
        let before = self.args.len();
        self.args.retain(|arg| arg != name);
        self.args.len() != before
    }

    /// Every `--name value` and `--name=value`.
    fn values(&mut self, name: &str) -> Result<Vec<String>, String> {
        let mut values = Vec::new();
        let mut i = 0;
        while i < self.args.len() {
            if self.args[i] == name {
                if i + 1 >= self.args.len() {
                    return Err(format!("{name} needs a value"));
                }
                values.push(self.args.remove(i + 1));
                self.args.remove(i);
            } else if let Some(value) = self.args[i].strip_prefix(name).and_then(|rest| rest.strip_prefix('=')) {
                values.push(value.to_string());
                self.args.remove(i);
            } else {
                i += 1;
            }
        }
        Ok(values)
    }

    /// The items of a list option, `None` when it is not given: every word after it up to the next
    /// option (`--sites a.com b.com`; an item with a space in it is quoted, as the shell has it:
    /// `--titles "Team chat" *Shorts*`), and with `--name=item` that item first. Given once at most.
    fn items(&mut self, name: &str) -> Result<Option<Vec<String>>, String> {
        let Some(at) = self.args.iter().position(|arg| arg == name || arg.starts_with(&format!("{name}="))) else {
            return Ok(None);
        };
        let first = self.args.remove(at);
        let mut values: Vec<String> = first.strip_prefix(&format!("{name}=")).map(str::to_string).into_iter().collect();
        while at < self.args.len() && !self.args[at].starts_with("--") {
            values.push(self.args.remove(at));
        }
        if self.args.iter().any(|arg| arg == name || arg.starts_with(&format!("{name}="))) {
            return Err(format!("{name} is given more than once; give all its items after it at once, separated by spaces"));
        }
        let items: Vec<String> = values.iter().map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).collect();
        if items.is_empty() {
            return Err(format!("{name} needs one or more items, separated by spaces"));
        }
        Ok(Some(items))
    }

    /// At most one `--name value`.
    fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        let mut values = self.values(name)?;
        if values.len() > 1 {
            return Err(format!("{name} is given more than once"));
        }
        Ok(values.pop())
    }

    /// The first argument that is not an option. Taken before the options are, so it must come
    /// first: `edit Work --list Social`.
    fn positional(&mut self) -> Option<String> {
        match self.args.first() {
            Some(arg) if !arg.starts_with("--") => Some(self.args.remove(0)),
            _ => None,
        }
    }

    fn finish(self) -> Result<(), String> {
        match self.args.first() {
            None => Ok(()),
            Some(arg) => Err(format!("Not understood: \"{arg}\"")),
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
                let both = serde_json::json!({ "session": session, "running": running });
                say!("{}", serde_json::to_string_pretty(&both).map_err(|e| failed(e.to_string()))?);
            } else {
                out(&describe_session(&overview, session, running));
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
            say!("Made the session \"{name}\" (id {id}). Start it with: bullseye start \"{name}\"");
        }
        Command::Edit { session, changes } => {
            let overview = overview()?;
            let mut session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?.clone();
            apply_session_changes(&overview, &mut session, changes)?;
            let name = session.name.clone();
            send(Request::SaveSession { session })?;
            say!("Saved the session \"{name}\".");
        }
        Command::Delete { session, force } => {
            let overview = overview()?;
            let session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?;
            if !confirm(&format!("Delete the session \"{}\"? This cannot be undone.", session.name), force)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteSession { id: session.id.clone() })?;
            say!("Deleted the session \"{}\".", session.name);
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
            if let Some(how) = lock_warning(&overview, &session) {
                let question = format!("Start \"{}\"? {how} Only unlocking ends it early.", session.name);
                if !confirm(&question, force)? {
                    return Err(failed("Not started."));
                }
            }
            send(Request::Start { id: session.id.clone() })?;
            say!("Started \"{}\".", session.name);
        }
        Command::Lock { session, lock, force } => {
            let overview = overview()?;
            let mut session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?.clone();
            session.lock = resolve_lock(&overview, lock)?;
            // As in the window: a lock that holds is only given after a second yes.
            if let Some(how) = lock_warning(&overview, &session) {
                let question = format!("Lock \"{}\" now? {how} Only unlocking ends it early.", session.name);
                if !confirm(&question, force)? {
                    return Err(failed("Not locked."));
                }
            }
            send(Request::Lock { id: session.id.clone(), lock: session.lock.clone() })?;
            if session.lock == Lock::None {
                say!("\"{}\" runs on without a lock.", session.name);
            } else {
                say!("Locked \"{}\" now.", session.name);
            }
        }
        Command::Stop { session, password, abort } => {
            let overview = overview()?;
            let session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?;
            // A session locked with a delay: stopping is asked for, waited out, and then done.
            if let Some(run) = overview.running.iter().find(|r| r.id == session.id && r.stop_delay.is_some()) {
                if abort {
                    send(Request::AbortStop { id: session.id.clone() })?;
                    say!("Stopping \"{}\" aborted: back to locked.", session.name);
                    return Ok(());
                }
                match (run.stop_remaining, &run.stop_remaining_words) {
                    (Some(0), _) => {}
                    (Some(_), Some(left)) => {
                        say!(
                            "\"{}\" can be stopped in {left}. Run `bullseye stop` again then, or `stop --abort` to end the wait.",
                            session.name
                        );
                        return Ok(());
                    }
                    _ => {
                        send(Request::AskToStop { id: session.id.clone() })?;
                        say!(
                            "Stopping \"{}\" was asked for: ready in {}. Run `bullseye stop` again then, or `stop --abort` to end the wait.",
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
                return Err(failed(format!("\"{}\" is locked until its text is typed in: stop it in the window", session.name)));
            }
            // A session locked with a password stops with it: asked for when it was not given.
            let password_locked = overview.running.iter().any(|r| r.id == session.id && r.password);
            let password = match password {
                None if password_locked => Some(read_password(&format!("Password of \"{}\": ", session.name)).map_err(Failure::Failed)?),
                given => given,
            };
            send(Request::Stop { id: session.id.clone(), password, text: None })?;
            say!("Stopped \"{}\".", session.name);
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
            say!("Made the target \"{name}\". Use it with: bullseye new <session> --targets \"{name}\"");
        }
        Command::TargetEdit { target: list, changes } => {
            let overview = overview()?;
            let mut list = find(&overview.targets, &list, "target", |l| (&l.id, &l.name))?.clone();
            changes.apply(&mut list);
            let name = list.name.clone();
            send(Request::SaveTarget { target: list })?;
            say!("Saved the target \"{name}\".");
        }
        Command::TargetDelete { target: list, force } => {
            let overview = overview()?;
            let list = find(&overview.targets, &list, "target", |l| (&l.id, &l.name))?;
            if !confirm(&format!("Delete the target \"{}\"?{} This cannot be undone.", list.name, fallout(&overview, &list.id)), force)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteTarget { id: list.id.clone() })?;
            say!("Deleted the target \"{}\".", list.name);
        }
        Command::ConditionTest { condition } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            let Rule::Webhook(hook) = &condition.rule else {
                return Err(failed(format!("\"{}\" is not a webhook condition", condition.name)));
            };
            say!("{}", webhook::test(hook).map_err(failed)?);
        }
        Command::ConditionShow { condition } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            say!("{}", condition.name);
            match &condition.rule {
                Rule::Schedule { week } => out(&per_day(week)),
                rule => say!("{}", describe_rule(&overview, rule)),
            }
            say!("Used by: {}", users(&overview, &condition.id));
        }
        Command::ConditionNew { name, rule } => {
            let rule = resolve_rule(&overview()?, rule)?;
            send(Request::SaveCondition { condition: Condition { id: new_id(), name: name.clone(), rule } })?;
            say!("Made the condition \"{name}\". Use it with: bullseye new <session> --condition \"{name}\"");
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
            say!("Saved the condition \"{name}\".");
        }
        Command::SessionItems { session, targets: lists, add } => {
            let overview = overview()?;
            let mut session = find(&overview.sessions, &session, "session", |b| (&b.id, &b.name))?.clone();
            let ids = lists
                .iter()
                .map(|l| find(&overview.targets, l, "target", |l| (&l.id, &l.name)).map(|l| (l.id.clone(), l.name.clone())))
                .collect::<Result<Vec<_>, _>>()?;
            session.targets = changed(&session.targets, &ids, add, |name| format!("\"{name}\" is not a target of \"{}\"", session.name))?;
            let name = session.name.clone();
            send(Request::SaveSession { session })?;
            say!("Saved the session \"{name}\".");
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
                let kept = changed(&now, &keyed, add, |entry| format!("\"{entry}\" is not in the {what} of \"{}\"", list.name))?;
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
            say!("Saved the target \"{name}\".");
        }
        Command::ConditionItems { condition, members, add } => {
            let overview = overview()?;
            let mut condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?.clone();
            let Rule::Combined { members: now, .. } = &mut condition.rule else {
                return Err(failed(format!("\"{}\" is not a combination; add and remove change the conditions of one", condition.name)));
            };
            let ids = members
                .iter()
                .map(|c| find(&overview.conditions, c, "condition", |c| (&c.id, &c.name)).map(|c| (c.id.clone(), c.name.clone())))
                .collect::<Result<Vec<_>, _>>()?;
            *now = changed(now, &ids, add, |name| format!("\"{name}\" is not in \"{}\"", condition.name))?;
            let name = condition.name.clone();
            send(Request::SaveCondition { condition })?;
            say!("Saved the condition \"{name}\".");
        }
        Command::ConditionDelete { condition, force } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            if !confirm(
                &format!("Delete the condition \"{}\"?{} This cannot be undone.", condition.name, fallout(&overview, &condition.id)),
                force,
            )? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteCondition { id: condition.id.clone() })?;
            say!("Deleted the condition \"{}\".", condition.name);
        }
        Command::ConditionCheck { times } => match Week::parse(&times).map_err(Failure::Usage)? {
            Some(week) => out(&per_day(&week)),
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
            let count = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
            let mut question = format!(
                "Replace every target, condition and session with the {}, {} and {} in {file}? This cannot be undone.",
                count(contents.targets, "target"),
                count(contents.conditions, "condition"),
                count(contents.sessions, "session"),
            );
            if !contents.unlocked.is_empty() {
                let names = contents.unlocked.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
                question.push_str(&format!(" {names} had a password lock, which is not in the file: no lock once imported."));
            }
            let apps = left_out_words(&left_out, checked);
            if !apps.is_empty() {
                question.push_str(&format!(" {apps}"));
            }
            if !confirm(&question, force)? {
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
            say!("Made \"{copy}\", a copy of the {noun} \"{name}\".");
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
                // The service tells the app to quit at its next policy request, within a second.
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

/// The Store apps an import leaves out, as they are not installed here, in a sentence; empty for none.
/// `checked`: whether what is installed could be told at all (else everything is kept).
fn left_out_words(left_out: &[definitions::LeftOut], checked: bool) -> String {
    if !checked {
        return "Which Store apps are installed here cannot be told, so all of them are kept.".into();
    }
    if left_out.is_empty() {
        return String::new();
    }
    let apps: Vec<String> = left_out.iter().map(|app| format!("\"{}\" in \"{}\" ({})", app.name, app.target, app.family)).collect();
    format!("Left out, as not installed on this PC: {}.", apps.join(", "))
}

/// The other way to unlock, in words.
fn describe_unlock_way(status: &UnlockStatus) -> String {
    let when = |open: &str, closed: &str| match (status.way_open, &status.way_left_words) {
        (true, _) => open.to_string(),
        (false, Some(left)) => format!("{closed} (in {left})"),
        (false, None) => closed.to_string(),
    };
    match &status.way {
        None => "Only the delay unlocks.".into(),
        Some(Lock::Text { .. }) => "Unlocking is also possible by typing a text in, in the window.".into(),
        Some(Lock::Password { .. }) => "Unlocking is also possible at once with a password (`unlock --password`).".into(),
        Some(Lock::Condition { .. }) => {
            when("Unlocking is open: its condition is not active.", "Unlocking is locked while its condition is active")
        }
        Some(_) => "Only the delay unlocks.".into(),
    }
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
                [] => Err(failed(format!("No flash drive \"{given}\" is connected; `bullseye drives` lists the ones that are"))),
                _ => Err(failed(format!("More than one flash drive is called \"{given}\"; give its serial number instead"))),
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

/// What a session's lock says of its condition besides its name: nothing for a schedule (the times are
/// on the Conditions page, and only there), and the drive for a flash drive, which is what makes
/// "active" mean something.
fn drive_detail(rule: &Rule) -> String {
    match rule {
        Rule::Schedule { .. } | Rule::Combined { .. } | Rule::Webhook(_) => String::new(),
        Rule::FlashDrive { drive, .. } => format!(" (the flash drive \"{drive}\" is not connected)"),
    }
}

/// A webhook condition as words.
fn describe_webhook(hook: &Hook) -> String {
    let matched = match (hook.matcher.on, hook.matcher.status, &hook.matcher.body) {
        (Match::Status, Some(status), _) => format!("HTTP {status}"),
        (Match::Body, _, Some(body)) => format!("body \"{body}\""),
        _ => "(not set)".into(),
    };
    let (usually, other) = if hook.fallback { ("active", "not active") } else { ("not active", "active") };
    format!(
        "{} {} every {}: {usually}, {other} on {matched}",
        if hook.method == Method::Post { "POST" } else { "GET" },
        hook.url,
        duration::period(hook.interval)
    )
}

/// A condition as words: its times, the drive it waits for, or the conditions it combines.
fn describe_rule(overview: &Overview, rule: &Rule) -> String {
    match rule {
        Rule::Schedule { week } => week.describe(),
        Rule::FlashDrive { drive, .. } => format!("while the flash drive \"{drive}\" is not connected"),
        Rule::Webhook(hook) => describe_webhook(hook),
        Rule::Combined { mode, members, .. } => {
            let names: Vec<String> = members
                .iter()
                .map(|id| overview.conditions.iter().find(|c| &c.id == id).map_or("(gone)".to_string(), |c| format!("\"{}\"", c.name)))
                .collect();
            format!("{} of {}", if *mode == Mode::Any { "any" } else { "all" }, names.join(", "))
        }
    }
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

/// What the start of a locked session asks about, as the window's confirmation; `None`: not locked.
fn lock_warning(overview: &Overview, session: &Definition) -> Option<String> {
    match &session.lock {
        Lock::None => None,
        Lock::Timed { kind: TimedKind::For, .. } => {
            let words = duration::countdown(session.lock.duration().ok()??.as_secs());
            Some(format!("It cannot be stopped, changed or deleted for {words}."))
        }
        Lock::Timed { kind: TimedKind::Until, value } => match localtime::until(value) {
            Ok(end) => Some(format!("It cannot be stopped, changed or deleted until {} ({} from now).", end.when, end.left)),
            // It will not start; the service says why.
            Err(_) => None,
        },
        Lock::UntilRestart => {
            Some("It cannot be stopped, changed or deleted until Windows restarts (a shut down with Fast Startup is not a restart).".into())
        }
        Lock::Condition { condition } => {
            let condition = overview.conditions.iter().find(|c| &c.id == condition)?;
            Some(format!(
                "It cannot be stopped while its lock condition \"{}\" is active{}.",
                condition.name,
                drive_detail(&condition.rule)
            ))
        }
        Lock::Delay { .. } => {
            let words = duration::countdown(session.lock.delay().ok()??.as_secs());
            Some(format!("It can only be stopped by asking to and then waiting {words}."))
        }
        Lock::Text { .. } => Some("It can only be stopped by typing its text in, in the window.".into()),
        Lock::Password { .. } => Some("It can only be stopped with its password.".into()),
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
        [] => Err(failed(format!("No {kind} called \"{wanted}\"; `bullseye status` lists them"))),
        _ => Err(failed(format!(
            "Several of the {kind}s are called \"{wanted}\"; use the id instead: {}",
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

/// The state of a session in one line, as its tile shows it.
fn state(running: Option<&Running>) -> String {
    let Some(running) = running else { return "Not running".into() };
    let mut parts = vec!["Running".to_string()];
    parts.push(match (running.enforcing, &running.condition_change_words) {
        (true, None) => "active".into(),
        (true, Some(change)) => format!("active for the next {change}"),
        (false, Some(change)) => format!("inactive for the next {change}"),
        (false, None) => "inactive".into(),
    });
    parts.push(match (&running.remaining_words, running.locked) {
        _ if running.password => "locked with a password".into(),
        _ if running.lock_text.is_some() => "locked until its text is typed".into(),
        _ if running.until_restart => "locked until Windows restarts".into(),
        (Some(left), _) => format!("locked for the next {left}"),
        (None, true) => match (&running.lock_condition, &running.lock_change_words) {
            (Some(Rule::FlashDrive { drive, .. }), _) => format!("locked until the flash drive \"{drive}\" is connected"),
            (_, Some(change)) => format!("locked for the next {change}"),
            (_, None) => "locked".into(),
        },
        (None, false) if running.lock_condition.is_some() => match (&running.lock_condition, &running.lock_change_words) {
            (Some(Rule::FlashDrive { drive, .. }), _) => format!("unlocked while the flash drive \"{drive}\" is connected"),
            (_, Some(change)) => format!("unlocked for the next {change}"),
            (_, None) => "unlocked".into(),
        },
        (None, false) => "no active lock".into(),
    });
    if let Some(allowance) = &running.allowance {
        parts.push(allowance_words(allowance));
    }
    parts.join(" · ")
}

/// What a running session's allowance is doing, in a few words: a cycle's phase and how long it lasts,
/// or what a budget has left.
fn allowance_words(status: &AllowanceStatus) -> String {
    match &status.terms {
        Terms::Pomodoro { .. } => match (status.open, &status.change_words) {
            (true, Some(change)) => format!("allowed for the next {change}"),
            (false, Some(change)) => format!("blocked for the next {change}"),
            (true, None) => "allowed".into(),
            (false, None) => "blocked".into(),
        },
        Terms::Delayed { .. } => match &status.change_words {
            Some(change) => format!("nothing enforced for the next {change}"),
            None => "enforcing".into(),
        },
        Terms::Budget { allowed, scope, .. } => {
            let total =
                duration::parse(allowed).ok().flatten().map(|d| duration::countdown(d.as_secs())).unwrap_or_else(|| allowed.clone());
            let reset = status.reset_words.as_ref().map(|left| format!(", reset in {left}")).unwrap_or_default();
            match scope {
                Scope::Total => format!("{} left of {total}{reset}", status.left_words.clone().unwrap_or_default()),
                Scope::Each => {
                    let used: Vec<String> = status.items.iter().map(|item| format!("{} {} left", item.item, item.left_words)).collect();
                    let used = if used.is_empty() { String::new() } else { format!(" ({})", used.join(", ")) };
                    format!("{total} for each item{reset}{used}")
                }
            }
        }
    }
}

/// Names in rows, padded to the longest.
fn table(rows: &[(String, String)]) -> String {
    let width = rows.iter().map(|(name, _)| name.chars().count()).max().unwrap_or(0);
    rows.iter().map(|(name, what)| format!("  {name:width$}  {what}\n")).collect()
}

/// The names of the sessions that use a target or condition (`Library::users_of`).
fn users(overview: &Overview, id: &str) -> String {
    let library = Library { sessions: overview.sessions.clone(), ..Library::default() };
    let names: Vec<&str> = library.users_of(id).iter().map(|b| b.name.as_str()).collect();
    if names.is_empty() {
        "(no session)".into()
    } else {
        names.join(", ")
    }
}

/// What deleting a target or condition does to the sessions that use it, in sentences each with a
/// space in front, as the window's question says it; empty when no session uses it.
fn fallout(overview: &Overview, id: &str) -> String {
    let library = Library { sessions: overview.sessions.clone(), conditions: overview.conditions.clone(), ..Library::default() };
    let fallout = library.fallout(id);
    let quoted = |names: &[String]| names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
    let mut text = String::new();
    for (names, what) in [
        (&fallout.without_it, "goes on without this target"),
        (&fallout.always, "is always active from then on"),
        (&fallout.unlocked, "has no lock from then on"),
        (&fallout.shrinks, "goes on without it"),
        (&fallout.removed, "is deleted too, as only one condition is left in it"),
    ] {
        if !names.is_empty() {
            text.push_str(&format!(" {} {what}.", quoted(names)));
        }
    }
    text
}

/// Everything, as the window's pages show it: the sessions, the targets, the conditions.
fn describe_overview(overview: &Overview) -> String {
    let sessions: Vec<(String, String)> =
        overview.sessions.iter().map(|b| (b.name.clone(), state(overview.running.iter().find(|r| r.id == b.id)))).collect();
    let lists: Vec<(String, String)> =
        overview.targets.iter().map(|l| (l.name.clone(), format!("used by {}", users(overview, &l.id)))).collect();
    let conditions: Vec<(String, String)> =
        overview.conditions.iter().map(|c| (c.name.clone(), describe_rule(overview, &c.rule))).collect();
    let mut text = String::new();
    for (title, rows, empty) in [
        ("Sessions", sessions, "  None yet. Make one with: bullseye new <name> --targets <targets>\n"),
        ("Targets", lists, "  None yet. Make one with: bullseye target new <name> --sites <sites>\n"),
        ("Conditions", conditions, "  None (sessions without one are always active).\n"),
    ] {
        text.push_str(&format!("{title}:\n"));
        text.push_str(&if rows.is_empty() { empty.to_string() } else { table(&rows) });
    }
    if !overview.clock_synced {
        text.push_str("(Waiting for internet time: until then, time with the service not running does not count.)\n");
    }
    text
}

fn entries(entries: &[String]) -> String {
    if entries.is_empty() {
        "(none)".to_string()
    } else {
        entries.join(", ")
    }
}

fn actions_on(actions: Actions) -> Vec<String> {
    match serde_json::to_value(actions) {
        Ok(Value::Object(fields)) => fields.iter().filter(|(_, on)| **on == Value::Bool(true)).map(|(name, _)| name.clone()).collect(),
        _ => Vec::new(),
    }
}

/// One session, everything about it.
fn describe_session(overview: &Overview, session: &Definition, running: Option<&Running>) -> String {
    let lists: Vec<&str> =
        session.targets.iter().map(|l| overview.targets.iter().find(|n| &n.id == l).map(|n| n.name.as_str()).unwrap_or("(gone)")).collect();
    let condition = match &session.condition {
        None => "always active".to_string(),
        Some(id) => match overview.conditions.iter().find(|c| &c.id == id) {
            Some(c) => c.name.clone(),
            None => "(gone)".into(),
        },
    };
    let allowance = session.allowance.as_ref().map_or("none".to_string(), Terms::describe);
    let lock = match &session.lock {
        Lock::None => "none: can be stopped at any time".to_string(),
        Lock::Timed { kind: TimedKind::For, value: duration } => match session.lock.duration() {
            Ok(Some(d)) => format!("duration, {} (cannot be stopped for that long once started)", duration::countdown(d.as_secs())),
            _ => format!("duration, {duration}"),
        },
        Lock::Timed { kind: TimedKind::Until, value: until } => match session.lock.until() {
            Ok(Some(at)) => format!("until {at} (cannot be stopped until then once started)"),
            _ => format!("until {until}"),
        },
        Lock::UntilRestart => "until restart: cannot be stopped until Windows restarts, and ends then".into(),
        Lock::Condition { condition } => match overview.conditions.iter().find(|c| &c.id == condition) {
            Some(c) => format!("condition: cannot be stopped while \"{}\" is active{}", c.name, drive_detail(&c.rule)),
            None => "condition: (condition gone)".into(),
        },
        Lock::Delay { delay } => match session.lock.delay() {
            Ok(Some(d)) => format!("delay, {} (stopping is asked for, then waited out)", duration::countdown(d.as_secs())),
            _ => format!("delay, {delay}"),
        },
        Lock::Text { .. } => "text: can only be stopped by typing its text in, in the window".into(),
        Lock::Password { .. } => "password: can only be stopped with its password".into(),
    };
    format!(
        "Name:        {}\nId:          {}\nState:       {}\nTargets:     {}\nCondition:   {condition}\nAllowance:   {allowance}\nLock:        {lock}\n",
        session.name,
        session.id,
        state(running),
        entries(&lists.iter().map(|l| l.to_string()).collect::<Vec<_>>()),
    )
}

/// One target, everything about it.
fn describe_target(overview: &Overview, list: &Target) -> String {
    format!(
        "Name:           {}\nId:             {}\nSites:          {}\nAlways allowed: {}\nPrograms:       {}\nStore apps:     {}\nWindow titles:  {}\nAllowed titles: {}\nActions:        {}\nUsed by:        {}\n",
        list.name,
        list.id,
        entries(&list.entries.filters),
        entries(&list.entries.allowed),
        entries(&list.entries.programs),
        entries(&list.entries.store_apps),
        entries(&list.entries.titles),
        entries(&list.entries.allowed_titles),
        entries(&actions_on(list.actions)),
        users(overview, &list.id),
    )
}

/// A weekly schedule as one line, then each day on a line of its own.
fn per_day(week: &Week) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut text = format!("{}\n", week.describe());
    for (name, spans) in NAMES.iter().zip(&week.days) {
        let mut day = Week::default();
        day.days[0] = spans.clone();
        let times = if spans.is_empty() { "inactive".to_string() } else { day.describe().trim_start_matches("Mon ").to_string() };
        text.push_str(&format!("  {name}  {times}\n"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tests::{session, target, weekly};
    use crate::definitions::Definitions;
    use crate::handler::{handle, Context};
    use crate::session::{RunningTarget, Sessions};
    use crate::state::Store;
    use crate::watchdog::AppSlot;
    use std::cell::RefCell;
    use std::sync::Arc;

    thread_local! {
        /// The service the commands of this test talk to (`serve`).
        pub static REMOTE: RefCell<Option<Remote>> = const { RefCell::new(None) };
        /// What the command being run says (`cli`).
        pub static SAID: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    fn args(line: &[&str]) -> Vec<String> {
        line.iter().map(|arg| arg.to_string()).collect()
    }

    /// A service of its own, in this process and with files of its own, which the commands run on
    /// this thread talk to from now on.
    fn serve(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions = Sessions::with_store(Store::new(dir.join("state.json")));
        let context = Arc::new(Context::new(sessions, Definitions::at(Some(dir.join("sessions.json"))), Arc::new(AppSlot::new(false))));
        REMOTE.with(|remote| *remote.borrow_mut() = Some(Remote::local(Arc::new(move |request| handle(&context, 0, request).0))));
        dir
    }

    /// A command line run as `bullseye` runs it: what it says, or why it did not work.
    fn cli(line: &[&str]) -> Result<String, String> {
        SAID.with(|said| *said.borrow_mut() = Some(String::new()));
        let result = parse(&args(line)).map_err(Failure::Usage).and_then(execute);
        let said = SAID.with(|said| said.borrow_mut().take()).unwrap_or_default();
        match result {
            Ok(()) => Ok(said),
            Err(Failure::Usage(why) | Failure::Failed(why)) => Err(why),
        }
    }

    /// What a command says, which has to work.
    fn ok(line: &[&str]) -> String {
        cli(line).unwrap_or_else(|why| panic!("{line:?}: {why}"))
    }

    /// Why a command does not work, which it has to.
    fn refused(line: &[&str]) -> String {
        cli(line).err().unwrap_or_else(|| panic!("{line:?} worked"))
    }

    #[test]
    fn sessions_targets_and_conditions_are_made_shown_changed_and_deleted() {
        serve("lifecycle");
        assert!(ok(&["status"]).contains("Sessions:\n  None yet"));
        assert_eq!(
            ok(&["target", "new", "Social", "--sites", "youtube.com", "--allowed-sites", "music.youtube.com"]),
            "Made the target \"Social\". Use it with: bullseye new <session> --targets \"Social\"\n"
        );
        assert!(ok(&["condition", "new", "Work", "--times", "Mon-Fri 08:00-17:00"]).starts_with("Made the condition \"Work\"."));
        let made = ok(&["new", "Evening", "--targets", "Social", "--condition", "Work", "--allowance", "budget", "--allowed", "30m"]);
        assert!(
            made.starts_with("Made the session \"Evening\" (id cli-") && made.ends_with("Start it with: bullseye start \"Evening\"\n"),
            "{made}"
        );

        // Shown as the pages show them, and as JSON for scripts.
        let shown = ok(&["show", "evening"]);
        assert!(shown.contains("Targets:     Social\nCondition:   Work\nAllowance:   30 minutes in total, never reset\n"), "{shown}");
        let json: Value = serde_json::from_str(&ok(&["show", "Evening", "--json"])).unwrap();
        assert_eq!(json["session"]["allowance"]["type"], "budget");
        assert!(json["running"].is_null());
        let status = ok(&["status"]);
        assert!(
            status.contains("  Evening  Not running")
                && status.contains("  Social  used by Evening")
                && status.contains("  Work  Mon-Fri 08:00-17:00")
        );
        let json: Value = serde_json::from_str(&ok(&["status", "--json"])).unwrap();
        assert_eq!(json["sessions"].as_array().map(Vec::len), Some(1));
        assert!(ok(&["target", "show", "Social"]).contains("Always allowed: music.youtube.com"));
        assert!(ok(&["target", "show", "Social"]).contains("Allowed titles: (none)"));
        let json: Value = serde_json::from_str(&ok(&["target", "show", "Social", "--json"])).unwrap();
        assert_eq!(json["filters"][0], "youtube.com");
        let condition = ok(&["condition", "show", "Work"]);
        assert!(condition.starts_with("Work\nMon-Fri 08:00-17:00\n") && condition.ends_with("Used by: Evening\n"), "{condition}");
        assert!(ok(&["condition", "check", "Mon 8-9"]).starts_with("Mon 08:00-09:00\n"));
        assert_eq!(ok(&["condition", "check", "always"]), "Always active: a session without a condition\n");
        assert!(cli(&["condition", "check", "Mon 25-26"]).is_err());

        // Changed, copied.
        assert_eq!(ok(&["edit", "Evening", "--name", "Night"]), "Saved the session \"Night\".\n");
        assert_eq!(ok(&["duplicate", "Night"]), "Made \"Night (2)\", a copy of the session \"Night\".\n");
        assert_eq!(ok(&["target", "duplicate", "Social"]), "Made \"Social (2)\", a copy of the target \"Social\".\n");
        assert_eq!(ok(&["condition", "duplicate", "Work"]), "Made \"Work (2)\", a copy of the condition \"Work\".\n");
        assert_eq!(
            ok(&["target", "edit", "Social (2)", "--sites", "reddit.com", "--clear", "allowed-sites"]),
            "Saved the target \"Social (2)\".\n"
        );
        let copy = ok(&["target", "show", "Social (2)"]);
        assert!(copy.contains("reddit.com") && !copy.contains("music.youtube.com"), "{copy}");
        assert_eq!(ok(&["condition", "edit", "Work (2)", "--name", "Weekend", "--times", "Sat-Sun"]), "Saved the condition \"Weekend\".\n");
        ok(&["condition", "new", "Either", "--any", "Work", "Weekend"]);
        assert!(ok(&["condition", "show", "Either"]).contains("any of \"Work\", \"Weekend\""));
        assert!(refused(&["condition", "new", "Pair", "--any", "Work"]).contains("at least two"));

        // Started and stopped.
        assert_eq!(ok(&["start", "Night"]), "Started \"Night\".\n");
        assert!(ok(&["status"]).contains("  Night      Running"), "{}", ok(&["status"]));
        assert_eq!(ok(&["stop", "Night"]), "Stopped \"Night\".\n");

        // Deleted.
        assert_eq!(ok(&["delete", "Night (2)", "--force"]), "Deleted the session \"Night (2)\".\n");
        assert_eq!(ok(&["target", "delete", "Social (2)", "--force"]), "Deleted the target \"Social (2)\".\n");
        assert_eq!(ok(&["condition", "delete", "Either", "--force"]), "Deleted the condition \"Either\".\n");

        // What is not there, or not of the kind.
        assert!(refused(&["show", "Nope"]).starts_with("No session called \"Nope\""));
        assert!(refused(&["new", "X", "--targets", "Nope"]).starts_with("No target called \"Nope\""));
        assert!(refused(&["target", "new", "Games", "--store-apps", "Microsoft.Xbox*"]).contains("exact package family name"));
        assert!(refused(&["new", "X", "--condition", "Nope"]).starts_with("No condition called \"Nope\""));
        assert_eq!(refused(&["condition", "test", "Work"]), "\"Work\" is not a webhook condition");
    }

    #[test]
    fn items_are_added_and_removed() {
        serve("add-remove");
        ok(&["target", "new", "Social", "--sites", "youtube.com", "reddit.com", "--programs", "Steam", "--actions", "chromium_tabs"]);
        ok(&["target", "new", "Games", "--programs", "game.exe"]);
        ok(&["target", "new", "News", "--sites", "nu.nl"]);
        let list = |name: &str| ok(&["target", "show", name]);

        // A target: entries added as typed (the service keeps them in order), and taken out however they are
        // written, as the list reads them (Steam is steam.exe); what is there already stays once.
        assert_eq!(
            ok(&["target", "add", "Social", "--sites", "x.com", "YouTube.com", "--titles", "Inbox, 3 unread", "--actions", "firefox_tabs"]),
            "Saved the target \"Social\".\n"
        );
        let shown = list("Social");
        assert!(shown.contains("Sites:          reddit.com, x.com, youtube.com"), "{shown}");
        assert!(
            shown.contains("Window titles:  Inbox, 3 unread") && shown.contains("firefox_tabs") && shown.contains("chromium_tabs"),
            "{shown}"
        );
        ok(&["target", "remove", "Social", "--sites", "Reddit.com", "--programs", "steam.exe", "--actions", "chromium_tabs"]);
        let shown = list("Social");
        assert!(shown.contains("Sites:          x.com, youtube.com") && shown.contains("Programs:       (none)"), "{shown}");
        assert!(!shown.contains("chromium_tabs") && shown.contains("firefox_tabs"), "{shown}");
        // What is not there is a typo, and nothing is saved.
        assert_eq!(
            refused(&["target", "remove", "Social", "--sites", "x.com", "tiktok.com"]),
            "\"tiktok.com\" is not in the sites of \"Social\""
        );
        assert!(list("Social").contains("x.com, youtube.com"));
        assert_eq!(
            refused(&["target", "remove", "Social", "--allowed-titles", "Notes"]),
            "\"Notes\" is not in the allowed titles of \"Social\""
        );
        assert!(refused(&["target", "add", "Social", "--actions", "lasers"]).starts_with("Unknown action \"lasers\""));
        assert_eq!(refused(&["target", "add", "Social"]), "Give what to add: --sites, --programs, --titles, … or --actions");
        // All the actions, on and off.
        ok(&["target", "add", "Social", "--actions", "all"]);
        assert!(list("Social").contains("chromium_tabs"));
        ok(&["target", "remove", "Social", "--actions", "all"]);
        assert!(list("Social").contains("Actions:        (none)"), "{}", list("Social"));

        // A session's targets.
        ok(&["new", "Work", "--targets", "Social"]);
        ok(&["add", "work", "--targets", "Games", "News", "Social"]);
        assert!(ok(&["show", "Work"]).contains("Targets:     Social, Games, News"), "{}", ok(&["show", "Work"]));
        ok(&["remove", "Work", "--targets", "social"]);
        assert!(ok(&["show", "Work"]).contains("Targets:     Games, News"));
        assert_eq!(refused(&["remove", "Work", "--targets", "Social"]), "\"Social\" is not a target of \"Work\"");
        assert!(refused(&["add", "Work", "--targets", "Nope"]).starts_with("No target called \"Nope\""));
        assert_eq!(refused(&["add", "Work"]), "Give the targets to add: --targets <targets>");

        // The conditions of a combination, and nothing else.
        for name in ["Mornings", "Evenings", "Weekend"] {
            ok(&["condition", "new", name, "--times", "sat"]);
        }
        ok(&["condition", "new", "Either", "--any", "Mornings", "Evenings"]);
        ok(&["condition", "add", "Either", "--conditions", "Weekend"]);
        assert!(
            ok(&["condition", "show", "Either"]).contains("any of \"Mornings\", \"Evenings\", \"Weekend\""),
            "{}",
            ok(&["condition", "show", "Either"])
        );
        ok(&["condition", "remove", "Either", "--conditions", "Mornings"]);
        assert!(ok(&["condition", "show", "Either"]).contains("any of \"Evenings\", \"Weekend\""));
        assert_eq!(refused(&["condition", "remove", "Either", "--conditions", "Mornings"]), "\"Mornings\" is not in \"Either\"");
        assert_eq!(
            refused(&["condition", "add", "Weekend", "--conditions", "Mornings"]),
            "\"Weekend\" is not a combination; add and remove change the conditions of one"
        );
        assert_eq!(refused(&["condition", "remove", "Either"]), "Give the conditions to remove: --conditions <conditions>");
        // The service still holds to its rules: a combination of fewer than two is refused.
        assert!(!refused(&["condition", "remove", "Either", "--conditions", "Weekend"]).is_empty());
        assert_eq!(refused(&["target"]), "`target` needs show, new, edit, add, remove, delete, duplicate");
    }

    #[test]
    fn a_webhook_condition_is_shown_and_tested() {
        serve("webhook");
        let url = "http://127.0.0.1:9/state";
        ok(&["condition", "new", "Home", "--webhook", url, "--match-body", "home", "--fallback", "inactive"]);
        let shown = ok(&["condition", "show", "Home"]);
        assert!(shown.contains("GET http://127.0.0.1:9/state every minute: not active, active on body \"home\""), "{shown}");
        ok(&["condition", "new", "Door", "--webhook", url, "--match-status", "423", "--every", "5m"]);
        let shown = ok(&["condition", "show", "Door"]);
        assert!(shown.contains("GET http://127.0.0.1:9/state every 5 minutes: active, not active on HTTP 423"), "{shown}");
        // Nothing answers there.
        assert!(cli(&["condition", "test", "Home"]).is_err());
    }

    #[test]
    fn locks_are_set_shown_and_held_by_the_service() {
        serve("locks");
        ok(&["target", "new", "L", "--sites", "a.com"]);
        ok(&["condition", "new", "Always", "--times", "Mon-Sun 00:00-24:00"]);
        let new = |name: &str, lock: &[&str]| ok(&[&["new", name, "--targets", "L"][..], lock].concat());
        new("Timed", &["--lock-duration", "1h"]);
        new("Slow", &["--lock-delay", "1h"]);
        new("Typed", &["--lock-text", "let me out"]);
        new("Kept", &["--lock-condition", "always"]);
        new("Restart", &["--lock-until-restart"]);
        new("Later", &["--lock-until", "2099-01-01 00:00"]);
        let lock_of = |name: &str| ok(&["show", name]).lines().find(|l| l.starts_with("Lock:")).unwrap_or_default().to_string();
        assert_eq!(lock_of("Timed"), "Lock:        duration, 1 hour (cannot be stopped for that long once started)");
        assert_eq!(lock_of("Slow"), "Lock:        delay, 1 hour (stopping is asked for, then waited out)");
        assert_eq!(lock_of("Typed"), "Lock:        text: can only be stopped by typing its text in, in the window");
        assert_eq!(lock_of("Kept"), "Lock:        condition: cannot be stopped while \"Always\" is active");
        assert_eq!(lock_of("Restart"), "Lock:        until restart: cannot be stopped until Windows restarts, and ends then");
        assert!(lock_of("Later").starts_with("Lock:        until Thursday 1 January 2099, 00:00"), "{}", lock_of("Later"));

        for name in ["Timed", "Slow", "Typed", "Kept", "Restart"] {
            assert_eq!(ok(&["start", name, "--force"]), format!("Started \"{name}\".\n"));
        }
        let status = ok(&["status"]);
        for line in [
            "Timed    Running · active · locked for the next",
            "Typed    Running · active · locked until its text is typed",
            "Restart  Running · active · locked until Windows restarts",
            "Kept     Running · active · locked",
        ] {
            assert!(status.contains(line), "{line:?} in {status}");
        }
        assert!(refused(&["stop", "Timed"]).contains("is locked for the next"));
        assert!(refused(&["stop", "Kept"]).contains("is locked by its lock schedule"));
        assert!(refused(&["stop", "Restart"]).contains("until Windows restarts"));
        assert_eq!(refused(&["stop", "Typed"]), "\"Typed\" is locked until its text is typed in: stop it in the window");
        // A delay: the first stop asks, the next says how long, and --abort ends the wait.
        assert!(ok(&["stop", "Slow"]).starts_with("Stopping \"Slow\" was asked for: ready in 1 hour."));
        assert!(ok(&["stop", "Slow"]).starts_with("\"Slow\" can be stopped in "));
        assert_eq!(ok(&["stop", "Slow", "--abort"]), "Stopping \"Slow\" aborted: back to locked.\n");
        // A password: set with start, as the tile sets it, and stopped with it.
        ok(&["new", "Pw", "--targets", "L"]);
        assert_eq!(ok(&["start", "Pw", "--lock-password", "secret", "--force"]), "Started \"Pw\".\n");
        assert!(ok(&["status"]).contains("Pw       Running · active · locked with a password"));
        assert!(cli(&["stop", "Pw", "--password", "wrong"]).is_err());
        assert_eq!(ok(&["stop", "Pw", "--password", "secret"]), "Stopped \"Pw\".\n");
        // A running session that is not locked is locked now; one that is keeps its lock.
        assert_eq!(ok(&["start", "Pw", "--no-lock"]), "Started \"Pw\".\n");
        assert_eq!(ok(&["lock", "Pw", "--no-lock"]), "\"Pw\" runs on without a lock.\n");
        assert!(refused(&["lock", "Pw", "--lock-duration", "1h"]).contains("No terminal to ask in; add --force"));
        assert_eq!(ok(&["lock", "Pw", "--lock-duration", "1h", "--force"]), "Locked \"Pw\" now.\n");
        assert!(ok(&["status"]).contains("Pw       Running · active · locked for the next"));
        assert_eq!(
            refused(&["lock", "Pw", "--no-lock"]),
            "This session is locked, so its lock cannot be changed until it can be stopped, except to end later or to wait longer"
        );
        assert!(refused(&["lock", "Pw"]).starts_with("Give the lock"));
        // Without a delay, unlocking is free and ends them all.
        assert_eq!(ok(&["unlock"]), "Unlocked: every session has ended.\n");
        assert!(!ok(&["status"]).contains("Running"));
    }

    #[test]
    fn unlocking_counts_down_and_has_another_way() {
        serve("unlock");
        ok(&["target", "new", "L", "--sites", "a.com"]);
        ok(&["new", "Timed", "--targets", "L", "--lock-duration", "1h"]);
        assert_eq!(ok(&["unlock-delay"]), "Unlocking takes no time: it is free.\n");
        assert_eq!(ok(&["unlock-delay", "1h"]), "Unlocking now takes 1 hour.\n");
        assert_eq!(ok(&["unlock-delay"]), "Unlocking takes 1 hour.\n");
        ok(&["start", "Timed", "--force"]);
        assert!(ok(&["unlock"]).starts_with("Unlocking was asked for: ready in 1 hour."));
        assert!(ok(&["unlock"]).starts_with("Unlocking is not ready yet: "));
        assert_eq!(ok(&["unlock", "--abort"]), "Unlocking aborted: back to locked.\n");
        assert_eq!(ok(&["unlock-way"]), "Only the delay unlocks.\n");
        // Locked and not ready: neither a shorter delay nor another way.
        assert!(cli(&["unlock-delay", "0"]).is_err());
        assert!(cli(&["unlock-way", "--text", "let me out"]).is_err());

        serve("unlock-way");
        ok(&["target", "new", "L", "--sites", "a.com"]);
        ok(&["condition", "new", "Always", "--times", "Mon-Sun 00:00-24:00"]);
        ok(&["new", "Timed", "--targets", "L", "--lock-duration", "1h"]);
        assert_eq!(ok(&["unlock-way", "--text", "let me out"]), "Unlocking is also possible by typing a text in, in the window.\n");
        assert_eq!(ok(&["unlock-way", "--condition", "Always"]), "Unlocking is locked while its condition is active\n");
        assert_eq!(
            ok(&["unlock-way", "--password", "secret"]),
            "Unlocking is also possible at once with a password (`unlock --password`).\n"
        );
        ok(&["unlock-delay", "1h"]);
        ok(&["start", "Timed", "--force"]);
        assert!(cli(&["unlock", "--password", "wrong"]).is_err());
        assert_eq!(ok(&["unlock", "--password", "secret"]), "Unlocked: every session has ended.\n");
        assert_eq!(ok(&["unlock-way", "--none"]), "Only the delay unlocks.\n");
        assert_eq!(ok(&["unlock-delay", "0"]), "Unlocking is free again.\n");
    }

    #[test]
    fn everything_moves_through_a_settings_file() {
        let dir = serve("export");
        let file = dir.join("bullseye.json");
        let path = file.to_string_lossy().to_string();
        ok(&["target", "new", "L", "--sites", "a.com"]);
        ok(&["new", "B", "--targets", "L", "--lock-password", "secret", "--allowance", "pomodoro", "--blocked", "50m", "--allowed", "10m"]);
        assert_eq!(ok(&["export", &path]), format!("Wrote every target, condition and session to {path}.\n"));
        serve("import");
        // (The file has no Store apps, so none are left out and nothing is said of them.)
        assert_eq!(ok(&["import", &path, "--force"]), format!("Imported {path}.\n"));
        let shown = ok(&["show", "B"]);
        assert!(shown.contains("Lock:        password") && shown.contains("Allowance:   Blocked for 50 minutes"), "{shown}");
        assert!(refused(&["import", &dir.join("missing.json").to_string_lossy()]).starts_with("Cannot read"));
    }

    #[test]
    fn a_webhook_is_read_from_its_options() {
        let line = [
            "condition",
            "new",
            "H",
            "--webhook",
            "https://x.io/a?b=1",
            "--method",
            "POST",
            "--every",
            "10m",
            "--match-status",
            "503",
            "--fallback",
            "active",
        ];
        let Ok(Command::ConditionNew { rule: RuleChoice::Webhook(hook), .. }) = parse(&args(&line)) else { panic!("not a webhook") };
        assert_eq!((hook.method, hook.interval, hook.fallback), (Method::Post, 600, true));
        assert_eq!(hook.matcher, Matcher { on: Match::Status, status: Some(503), body: None });
        let line = ["condition", "new", "H", "--webhook", "https://x.io/a", "--match-body", "on", "--fallback", "inactive"];
        let Ok(Command::ConditionNew { rule: RuleChoice::Webhook(hook), .. }) = parse(&args(&line)) else { panic!("not a webhook") };
        assert_eq!((hook.method, hook.interval, hook.fallback), (Method::Get, 60, false));
        assert_eq!(hook.matcher, Matcher { on: Match::Body, status: None, body: Some("on".into()) });
        let base = ["condition", "new", "H", "--webhook", "https://x.io/a", "--match-status", "404"];
        for (extra, why) in [
            (&["--method", "put"][..], "--method is get or post"),
            (&["--fallback", "maybe"], "--fallback is active or inactive"),
            (&["--every", "soon"], ""),
            (&["--every", "30s"], "at most once a minute"),
        ] {
            let error = parse(&args(&[&base[..], extra].concat())).err().unwrap_or_else(|| panic!("{extra:?} worked"));
            assert!(error.contains(why), "{extra:?}: {error}");
        }
        // One match: neither, or both, is refused, saying what the match is for.
        let hook = ["condition", "new", "H", "--webhook", "https://x.io/a"];
        assert_eq!(
            parse(&args(&hook)).unwrap_err(),
            "Give one of --match-status and --match-body: the answer that makes the condition not active"
        );
        assert!(parse(&args(&[&hook[..], &["--fallback", "inactive", "--match-status", "200", "--match-body", "on"]].concat()))
            .unwrap_err()
            .ends_with("the answer that makes the condition active"));
        assert!(parse(&args(&[&hook[..], &["--match-status", "OK"]].concat())).unwrap_err().contains("is a number"));
        // An option given twice is a mistake, not a choice.
        assert!(parse(&args(&["new", "X", "--condition", "A", "--condition", "B"])).unwrap_err().contains("more than once"));
    }

    #[test]
    fn a_combination_of_all_and_a_list_renamed() {
        serve("all");
        ok(&["target", "new", "L", "--sites", "a.com"]);
        assert_eq!(ok(&["target", "edit", "L", "--name", "Media"]), "Saved the target \"Media\".\n");
        ok(&["condition", "new", "Work", "--times", "Mon-Fri 08:00-17:00"]);
        ok(&["condition", "new", "Late", "--times", "Mon-Sun 16:00-18:00"]);
        ok(&["condition", "new", "Both", "--all", "Work", "Late"]);
        assert!(ok(&["condition", "show", "Both"]).contains("all of \"Work\", \"Late\""));
        // A lock until a date and time asks first, with the time it ends.
        ok(&["new", "Later", "--targets", "Media", "--lock-until", "2099-01-01 00:00"]);
        assert_eq!(ok(&["start", "Later", "--force"]), "Started \"Later\".\n");
        assert!(ok(&["status"]).contains("Later  Running · active · locked for the next"));
    }

    #[test]
    fn a_file_with_a_password_lock_but_no_hash_imports_without_a_lock() {
        let dir = serve("import-unlocked");
        let file = dir.join("old.json");
        let mut locked = session("b", &["l"]);
        locked.lock = Lock::Password { password: None, hash: None };
        let library = Library { targets: vec![target("l", &["a.com"])], sessions: vec![locked], ..Library::default() };
        std::fs::create_dir_all(&dir).unwrap();
        definitions::write_settings(&file, library).unwrap();
        ok(&["import", &file.to_string_lossy(), "--force"]);
        assert!(ok(&["show", "Session b"]).contains("Lock:        none"));
    }

    #[test]
    fn the_way_to_unlock_in_words() {
        let with = |way: Lock, open: bool, left: Option<&str>| {
            describe_unlock_way(&UnlockStatus {
                way: Some(way),
                way_open: open,
                way_left_words: left.map(str::to_string),
                ..UnlockStatus::default()
            })
        };
        let condition = || Lock::Condition { condition: "c".into() };
        assert_eq!(with(condition(), true, None), "Unlocking is open: its condition is not active.");
        assert_eq!(with(condition(), false, Some("2 hours")), "Unlocking is locked while its condition is active (in 2 hours)");
        assert_eq!(with(Lock::timed_for("1h"), false, None), "Only the delay unlocks.");
    }

    /// A running session as the status gives it, always active and not locked.
    fn a_running(id: &str) -> Running {
        Running {
            id: id.into(),
            name: id.into(),
            targets: vec![],
            remaining: None,
            lock_condition: None,
            until_restart: false,
            lock_text: None,
            stop_delay: None,
            stop_delay_words: None,
            stop_remaining: None,
            stop_remaining_words: None,
            password: false,
            locked: false,
            condition: None,
            enforcing: true,
            condition_change: None,
            remaining_words: None,
            condition_change_words: None,
            lock_change: None,
            lock_change_words: None,
            allowance: None,
        }
    }

    #[test]
    fn a_running_session_reads_like_its_tile() {
        let drive = Some(Rule::FlashDrive { serial: "x".into(), drive: "Key".into() });
        let locked = Running { locked: true, lock_condition: drive.clone(), ..a_running("a") };
        assert_eq!(state(Some(&locked)), "Running · active · locked until the flash drive \"Key\" is connected");
        let open = Running { lock_condition: drive, ..a_running("a") };
        assert_eq!(state(Some(&open)), "Running · active · unlocked while the flash drive \"Key\" is connected");
        let schedule = weekly("s", "mon 8-9").rule;
        let timed =
            Running { locked: true, lock_condition: Some(schedule.clone()), lock_change_words: Some("1 hour".into()), ..a_running("a") };
        assert_eq!(state(Some(&timed)), "Running · active · locked for the next 1 hour");
        let free = Running { lock_condition: Some(schedule.clone()), ..a_running("a") };
        assert_eq!(state(Some(&free)), "Running · active · unlocked");
        let quiet = Running { enforcing: false, locked: true, lock_condition: Some(schedule), ..a_running("a") };
        assert_eq!(state(Some(&quiet)), "Running · inactive · locked");
        assert_eq!(state(Some(&a_running("a"))), "Running · active · no active lock");
        assert_eq!(state(None), "Not running");
        // An allowance at the end: a cycle with no time to tell, a budget for each item.
        let cycle = AllowanceStatus {
            terms: Terms::Pomodoro { blocked: "50m".into(), allowed: "10m".into(), anchor: Anchor::Start },
            open: true,
            change: None,
            change_words: None,
            left: None,
            left_words: None,
            reset_in: None,
            reset_words: None,
            items: vec![],
        };
        assert_eq!(allowance_words(&cycle), "allowed");
        assert_eq!(allowance_words(&AllowanceStatus { open: false, ..cycle.clone() }), "blocked");
        let each = AllowanceStatus {
            terms: Terms::Budget {
                allowed: "10m".into(),
                scope: Scope::Each,
                reset: Reset::Every,
                every: "1h".into(),
                anchor: Anchor::Clock,
            },
            reset_words: Some("40 minutes".into()),
            items: vec![crate::allowance::ItemLeft { item: "youtube.com".into(), left: 240, left_words: "4 minutes".into() }],
            ..cycle.clone()
        };
        assert_eq!(allowance_words(&each), "10 minutes for each item, reset in 40 minutes (youtube.com 4 minutes left)");
        assert_eq!(allowance_words(&AllowanceStatus { items: vec![], reset_words: None, ..each }), "10 minutes for each item");
        let with = Running { allowance: Some(cycle), ..a_running("a") };
        assert_eq!(state(Some(&with)), "Running · active · no active lock · allowed");
    }

    #[test]
    fn a_session_with_parts_gone_or_unreadable_is_still_shown() {
        let mut overview = library();
        let mut b = session("b2", &["gone"]);
        b.condition = Some("gone".into());
        let lock_of = |lock: Lock| {
            let shown = describe_session(&overview, &Definition { lock, ..b.clone() }, None);
            shown.lines().find(|l| l.starts_with("Lock:")).unwrap_or_default().to_string()
        };
        assert_eq!(lock_of(Lock::timed_for("soon")), "Lock:        duration, soon");
        assert_eq!(lock_of(Lock::timed_until("someday")), "Lock:        until someday");
        assert_eq!(lock_of(Lock::Delay { delay: "a while".into() }), "Lock:        delay, a while");
        assert_eq!(lock_of(Lock::Condition { condition: "gone".into() }), "Lock:        condition: (condition gone)");
        assert_eq!(lock_of(Lock::Password { password: None, hash: None }), "Lock:        password: can only be stopped with its password");
        let shown = describe_session(&overview, &b, None);
        assert!(shown.contains("Targets:     (gone)\nCondition:   (gone)\n"), "{shown}");
        // A flash drive, as a lock's condition and as a condition.
        let drive =
            Condition { id: "d".into(), name: "Key".into(), rule: Rule::FlashDrive { serial: "x".into(), drive: "Kingston".into() } };
        overview.conditions.push(drive.clone());
        let shown = describe_session(&overview, &Definition { lock: Lock::Condition { condition: "d".into() }, ..b.clone() }, None);
        assert!(shown.contains("cannot be stopped while \"Key\" is active (the flash drive \"Kingston\" is not connected)"), "{shown}");
        assert_eq!(describe_rule(&overview, &drive.rule), "while the flash drive \"Kingston\" is not connected");
        assert_eq!(
            lock_warning(&overview, &Definition { lock: Lock::Condition { condition: "d".into() }, ..b.clone() }).unwrap(),
            "It cannot be stopped while its lock condition \"Key\" is active (the flash drive \"Kingston\" is not connected)."
        );
        assert_eq!(lock_warning(&overview, &Definition { lock: Lock::timed_until("2001-01-01 00:00"), ..b.clone() }), None);
        // A combination with one of its conditions gone.
        let combined = Rule::Combined { mode: Mode::Any, members: vec!["s1".into(), "gone".into()], rules: vec![] };
        assert_eq!(describe_rule(&overview, &combined), "any of \"Work hours\", (gone)");
        // A webhook matcher not set (as saved by hand).
        let hook = Hook {
            url: "https://x.io".into(),
            method: Method::Get,
            interval: 60,
            fallback: true,
            matcher: Matcher { on: Match::Status, status: None, body: None },
        };
        assert_eq!(describe_webhook(&hook), "GET https://x.io every minute: active, not active on (not set)");
    }

    #[test]
    fn an_import_says_which_store_apps_it_leaves_out() {
        let claude =
            definitions::LeftOut { target: "Claude".into(), family: "AnthropicPBC.Claude_fnn82j28hfe8t".into(), name: "Claude".into() };
        assert_eq!(
            left_out_words(std::slice::from_ref(&claude), true),
            "Left out, as not installed on this PC: \"Claude\" in \"Claude\" (AnthropicPBC.Claude_fnn82j28hfe8t)."
        );
        assert_eq!(left_out_words(&[], true), "");
        assert!(left_out_words(&[claude], false).ends_with("so all of them are kept."));
    }

    #[test]
    fn help_and_version_need_no_service() {
        let help = cli(&["help"]).unwrap();
        assert!(help.starts_with("Bullseye command line") && help.contains("chromium_tabs") && !help.contains("{actions}"));
        assert_eq!(cli(&["version"]).unwrap(), format!("bullseye {}\n", env!("CARGO_PKG_VERSION")));
    }

    fn library() -> Overview {
        let mut social = target("l1", &["youtube.com"]);
        social.name = "Social".into();
        let mut work = weekly("s1", "mon-fri 8-17");
        work.name = "Work hours".into();
        let mut b = session("b1", &["l1"]);
        b.name = "Work".into();
        b.condition = Some("s1".into());
        b.lock = Lock::Condition { condition: "s1".into() };
        Overview {
            targets: vec![social],
            conditions: vec![work],
            sessions: vec![b],
            running: vec![],
            unlock: UnlockStatus::default(),
            clock_synced: true,
        }
    }

    /// Every command the window can invoke has CLI commands that do the same, in the help and in
    /// `CLI.md`. When this fails, the window has learned something the CLI cannot do yet.
    #[test]
    fn the_cli_can_do_whatever_the_window_can() {
        let lib = include_str!("lib.rs");
        let start = lib.find("generate_handler![").expect("no generate_handler in lib.rs") + "generate_handler![".len();
        let list = &lib[start..start + lib[start..].find(']').unwrap()];
        let commands: Vec<&str> = list.split(',').map(|c| c.trim().rsplit("::").next().unwrap()).filter(|c| !c.is_empty()).collect();
        assert!(commands.len() >= 14, "{commands:?}");
        let doc = include_str!("../../CLI.md");
        for command in commands {
            let Some((_, cli)) = GUI_COMMANDS.iter().find(|(gui, _)| *gui == command) else {
                panic!("the window can invoke `{command}`, which is not in cli::GUI_COMMANDS: give the CLI a command for it");
            };
            for cli in cli.iter() {
                assert!(HELP.contains(&format!("\n  {cli} ")), "the help does not list `{cli}`");
                assert!(doc.contains(&format!("bullseye {cli}")), "CLI.md does not explain `{cli}`");
            }
        }
    }

    /// The page's action checkboxes are the CLI's action names.
    #[test]
    fn the_actions_are_the_same_as_in_the_window() {
        let names = action_names();
        assert!(!names.is_empty());
        let page = include_str!("../../src/js/forms.js");
        let strings = include_str!("../../src/strings.js");
        for name in names {
            assert!(page.contains(&format!("action(\"{name}\")")), "{name} is not on the page");
            assert!(strings.contains(&format!("{name}: {{")), "{name} has no text in strings.js");
        }
    }

    #[test]
    fn windows_by_title_are_closed_unless_minimized() {
        let parse_actions = |names: &str| parse_actions(&names.split(' ').map(str::to_string).collect::<Vec<_>>());
        let all = parse_actions("all").unwrap();
        assert_eq!(all, Actions::default(), "all is every action as a new target has them: closing windows by title");
        assert!(!all.window_titles_minimize);
        let down = parse_actions("chromium_tabs window_titles_minimize").unwrap();
        assert!(down.window_titles_minimize && down.chromium_tabs && !down.firefox_tabs);
        assert!(parse_actions("all window_titles_minimize").unwrap().window_titles_minimize);
        // The lists with one way to act on them have no switch: their entries are it.
        for gone in ["exe_names", "store_apps", "window_titles", "window_titles_close"] {
            assert!(parse_actions(gone).unwrap_err().starts_with("Unknown action"), "{gone}");
        }
    }

    #[test]
    fn a_new_target_with_everything() {
        let line = args(&[
            "target",
            "new",
            "Social",
            "--sites",
            "youtube.com",
            "reddit.com",
            "--allowed-sites",
            "youtube.com/watch*",
            "--programs",
            "steam",
            "--store-apps",
            "Microsoft.XboxApp_8wekyb3d8bbwe",
            "--titles",
            "*YouTube*",
            "--allowed-titles",
            "*Visual Studio Code",
            "--actions",
            "chromium_tabs",
            "window_titles_minimize",
        ]);
        let Ok(Command::TargetNew { name, changes }) = parse(&line) else { panic!("not a new list") };
        assert_eq!(name, "Social");
        let mut list = target("x", &[]);
        changes.apply(&mut list);
        assert_eq!(list.entries.filters, vec!["youtube.com", "reddit.com"]);
        assert_eq!(list.entries.allowed, vec!["youtube.com/watch*"]);
        assert_eq!(list.entries.programs, vec!["steam"]);
        assert_eq!(list.entries.store_apps, vec!["Microsoft.XboxApp_8wekyb3d8bbwe"]);
        assert_eq!(list.entries.titles, vec!["*YouTube*"]);
        assert_eq!(list.entries.allowed_titles, vec!["*Visual Studio Code"]);
        assert_eq!(
            list.actions,
            Actions {
                chromium_tabs: true,
                chromium_split_view: false,
                obsidian_web_viewer: false,
                chromium_app_windows: false,
                firefox_tabs: false,
                firefox_split_view: false,
                window_titles_minimize: true,
            }
        );
    }

    #[test]
    fn a_new_session_finds_its_targets_and_condition_by_name() {
        let line = args(&["new", "Evening", "--targets", "social", "l1", "--condition", "WORK HOURS", "--lock-duration", "2h 30m"]);
        let Ok(Command::New { name, changes }) = parse(&line) else { panic!("not new") };
        assert_eq!(name, "Evening");
        let mut b = session("x", &[]);
        assert!(apply_session_changes(&library(), &mut b, changes).is_ok());
        assert_eq!(
            (b.targets, b.condition, b.lock),
            (vec!["l1".to_string(), "l1".to_string()], Some("s1".to_string()), Lock::timed_for("2h 30m"))
        );

        // `always`, and a list that does not exist.
        let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--condition", "always", "--no-lock"])) else {
            panic!("not edit")
        };
        let mut b = library().sessions[0].clone();
        assert!(apply_session_changes(&library(), &mut b, changes).is_ok());
        assert_eq!((b.condition, b.lock), (None, Lock::None));
        let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--targets", "Games"])) else { panic!("not edit") };
        assert!(apply_session_changes(&library(), &mut library().sessions[0].clone(), changes).is_err());
    }

    #[test]
    fn start_can_set_the_lock_as_the_tile_does() {
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-condition", "Work hours", "--force"])),
            Ok(Command::Start { session: "Work".into(), lock: Some(Lock::Condition { condition: "Work hours".into() }), force: true })
        );
        assert_eq!(parse(&args(&["start", "Work"])), Ok(Command::Start { session: "Work".into(), lock: None, force: false }));
        assert_eq!(
            parse(&args(&["lock", "Work", "--lock-duration", "2h", "--force"])),
            Ok(Command::Lock { session: "Work".into(), lock: Lock::timed_for("2h"), force: true })
        );
        assert!(parse(&args(&["lock", "Work"])).is_err());
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-until", "2026-10-06 18:00"])),
            Ok(Command::Start { session: "Work".into(), lock: Some(Lock::timed_until("2026-10-06 18:00")), force: false })
        );
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-until-restart"])),
            Ok(Command::Start { session: "Work".into(), lock: Some(Lock::UntilRestart), force: false })
        );
        assert!(parse(&args(&["start", "Work", "--lock-until", "tomorrow"])).is_err());
        assert!(parse(&args(&["start", "Work", "--lock-until-restart", "--no-lock"])).is_err());
        // What starting asks, per lock.
        let overview = library();
        let warning = lock_warning(&overview, &overview.sessions[0]).unwrap();
        assert!(warning.contains("while its lock condition \"Work hours\" is active."), "{warning}");
        assert!(!warning.contains("08:00"), "{warning}");
        let timed = Definition { lock: Lock::timed_for("2h5m"), ..overview.sessions[0].clone() };
        assert!(lock_warning(&overview, &timed).unwrap().contains("for 2 hours and 5 minutes."));
        assert_eq!(lock_warning(&overview, &Definition { lock: Lock::None, ..timed }), None);
    }

    #[test]
    fn conditions_are_made_from_times_or_a_drive() {
        assert_eq!(
            parse(&args(&["condition", "new", "Work hours", "--times", "weekdays 9-17"])),
            Ok(Command::ConditionNew { name: "Work hours".into(), rule: RuleChoice::Times("weekdays 9-17".into()) })
        );
        assert_eq!(
            parse(&args(&["condition", "edit", "Work hours", "--times", "mon-thu 9-17"])),
            Ok(Command::ConditionEdit { condition: "Work hours".into(), name: None, rule: Some(RuleChoice::Times("mon-thu 9-17".into())) })
        );
        assert!(times_of("always").is_err() && times_of("someday").is_err());
        assert_eq!(times_of("sat").ok().map(|w| w.describe()), Some("Sat 00:00-24:00".into()));
    }

    #[test]
    fn several_items_are_the_words_after_their_option() {
        let items = |line: &[&str], name: &str| Options::new(&args(line)).items(name);
        // Each word up to the next option is an item, also the first with =; a word with a space
        // in it (quoted in the shell) is one item, and a comma is part of an item.
        let both = Ok(Some(vec!["a.com".to_string(), "b.com".into()]));
        assert_eq!(items(&["--sites", "a.com", "b.com", "--force"], "--sites"), both);
        assert_eq!(items(&["--sites=a.com", "b.com"], "--sites"), both);
        assert_eq!(
            items(&["--titles", "Inbox, 3 unread", " *Chat* "], "--titles"),
            Ok(Some(vec!["Inbox, 3 unread".to_string(), "*Chat*".into()]))
        );
        assert_eq!(items(&["--force"], "--sites"), Ok(None));
        // A single value is one word, and needs one.
        assert_eq!(Options::new(&args(&["--name"])).value("--name"), Err("--name needs a value".into()));
        // Once, and with something in it.
        assert!(items(&["--sites", "a.com", "--sites", "b.com"], "--sites").unwrap_err().contains("more than once"));
        assert_eq!(items(&["--sites", "--force"], "--sites"), Err("--sites needs one or more items, separated by spaces".into()));
        assert_eq!(items(&["--sites=", " "], "--sites"), Err("--sites needs one or more items, separated by spaces".into()));
        // What is left after the items is for the other options.
        let mut options = Options::new(&args(&["--any", "Work", "Weekend", "--name", "Either"]));
        assert_eq!(options.items("--any"), Ok(Some(vec!["Work".to_string(), "Weekend".into()])));
        assert_eq!(options.value("--name"), Ok(Some("Either".into())));
        assert_eq!(options.finish(), Ok(()));
        // The commands take them so.
        let Ok(Command::TargetNew { changes, .. }) = parse(&args(&["target", "new", "A", "--titles", "Inbox, 3 unread", "*Chat*"])) else {
            panic!("not new")
        };
        assert_eq!(changes.lists.get("titles"), Some(&vec!["Inbox, 3 unread".to_string(), "*Chat*".into()]));
        let Ok(Command::TargetEdit { changes, .. }) = parse(&args(&["target", "edit", "A", "--clear", "sites", "allowed-titles"])) else {
            panic!("not edit")
        };
        assert_eq!(changes.lists.keys().copied().collect::<Vec<_>>(), vec!["allowed_titles", "filters"]);
        assert_eq!(changes.lists.get("filters"), Some(&Vec::new()));
        let Ok(Command::New { changes, .. }) = parse(&args(&["new", "B", "--targets", "Social", "Games"])) else { panic!("not new") };
        assert_eq!(changes.targets, Some(vec!["Social".to_string(), "Games".into()]));
    }

    #[test]
    fn lists_come_from_files_too() {
        let path = std::env::temp_dir().join(format!("cli-{}-sites.txt", std::process::id()));
        std::fs::write(&path, "a.com\r\n\r\nb.com\n").unwrap();
        let line = args(&["target", "new", "A", "--sites-file", path.to_str().unwrap(), "--sites", "c.com"]);
        let Ok(Command::TargetNew { changes, .. }) = parse(&line) else { panic!("not new") };
        assert_eq!(changes.lists.get("filters"), Some(&vec!["c.com".to_string(), "a.com".into(), "b.com".into()]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn what_is_not_understood() {
        for bad in [
            &["frobnicate"][..],
            &["new"],
            &["new", "--targets", "a"],
            &["new", "A", "--lock-duration", "soon"],
            &["new", "A", "--lock-duration", ""],
            &["new", "A", "--lock-duration", "1h", "--no-lock"],
            &["new", "A", "--name", "B"],
            &["new", "A", "--sites", "a.com"],
            &["target"],
            &["target", "new"],
            &["target", "new", "A", "--sites"],
            &["target", "new", "A", "--actions", "chromium_tabs,lasers"],
            &["target", "new", "A", "--clear", "sites"],
            &["target", "edit", "A", "--clear", "everything"],
            &["target", "frob", "A"],
            &["condition", "new", "A"],
            &["condition", "new", "A", "--times", "mon 9-17", "--drive", "Kingston"],
            &["condition", "check"],
            &["edit"],
            &["start"],
            &["start", "A", "B"],
            &["stop", "A", "now"],
            &["target", "new", "A", "--sites-file", "Z:/no/such/file.txt"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_other_commands() {
        assert_eq!(parse(&[]), Ok(Command::Help));
        assert_eq!(parse(&args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse(&args(&["status", "--json"])), Ok(Command::Status { json: true }));
        assert_eq!(parse(&args(&["show", "Work", "--json"])), Ok(Command::Show { session: "Work".into(), json: true }));
        assert_eq!(parse(&args(&["delete", "Work", "--force"])), Ok(Command::Delete { session: "Work".into(), force: true }));
        assert_eq!(parse(&args(&["export", "b.json"])), Ok(Command::Export { file: "b.json".into() }));
        assert_eq!(parse(&args(&["import", "b.json", "--force"])), Ok(Command::Import { file: "b.json".into(), force: true }));
        assert!(parse(&args(&["import"])).is_err());
        assert_eq!(parse(&args(&["duplicate", "Work"])), Ok(Command::Duplicate { kind: Kind::Session, item: "Work".into() }));
        assert_eq!(parse(&args(&["target", "duplicate", "Social"])), Ok(Command::Duplicate { kind: Kind::Target, item: "Social".into() }));
        assert_eq!(
            parse(&args(&["condition", "duplicate", "Evenings"])),
            Ok(Command::Duplicate { kind: Kind::Condition, item: "Evenings".into() })
        );
        assert_eq!(parse(&args(&["stop", "Work"])), Ok(Command::Stop { session: "Work".into(), password: None, abort: false }));
        assert_eq!(
            parse(&args(&["stop", "Work", "--password", "pw"])),
            Ok(Command::Stop { session: "Work".into(), password: Some("pw".into()), abort: false })
        );
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-password", "partner"])),
            Ok(Command::Start {
                session: "Work".into(),
                lock: Some(Lock::Password { password: Some("partner".into()), hash: None }),
                force: false
            })
        );
        assert!(parse(&args(&["start", "Work", "--lock-password", "pw", "--no-lock"])).is_err());
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-text", "I will not scroll", "--force"])),
            Ok(Command::Start { session: "Work".into(), lock: Some(Lock::Text { text: "I will not scroll".into() }), force: true })
        );
        assert!(parse(&args(&["start", "Work", "--lock-text", "x", "--lock-password", "pw"])).is_err());
        // A flash drive is named here and looked up among the connected ones when the command runs.
        assert_eq!(
            parse(&args(&["condition", "new", "Key", "--drive", "Kingston"])),
            Ok(Command::ConditionNew { name: "Key".into(), rule: RuleChoice::Drive("Kingston".into()) })
        );
        assert_eq!(parse(&args(&["drives", "--json"])), Ok(Command::Drives { json: true }));
        assert_eq!(parse(&args(&["target", "show", "Social"])), Ok(Command::TargetShow { target: "Social".into(), json: false }));
        assert_eq!(
            parse(&args(&["target", "delete", "Social", "--force"])),
            Ok(Command::TargetDelete { target: "Social".into(), force: true })
        );
        assert_eq!(parse(&args(&["condition", "show", "Work hours"])), Ok(Command::ConditionShow { condition: "Work hours".into() }));
        assert_eq!(
            parse(&args(&["condition", "delete", "Work hours"])),
            Ok(Command::ConditionDelete { condition: "Work hours".into(), force: false })
        );
        assert_eq!(parse(&args(&["condition", "check", "weekdays 9-5pm"])), Ok(Command::ConditionCheck { times: "weekdays 9-5pm".into() }));
        assert_eq!(parse(&args(&["unlock"])), Ok(Command::Unlock { abort: false, password: None }));
        assert_eq!(parse(&args(&["unlock", "--password", "pw"])), Ok(Command::Unlock { abort: false, password: Some("pw".into()) }));
        assert_eq!(parse(&args(&["unlock-way"])), Ok(Command::UnlockWay { way: None }));
        assert_eq!(parse(&args(&["unlock-way", "--none"])), Ok(Command::UnlockWay { way: Some(None) }));
        assert_eq!(
            parse(&args(&["unlock-way", "--text", "let me out"])),
            Ok(Command::UnlockWay { way: Some(Some(Lock::Text { text: "let me out".into() })) })
        );
        assert!(parse(&args(&["unlock-way", "--text", "x", "--password", "y"])).is_err());
        assert!(parse(&args(&["unlock-way", "--duration", "1h"])).is_err());
        assert_eq!(parse(&args(&["unlock", "--abort"])), Ok(Command::Unlock { abort: true, password: None }));
        assert_eq!(parse(&args(&["unlock-delay"])), Ok(Command::UnlockDelay { delay: None }));
        assert_eq!(parse(&args(&["unlock-delay", "2h 30m"])), Ok(Command::UnlockDelay { delay: Some("2h 30m".into()) }));
        assert_eq!(parse(&args(&["unlock-delay", "0"])), Ok(Command::UnlockDelay { delay: Some("0".into()) }));
    }

    #[test]
    fn deleting_says_what_it_does_to_the_sessions() {
        let overview = library();
        assert_eq!(fallout(&overview, "l1"), " \"Work\" goes on without this target.");
        assert_eq!(fallout(&overview, "s1"), " \"Work\" is always active from then on. \"Work\" has no lock from then on.");
        assert_eq!(fallout(&overview, "other"), "");
    }

    #[test]
    fn things_are_found_by_id_or_name() {
        let lists = vec![
            target("a1", &[]),
            Target { name: "Games".into(), ..target("b2", &[]) },
            Target { name: "games".into(), ..target("c3", &[]) },
        ];
        fn key(l: &Target) -> (&String, &String) {
            (&l.id, &l.name)
        }
        assert_eq!(find(&lists, "a1", "target", key).ok().map(|l| l.id.as_str()), Some("a1"));
        assert_eq!(find(&lists, "list A1", "target", key).ok().map(|l| l.id.as_str()), Some("a1"));
        assert!(matches!(find(&lists, "Games", "target", key), Err(Failure::Failed(why)) if why.contains("b2, c3")));
        assert!(find(&lists, "Nope", "target", key).is_err());
        assert!(new_id().chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && new_id().len() <= 64);
    }

    #[test]
    fn the_status_reads_like_the_pages() {
        let empty = Overview {
            targets: vec![],
            conditions: vec![],
            sessions: vec![],
            running: vec![],
            unlock: UnlockStatus::default(),
            clock_synced: true,
        };
        assert!(describe_overview(&empty).contains("Sessions:\n  None yet"));
        let mut overview = library();
        overview.running.push(Running {
            id: "b1".into(),
            name: "Work".into(),
            targets: vec![RunningTarget {
                name: "Social".into(),
                entries: Entries { filters: vec!["youtube.com".into()], ..Entries::default() },
                actions: Actions::default(),
            }],
            remaining: None,
            lock_condition: Week::parse("mon-fri 8-17").unwrap().map(|week| Rule::Schedule { week }),
            until_restart: false,
            lock_text: None,
            stop_delay: None,
            stop_delay_words: None,
            stop_remaining: None,
            stop_remaining_words: None,
            password: false,
            locked: false,
            condition: Week::parse("mon-fri 8-17").unwrap().map(|week| Rule::Schedule { week }),
            enforcing: false,
            condition_change: Some(3600),
            remaining_words: None,
            condition_change_words: Some(duration::countdown(3600)),
            lock_change: Some(3600),
            lock_change_words: Some(duration::countdown(3600)),
            allowance: None,
        });
        overview.clock_synced = false;
        let text = describe_overview(&overview);
        assert!(text.contains("  Work  Running · inactive for the next 1 hour · unlocked for the next 1 hour"), "{text}");
        assert!(text.contains("Targets:\n  Social  used by Work"), "{text}");
        assert!(text.contains("Conditions:\n  Work hours  Mon-Fri 08:00-17:00"), "{text}");
        assert!(text.contains("Waiting for internet time"), "{text}");

        let session = describe_session(&overview, &overview.sessions[0], overview.running.first());
        assert!(session.contains("Targets:     Social") && session.contains("Condition:   Work hours\n"), "{session}");
        assert!(!session.contains("08:00-17:00"), "{session}");
        assert!(session.contains("Lock:        condition: cannot be stopped while \"Work hours\" is active"), "{session}");
        assert_eq!(
            resolve_lock(&overview, Lock::Condition { condition: "work HOURS".into() }).ok(),
            Some(Lock::Condition { condition: "s1".into() })
        );
        let list = describe_target(&overview, &overview.targets[0]);
        assert!(list.contains("Sites:          youtube.com") && list.contains("Used by:        Work"), "{list}");
    }

    /// What `new` and `edit` say of the allowance, as terms over the ones the session has.
    fn allowance_of(line: &[&str], old: Option<Terms>) -> Result<Option<Terms>, String> {
        let changes = match parse(&args(line))? {
            Command::New { changes, .. } | Command::Edit { changes, .. } => changes,
            other => panic!("not new or edit: {other:?}"),
        };
        let mut b = library().sessions[0].clone();
        b.allowance = old;
        match apply_session_changes(&library(), &mut b, changes) {
            Ok(()) => Ok(b.allowance),
            Err(Failure::Failed(why) | Failure::Usage(why)) => Err(why),
        }
    }

    #[test]
    fn a_session_has_its_allowance_set_by_its_options() {
        let budget = |allowed: &str, scope, reset| {
            Some(Terms::Budget { allowed: allowed.into(), scope, reset, every: String::new(), anchor: Anchor::Start })
        };
        let every = |allowed: &str, scope, period: &str, anchor| {
            Some(Terms::Budget { allowed: allowed.into(), scope, reset: Reset::Every, every: period.into(), anchor })
        };
        let cycle =
            |blocked: &str, allowed: &str, anchor| Some(Terms::Pomodoro { blocked: blocked.into(), allowed: allowed.into(), anchor });
        assert_eq!(
            allowance_of(
                &["new", "Social", "--allowance", "budget", "--allowed", "30m", "--scope", "each", "--reset=1d", "--anchor", "clock"],
                None
            ),
            Ok(every("30m", Scope::Each, "1d", Anchor::Clock))
        );
        // Every so long, from the start by default; never and by condition.
        assert_eq!(
            allowance_of(&["new", "A", "--allowed", "1h", "--reset", "36h"], None),
            Ok(every("1h", Scope::Total, "36h", Anchor::Start))
        );
        assert_eq!(
            allowance_of(&["new", "A", "--allowed", "1h", "--reset", "Condition"], None),
            Ok(budget("1h", Scope::Total, Reset::Condition))
        );
        // --anchor alone keeps a budget a budget, and a reset every so long keeps its period.
        let daily = every("30m", Scope::Total, "1d", Anchor::Start);
        assert_eq!(
            allowance_of(&["edit", "Work", "--anchor", "clock"], daily.clone()),
            Ok(every("30m", Scope::Total, "1d", Anchor::Clock))
        );
        assert_eq!(allowance_of(&["edit", "Work", "--reset", "never"], daily), Ok(budget("30m", Scope::Total, Reset::Never)));
        assert!(allowance_of(&["new", "A", "--allowed", "1h", "--reset", "7h", "--anchor", "clock"], None)
            .unwrap_err()
            .contains("fits a day evenly"));
        // The defaults: the whole session, never reset; the kind can be left out.
        assert_eq!(allowance_of(&["new", "A", "--allowed", "1h"], None), Ok(budget("1h", Scope::Total, Reset::Never)));
        // A cycle is told by its own options.
        assert_eq!(
            allowance_of(&["new", "P", "--blocked", "50m", "--allowed", "10m", "--anchor", "clock"], None),
            Ok(cycle("50m", "10m", Anchor::Clock))
        );
        // Without its options the allowance stays as it is; editing replaces what is named and keeps the rest.
        let old = cycle("50m", "10m", Anchor::Start);
        assert_eq!(allowance_of(&["edit", "Work", "--name", "Pauses"], old.clone()), Ok(old.clone()));
        assert_eq!(allowance_of(&["edit", "Work", "--allowed", "5m"], old.clone()), Ok(cycle("50m", "5m", Anchor::Start)));
        // Another kind starts afresh, and none takes it away.
        assert_eq!(
            allowance_of(&["edit", "Work", "--allowance", "budget", "--allowed", "20m"], old.clone()),
            Ok(budget("20m", Scope::Total, Reset::Never))
        );
        assert_eq!(allowance_of(&["edit", "Work", "--allowance", "NONE"], old.clone()), Ok(None));
        // What cannot be: no time, a cycle that does not fit the day on the clock, and words that are not options.
        assert!(allowance_of(&["new", "A", "--allowance", "budget"], None).is_err());
        assert!(allowance_of(&["new", "P", "--blocked", "25m", "--allowed", "10m", "--anchor", "clock"], None).is_err());
        for bad in [
            &["new", "A", "--allowance", "forever"][..],
            &["new", "A", "--scope", "some"],
            &["new", "A", "--reset", "monthly"],
            &["new", "A", "--anchor", "noon"],
            &["new", "A", "--allowance", "none", "--allowed", "5m"],
            &["allowance", "new", "A"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }

        // It is shown on the session, and what it is doing with a running one.
        let mut overview = library();
        overview.sessions[0].allowance = cycle("50m", "10m", Anchor::Clock);
        let shown = describe_session(&overview, &overview.sessions[0], None);
        assert!(shown.contains("Allowance:   Blocked for 50 minutes, then allowed for 10 minutes, on the clock"), "{shown}");
        assert!(describe_session(&library(), &library().sessions[0], None).contains("Allowance:   none\n"));
        let status = AllowanceStatus {
            terms: Terms::Pomodoro { blocked: "50m".into(), allowed: "10m".into(), anchor: Anchor::Clock },
            open: true,
            change: Some(240),
            change_words: Some("4 minutes".into()),
            left: None,
            left_words: None,
            reset_in: None,
            reset_words: None,
            items: vec![],
        };
        assert_eq!(allowance_words(&status), "allowed for the next 4 minutes");
        assert_eq!(allowance_words(&AllowanceStatus { open: false, ..status }), "blocked for the next 4 minutes");
        let used = AllowanceStatus {
            terms: Terms::Budget {
                allowed: "30m".into(),
                scope: Scope::Total,
                reset: Reset::Every,
                every: "1d".into(),
                anchor: Anchor::Clock,
            },
            open: true,
            change: None,
            change_words: None,
            left: Some(720),
            left_words: Some("12 minutes".into()),
            reset_in: Some(10800),
            reset_words: Some("3 hours".into()),
            items: vec![],
        };
        assert_eq!(allowance_words(&used), "12 minutes left of 30 minutes, reset in 3 hours");
    }

    #[test]
    fn a_delayed_start_is_set_by_after_or_from() {
        let delayed = |kind, value: &str| Some(Terms::Delayed { kind, value: value.into() });
        assert_eq!(allowance_of(&["new", "A", "--after", "30m"], None), Ok(delayed(DelayKind::After, "30m")));
        assert_eq!(allowance_of(&["new", "A", "--allowance", "delayed", "--after", "1h"], None), Ok(delayed(DelayKind::After, "1h")));
        assert_eq!(allowance_of(&["new", "A", "--from", "2026-10-06 18:00"], None), Ok(delayed(DelayKind::From, "2026-10-06 18:00")));
        // Over any allowance, a delayed start replaces it; over an old delayed start, the one named wins.
        let cycle = Some(Terms::Pomodoro { blocked: "50m".into(), allowed: "10m".into(), anchor: Anchor::Start });
        assert_eq!(allowance_of(&["edit", "Work", "--after", "5m"], cycle.clone()), Ok(delayed(DelayKind::After, "5m")));
        let old = delayed(DelayKind::After, "30m");
        assert_eq!(
            allowance_of(&["edit", "Work", "--from", "2026-10-06 18:00"], old.clone()),
            Ok(delayed(DelayKind::From, "2026-10-06 18:00"))
        );
        assert_eq!(allowance_of(&["edit", "Work", "--after", "10m"], old.clone()), Ok(delayed(DelayKind::After, "10m")));
        // Without --after or --from the old delayed start stays; without one there is nothing to keep.
        assert_eq!(allowance_of(&["edit", "Work", "--allowance", "delayed"], old.clone()), Ok(old.clone()));
        assert!(allowance_of(&["edit", "Work", "--allowance", "delayed"], cycle).unwrap_err().contains("Give --after"));
        assert!(allowance_of(&["new", "A", "--allowance", "delayed"], None).unwrap_err().contains("Give --after"));
        // The start is checked before anything is sent.
        assert!(allowance_of(&["new", "A", "--after", "soon"], None).is_err());
        assert!(allowance_of(&["new", "A", "--after", "0m"], None).is_err());
        assert!(allowance_of(&["new", "A", "--from", "tomorrow"], None).is_err());
        // None takes it away.
        assert_eq!(allowance_of(&["edit", "Work", "--allowance", "none"], old), Ok(None));
        // What does not go together.
        for (line, why) in [
            (&["new", "A", "--after", "5m", "--from", "2026-10-06 18:00"][..], "not both"),
            (&["new", "A", "--after", "5m", "--allowed", "5m"], "takes only --after or --from"),
            (&["new", "A", "--from", "2026-10-06 18:00", "--blocked", "5m"], "takes only --after or --from"),
            (&["new", "A", "--allowance", "delayed", "--scope", "each"], "takes only --after or --from"),
            (&["new", "A", "--allowance", "delayed", "--reset", "never"], "takes only --after or --from"),
            (&["new", "A", "--allowance", "delayed", "--anchor", "clock"], "takes only --after or --from"),
            (&["new", "A", "--allowance", "none", "--after", "5m"], "takes no other allowance options"),
            (&["new", "A", "--allowance", "none", "--from", "2026-10-06 18:00"], "takes no other allowance options"),
            (&["new", "A", "--allowance", "budget", "--after", "5m"], "go with --allowance delayed"),
            (&["new", "A", "--allowance", "pomodoro", "--from", "2026-10-06 18:00"], "go with --allowance delayed"),
        ] {
            let error = parse(&args(line)).unwrap_err();
            assert!(error.contains(why), "{line:?}: {error}");
        }
        assert!(parse(&args(&["new", "A", "--allowance", "wait"])).is_err());
        // It is shown on the session, and what it is doing with a running one.
        let mut overview = library();
        overview.sessions[0].allowance = delayed(DelayKind::After, "30m");
        let shown = describe_session(&overview, &overview.sessions[0], None);
        assert!(shown.contains("Allowance:   Nothing is enforced for the first 30 minutes\n"), "{shown}");
        overview.sessions[0].allowance = delayed(DelayKind::From, "2026-10-06 18:00");
        let shown = describe_session(&overview, &overview.sessions[0], None);
        assert!(shown.contains("Allowance:   Nothing is enforced until Tuesday 6 October 2026, 18:00\n"), "{shown}");
        let status = AllowanceStatus {
            terms: Terms::Delayed { kind: DelayKind::After, value: "30m".into() },
            open: true,
            change: Some(240),
            change_words: Some("4 minutes".into()),
            left: None,
            left_words: None,
            reset_in: None,
            reset_words: None,
            items: vec![],
        };
        assert_eq!(allowance_words(&status), "nothing enforced for the next 4 minutes");
        let begun = AllowanceStatus { open: false, change: None, change_words: None, ..status };
        assert_eq!(allowance_words(&begun), "enforcing");
    }

    #[test]
    fn a_schedule_is_shown_per_day() {
        let text = per_day(&Week::parse("mon-fri 8-12, 13-17; sat 10-12").unwrap().unwrap());
        assert!(text.starts_with("Mon-Fri 08:00-12:00, 13:00-17:00; Sat 10:00-12:00\n"), "{text}");
        assert!(text.contains("  Tue  08:00-12:00, 13:00-17:00\n"), "{text}");
        assert!(text.contains("  Sun  inactive\n"), "{text}");
    }
}
