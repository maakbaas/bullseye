//! The command line: everything the window can do, from a terminal or a script
//! (`bullseye-cli.exe`, explained in `CLI.md`). It talks to the service through the same
//! `client::Remote` as the window, so the service checks everything done here exactly as it checks
//! the window, and texts such as schedules and durations come from the same Rust functions.
//!
//! **Keep it level with the window.** Every command the window can invoke (the list in
//! `generate_handler!` in `lib.rs`) is in `GUI_COMMANDS` with the CLI commands that do the same, and
//! a test fails when one is missing, or missing from `HELP` or `CLI.md`. Enforcer names come from
//! `Enforcers` itself, so a new enforcer is here without any change. A new list, condition kind or
//! lock method needs an option here, a line in `HELP` and in `CLI.md`.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::allowance::{Anchor, Reset, Scope, Status as AllowanceStatus, Terms};
use crate::block::Running;
use crate::client::Remote;
use crate::condition::{Mode, Rule};
use crate::definitions::{self, BlockList, Condition, Definition, Kind, Library};
use crate::duration;
use crate::lists::Entries;
use crate::localtime;
use crate::lock::Lock;
use crate::proto::{Overview, Request, UnlockStatus};
use crate::schedule::Week;
use crate::state::Enforcers;
use crate::webhook::{self, Hook, Match, Matcher, Method};

/// Each command the window can invoke, and the CLI commands that do the same. Empty: it is about
/// the window itself.
pub const GUI_COMMANDS: [(&str, &[&str]); 30] = [
    ("block_status", &["status", "show", "list show", "condition show"]),
    ("save_block", &["new", "edit"]),
    ("delete_block", &["delete"]),
    ("save_list", &["list new", "list edit"]),
    ("delete_list", &["list delete"]),
    // The installed Store apps the editor offers to tick.
    ("installed_apps", &["list apps"]),
    // The flash drives the editor of a flash drive condition offers.
    ("connected_drives", &["drives"]),
    ("save_condition", &["condition new", "condition edit"]),
    ("delete_condition", &["condition delete"]),
    ("duplicate", &["duplicate", "list duplicate", "condition duplicate"]),
    ("start_block", &["start"]),
    ("stop_block", &["stop"]),
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
    // A block's allowance in words: what `show` says of it.
    ("describe_allowance", &["show"]),
    // A lock's length or end in words: what `show` and `start` say of a duration or until lock.
    ("describe_duration", &["show"]),
    ("describe_until", &["show", "start"]),
    // What the question before a deletion says.
    ("deletion_fallout", &["list delete", "condition delete"]),
    ("export_settings", &["export"]),
    // A settings file's contents: what the question before importing it says.
    ("open_settings", &["import"]),
    ("import_settings", &["import"]),
    ("window_ready", &[]),
];

const HELP: &str = "\
Bullseye command line: everything the window can do.

Usage: bullseye-cli <command> [options]
A <block>, <list> or <condition> is its name (any case) or its id.

A block is what you start: one or more block lists (what to block), a condition (when it is
active; 'always', a weekly schedule, or a flash drive that is not connected), at most one
allowance (what lets it go quiet for a while) and a lock (how it is kept from being stopped).

Blocks:
  status [--json]          Every block (running or not, active or not, locked or not), every
                           block list and every condition.
  show <block> [--json]    One block: its lists, condition, lock and state.
  new <name> [options]     Make a block. Options:
      --list <list>            A block list it enforces (repeatable).
      --condition <condition>  When it is active: a condition's name, or 'always' (the default).
      --allowance <budget|pomodoro|none>
                               What lets it go quiet for a while (none by default): a budget of
                               use, or a Pomodoro cycle; use is what the app sees: a page shown
                               in a tab, a window, a program that runs. Given with the options
                               below, it can be left out (they say which); edit changes only
                               what they name.
      --allowed <duration>     Budget: the time allowed, e.g. 30m. Pomodoro: how long
                               everything is allowed for each cycle.
      --scope <total|each>     Budget: for the whole block (the default) or for each site,
                               program, Store app and window title on its own.
      --reset <never|condition|duration>
                               Budget: when what was used is given back: never while the
                               block runs (the default), each time the block's condition turns
                               on, or every so long, e.g. 1d or 12h, counted by --anchor.
      --blocked <duration>     Pomodoro: how long it is blocked for before it allows.
      --anchor <start|clock>   What a Pomodoro, or a budget's reset every so long, is counted
                               from: the block's start (the default), or midnight on the clock,
                               so an hour starts at xx:00 (it must then fit a day evenly).
      --lock-duration <duration>
                               Locked for this long once started, e.g. 2h 30m: it cannot be
                               stopped and ends by itself.
      --lock-until <date time> Locked until then, e.g. \"2026-10-06 18:00\" (local time): it
                               cannot be stopped and ends by itself.
      --lock-until-restart     Locked until Windows restarts, and ends then. A shut down with
                               Fast Startup is not a restart.
      --lock-condition <condition>
                               Locked while this condition is active: it cannot be stopped then,
                               and runs until stopped otherwise. With the block's own condition,
                               it is locked exactly while it is active.
      --lock-password <pw>     Locked with a password: it runs until stopped with that password
                               (or by unlocking). Only a hash of it is kept.
      --lock-delay <time>      Locked behind a wait, like unlocking: stopping is asked for, counts
                               down this long (e.g. 30m), can be aborted, and then it can be
                               stopped.
      --lock-text <text>       Locked until the text is typed in exactly: the window shows it
                               above an input that cannot be pasted into, and that is the only
                               place to stop such a block (or by unlocking).
      --no-lock                Not locked: it can be stopped at any time (the default).
  edit <block> [options]   Change a block: the options of new, which replace what they name
                           (--list replaces all its lists), and --name <name>. A running block
                           takes the change at once (not its lock); while it is locked, only its
                           name and a stricter allowance: a budget shorter or reset less often,
                           or none; a cycle cannot change.
  delete <block> [--yes]   Delete a block that is not running. Asks first, unless --yes.
  duplicate <block>        Make a copy of a block, named after it with a number: \"Work (2)\".
  start <block> [--yes]    Start a block. The lock options of new set its lock first, as the lock
                           on its tile in the window does. A locked block asks first, unless --yes.
  stop <block> [--password <pw>]
                           Stop a running block that is not locked right now; one locked with a
                           password with its password (asked for when not given). One locked
                           with a text is stopped in the window, by typing the text. One locked
                           with a delay: the first stop asks for it and starts the wait, the
                           next one, once it is over, stops the block; `stop <block> --abort`
                           ends the wait.

Block lists:
  list show <list> [--json]   One block list, and the blocks that use it.
  list apps [--json]          The Store apps installed, by name and package family name, to
                              block with --store-app.
  list new <name> [options]   Make a block list. Options:
      --site <entry>           A site to block (repeatable): the site with its subdomains and
                               everything under it, e.g. youtube.com. * as a wildcard.
      --allow <entry>          A site that is always allowed, also when blocked (repeatable).
      --program <name>         A program to close, by exe name (repeatable), e.g. steam.exe.
      --store-app <family>     A Store app to close, by its exact package family name
                               (repeatable), e.g. Microsoft.WindowsCalculator_8wekyb3d8bbwe, as
                               'list apps' shows them; no *.
      --title <title>          A window to close, by its title (repeatable), * as a wildcard,
                               e.g. \"*YouTube*\". The whole window closes, tabs and all.
      --allow-title <title>    A window title that is always left alone, also when one of
                               the titles matches it (repeatable).
      --sites-file <file>      Sites to block, one per line. Likewise --allowed-file,
                               --programs-file, --store-apps-file, --titles-file and
                               --allowed-titles-file.
      --enforcers <list>       Comma-separated, or 'all' (the default). Names:
                               {enforcers}
  list edit <list> [options]  Change a block list: the options of list new, which replace what
                              they name, --name <name> and --clear <sites|allowed|programs|
                              store-apps|titles|allowed-titles> (repeatable). While a locked
                              block uses it, only stricter: more to block, fewer allowed, more
                              enforcers.
  list delete <list> [--yes]  Delete a block list that no running block uses; it is taken out of
                              the blocks that use it (a block left without one is deleted).
  list duplicate <list>       Make a copy of a block list, named after it with a number.

Conditions:
  condition show <condition>        One condition, and the blocks that use it.
  condition new <name> (--times <times> | --drive <drive> | --webhook <url> [webhook options] | --any <condition>... | --all <condition>...)
                                    Make a condition: a weekly schedule, e.g. --times
                                    \"Mon-Fri 08:00-17:00; Sat 10-12\"; or a flash drive, by name
                                    or serial number ('drives' lists them): active while it is
                                    not connected; or a combination of two or more other
                                    conditions (--any or --all, once for each): active when any
                                    of them is, or when all are. Not of other combinations. A
                                    webhook asks one web address (it can hold GET parameters) every
                                    --every <time> (at least 1m, default 5m), with --method get|post
                                    (default get). It is active when the answer matches the true
                                    matcher --true-status <code> and/or --true-body <text>; if not,
                                    not active when it matches the false matcher --false-status
                                    and/or --false-body (a body must equal the text); if neither
                                    does, or no answer comes, --fallback active|inactive (default
                                    active) says. Both matchers are needed.
  condition edit <condition> [--name <name>] [--times <times> | --drive <drive> | --webhook <url> [webhook options] | --any <condition>... | --all <condition>...]
                                    Change a condition. While a locked block uses it, a schedule
                                    can only get more times, a drive cannot change, and a
                                    combination can only get more active.
  condition delete <condition> [--yes]
                                    Delete a condition that no running block uses, directly or
                                    in a combination; blocks it was the condition of are always
                                    active, and those it locked are not locked any more. It is
                                    taken out of its combinations, and one left with a single
                                    condition is deleted too.
  condition duplicate <condition>   Make a copy of a condition, named after it with a number.
  condition test <condition>        Ask a webhook condition once: the status, the start of the
                                    body, and what the matchers make of it.
  condition check <times>           Check times and show them per day, as a schedule takes them.

Moving to another computer:
  export <file>            Write every block list, condition and block to a settings
                           file, as Export in Settings does. Password locks are in it without
                           their password.
  import <file> [--yes]    Replace every block list, condition and block with those in a settings
                           file, as Import in Settings does; not while a block is locked, and
                           blocks that run unlocked end with it. A block with a password lock
                           comes without a lock. Asks first, unless --yes.

Other:
  drives [--json]          The flash drives connected, by name and serial number, for
                           'condition new --drive'.
  unlock [--abort] [--password <pw>]
                           End every running block, as the window's Unlock does. By default
                           unlocking is free. With an unlock delay it is a countdown: the first
                           'unlock' starts it, a later one ends every block once it is over, and
                           --abort stops it (back to locked). The other way to unlock (below)
                           works instead of the countdown, never as well: with a password
                           (asked for when not given) unlocking is at once; one set to a text is
                           used in the window, by typing the text.
  unlock-way [options]     Show the other way to unlock besides the delay, or set it: --condition
                           <condition> (locked while it is active, open when it is not),
                           --password <pw>, --text <text> or --none to remove it. Setting one needs unlocking to be ready, or no
                           block to be locked; removing it is free.
  unlock-delay [<time>]    Show how long unlocking takes, or set it: 0 to 24 hours, e.g. 30m
                           or 2h 30m, or 0 for none. A longer delay is free; a shorter one needs
                           unlocking to be ready, or no block to be locked.
  help                    Show this.
  version                  Show the version.

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
        block: String,
        json: bool,
    },
    New {
        name: String,
        changes: BlockChanges,
    },
    Edit {
        block: String,
        changes: BlockChanges,
    },
    Delete {
        block: String,
        yes: bool,
    },
    Start {
        block: String,
        lock: Option<Lock>,
        yes: bool,
    },
    Stop {
        block: String,
        password: Option<String>,
        abort: bool,
    },
    ListShow {
        list: String,
        json: bool,
    },
    ListApps {
        json: bool,
    },
    Drives {
        json: bool,
    },
    ListNew {
        name: String,
        changes: ListChanges,
    },
    ListEdit {
        list: String,
        changes: ListChanges,
    },
    ListDelete {
        list: String,
        yes: bool,
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
        yes: bool,
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
        yes: bool,
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

/// What `new` and `edit` set on a block; `None` leaves it. Lists and the condition by name or id,
/// looked up when the command runs.
#[derive(Debug, PartialEq, Default)]
struct BlockChanges {
    name: Option<String>,
    lists: Option<Vec<String>>,
    /// `Some(None)`: always.
    condition: Option<Option<String>>,
    /// What `--allowance` and its options set; `None`: they were not given.
    allowance: Option<AllowanceChanges>,
    lock: Option<Lock>,
}

/// What `--allowance` and its options of `new` and `edit` set on a block's allowance; `None` leaves
/// it (with no allowance yet, the default). Read when the command runs (`build_terms`).
#[derive(Debug, PartialEq, Default)]
struct AllowanceChanges {
    /// `budget`, `pomodoro` or `none`.
    kind: Option<String>,
    allowed: Option<String>,
    scope: Option<Scope>,
    reset: Option<Reset>,
    /// With `Reset::Every`: how often, as typed (`1d`).
    every: Option<String>,
    blocked: Option<String>,
    anchor: Option<Anchor>,
}

/// What `list new` and `list edit` set on a block list; `None` leaves it.
#[derive(Debug, PartialEq, Default)]
struct ListChanges {
    name: Option<String>,
    /// The lists to set, by their field of `Entries`.
    lists: BTreeMap<&'static str, Vec<String>>,
    enforcers: Option<Enforcers>,
}

/// The options that set a list, one per list: the option, the one that reads it from a file, its
/// name in `--clear`, and its field of `Entries`. **A new list is a line here.**
const LIST_OPTIONS: [(&str, &str, &str, &str); 6] = [
    ("--site", "--sites-file", "sites", "filters"),
    ("--allow", "--allowed-file", "allowed", "allowed"),
    ("--program", "--programs-file", "programs", "programs"),
    ("--store-app", "--store-apps-file", "store-apps", "store_apps"),
    ("--title", "--titles-file", "titles", "titles"),
    ("--allow-title", "--allowed-titles-file", "allowed-titles", "allowed_titles"),
];

impl ListChanges {
    fn apply(self, list: &mut BlockList) {
        if let Some(name) = self.name {
            list.name = name;
        }
        for (key, entries) in self.lists {
            if let Some(field) = list.entries.field(key) {
                *field = entries;
            }
        }
        if let Some(enforcers) = self.enforcers {
            list.enforcers = enforcers;
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
            eprintln!("{why}\n\nRun `bullseye-cli help` for the commands.");
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
    // `list` and `condition` take a second word.
    let (command, rest) = match (command.as_str(), rest.split_first()) {
        ("list" | "condition", Some((verb, rest))) => (format!("{command} {verb}"), rest),
        ("list" | "condition", None) => {
            return Err(format!(
                "`{command}` needs show, new, edit, delete, duplicate{}",
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
        "show" => Command::Show { block: named(&mut options, "block")?, json: options.flag("--json") },
        "new" => {
            let name = options.positional().ok_or("Give the new block a name")?;
            Command::New { name, changes: parse_block_changes(&mut options, false)? }
        }
        "edit" => Command::Edit { block: named(&mut options, "block")?, changes: parse_block_changes(&mut options, true)? },
        "delete" => Command::Delete { block: named(&mut options, "block")?, yes: options.flag("--yes") },
        "export" => Command::Export { file: options.positional().ok_or("Give the file to write, e.g. bullseye.json")? },
        "import" => Command::Import { file: options.positional().ok_or("Give the settings file to read")?, yes: options.flag("--yes") },
        "duplicate" => Command::Duplicate { kind: Kind::Block, item: named(&mut options, "block")? },
        "list duplicate" => Command::Duplicate { kind: Kind::List, item: named(&mut options, "block list")? },
        "condition duplicate" => Command::Duplicate { kind: Kind::Condition, item: named(&mut options, "condition")? },
        "start" => Command::Start { block: named(&mut options, "block")?, lock: parse_lock(&mut options)?, yes: options.flag("--yes") },
        "stop" => {
            Command::Stop { block: named(&mut options, "block")?, password: options.value("--password")?, abort: options.flag("--abort") }
        }
        "list show" => Command::ListShow { list: named(&mut options, "block list")?, json: options.flag("--json") },
        "list apps" => Command::ListApps { json: options.flag("--json") },
        "drives" => Command::Drives { json: options.flag("--json") },
        "list new" => {
            let name = options.positional().ok_or("Give the new block list a name")?;
            Command::ListNew { name, changes: parse_list_changes(&mut options, false)? }
        }
        "list edit" => Command::ListEdit { list: named(&mut options, "block list")?, changes: parse_list_changes(&mut options, true)? },
        "list delete" => Command::ListDelete { list: named(&mut options, "block list")?, yes: options.flag("--yes") },
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
        "condition delete" => Command::ConditionDelete { condition: named(&mut options, "condition")?, yes: options.flag("--yes") },
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
    let duration = options.value("--lock-duration")?.map(|duration| Lock::Duration { duration });
    let until = options.value("--lock-until")?.map(|until| Lock::Until { until });
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
fn parse_block_changes(options: &mut Options, editing: bool) -> Result<BlockChanges, String> {
    let lists = options.values("--list")?;
    Ok(BlockChanges {
        name: if editing { options.value("--name")? } else { None },
        lists: (!lists.is_empty()).then_some(lists),
        condition: options.value("--condition")?.map(|c| (!c.eq_ignore_ascii_case("always")).then_some(c)),
        allowance: parse_allowance_changes(options)?,
        lock: parse_lock(options)?,
    })
}

/// `--allowance`, `--allowed`, `--scope`, `--reset`, `--blocked` and `--anchor` of a block; `None` when
/// none of them is given. The words are checked here; whether they make terms is `build_terms`'s to
/// say when the command runs.
fn parse_allowance_changes(options: &mut Options) -> Result<Option<AllowanceChanges>, String> {
    let kind = options.value("--allowance")?.map(|kind| kind.to_lowercase());
    if let Some(kind) = &kind {
        if !["budget", "pomodoro", "none"].contains(&kind.as_str()) {
            return Err(format!("--allowance takes budget, pomodoro or none, not \"{kind}\""));
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
    let changes = AllowanceChanges { kind, allowed, scope, reset, every, blocked, anchor };
    if changes == AllowanceChanges::default() {
        return Ok(None);
    }
    let terms_given = changes.allowed.is_some()
        || changes.scope.is_some()
        || changes.reset.is_some()
        || changes.blocked.is_some()
        || changes.anchor.is_some();
    if changes.kind.as_deref() == Some("none") && terms_given {
        return Err("--allowance none takes no other allowance options".into());
    }
    Ok(Some(changes))
}

/// An allowance's terms from what was given, over the ones it has (`old`, when editing): what is not
/// named stays, and a new one has defaults (the whole block, never reset, counted from the start).
/// `--blocked` makes a Pomodoro, `--scope` or `--reset` a budget, and `--allowed` or `--anchor` alone
/// keep the kind there is. Checked, so a mistake is told before anything is sent.
fn build_terms(changes: AllowanceChanges, old: Option<&Terms>) -> Result<Terms, String> {
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

/// `--times <times>`, `--drive <drive>`, `--any <condition>...` or `--all <condition>...` of a
/// condition, at most one kind.
fn parse_rule(options: &mut Options) -> Result<Option<RuleChoice>, String> {
    let mut given: Vec<RuleChoice> = Vec::new();
    given.extend(options.value("--times")?.map(RuleChoice::Times));
    given.extend(options.value("--drive")?.map(RuleChoice::Drive));
    given.extend(parse_webhook(options)?.map(RuleChoice::Webhook));
    let any = options.values("--any")?;
    let all = options.values("--all")?;
    if !any.is_empty() {
        given.push(RuleChoice::Any(any));
    }
    if !all.is_empty() {
        given.push(RuleChoice::All(all));
    }
    match given.len() {
        0 | 1 => Ok(given.pop()),
        _ => Err("Give one of --times, --drive, --webhook, --any or --all".into()),
    }
}

/// `--webhook <url>` with `--method get|post`, `--every <time>` (at least 1m, default 5m), `--fallback
/// active|inactive` (default active), and what each answer means: `--true-status` / `--true-body` and
/// `--false-status` / `--false-body`, each matcher by what is given of it (status, body or both).
fn parse_webhook(options: &mut Options) -> Result<Option<Hook>, String> {
    let Some(url) = options.value("--webhook")? else { return Ok(None) };
    let method = match options.value("--method")?.map(|m| m.to_lowercase()).as_deref() {
        None | Some("get") => Method::Get,
        Some("post") => Method::Post,
        Some(other) => return Err(format!("--method is get or post, not \"{other}\"")),
    };
    let interval = match options.value("--every")? {
        Some(text) => duration::parse(&text)?.ok_or("Give the time between requests, e.g. 5m")?.as_secs(),
        None => 300,
    };
    let fallback = match options.value("--fallback")?.map(|f| f.to_lowercase()).as_deref() {
        None | Some("active") => true,
        Some("inactive") => false,
        Some(other) => return Err(format!("--fallback is active or inactive, not \"{other}\"")),
    };
    let mut matcher = |what: &str| -> Result<Matcher, String> {
        let status = options.value(&format!("--{what}-status"))?;
        let body = options.value(&format!("--{what}-body"))?;
        let status = status.map(|s| s.trim().parse::<u16>().map_err(|_| format!("--{what}-status is a number, e.g. 200"))).transpose()?;
        let on = match (status.is_some(), body.is_some()) {
            (true, true) => Match::Both,
            (true, false) => Match::Status,
            (false, true) => Match::Body,
            (false, false) => {
                return Err(format!("Give --{what}-status and/or --{what}-body: what an answer must be for the condition to be {what}"))
            }
        };
        Ok(Matcher { on, status, body })
    };
    let when_true = matcher("true")?;
    let when_false = matcher("false")?;
    let hook = Hook { url, method, interval, fallback, when_true, when_false };
    hook.check()?;
    Ok(Some(hook))
}

/// The options of `list new` and `list edit`. Only `list edit` takes `--name` and `--clear`.
fn parse_list_changes(options: &mut Options, editing: bool) -> Result<ListChanges, String> {
    let mut changes = ListChanges::default();
    let cleared = if editing { options.values("--clear")? } else { Vec::new() };
    let names: Vec<&str> = LIST_OPTIONS.iter().map(|(_, _, name, _)| *name).collect();
    for list in &cleared {
        if !names.contains(&list.as_str()) {
            return Err(format!("--clear takes {}, not \"{list}\"", names.join(", ")));
        }
    }
    for (option, file, cleared_as, key) in LIST_OPTIONS {
        let mut entries = options.values(option)?;
        for path in options.values(file)? {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {path}: {e}"))?;
            entries.extend(text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string));
        }
        if !entries.is_empty() || cleared.iter().any(|c| c == cleared_as) {
            changes.lists.insert(key, entries);
        }
    }
    if editing {
        changes.name = options.value("--name")?;
    }
    changes.enforcers = match options.value("--enforcers")? {
        Some(list) => Some(parse_enforcers(&list)?),
        None if editing => None,
        None => Some(parse_enforcers("all")?),
    };
    Ok(changes)
}

/// The names of the enforcers, as `Enforcers` calls its fields: a new enforcer is known here
/// without a change.
fn enforcer_names() -> Vec<String> {
    match serde_json::to_value(Enforcers::default()) {
        Ok(Value::Object(fields)) => fields.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// `chromium_tabs,firefox_tabs`, or `all`: every enforcer as a new block list has them (windows by
/// title closed, not minimized: `window_titles_minimize` says to minimize them).
fn parse_enforcers(list: &str) -> Result<Enforcers, String> {
    let names = enforcer_names();
    let mut chosen: Map<String, Value> = names.iter().map(|name| (name.clone(), Value::Bool(false))).collect();
    for name in list.split(',').map(str::trim).filter(|name| !name.is_empty()) {
        if name == "all" {
            if let Ok(Value::Object(all)) = serde_json::to_value(Enforcers::default()) {
                for (name, on) in all.into_iter().filter(|(_, on)| *on == Value::Bool(true)) {
                    chosen.insert(name, on);
                }
            }
        } else if let Some(on) = chosen.get_mut(name) {
            *on = Value::Bool(true);
        } else {
            return Err(format!("Unknown enforcer \"{name}\"; the enforcers are {}", names.join(", ")));
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
        Command::Help => say!("{}", HELP.replace("{enforcers}", &enforcer_names().join(", "))),
        Command::Version => say!("bullseye-cli {}", env!("CARGO_PKG_VERSION")),
        Command::Status { json } => {
            let overview = overview()?;
            if json {
                say!("{}", serde_json::to_string_pretty(&overview).map_err(|e| failed(e.to_string()))?);
            } else {
                out(&describe_overview(&overview));
            }
        }
        Command::Show { block, json } => {
            let overview = overview()?;
            let block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?;
            let running = overview.running.iter().find(|r| r.id == block.id);
            if json {
                let both = serde_json::json!({ "block": block, "running": running });
                say!("{}", serde_json::to_string_pretty(&both).map_err(|e| failed(e.to_string()))?);
            } else {
                out(&describe_block(&overview, block, running));
            }
        }
        Command::New { name, changes } => {
            let overview = overview()?;
            let mut block = Definition {
                id: new_id(),
                name,
                lists: Vec::new(),
                condition: None,
                allowance: None,
                lock: Lock::None,
                kept_locks: Vec::new(),
            };
            apply_block_changes(&overview, &mut block, changes)?;
            let (id, name) = (block.id.clone(), block.name.clone());
            send(Request::SaveBlock { block })?;
            say!("Made the block \"{name}\" (id {id}). Start it with: bullseye-cli start \"{name}\"");
        }
        Command::Edit { block, changes } => {
            let overview = overview()?;
            let mut block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?.clone();
            apply_block_changes(&overview, &mut block, changes)?;
            let name = block.name.clone();
            send(Request::SaveBlock { block })?;
            say!("Saved the block \"{name}\".");
        }
        Command::Delete { block, yes } => {
            let overview = overview()?;
            let block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?;
            if !confirm(&format!("Delete the block \"{}\"? This cannot be undone.", block.name), yes)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteBlock { id: block.id.clone() })?;
            say!("Deleted the block \"{}\".", block.name);
        }
        Command::Start { block, lock, yes } => {
            let overview = overview()?;
            let mut block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?.clone();
            // As the lock on the block's tile: set on the block, then started with it.
            let lock = lock.map(|lock| resolve_lock(&overview, lock)).transpose()?;
            if let Some(lock) = lock.filter(|lock| *lock != block.lock) {
                block.lock = lock;
                send(Request::SaveBlock { block: block.clone() })?;
            }
            // As in the window: a block that cannot be stopped is only started after a second yes.
            if let Some(how) = lock_warning(&overview, &block) {
                let question = format!("Start \"{}\"? {how} Only unlocking ends it early.", block.name);
                if !confirm(&question, yes)? {
                    return Err(failed("Not started."));
                }
            }
            send(Request::Start { id: block.id.clone() })?;
            say!("Started \"{}\".", block.name);
        }
        Command::Stop { block, password, abort } => {
            let overview = overview()?;
            let block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?;
            // A block locked with a delay: stopping is asked for, waited out, and then done.
            if let Some(run) = overview.running.iter().find(|r| r.id == block.id && r.stop_delay.is_some()) {
                if abort {
                    send(Request::AbortStop { id: block.id.clone() })?;
                    say!("Stopping \"{}\" aborted: back to locked.", block.name);
                    return Ok(());
                }
                match (run.stop_remaining, &run.stop_remaining_words) {
                    (Some(0), _) => {}
                    (Some(_), Some(left)) => {
                        say!(
                            "\"{}\" can be stopped in {left}. Run `bullseye-cli stop` again then, or `stop --abort` to end the wait.",
                            block.name
                        );
                        return Ok(());
                    }
                    _ => {
                        send(Request::AskToStop { id: block.id.clone() })?;
                        say!(
                            "Stopping \"{}\" was asked for: ready in {}. Run `bullseye-cli stop` again then, or `stop --abort` to end the wait.",
                            block.name,
                            run.stop_delay_words.clone().unwrap_or_default()
                        );
                        return Ok(());
                    }
                }
            }
            // A block locked with a text stops in the window, by typing the text: typing it is the
            // point, and a command line cannot stop a paste.
            if overview.running.iter().any(|r| r.id == block.id && r.lock_text.is_some()) {
                return Err(failed(format!("\"{}\" is locked until its text is typed in: stop it in the window", block.name)));
            }
            // A block locked with a password stops with it: asked for when it was not given.
            let password_locked = overview.running.iter().any(|r| r.id == block.id && r.password);
            let password = match password {
                None if password_locked => Some(read_password(&format!("Password of \"{}\": ", block.name)).map_err(Failure::Failed)?),
                given => given,
            };
            send(Request::Stop { id: block.id.clone(), password, text: None })?;
            say!("Stopped \"{}\".", block.name);
        }
        Command::ListShow { list, json } => {
            let overview = overview()?;
            let list = find(&overview.lists, &list, "block list", |l| (&l.id, &l.name))?;
            if json {
                say!("{}", serde_json::to_string_pretty(list).map_err(|e| failed(e.to_string()))?);
            } else {
                out(&describe_list(&overview, list));
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
        Command::ListApps { json } => {
            let apps = crate::store_apps::installed().map_err(Failure::Failed)?;
            if json {
                say!("{}", serde_json::to_string_pretty(&apps).map_err(|e| failed(e.to_string()))?);
            } else {
                for app in apps {
                    say!("{:<40} {}", app.name, app.family);
                }
            }
        }
        Command::ListNew { name, changes } => {
            let mut list = BlockList { id: new_id(), name, entries: Entries::default(), enforcers: Enforcers::default() };
            changes.apply(&mut list);
            let name = list.name.clone();
            send(Request::SaveList { list })?;
            say!("Made the block list \"{name}\". Use it with: bullseye-cli new <block> --list \"{name}\"");
        }
        Command::ListEdit { list, changes } => {
            let overview = overview()?;
            let mut list = find(&overview.lists, &list, "block list", |l| (&l.id, &l.name))?.clone();
            changes.apply(&mut list);
            let name = list.name.clone();
            send(Request::SaveList { list })?;
            say!("Saved the block list \"{name}\".");
        }
        Command::ListDelete { list, yes } => {
            let overview = overview()?;
            let list = find(&overview.lists, &list, "block list", |l| (&l.id, &l.name))?;
            if !confirm(&format!("Delete the block list \"{}\"?{} This cannot be undone.", list.name, fallout(&overview, &list.id)), yes)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteList { id: list.id.clone() })?;
            say!("Deleted the block list \"{}\".", list.name);
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
            say!("Made the condition \"{name}\". Use it with: bullseye-cli new <block> --condition \"{name}\"");
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
        Command::ConditionDelete { condition, yes } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            if !confirm(
                &format!("Delete the condition \"{}\"?{} This cannot be undone.", condition.name, fallout(&overview, &condition.id)),
                yes,
            )? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteCondition { id: condition.id.clone() })?;
            say!("Deleted the condition \"{}\".", condition.name);
        }
        Command::ConditionCheck { times } => match Week::parse(&times).map_err(Failure::Usage)? {
            Some(week) => out(&per_day(&week)),
            None => say!("Always active: a block without a condition"),
        },
        Command::Export { file } => {
            definitions::write_settings(std::path::Path::new(&file), remote().library().map_err(failed)?).map_err(failed)?;
            say!("Wrote every block list, condition and block to {file}.");
        }
        Command::Import { file, yes } => {
            let library = definitions::read_settings(std::path::Path::new(&file)).map_err(failed)?;
            let contents = library.contents();
            // A Store app this computer has not installed is left out, as the window does.
            let installed = crate::store_apps::installed_families();
            let checked = installed.is_some() || !library.has_store_apps();
            let (library, left_out) = library.without_missing_apps(installed.as_deref());
            let count = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
            let mut question = format!(
                "Replace every block list, condition and block with the {}, {} and {} in {file}? This cannot be undone.",
                count(contents.lists, "block list"),
                count(contents.conditions, "condition"),
                count(contents.blocks, "block"),
            );
            if !contents.unlocked.is_empty() {
                let names = contents.unlocked.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
                question.push_str(&format!(" {names} had a password lock, which is not in the file: no lock once imported."));
            }
            let apps = left_out_words(&left_out, checked);
            if !apps.is_empty() {
                question.push_str(&format!(" {apps}"));
            }
            if !confirm(&question, yes)? {
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
                    Kind::Block => overview.blocks.iter().map(|b| (b.id.clone(), b.name.clone())).collect(),
                    Kind::List => overview.lists.iter().map(|l| (l.id.clone(), l.name.clone())).collect(),
                    Kind::Condition => overview.conditions.iter().map(|c| (c.id.clone(), c.name.clone())).collect(),
                }
            };
            let noun = match kind {
                Kind::Block => "block",
                Kind::List => "block list",
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
                say!("Unlocked: every block has ended.");
            } else if status.remaining == Some(0) || status.way_open {
                send(Request::Unlock)?;
                // The service tells the app to quit at its next policy request, within a second.
                say!("Unlocked: every block has ended.");
            } else if let Some(left) = status.remaining_words {
                say!("Unlocking is not ready yet: {left} to go. Run `bullseye-cli unlock` again then, or `bullseye-cli unlock --abort` to stop.");
            } else {
                send(Request::AskForUnlock)?;
                say!(
                    "Unlocking was asked for: ready in {}. Run `bullseye-cli unlock` again then, or `bullseye-cli unlock --abort` to stop.",
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
    let apps: Vec<String> = left_out.iter().map(|app| format!("\"{}\" in \"{}\" ({})", app.name, app.list, app.family)).collect();
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

/// Weekly times, as a schedule takes them; "always" is not a schedule (a block without a condition is
/// always active).
fn times_of(text: &str) -> Result<Week, Failure> {
    match Week::parse(text).map_err(failed)? {
        Some(week) => Ok(week),
        None => Err(failed("Give the times, e.g. \"Mon-Fri 08:00-17:00\"; a block that is always active needs no condition")),
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
                [] => Err(failed(format!("No flash drive \"{given}\" is connected; `bullseye-cli drives` lists the ones that are"))),
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

/// What a block's lock says of its condition besides its name: nothing for a schedule (the times are
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
    let matcher = |m: &Matcher| match (m.on, m.status, &m.body) {
        (Match::Status, Some(status), _) => format!("HTTP {status}"),
        (Match::Body, _, Some(body)) => format!("body \"{body}\""),
        (_, Some(status), Some(body)) => format!("HTTP {status} and body \"{body}\""),
        _ => "(not set)".into(),
    };
    format!(
        "{} {} every {}: active on {}, not active on {}, otherwise {}",
        if hook.method == Method::Post { "POST" } else { "GET" },
        hook.url,
        duration::countdown(hook.interval),
        matcher(&hook.when_true),
        matcher(&hook.when_false),
        if hook.fallback { "active" } else { "not active" }
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
/// allowance's terms over the ones the block has.
fn apply_block_changes(overview: &Overview, block: &mut Definition, changes: BlockChanges) -> Result<(), Failure> {
    if let Some(name) = changes.name {
        block.name = name;
    }
    if let Some(lists) = changes.lists {
        block.lists = lists
            .iter()
            .map(|l| find(&overview.lists, l, "block list", |l| (&l.id, &l.name)).map(|l| l.id.clone()))
            .collect::<Result<_, _>>()?;
    }
    if let Some(condition) = changes.condition {
        block.condition = match condition {
            Some(c) => Some(find(&overview.conditions, &c, "condition", |c| (&c.id, &c.name))?.id.clone()),
            None => None,
        };
    }
    if let Some(allowance) = changes.allowance {
        block.allowance = match allowance.kind.as_deref() {
            Some("none") => None,
            _ => Some(build_terms(allowance, block.allowance.as_ref()).map_err(failed)?),
        };
    }
    if let Some(lock) = changes.lock {
        block.lock = resolve_lock(overview, lock)?;
    }
    Ok(())
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

/// What the start of a locked block asks about, as the window's confirmation; `None`: not locked.
fn lock_warning(overview: &Overview, block: &Definition) -> Option<String> {
    match &block.lock {
        Lock::None => None,
        Lock::Duration { .. } => {
            let words = duration::countdown(block.lock.duration().ok()??.as_secs());
            Some(format!("It cannot be stopped, changed or deleted for {words}."))
        }
        Lock::Until { until } => match localtime::until(until) {
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
            let words = duration::countdown(block.lock.delay().ok()??.as_secs());
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
        [] => Err(failed(format!("No {kind} called \"{wanted}\"; `bullseye-cli status` lists them"))),
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

/// Asks a yes/no question in the terminal; `yes` (from `--yes`) answers it beforehand. Without a
/// terminal there is nobody to ask, so `--yes` is needed.
fn confirm(question: &str, yes: bool) -> Result<bool, Failure> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(failed(format!("{question}\nNo terminal to ask in; add --yes to go ahead.")));
    }
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).map_err(|e| failed(e.to_string()))?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// A password, typed after `prompt` without showing it; from standard input as it is when that is
/// not a terminal (`echo pw | bullseye-cli unlock`).
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

/// The state of a block in one line, as its tile shows it.
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

/// What a running block's allowance is doing, in a few words: a cycle's phase and how long it lasts,
/// or what a budget has left.
fn allowance_words(status: &AllowanceStatus) -> String {
    match &status.terms {
        Terms::Pomodoro { .. } => match (status.open, &status.change_words) {
            (true, Some(change)) => format!("allowed for the next {change}"),
            (false, Some(change)) => format!("blocked for the next {change}"),
            (true, None) => "allowed".into(),
            (false, None) => "blocked".into(),
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

/// The names of the blocks that use a block list or condition (`Library::users_of`).
fn users(overview: &Overview, id: &str) -> String {
    let library = Library { blocks: overview.blocks.clone(), ..Library::default() };
    let names: Vec<&str> = library.users_of(id).iter().map(|b| b.name.as_str()).collect();
    if names.is_empty() {
        "(no block)".into()
    } else {
        names.join(", ")
    }
}

/// What deleting a block list or condition does to the blocks that use it, in sentences each with a
/// space in front, as the window's question says it; empty when no block uses it.
fn fallout(overview: &Overview, id: &str) -> String {
    let library = Library { blocks: overview.blocks.clone(), conditions: overview.conditions.clone(), ..Library::default() };
    let fallout = library.fallout(id);
    let quoted = |names: &[String]| names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
    let mut text = String::new();
    for (names, what) in [
        (&fallout.deleted, "is deleted too, as it has no other block list"),
        (&fallout.without_it, "keeps its other block lists"),
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

/// Everything, as the window's pages show it: the blocks, the block lists, the conditions.
fn describe_overview(overview: &Overview) -> String {
    let blocks: Vec<(String, String)> =
        overview.blocks.iter().map(|b| (b.name.clone(), state(overview.running.iter().find(|r| r.id == b.id)))).collect();
    let lists: Vec<(String, String)> =
        overview.lists.iter().map(|l| (l.name.clone(), format!("used by {}", users(overview, &l.id)))).collect();
    let conditions: Vec<(String, String)> =
        overview.conditions.iter().map(|c| (c.name.clone(), describe_rule(overview, &c.rule))).collect();
    let mut text = String::new();
    for (title, rows, empty) in [
        ("Blocks", blocks, "  None yet. Make one with: bullseye-cli new <name> --list <block list>\n"),
        ("Block lists", lists, "  None yet. Make one with: bullseye-cli list new <name> --site <site>\n"),
        ("Conditions", conditions, "  None (blocks without one are always active).\n"),
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

fn enforcers_on(enforcers: Enforcers) -> Vec<String> {
    match serde_json::to_value(enforcers) {
        Ok(Value::Object(fields)) => fields.iter().filter(|(_, on)| **on == Value::Bool(true)).map(|(name, _)| name.clone()).collect(),
        _ => Vec::new(),
    }
}

/// One block, everything about it.
fn describe_block(overview: &Overview, block: &Definition, running: Option<&Running>) -> String {
    let lists: Vec<&str> =
        block.lists.iter().map(|l| overview.lists.iter().find(|n| &n.id == l).map(|n| n.name.as_str()).unwrap_or("(gone)")).collect();
    let condition = match &block.condition {
        None => "always active".to_string(),
        Some(id) => match overview.conditions.iter().find(|c| &c.id == id) {
            Some(c) => c.name.clone(),
            None => "(gone)".into(),
        },
    };
    let allowance = block.allowance.as_ref().map_or("none".to_string(), Terms::describe);
    let lock = match &block.lock {
        Lock::None => "none: can be stopped at any time".to_string(),
        Lock::Duration { duration } => match block.lock.duration() {
            Ok(Some(d)) => format!("duration, {} (cannot be stopped for that long once started)", duration::countdown(d.as_secs())),
            _ => format!("duration, {duration}"),
        },
        Lock::Until { until } => match block.lock.until() {
            Ok(Some(at)) => format!("until {at} (cannot be stopped until then once started)"),
            _ => format!("until {until}"),
        },
        Lock::UntilRestart => "until restart: cannot be stopped until Windows restarts, and ends then".into(),
        Lock::Condition { condition } => match overview.conditions.iter().find(|c| &c.id == condition) {
            Some(c) => format!("condition: cannot be stopped while \"{}\" is active{}", c.name, drive_detail(&c.rule)),
            None => "condition: (condition gone)".into(),
        },
        Lock::Delay { delay } => match block.lock.delay() {
            Ok(Some(d)) => format!("delay, {} (stopping is asked for, then waited out)", duration::countdown(d.as_secs())),
            _ => format!("delay, {delay}"),
        },
        Lock::Text { .. } => "text: can only be stopped by typing its text in, in the window".into(),
        Lock::Password { .. } => "password: can only be stopped with its password".into(),
    };
    format!(
        "Name:        {}\nId:          {}\nState:       {}\nBlock lists: {}\nCondition:   {condition}\nAllowance:   {allowance}\nLock:        {lock}\n",
        block.name,
        block.id,
        state(running),
        entries(&lists.iter().map(|l| l.to_string()).collect::<Vec<_>>()),
    )
}

/// One block list, everything about it.
fn describe_list(overview: &Overview, list: &BlockList) -> String {
    format!(
        "Name:           {}\nId:             {}\nBlock list:     {}\nAlways allowed: {}\nPrograms:       {}\nStore apps:     {}\nWindow titles:  {}\nAllowed titles: {}\nEnforcers:      {}\nUsed by:        {}\n",
        list.name,
        list.id,
        entries(&list.entries.filters),
        entries(&list.entries.allowed),
        entries(&list.entries.programs),
        entries(&list.entries.store_apps),
        entries(&list.entries.titles),
        entries(&list.entries.allowed_titles),
        entries(&enforcers_on(list.enforcers)),
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
    use crate::block::{Blocklist, RunningList};
    use crate::definitions::tests::{block, block_list, weekly};
    use crate::definitions::Definitions;
    use crate::handler::{handle, Context};
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
        let blocklist = Blocklist::with_store(Store::new(dir.join("state.json")));
        let context = Arc::new(Context::new(blocklist, Definitions::at(Some(dir.join("blocks.json"))), Arc::new(AppSlot::new(false))));
        REMOTE.with(|remote| *remote.borrow_mut() = Some(Remote::local(Arc::new(move |request| handle(&context, 0, request).0))));
        dir
    }

    /// A command line run as `bullseye-cli` runs it: what it says, or why it did not work.
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
    fn blocks_lists_and_conditions_are_made_shown_changed_and_deleted() {
        serve("lifecycle");
        assert!(ok(&["status"]).contains("Blocks:\n  None yet"));
        assert_eq!(
            ok(&["list", "new", "Social", "--site", "youtube.com", "--allow", "music.youtube.com"]),
            "Made the block list \"Social\". Use it with: bullseye-cli new <block> --list \"Social\"\n"
        );
        assert!(ok(&["condition", "new", "Work", "--times", "Mon-Fri 08:00-17:00"]).starts_with("Made the condition \"Work\"."));
        let made = ok(&["new", "Evening", "--list", "Social", "--condition", "Work", "--allowance", "budget", "--allowed", "30m"]);
        assert!(
            made.starts_with("Made the block \"Evening\" (id cli-") && made.ends_with("Start it with: bullseye-cli start \"Evening\"\n"),
            "{made}"
        );

        // Shown as the pages show them, and as JSON for scripts.
        let shown = ok(&["show", "evening"]);
        assert!(shown.contains("Block lists: Social\nCondition:   Work\nAllowance:   30 minutes in total, never reset\n"), "{shown}");
        let json: Value = serde_json::from_str(&ok(&["show", "Evening", "--json"])).unwrap();
        assert_eq!(json["block"]["allowance"]["type"], "budget");
        assert!(json["running"].is_null());
        let status = ok(&["status"]);
        assert!(
            status.contains("  Evening  Not running")
                && status.contains("  Social  used by Evening")
                && status.contains("  Work  Mon-Fri 08:00-17:00")
        );
        let json: Value = serde_json::from_str(&ok(&["status", "--json"])).unwrap();
        assert_eq!(json["blocks"].as_array().map(Vec::len), Some(1));
        assert!(ok(&["list", "show", "Social"]).contains("Always allowed: music.youtube.com"));
        assert!(ok(&["list", "show", "Social"]).contains("Allowed titles: (none)"));
        let json: Value = serde_json::from_str(&ok(&["list", "show", "Social", "--json"])).unwrap();
        assert_eq!(json["filters"][0], "youtube.com");
        let condition = ok(&["condition", "show", "Work"]);
        assert!(condition.starts_with("Work\nMon-Fri 08:00-17:00\n") && condition.ends_with("Used by: Evening\n"), "{condition}");
        assert!(ok(&["condition", "check", "Mon 8-9"]).starts_with("Mon 08:00-09:00\n"));
        assert_eq!(ok(&["condition", "check", "always"]), "Always active: a block without a condition\n");
        assert!(cli(&["condition", "check", "Mon 25-26"]).is_err());

        // Changed, copied.
        assert_eq!(ok(&["edit", "Evening", "--name", "Night"]), "Saved the block \"Night\".\n");
        assert_eq!(ok(&["duplicate", "Night"]), "Made \"Night (2)\", a copy of the block \"Night\".\n");
        assert_eq!(ok(&["list", "duplicate", "Social"]), "Made \"Social (2)\", a copy of the block list \"Social\".\n");
        assert_eq!(ok(&["condition", "duplicate", "Work"]), "Made \"Work (2)\", a copy of the condition \"Work\".\n");
        assert_eq!(
            ok(&["list", "edit", "Social (2)", "--site", "reddit.com", "--clear", "allowed"]),
            "Saved the block list \"Social (2)\".\n"
        );
        let copy = ok(&["list", "show", "Social (2)"]);
        assert!(copy.contains("reddit.com") && !copy.contains("music.youtube.com"), "{copy}");
        assert_eq!(ok(&["condition", "edit", "Work (2)", "--name", "Weekend", "--times", "Sat-Sun"]), "Saved the condition \"Weekend\".\n");
        ok(&["condition", "new", "Either", "--any", "Work", "--any", "Weekend"]);
        assert!(ok(&["condition", "show", "Either"]).contains("any of \"Work\", \"Weekend\""));
        assert!(refused(&["condition", "new", "Pair", "--any", "Work"]).contains("at least two"));

        // Started and stopped.
        assert_eq!(ok(&["start", "Night"]), "Started \"Night\".\n");
        assert!(ok(&["status"]).contains("  Night      Running"), "{}", ok(&["status"]));
        assert_eq!(ok(&["stop", "Night"]), "Stopped \"Night\".\n");

        // Deleted.
        assert_eq!(ok(&["delete", "Night (2)", "--yes"]), "Deleted the block \"Night (2)\".\n");
        assert_eq!(ok(&["list", "delete", "Social (2)", "--yes"]), "Deleted the block list \"Social (2)\".\n");
        assert_eq!(ok(&["condition", "delete", "Either", "--yes"]), "Deleted the condition \"Either\".\n");

        // What is not there, or not of the kind.
        assert!(refused(&["show", "Nope"]).starts_with("No block called \"Nope\""));
        assert!(refused(&["new", "X", "--list", "Nope"]).starts_with("No block list called \"Nope\""));
        assert!(refused(&["list", "new", "Games", "--store-app", "Microsoft.Xbox*"]).contains("exact package family name"));
        assert!(refused(&["new", "X", "--condition", "Nope"]).starts_with("No condition called \"Nope\""));
        assert_eq!(refused(&["condition", "test", "Work"]), "\"Work\" is not a webhook condition");
    }

    #[test]
    fn a_webhook_condition_is_shown_and_tested() {
        serve("webhook");
        let url = "http://127.0.0.1:9/state";
        ok(&["condition", "new", "Home", "--webhook", url, "--true-status", "200", "--false-body", "away", "--fallback", "inactive"]);
        let shown = ok(&["condition", "show", "Home"]);
        assert!(
            shown.contains(
                "GET http://127.0.0.1:9/state every 5 minutes: active on HTTP 200, not active on body \"away\", otherwise not active"
            ),
            "{shown}"
        );
        // Nothing answers there.
        assert!(cli(&["condition", "test", "Home"]).is_err());
    }

    #[test]
    fn locks_are_set_shown_and_held_by_the_service() {
        serve("locks");
        ok(&["list", "new", "L", "--site", "a.com"]);
        ok(&["condition", "new", "Always", "--times", "Mon-Sun 00:00-24:00"]);
        let new = |name: &str, lock: &[&str]| ok(&[&["new", name, "--list", "L"][..], lock].concat());
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
            assert_eq!(ok(&["start", name, "--yes"]), format!("Started \"{name}\".\n"));
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
        ok(&["new", "Pw", "--list", "L"]);
        assert_eq!(ok(&["start", "Pw", "--lock-password", "secret", "--yes"]), "Started \"Pw\".\n");
        assert!(ok(&["status"]).contains("Pw       Running · active · locked with a password"));
        assert!(cli(&["stop", "Pw", "--password", "wrong"]).is_err());
        assert_eq!(ok(&["stop", "Pw", "--password", "secret"]), "Stopped \"Pw\".\n");
        // Without a delay, unlocking is free and ends them all.
        assert_eq!(ok(&["unlock"]), "Unlocked: every block has ended.\n");
        assert!(!ok(&["status"]).contains("Running"));
    }

    #[test]
    fn unlocking_counts_down_and_has_another_way() {
        serve("unlock");
        ok(&["list", "new", "L", "--site", "a.com"]);
        ok(&["new", "Timed", "--list", "L", "--lock-duration", "1h"]);
        assert_eq!(ok(&["unlock-delay"]), "Unlocking takes no time: it is free.\n");
        assert_eq!(ok(&["unlock-delay", "1h"]), "Unlocking now takes 1 hour.\n");
        assert_eq!(ok(&["unlock-delay"]), "Unlocking takes 1 hour.\n");
        ok(&["start", "Timed", "--yes"]);
        assert!(ok(&["unlock"]).starts_with("Unlocking was asked for: ready in 1 hour."));
        assert!(ok(&["unlock"]).starts_with("Unlocking is not ready yet: "));
        assert_eq!(ok(&["unlock", "--abort"]), "Unlocking aborted: back to locked.\n");
        assert_eq!(ok(&["unlock-way"]), "Only the delay unlocks.\n");
        // Locked and not ready: neither a shorter delay nor another way.
        assert!(cli(&["unlock-delay", "0"]).is_err());
        assert!(cli(&["unlock-way", "--text", "let me out"]).is_err());

        serve("unlock-way");
        ok(&["list", "new", "L", "--site", "a.com"]);
        ok(&["condition", "new", "Always", "--times", "Mon-Sun 00:00-24:00"]);
        ok(&["new", "Timed", "--list", "L", "--lock-duration", "1h"]);
        assert_eq!(ok(&["unlock-way", "--text", "let me out"]), "Unlocking is also possible by typing a text in, in the window.\n");
        assert_eq!(ok(&["unlock-way", "--condition", "Always"]), "Unlocking is locked while its condition is active\n");
        assert_eq!(
            ok(&["unlock-way", "--password", "secret"]),
            "Unlocking is also possible at once with a password (`unlock --password`).\n"
        );
        ok(&["unlock-delay", "1h"]);
        ok(&["start", "Timed", "--yes"]);
        assert!(cli(&["unlock", "--password", "wrong"]).is_err());
        assert_eq!(ok(&["unlock", "--password", "secret"]), "Unlocked: every block has ended.\n");
        assert_eq!(ok(&["unlock-way", "--none"]), "Only the delay unlocks.\n");
        assert_eq!(ok(&["unlock-delay", "0"]), "Unlocking is free again.\n");
    }

    #[test]
    fn everything_moves_through_a_settings_file() {
        let dir = serve("export");
        let file = dir.join("bullseye.json");
        let path = file.to_string_lossy().to_string();
        ok(&["list", "new", "L", "--site", "a.com"]);
        ok(&["new", "B", "--list", "L", "--lock-password", "secret", "--allowance", "pomodoro", "--blocked", "50m", "--allowed", "10m"]);
        assert_eq!(ok(&["export", &path]), format!("Wrote every block list, condition and block to {path}.\n"));
        serve("import");
        // (The file has no Store apps, so none are left out and nothing is said of them.)
        assert_eq!(ok(&["import", &path, "--yes"]), format!("Imported {path}.\n"));
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
            "--true-status",
            "200",
            "--true-body",
            "on",
            "--false-status",
            "503",
            "--fallback",
            "active",
        ];
        let Ok(Command::ConditionNew { rule: RuleChoice::Webhook(hook), .. }) = parse(&args(&line)) else { panic!("not a webhook") };
        assert_eq!((hook.method, hook.interval, hook.fallback), (Method::Post, 600, true));
        assert_eq!(hook.when_true, Matcher { on: Match::Both, status: Some(200), body: Some("on".into()) });
        assert_eq!(hook.when_false, Matcher { on: Match::Status, status: Some(503), body: None });
        let base = ["condition", "new", "H", "--webhook", "https://x.io/a", "--true-status", "200", "--false-status", "404"];
        for (extra, why) in [
            (&["--method", "put"][..], "--method is get or post"),
            (&["--fallback", "maybe"], "--fallback is active or inactive"),
            (&["--every", "soon"], ""),
            (&["--every", "30s"], "at most once a minute"),
        ] {
            let error = parse(&args(&[&base[..], extra].concat())).err().unwrap_or_else(|| panic!("{extra:?} worked"));
            assert!(error.contains(why), "{extra:?}: {error}");
        }
        assert!(parse(&args(&base[..base.len() - 2])).unwrap_err().starts_with("Give --false-status and/or --false-body"));
        assert!(parse(&args(&["condition", "new", "H", "--webhook", "https://x.io/a", "--true-status", "OK"]))
            .unwrap_err()
            .contains("is a number"));
        // An option given twice is a mistake, not a choice.
        assert!(parse(&args(&["new", "X", "--condition", "A", "--condition", "B"])).unwrap_err().contains("more than once"));
    }

    #[test]
    fn a_combination_of_all_and_a_list_renamed() {
        serve("all");
        ok(&["list", "new", "L", "--site", "a.com"]);
        assert_eq!(ok(&["list", "edit", "L", "--name", "Media"]), "Saved the block list \"Media\".\n");
        ok(&["condition", "new", "Work", "--times", "Mon-Fri 08:00-17:00"]);
        ok(&["condition", "new", "Late", "--times", "Mon-Sun 16:00-18:00"]);
        ok(&["condition", "new", "Both", "--all", "Work", "--all", "Late"]);
        assert!(ok(&["condition", "show", "Both"]).contains("all of \"Work\", \"Late\""));
        // A lock until a date and time asks first, with the time it ends.
        ok(&["new", "Later", "--list", "Media", "--lock-until", "2099-01-01 00:00"]);
        assert_eq!(ok(&["start", "Later", "--yes"]), "Started \"Later\".\n");
        assert!(ok(&["status"]).contains("Later  Running · active · locked for the next"));
    }

    #[test]
    fn a_file_with_a_password_lock_but_no_hash_imports_without_a_lock() {
        let dir = serve("import-unlocked");
        let file = dir.join("old.json");
        let mut locked = block("b", &["l"]);
        locked.lock = Lock::Password { password: None, hash: None };
        let library = Library { lists: vec![block_list("l", &["a.com"])], blocks: vec![locked], ..Library::default() };
        std::fs::create_dir_all(&dir).unwrap();
        definitions::write_settings(&file, library).unwrap();
        ok(&["import", &file.to_string_lossy(), "--yes"]);
        assert!(ok(&["show", "Block b"]).contains("Lock:        none"));
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
        assert_eq!(with(Lock::Duration { duration: "1h".into() }, false, None), "Only the delay unlocks.");
    }

    /// A running block as the status gives it, always active and not locked.
    fn a_running(id: &str) -> Running {
        Running {
            id: id.into(),
            name: id.into(),
            lists: vec![],
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
    fn a_running_block_reads_like_its_tile() {
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
    fn a_block_with_parts_gone_or_unreadable_is_still_shown() {
        let mut overview = library();
        let mut b = block("b2", &["gone"]);
        b.condition = Some("gone".into());
        let lock_of = |lock: Lock| {
            let shown = describe_block(&overview, &Definition { lock, ..b.clone() }, None);
            shown.lines().find(|l| l.starts_with("Lock:")).unwrap_or_default().to_string()
        };
        assert_eq!(lock_of(Lock::Duration { duration: "soon".into() }), "Lock:        duration, soon");
        assert_eq!(lock_of(Lock::Until { until: "someday".into() }), "Lock:        until someday");
        assert_eq!(lock_of(Lock::Delay { delay: "a while".into() }), "Lock:        delay, a while");
        assert_eq!(lock_of(Lock::Condition { condition: "gone".into() }), "Lock:        condition: (condition gone)");
        assert_eq!(lock_of(Lock::Password { password: None, hash: None }), "Lock:        password: can only be stopped with its password");
        let shown = describe_block(&overview, &b, None);
        assert!(shown.contains("Block lists: (gone)\nCondition:   (gone)\n"), "{shown}");
        // A flash drive, as a lock's condition and as a condition.
        let drive =
            Condition { id: "d".into(), name: "Key".into(), rule: Rule::FlashDrive { serial: "x".into(), drive: "Kingston".into() } };
        overview.conditions.push(drive.clone());
        let shown = describe_block(&overview, &Definition { lock: Lock::Condition { condition: "d".into() }, ..b.clone() }, None);
        assert!(shown.contains("cannot be stopped while \"Key\" is active (the flash drive \"Kingston\" is not connected)"), "{shown}");
        assert_eq!(describe_rule(&overview, &drive.rule), "while the flash drive \"Kingston\" is not connected");
        assert_eq!(
            lock_warning(&overview, &Definition { lock: Lock::Condition { condition: "d".into() }, ..b.clone() }).unwrap(),
            "It cannot be stopped while its lock condition \"Key\" is active (the flash drive \"Kingston\" is not connected)."
        );
        assert_eq!(lock_warning(&overview, &Definition { lock: Lock::Until { until: "2001-01-01 00:00".into() }, ..b.clone() }), None);
        // A combination with one of its conditions gone.
        let combined = Rule::Combined { mode: Mode::Any, members: vec!["s1".into(), "gone".into()], rules: vec![] };
        assert_eq!(describe_rule(&overview, &combined), "any of \"Work hours\", (gone)");
        // A webhook matcher not set (as saved by hand).
        let hook = Hook {
            url: "https://x.io".into(),
            method: Method::Get,
            interval: 60,
            fallback: true,
            when_true: Matcher { on: Match::Both, status: Some(200), body: Some("on".into()) },
            when_false: Matcher { on: Match::Status, status: None, body: None },
        };
        assert_eq!(
            describe_webhook(&hook),
            "GET https://x.io every 1 minute: active on HTTP 200 and body \"on\", not active on (not set), otherwise active"
        );
    }

    #[test]
    fn an_import_says_which_store_apps_it_leaves_out() {
        let claude =
            definitions::LeftOut { list: "Claude".into(), family: "AnthropicPBC.Claude_fnn82j28hfe8t".into(), name: "Claude".into() };
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
        assert!(help.starts_with("Bullseye command line") && help.contains("chromium_tabs") && !help.contains("{enforcers}"));
        assert_eq!(cli(&["version"]).unwrap(), format!("bullseye-cli {}\n", env!("CARGO_PKG_VERSION")));
    }

    fn library() -> Overview {
        let mut social = block_list("l1", &["youtube.com"]);
        social.name = "Social".into();
        let mut work = weekly("s1", "mon-fri 8-17");
        work.name = "Work hours".into();
        let mut b = block("b1", &["l1"]);
        b.name = "Work".into();
        b.condition = Some("s1".into());
        b.lock = Lock::Condition { condition: "s1".into() };
        Overview {
            lists: vec![social],
            conditions: vec![work],
            blocks: vec![b],
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
                assert!(doc.contains(&format!("bullseye-cli {cli}")), "CLI.md does not explain `{cli}`");
            }
        }
    }

    /// The page's enforcer checkboxes are the CLI's enforcer names.
    #[test]
    fn the_enforcers_are_the_same_as_in_the_window() {
        let names = enforcer_names();
        assert!(!names.is_empty());
        let page = include_str!("../../src/js/forms.js");
        let strings = include_str!("../../src/strings.js");
        for name in names {
            assert!(page.contains(&format!("enforcer(\"{name}\")")), "{name} is not on the page");
            assert!(strings.contains(&format!("{name}: {{")), "{name} has no text in strings.js");
        }
    }

    #[test]
    fn windows_by_title_are_closed_unless_minimized() {
        let all = parse_enforcers("all").unwrap();
        assert_eq!(all, Enforcers::default(), "all is every enforcer as a new block list has them: closing windows by title");
        assert!(!all.window_titles_minimize);
        let down = parse_enforcers("chromium_tabs, window_titles_minimize").unwrap();
        assert!(down.window_titles_minimize && down.chromium_tabs && !down.firefox_tabs);
        assert!(parse_enforcers("all,window_titles_minimize").unwrap().window_titles_minimize);
        // The lists with one way to act on them have no switch: their entries are it.
        for gone in ["exe_names", "store_apps", "window_titles", "window_titles_close"] {
            assert!(parse_enforcers(gone).unwrap_err().starts_with("Unknown enforcer"), "{gone}");
        }
    }

    #[test]
    fn a_new_block_list_with_everything() {
        let line = args(&[
            "list",
            "new",
            "Social",
            "--site",
            "youtube.com",
            "--site=reddit.com",
            "--allow",
            "youtube.com/watch*",
            "--program",
            "steam",
            "--store-app",
            "Microsoft.XboxApp_8wekyb3d8bbwe",
            "--title",
            "*YouTube*",
            "--allow-title",
            "*Visual Studio Code",
            "--enforcers",
            "chromium_tabs, window_titles_minimize",
        ]);
        let Ok(Command::ListNew { name, changes }) = parse(&line) else { panic!("not a new list") };
        assert_eq!(name, "Social");
        let mut list = block_list("x", &[]);
        changes.apply(&mut list);
        assert_eq!(list.entries.filters, vec!["youtube.com", "reddit.com"]);
        assert_eq!(list.entries.allowed, vec!["youtube.com/watch*"]);
        assert_eq!(list.entries.programs, vec!["steam"]);
        assert_eq!(list.entries.store_apps, vec!["Microsoft.XboxApp_8wekyb3d8bbwe"]);
        assert_eq!(list.entries.titles, vec!["*YouTube*"]);
        assert_eq!(list.entries.allowed_titles, vec!["*Visual Studio Code"]);
        assert_eq!(
            list.enforcers,
            Enforcers {
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
    fn a_new_block_finds_its_lists_and_condition_by_name() {
        let line = args(&["new", "Evening", "--list", "social", "--list", "l1", "--condition", "WORK HOURS", "--lock-duration", "2h 30m"]);
        let Ok(Command::New { name, changes }) = parse(&line) else { panic!("not new") };
        assert_eq!(name, "Evening");
        let mut b = block("x", &[]);
        assert!(apply_block_changes(&library(), &mut b, changes).is_ok());
        assert_eq!(
            (b.lists, b.condition, b.lock),
            (vec!["l1".to_string(), "l1".to_string()], Some("s1".to_string()), Lock::Duration { duration: "2h 30m".into() })
        );

        // `always`, and a list that does not exist.
        let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--condition", "always", "--no-lock"])) else {
            panic!("not edit")
        };
        let mut b = library().blocks[0].clone();
        assert!(apply_block_changes(&library(), &mut b, changes).is_ok());
        assert_eq!((b.condition, b.lock), (None, Lock::None));
        let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--list", "Games"])) else { panic!("not edit") };
        assert!(apply_block_changes(&library(), &mut library().blocks[0].clone(), changes).is_err());
    }

    #[test]
    fn start_can_set_the_lock_as_the_tile_does() {
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-condition", "Work hours", "--yes"])),
            Ok(Command::Start { block: "Work".into(), lock: Some(Lock::Condition { condition: "Work hours".into() }), yes: true })
        );
        assert_eq!(parse(&args(&["start", "Work"])), Ok(Command::Start { block: "Work".into(), lock: None, yes: false }));
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-until", "2026-10-06 18:00"])),
            Ok(Command::Start { block: "Work".into(), lock: Some(Lock::Until { until: "2026-10-06 18:00".into() }), yes: false })
        );
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-until-restart"])),
            Ok(Command::Start { block: "Work".into(), lock: Some(Lock::UntilRestart), yes: false })
        );
        assert!(parse(&args(&["start", "Work", "--lock-until", "tomorrow"])).is_err());
        assert!(parse(&args(&["start", "Work", "--lock-until-restart", "--no-lock"])).is_err());
        // What starting asks, per lock.
        let overview = library();
        let warning = lock_warning(&overview, &overview.blocks[0]).unwrap();
        assert!(warning.contains("while its lock condition \"Work hours\" is active."), "{warning}");
        assert!(!warning.contains("08:00"), "{warning}");
        let timed = Definition { lock: Lock::Duration { duration: "2h5m".into() }, ..overview.blocks[0].clone() };
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
    fn lists_come_from_files_too() {
        let path = std::env::temp_dir().join(format!("cli-{}-sites.txt", std::process::id()));
        std::fs::write(&path, "a.com\r\n\r\nb.com\n").unwrap();
        let line = args(&["list", "new", "A", "--sites-file", path.to_str().unwrap(), "--site", "c.com"]);
        let Ok(Command::ListNew { changes, .. }) = parse(&line) else { panic!("not new") };
        assert_eq!(changes.lists.get("filters"), Some(&vec!["c.com".to_string(), "a.com".into(), "b.com".into()]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn what_is_not_understood() {
        for bad in [
            &["frobnicate"][..],
            &["new"],
            &["new", "--list", "a"],
            &["new", "A", "--lock-duration", "soon"],
            &["new", "A", "--lock-duration", ""],
            &["new", "A", "--lock-duration", "1h", "--no-lock"],
            &["new", "A", "--name", "B"],
            &["new", "A", "--site", "a.com"],
            &["list"],
            &["list", "new"],
            &["list", "new", "A", "--site"],
            &["list", "new", "A", "--enforcers", "chromium_tabs,lasers"],
            &["list", "new", "A", "--clear", "sites"],
            &["list", "edit", "A", "--clear", "everything"],
            &["list", "frob", "A"],
            &["condition", "new", "A"],
            &["condition", "new", "A", "--times", "mon 9-17", "--drive", "Kingston"],
            &["condition", "check"],
            &["edit"],
            &["start"],
            &["start", "A", "B"],
            &["stop", "A", "now"],
            &["list", "new", "A", "--sites-file", "Z:/no/such/file.txt"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_other_commands() {
        assert_eq!(parse(&[]), Ok(Command::Help));
        assert_eq!(parse(&args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse(&args(&["status", "--json"])), Ok(Command::Status { json: true }));
        assert_eq!(parse(&args(&["show", "Work", "--json"])), Ok(Command::Show { block: "Work".into(), json: true }));
        assert_eq!(parse(&args(&["delete", "Work", "--yes"])), Ok(Command::Delete { block: "Work".into(), yes: true }));
        assert_eq!(parse(&args(&["export", "b.json"])), Ok(Command::Export { file: "b.json".into() }));
        assert_eq!(parse(&args(&["import", "b.json", "--yes"])), Ok(Command::Import { file: "b.json".into(), yes: true }));
        assert!(parse(&args(&["import"])).is_err());
        assert_eq!(parse(&args(&["duplicate", "Work"])), Ok(Command::Duplicate { kind: Kind::Block, item: "Work".into() }));
        assert_eq!(parse(&args(&["list", "duplicate", "Social"])), Ok(Command::Duplicate { kind: Kind::List, item: "Social".into() }));
        assert_eq!(
            parse(&args(&["condition", "duplicate", "Evenings"])),
            Ok(Command::Duplicate { kind: Kind::Condition, item: "Evenings".into() })
        );
        assert_eq!(parse(&args(&["stop", "Work"])), Ok(Command::Stop { block: "Work".into(), password: None, abort: false }));
        assert_eq!(
            parse(&args(&["stop", "Work", "--password", "pw"])),
            Ok(Command::Stop { block: "Work".into(), password: Some("pw".into()), abort: false })
        );
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-password", "partner"])),
            Ok(Command::Start {
                block: "Work".into(),
                lock: Some(Lock::Password { password: Some("partner".into()), hash: None }),
                yes: false
            })
        );
        assert!(parse(&args(&["start", "Work", "--lock-password", "pw", "--no-lock"])).is_err());
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-text", "I will not scroll", "--yes"])),
            Ok(Command::Start { block: "Work".into(), lock: Some(Lock::Text { text: "I will not scroll".into() }), yes: true })
        );
        assert!(parse(&args(&["start", "Work", "--lock-text", "x", "--lock-password", "pw"])).is_err());
        // A flash drive is named here and looked up among the connected ones when the command runs.
        assert_eq!(
            parse(&args(&["condition", "new", "Key", "--drive", "Kingston"])),
            Ok(Command::ConditionNew { name: "Key".into(), rule: RuleChoice::Drive("Kingston".into()) })
        );
        assert_eq!(parse(&args(&["drives", "--json"])), Ok(Command::Drives { json: true }));
        assert_eq!(parse(&args(&["list", "show", "Social"])), Ok(Command::ListShow { list: "Social".into(), json: false }));
        assert_eq!(parse(&args(&["list", "delete", "Social", "--yes"])), Ok(Command::ListDelete { list: "Social".into(), yes: true }));
        assert_eq!(parse(&args(&["condition", "show", "Work hours"])), Ok(Command::ConditionShow { condition: "Work hours".into() }));
        assert_eq!(
            parse(&args(&["condition", "delete", "Work hours"])),
            Ok(Command::ConditionDelete { condition: "Work hours".into(), yes: false })
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
    fn deleting_says_what_it_does_to_the_blocks() {
        let overview = library();
        assert_eq!(fallout(&overview, "l1"), " \"Work\" is deleted too, as it has no other block list.");
        assert_eq!(fallout(&overview, "s1"), " \"Work\" is always active from then on. \"Work\" has no lock from then on.");
        assert_eq!(fallout(&overview, "other"), "");
    }

    #[test]
    fn things_are_found_by_id_or_name() {
        let lists = vec![
            block_list("a1", &[]),
            BlockList { name: "Games".into(), ..block_list("b2", &[]) },
            BlockList { name: "games".into(), ..block_list("c3", &[]) },
        ];
        fn key(l: &BlockList) -> (&String, &String) {
            (&l.id, &l.name)
        }
        assert_eq!(find(&lists, "a1", "block list", key).ok().map(|l| l.id.as_str()), Some("a1"));
        assert_eq!(find(&lists, "list A1", "block list", key).ok().map(|l| l.id.as_str()), Some("a1"));
        assert!(matches!(find(&lists, "Games", "block list", key), Err(Failure::Failed(why)) if why.contains("b2, c3")));
        assert!(find(&lists, "Nope", "block list", key).is_err());
        assert!(new_id().chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && new_id().len() <= 64);
    }

    #[test]
    fn the_status_reads_like_the_pages() {
        let empty = Overview {
            lists: vec![],
            conditions: vec![],
            blocks: vec![],
            running: vec![],
            unlock: UnlockStatus::default(),
            clock_synced: true,
        };
        assert!(describe_overview(&empty).contains("Blocks:\n  None yet"));
        let mut overview = library();
        overview.running.push(Running {
            id: "b1".into(),
            name: "Work".into(),
            lists: vec![RunningList {
                name: "Social".into(),
                entries: Entries { filters: vec!["youtube.com".into()], ..Entries::default() },
                enforcers: Enforcers::default(),
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
        assert!(text.contains("Block lists:\n  Social  used by Work"), "{text}");
        assert!(text.contains("Conditions:\n  Work hours  Mon-Fri 08:00-17:00"), "{text}");
        assert!(text.contains("Waiting for internet time"), "{text}");

        let block = describe_block(&overview, &overview.blocks[0], overview.running.first());
        assert!(block.contains("Block lists: Social") && block.contains("Condition:   Work hours\n"), "{block}");
        assert!(!block.contains("08:00-17:00"), "{block}");
        assert!(block.contains("Lock:        condition: cannot be stopped while \"Work hours\" is active"), "{block}");
        assert_eq!(
            resolve_lock(&overview, Lock::Condition { condition: "work HOURS".into() }).ok(),
            Some(Lock::Condition { condition: "s1".into() })
        );
        let list = describe_list(&overview, &overview.lists[0]);
        assert!(list.contains("Block list:     youtube.com") && list.contains("Used by:        Work"), "{list}");
    }

    /// What `new` and `edit` say of the allowance, as terms over the ones the block has.
    fn allowance_of(line: &[&str], old: Option<Terms>) -> Result<Option<Terms>, String> {
        let changes = match parse(&args(line))? {
            Command::New { changes, .. } | Command::Edit { changes, .. } => changes,
            other => panic!("not new or edit: {other:?}"),
        };
        let mut b = library().blocks[0].clone();
        b.allowance = old;
        match apply_block_changes(&library(), &mut b, changes) {
            Ok(()) => Ok(b.allowance),
            Err(Failure::Failed(why) | Failure::Usage(why)) => Err(why),
        }
    }

    #[test]
    fn a_block_has_its_allowance_set_by_its_options() {
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
        // The defaults: the whole block, never reset; the kind can be left out.
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

        // It is shown on the block, and what it is doing with a running one.
        let mut overview = library();
        overview.blocks[0].allowance = cycle("50m", "10m", Anchor::Clock);
        let shown = describe_block(&overview, &overview.blocks[0], None);
        assert!(shown.contains("Allowance:   Blocked for 50 minutes, then allowed for 10 minutes, on the clock"), "{shown}");
        assert!(describe_block(&library(), &library().blocks[0], None).contains("Allowance:   none\n"));
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
    fn a_schedule_is_shown_per_day() {
        let text = per_day(&Week::parse("mon-fri 8-12, 13-17; sat 10-12").unwrap().unwrap());
        assert!(text.starts_with("Mon-Fri 08:00-12:00, 13:00-17:00; Sat 10:00-12:00\n"), "{text}");
        assert!(text.contains("  Tue  08:00-12:00, 13:00-17:00\n"), "{text}");
        assert!(text.contains("  Sun  inactive\n"), "{text}");
    }
}
