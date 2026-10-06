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

use crate::block::Running;
use crate::client::Remote;
use crate::condition::{Mode, Rule};
use crate::definitions::{self, BlockList, Condition, Definition, Kind, Library};
use crate::duration;
use crate::lists::Entries;
use crate::localtime;
use crate::lock::Lock;
use crate::proto::{Overview, Request};
use crate::schedule::Week;
use crate::state::Enforcers;

/// Each command the window can invoke, and the CLI commands that do the same. Empty: it is about
/// the window itself.
pub const GUI_COMMANDS: [(&str, &[&str]); 24] = [
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
    ("unlock_all", &["unlock"]),
    ("request_unlock", &["unlock"]),
    ("abort_unlock", &["unlock"]),
    ("set_unlock_delay", &["unlock-delay"]),
    ("describe_schedule", &["condition check"]),
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
active; 'always', a weekly schedule, or a flash drive that is not connected) and a lock (how it
is kept from being stopped).

Blocks:
  status [--json]          Every block (running or not, active or not, locked or not), every
                           block list and every condition.
  show <block> [--json]    One block: its lists, condition, lock and state.
  new <name> [options]     Make a block. Options:
      --list <list>            A block list it enforces (repeatable).
      --condition <condition>  When it is active: a condition's name, or 'always' (the default).
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
      --lock-text <text>       Locked until the text is typed in exactly: the window shows it
                               above an input that cannot be pasted into, and that is the only
                               place to stop such a block (or by unlocking).
      --no-lock                Not locked: it can be stopped at any time (the default).
  edit <block> [options]   Change a block that is not running: the options of new, which replace
                           what they name (--list replaces all its lists), and --name <name>.
  delete <block> [--yes]   Delete a block that is not running. Asks first, unless --yes.
  duplicate <block>        Make a copy of a block, named after it with a number: \"Work (2)\".
  start <block> [--yes]    Start a block. The lock options of new set its lock first, as the lock
                           on its tile in the window does. A locked block asks first, unless --yes.
  stop <block> [--password <pw>]
                           Stop a running block that is not locked right now; one locked with a
                           password with its password (asked for when not given). One locked
                           with a text is stopped in the window, by typing the text.

Block lists:
  list show <list> [--json]   One block list, and the blocks that use it.
  list apps [--json]          The Store apps installed, by name and package family name, to
                              block with --store-app.
  list new <name> [options]   Make a block list. Options:
      --site <entry>           A site to block (repeatable): the site with its subdomains and
                               everything under it, e.g. youtube.com. * as a wildcard.
      --allow <entry>          A site that is always allowed, also when blocked (repeatable).
      --program <name>         A program to close, by exe name (repeatable), e.g. steam.exe.
      --store-app <family>     A Store app to close, by package family name (repeatable), e.g.
                               Microsoft.WindowsCalculator_8wekyb3d8bbwe; * as a wildcard.
                               'list apps' shows them.
      --title <title>          A window to close, by its title (repeatable), * as a wildcard,
                               e.g. \"*YouTube*\". The whole window closes, tabs and all.
      --sites-file <file>      Sites to block, one per line. Likewise --allowed-file,
                               --programs-file, --store-apps-file and --titles-file.
      --enforcers <list>       Comma-separated, or 'all' (the default). Names:
                               {enforcers}
  list edit <list> [options]  Change a block list: the options of list new, which replace what
                              they name, --name <name> and --clear <sites|allowed|programs|
                              store-apps|titles> (repeatable). While a locked block uses it,
                              only stricter: more to block, fewer allowed, more enforcers.
  list delete <list> [--yes]  Delete a block list that no running block uses; it is taken out of
                              the blocks that use it (a block left without one is deleted).
  list duplicate <list>       Make a copy of a block list, named after it with a number.

Conditions:
  condition show <condition>        One condition, and the blocks that use it.
  condition new <name> (--times <times> | --drive <drive> | --any <condition>... | --all <condition>...)
                                    Make a condition: a weekly schedule, e.g. --times
                                    \"Mon-Fri 08:00-17:00; Sat 10-12\"; or a flash drive, by name
                                    or serial number ('drives' lists them): active while it is
                                    not connected; or a combination of two or more other
                                    conditions (--any or --all, once for each): active when any
                                    of them is, or when all are. Not of other combinations.
  condition edit <condition> [--name <name>] [--times <times> | --drive <drive> | --any <condition>... | --all <condition>...]
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
  condition check <times>           Check times and show them per day, as a schedule takes them.

Moving to another computer:
  export <file>            Write every block list, condition and block to a settings file, as
                           Export in Settings does. Password locks are in it without their
                           password.
  import <file> [--yes]    Replace every block list, condition and block with those in a settings
                           file, as Import in Settings does; not while a block is locked, and
                           blocks that run unlocked end with it. A block with a password lock
                           comes without a lock. Asks first, unless --yes.

Other:
  drives [--json]          The flash drives connected, by name and serial number, for
                           'condition new --drive'.
  unlock [--abort]         End every running block, as the window's Unlock does. By default
                           unlocking is free. With an unlock delay it is a countdown: the first
                           'unlock' starts it, a later one ends every block once it is over, and
                           --abort stops it (back to locked).
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
}

/// What `new` and `edit` set on a block; `None` leaves it. Lists and the condition by name or id,
/// looked up when the command runs.
#[derive(Debug, PartialEq, Default)]
struct BlockChanges {
    name: Option<String>,
    lists: Option<Vec<String>>,
    /// `Some(None)`: always.
    condition: Option<Option<String>>,
    lock: Option<Lock>,
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
const LIST_OPTIONS: [(&str, &str, &str, &str); 5] = [
    ("--site", "--sites-file", "sites", "filters"),
    ("--allow", "--allowed-file", "allowed", "allowed"),
    ("--program", "--programs-file", "programs", "programs"),
    ("--store-app", "--store-apps-file", "store-apps", "store_apps"),
    ("--title", "--titles-file", "titles", "titles"),
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
        "stop" => Command::Stop { block: named(&mut options, "block")?, password: options.value("--password")? },
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
        "condition check" => {
            Command::ConditionCheck { times: options.positional().ok_or("Give the times to check, e.g. \"Mon-Fri 08:00-17:00\"")? }
        }
        "unlock" => Command::Unlock { abort: options.flag("--abort") },
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
    let unlocked = options.flag("--no-lock").then_some(Lock::None);
    let mut given: Vec<Lock> = [duration, until, restart, conditional, password, typed, unlocked].into_iter().flatten().collect();
    let lock = match given.len() {
        0 => return Ok(None),
        1 => given.remove(0),
        _ => return Err(
            "Give one of --lock-duration, --lock-until, --lock-until-restart, --lock-condition, --lock-password, --lock-text and --no-lock"
                .into(),
        ),
    };
    // The service checks it again; this is for a clear message before anything is sent.
    lock.check()?;
    Ok(Some(lock))
}

/// The options of `new` and `edit`. Only `edit` takes `--name`.
fn parse_block_changes(options: &mut Options, editing: bool) -> Result<BlockChanges, String> {
    let lists = options.values("--list")?;
    Ok(BlockChanges {
        name: if editing { options.value("--name")? } else { None },
        lists: (!lists.is_empty()).then_some(lists),
        condition: options.value("--condition")?.map(|c| (!c.eq_ignore_ascii_case("always")).then_some(c)),
        lock: parse_lock(options)?,
    })
}

/// `--times <times>`, `--drive <drive>`, `--any <condition>...` or `--all <condition>...` of a
/// condition, at most one kind.
fn parse_rule(options: &mut Options) -> Result<Option<RuleChoice>, String> {
    let mut given: Vec<RuleChoice> = Vec::new();
    given.extend(options.value("--times")?.map(RuleChoice::Times));
    given.extend(options.value("--drive")?.map(RuleChoice::Drive));
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
        _ => Err("Give one of --times, --drive, --any or --all".into()),
    }
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

/// `chromium_tabs,exe_names`, or `all`.
fn parse_enforcers(list: &str) -> Result<Enforcers, String> {
    let names = enforcer_names();
    let mut chosen: Map<String, Value> = names.iter().map(|name| (name.clone(), Value::Bool(false))).collect();
    for name in list.split(',').map(str::trim).filter(|name| !name.is_empty()) {
        if name == "all" {
            chosen.values_mut().for_each(|on| *on = Value::Bool(true));
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
        Command::Help => println!("{}", HELP.replace("{enforcers}", &enforcer_names().join(", "))),
        Command::Version => println!("bullseye-cli {}", env!("CARGO_PKG_VERSION")),
        Command::Status { json } => {
            let overview = overview()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&overview).map_err(|e| failed(e.to_string()))?);
            } else {
                print!("{}", describe_overview(&overview));
            }
        }
        Command::Show { block, json } => {
            let overview = overview()?;
            let block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?;
            let running = overview.running.iter().find(|r| r.id == block.id);
            if json {
                let both = serde_json::json!({ "block": block, "running": running });
                println!("{}", serde_json::to_string_pretty(&both).map_err(|e| failed(e.to_string()))?);
            } else {
                print!("{}", describe_block(&overview, block, running));
            }
        }
        Command::New { name, changes } => {
            let overview = overview()?;
            let mut block = Definition { id: new_id(), name, lists: Vec::new(), condition: None, lock: Lock::None, kept_locks: Vec::new() };
            apply_block_changes(&overview, &mut block, changes)?;
            let (id, name) = (block.id.clone(), block.name.clone());
            send(Request::SaveBlock { block })?;
            println!("Made the block \"{name}\" (id {id}). Start it with: bullseye-cli start \"{name}\"");
        }
        Command::Edit { block, changes } => {
            let overview = overview()?;
            let mut block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?.clone();
            apply_block_changes(&overview, &mut block, changes)?;
            let name = block.name.clone();
            send(Request::SaveBlock { block })?;
            println!("Saved the block \"{name}\".");
        }
        Command::Delete { block, yes } => {
            let overview = overview()?;
            let block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?;
            if !confirm(&format!("Delete the block \"{}\"? This cannot be undone.", block.name), yes)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteBlock { id: block.id.clone() })?;
            println!("Deleted the block \"{}\".", block.name);
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
            println!("Started \"{}\".", block.name);
        }
        Command::Stop { block, password } => {
            let overview = overview()?;
            let block = find(&overview.blocks, &block, "block", |b| (&b.id, &b.name))?;
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
            println!("Stopped \"{}\".", block.name);
        }
        Command::ListShow { list, json } => {
            let overview = overview()?;
            let list = find(&overview.lists, &list, "block list", |l| (&l.id, &l.name))?;
            if json {
                println!("{}", serde_json::to_string_pretty(list).map_err(|e| failed(e.to_string()))?);
            } else {
                print!("{}", describe_list(&overview, list));
            }
        }
        Command::Drives { json } => {
            let drives = crate::drives::connected().map_err(Failure::Failed)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&drives).map_err(|e| failed(e.to_string()))?);
            } else if drives.is_empty() {
                println!("No flash drive with a serial number is connected.");
            } else {
                for drive in drives {
                    println!("{:<40} {}", drive.name, drive.serial);
                }
            }
        }
        Command::ListApps { json } => {
            let apps = crate::store_apps::installed().map_err(Failure::Failed)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&apps).map_err(|e| failed(e.to_string()))?);
            } else {
                for app in apps {
                    println!("{:<40} {}", app.name, app.family);
                }
            }
        }
        Command::ListNew { name, changes } => {
            let mut list = BlockList { id: new_id(), name, entries: Entries::default(), enforcers: Enforcers::default() };
            changes.apply(&mut list);
            let name = list.name.clone();
            send(Request::SaveList { list })?;
            println!("Made the block list \"{name}\". Use it with: bullseye-cli new <block> --list \"{name}\"");
        }
        Command::ListEdit { list, changes } => {
            let overview = overview()?;
            let mut list = find(&overview.lists, &list, "block list", |l| (&l.id, &l.name))?.clone();
            changes.apply(&mut list);
            let name = list.name.clone();
            send(Request::SaveList { list })?;
            println!("Saved the block list \"{name}\".");
        }
        Command::ListDelete { list, yes } => {
            let overview = overview()?;
            let list = find(&overview.lists, &list, "block list", |l| (&l.id, &l.name))?;
            if !confirm(&format!("Delete the block list \"{}\"?{} This cannot be undone.", list.name, fallout(&overview, &list.id)), yes)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteList { id: list.id.clone() })?;
            println!("Deleted the block list \"{}\".", list.name);
        }
        Command::ConditionShow { condition } => {
            let overview = overview()?;
            let condition = find(&overview.conditions, &condition, "condition", |c| (&c.id, &c.name))?;
            println!("{}", condition.name);
            match &condition.rule {
                Rule::Schedule { week } => print!("{}", per_day(week)),
                rule => println!("{}", describe_rule(&overview, rule)),
            }
            println!("Used by: {}", users(&overview, &condition.id));
        }
        Command::ConditionNew { name, rule } => {
            let rule = resolve_rule(&overview()?, rule)?;
            send(Request::SaveCondition { condition: Condition { id: new_id(), name: name.clone(), rule } })?;
            println!("Made the condition \"{name}\". Use it with: bullseye-cli new <block> --condition \"{name}\"");
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
            println!("Saved the condition \"{name}\".");
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
            println!("Deleted the condition \"{}\".", condition.name);
        }
        Command::ConditionCheck { times } => match Week::parse(&times).map_err(Failure::Usage)? {
            Some(week) => print!("{}", per_day(&week)),
            None => println!("Always active: a block without a condition"),
        },
        Command::Export { file } => {
            definitions::write_settings(std::path::Path::new(&file), overview()?.library()).map_err(failed)?;
            println!("Wrote every block list, condition and block to {file}.");
        }
        Command::Import { file, yes } => {
            let library = definitions::read_settings(std::path::Path::new(&file)).map_err(failed)?;
            let contents = library.contents();
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
            if !confirm(&question, yes)? {
                return Err(failed("Not imported."));
            }
            send(Request::Import { library })?;
            println!("Imported {file}.");
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
            println!("Made \"{copy}\", a copy of the {noun} \"{name}\".");
        }
        Command::Unlock { abort } => {
            let status = overview()?.unlock;
            if abort {
                send(Request::AbortUnlock)?;
                println!("Unlocking aborted: back to locked.");
            } else if status.remaining == Some(0) {
                send(Request::Unlock)?;
                // The service tells the app to quit at its next policy request, within a second.
                println!("Unlocked: every block has ended.");
            } else if let Some(left) = status.remaining_words {
                println!("Unlocking is not ready yet: {left} to go. Run `bullseye-cli unlock` again then, or `bullseye-cli unlock --abort` to stop.");
            } else {
                send(Request::AskForUnlock)?;
                println!(
                    "Unlocking was asked for: ready in {}. Run `bullseye-cli unlock` again then, or `bullseye-cli unlock --abort` to stop.",
                    status.delay_words
                );
            }
        }
        Command::UnlockDelay { delay } => match delay {
            None => {
                let status = overview()?.unlock;
                if status.delay == 0 {
                    println!("Unlocking takes no time: it is free.");
                } else {
                    println!("Unlocking takes {}.", status.delay_words);
                }
            }
            Some(delay) => {
                send(Request::SetUnlockDelay { delay })?;
                let status = overview()?.unlock;
                println!(
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
        Rule::Schedule { .. } | Rule::Combined { .. } => String::new(),
        Rule::FlashDrive { drive, .. } => format!(" (the flash drive \"{drive}\" is not connected)"),
    }
}

/// A condition as words: its times, the drive it waits for, or the conditions it combines.
fn describe_rule(overview: &Overview, rule: &Rule) -> String {
    match rule {
        Rule::Schedule { week } => week.describe(),
        Rule::FlashDrive { drive, .. } => format!("while the flash drive \"{drive}\" is not connected"),
        Rule::Combined { mode, members, .. } => {
            let names: Vec<String> = members
                .iter()
                .map(|id| overview.conditions.iter().find(|c| &c.id == id).map_or("(gone)".to_string(), |c| format!("\"{}\"", c.name)))
                .collect();
            format!("{} of {}", if *mode == Mode::Any { "any" } else { "all" }, names.join(", "))
        }
    }
}

/// `new` and `edit`'s changes, with lists and the condition looked up by name or id.
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
        Lock::Text { .. } => Some("It can only be stopped by typing its text in, in the window.".into()),
        Lock::Password { .. } => Some("It can only be stopped with its password.".into()),
    }
}

/// The service, as the window reaches it: the same `Remote`, so the same requests and answers.
fn remote() -> Remote {
    Remote::for_cli()
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
        (true, None) => "blocking now".into(),
        (true, Some(change)) => format!("blocking now, inactive in {change}"),
        (false, Some(change)) => format!("inactive by its condition, active in {change}"),
        (false, None) => "inactive by its condition".into(),
    });
    parts.push(match (&running.remaining_words, running.locked) {
        _ if running.password => "locked with a password".into(),
        _ if running.lock_text.is_some() => "locked until its text is typed in".into(),
        _ if running.until_restart => "locked until Windows restarts".into(),
        (Some(left), _) => format!("locked, {left} left"),
        (None, true) => match (&running.lock_condition, &running.lock_change_words) {
            (Some(Rule::FlashDrive { drive, .. }), _) => format!("locked, the flash drive \"{drive}\" is not connected"),
            (_, Some(change)) => format!("locked by its lock condition, for another {change}"),
            (_, None) => "locked by its lock condition".into(),
        },
        (None, false) if running.lock_condition.is_some() => match (&running.lock_condition, &running.lock_change_words) {
            (Some(Rule::FlashDrive { drive, .. }), _) => format!("can be stopped while the flash drive \"{drive}\" is connected"),
            (_, Some(change)) => format!("can be stopped, locked again in {change}"),
            (_, None) => "can be stopped".into(),
        },
        (None, false) => "not locked".into(),
    });
    parts.join(" · ")
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
        Lock::Text { .. } => "text: can only be stopped by typing its text in, in the window".into(),
        Lock::Password { .. } => "password: can only be stopped with its password".into(),
    };
    format!(
        "Name:        {}\nId:          {}\nState:       {}\nBlock lists: {}\nCondition:   {condition}\nLock:        {lock}\n",
        block.name,
        block.id,
        state(running),
        entries(&lists.iter().map(|l| l.to_string()).collect::<Vec<_>>()),
    )
}

/// One block list, everything about it.
fn describe_list(overview: &Overview, list: &BlockList) -> String {
    format!(
        "Name:           {}\nId:             {}\nBlock list:     {}\nAlways allowed: {}\nPrograms:       {}\nStore apps:     {}\nWindow titles:  {}\nEnforcers:      {}\nUsed by:        {}\n",
        list.name,
        list.id,
        entries(&list.entries.filters),
        entries(&list.entries.allowed),
        entries(&list.entries.programs),
        entries(&list.entries.store_apps),
        entries(&list.entries.titles),
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
    use crate::block::RunningList;
    use crate::definitions::tests::{block, block_list, weekly};
    use crate::proto::UnlockStatus;

    fn args(line: &[&str]) -> Vec<String> {
        line.iter().map(|arg| arg.to_string()).collect()
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
            "Microsoft.Xbox*",
            "--title",
            "*YouTube*",
            "--enforcers",
            "chromium_tabs, exe_names",
        ]);
        let Ok(Command::ListNew { name, changes }) = parse(&line) else { panic!("not a new list") };
        assert_eq!(name, "Social");
        let mut list = block_list("x", &[]);
        changes.apply(&mut list);
        assert_eq!(list.entries.filters, vec!["youtube.com", "reddit.com"]);
        assert_eq!(list.entries.allowed, vec!["youtube.com/watch*"]);
        assert_eq!(list.entries.programs, vec!["steam"]);
        assert_eq!(list.entries.store_apps, vec!["Microsoft.Xbox*"]);
        assert_eq!(list.entries.titles, vec!["*YouTube*"]);
        assert_eq!(
            list.enforcers,
            Enforcers {
                chromium_tabs: true,
                chromium_split_view: false,
                obsidian_web_viewer: false,
                exe_names: true,
                chromium_app_windows: false,
                firefox_tabs: false,
                firefox_split_view: false,
                store_apps: false,
                window_titles: false,
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
        assert!(lock_warning(&overview, &timed).unwrap().contains("for 2:05 hours."));
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
        assert_eq!(parse(&args(&["stop", "Work"])), Ok(Command::Stop { block: "Work".into(), password: None }));
        assert_eq!(
            parse(&args(&["stop", "Work", "--password", "pw"])),
            Ok(Command::Stop { block: "Work".into(), password: Some("pw".into()) })
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
        assert_eq!(parse(&args(&["unlock"])), Ok(Command::Unlock { abort: false }));
        assert_eq!(parse(&args(&["unlock", "--abort"])), Ok(Command::Unlock { abort: true }));
        assert_eq!(parse(&args(&["unlock-delay"])), Ok(Command::UnlockDelay { delay: None }));
        assert_eq!(parse(&args(&["unlock-delay", "2h 30m"])), Ok(Command::UnlockDelay { delay: Some("2h 30m".into()) }));
        assert_eq!(parse(&args(&["unlock-delay", "0"])), Ok(Command::UnlockDelay { delay: Some("0".into()) }));
        assert!(parse(&args(&["unlock", "--password", "pw"])).is_err());
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
            password: false,
            locked: false,
            condition: Week::parse("mon-fri 8-17").unwrap().map(|week| Rule::Schedule { week }),
            enforcing: false,
            condition_change: Some(3600),
            remaining_words: None,
            condition_change_words: Some(duration::countdown(3600)),
            lock_change: Some(3600),
            lock_change_words: Some(duration::countdown(3600)),
        });
        overview.clock_synced = false;
        let text = describe_overview(&overview);
        assert!(
            text.contains("  Work  Running · inactive by its condition, active in 1:00 hours · can be stopped, locked again in 1:00 hours"),
            "{text}"
        );
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

    #[test]
    fn a_schedule_is_shown_per_day() {
        let text = per_day(&Week::parse("mon-fri 8-12, 13-17; sat 10-12").unwrap().unwrap());
        assert!(text.starts_with("Mon-Fri 08:00-12:00, 13:00-17:00; Sat 10:00-12:00\n"), "{text}");
        assert!(text.contains("  Tue  08:00-12:00, 13:00-17:00\n"), "{text}");
        assert!(text.contains("  Sun  inactive\n"), "{text}");
    }
}
