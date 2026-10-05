//! The command line: everything the window can do, from a terminal or a script
//! (`bullseye-cli.exe`, explained in `CLI.md`). It talks to the service through the same
//! `client::Remote` as the window, so the service checks everything done here exactly as it checks
//! the window, and texts such as schedules and durations come from the same Rust functions.
//!
//! **Keep it level with the window.** Every command the window can invoke (the list in
//! `generate_handler!` in `lib.rs`) is in `GUI_COMMANDS` with the CLI commands that do the same, and
//! a test fails when one is missing, or missing from `HELP` or `CLI.md`. Enforcer names come from
//! `Enforcers` itself, so a new enforcer is here without any change. A new list, schedule kind or
//! lock method needs an option here, a line in `HELP` and in `CLI.md`.

use std::io::{BufRead, IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::block::Running;
use crate::client::Remote;
use crate::definitions::{BlockList, Definition, Library, NamedSchedule};
use crate::duration;
use crate::lock::Lock;
use crate::proto::{Overview, Request};
use crate::schedule::Week;
use crate::state::Enforcers;

/// Each command the window can invoke, and the CLI commands that do the same. Empty: it is about
/// the window itself.
pub const GUI_COMMANDS: [(&str, &[&str]); 14] = [
    ("block_status", &["status", "show", "list show", "schedule show"]),
    ("save_block", &["new", "edit"]),
    ("delete_block", &["delete"]),
    ("save_list", &["list new", "list edit"]),
    ("delete_list", &["list delete"]),
    ("save_schedule", &["schedule new", "schedule edit"]),
    ("delete_schedule", &["schedule delete"]),
    ("start_block", &["start"]),
    ("stop_block", &["stop"]),
    ("force_exit", &["unlock"]),
    ("parse_schedule", &["schedule check"]),
    ("describe_schedule", &["schedule check"]),
    // A lock's length in words: what `show` says of a timer lock.
    ("describe_duration", &["show"]),
    ("window_ready", &[]),
];

const HELP: &str = "\
Bullseye command line: everything the window can do.

Usage: bullseye-cli <command> [options]
A <block>, <list> or <schedule> is its name (any case) or its id.

A block is what you start: one or more block lists (what to block), a schedule (when it is
active; 'always' or a weekly schedule) and a lock (how it is kept from being stopped).

Blocks:
  status [--json]          Every block (running or not, active or not, locked or not), every
                           block list and every schedule.
  show <block> [--json]    One block: its lists, schedule, lock and state.
  new <name> [options]     Make a block. Options:
      --list <list>            A block list it enforces (repeatable).
      --schedule <schedule>    When it is active: a schedule's name, or 'always' (the default).
      --lock-timer <duration>  Locked for this long once started, e.g. 2h 30m: it cannot be
                               stopped and ends by itself.
      --lock-schedule <schedule>
                               Locked while this schedule is active: it cannot be stopped then,
                               and runs until stopped outside those times. With the block's own
                               schedule, it is locked exactly while it is active.
      --lock-password <pw>     Locked with a password: it runs until stopped with that password
                               (or the unlock password). Only a hash of it is kept.
      --no-lock                Not locked: it can be stopped at any time (the default).
  edit <block> [options]   Change a block that is not running: the options of new, which replace
                           what they name (--list replaces all its lists), and --name <name>.
  delete <block> [--yes]   Delete a block that is not running. Asks first, unless --yes.
  start <block> [--yes]    Start a block. The lock options of new set its lock first, as the lock
                           on its tile in the window does. A locked block asks first, unless --yes.
  stop <block> [--password <pw>]
                           Stop a running block that is not locked right now; one locked with a
                           password with its password (asked for when not given).

Block lists:
  list show <list> [--json]   One block list, and the blocks that use it.
  list new <name> [options]   Make a block list. Options:
      --site <entry>           A site to block (repeatable). Address prefix, * as a wildcard.
      --allow <entry>          A site that is always allowed, also when blocked (repeatable).
      --program <name>         A program to close, by exe name (repeatable), e.g. steam.exe.
      --sites-file <file>      Sites to block, one per line. Likewise --allowed-file and
                               --programs-file.
      --enforcers <list>       Comma-separated, or 'all' (the default). Names:
                               {enforcers}
  list edit <list> [options]  Change a block list that no running block uses: the options of
                              list new, which replace what they name, --name <name> and
                              --clear <sites|allowed|programs> (repeatable).
  list delete <list> [--yes]  Delete a block list that no block uses.

Schedules:
  schedule show <schedule>          One schedule, per day, and the blocks that use it.
  schedule new <name> <times>       Make a weekly schedule, e.g. \"Mon-Fri 08:00-17:00; Sat 10-12\".
  schedule edit <schedule> [--name <name>] [--times <times>]
                                    Change a schedule that no running block uses.
  schedule delete <schedule> [--yes]
                                    Delete a schedule that no block uses.
  schedule check <times>            Check times and show them per day, as a schedule takes them.

Other:
  unlock [--password <pw>] End every running block with the unlock password, and close the app,
                           as the window's Unlock does. Asks for the password when not given (or
                           reads standard input when that is not a terminal).
  help                     Show this.
  version                  Show the version.

Durations: 30s, 10m, 2h, 1d, or several together: 2h 30m, 1d 12h.
Exit codes: 0 done, 1 failed, refused or cancelled, 2 not understood.";

/// What the CLI was asked to do.
#[derive(Debug, PartialEq)]
enum Command {
    Help,
    Version,
    Status { json: bool },
    Show { block: String, json: bool },
    New { name: String, changes: BlockChanges },
    Edit { block: String, changes: BlockChanges },
    Delete { block: String, yes: bool },
    Start { block: String, lock: Option<Lock>, yes: bool },
    Stop { block: String, password: Option<String> },
    ListShow { list: String, json: bool },
    ListNew { name: String, changes: ListChanges },
    ListEdit { list: String, changes: ListChanges },
    ListDelete { list: String, yes: bool },
    ScheduleShow { schedule: String },
    ScheduleNew { name: String, times: String },
    ScheduleEdit { schedule: String, name: Option<String>, times: Option<String> },
    ScheduleDelete { schedule: String, yes: bool },
    ScheduleCheck { times: String },
    Unlock { password: Option<String> },
}

/// What `new` and `edit` set on a block; `None` leaves it. Lists and the schedule by name or id,
/// looked up when the command runs.
#[derive(Debug, PartialEq, Default)]
struct BlockChanges {
    name: Option<String>,
    lists: Option<Vec<String>>,
    /// `Some(None)`: always.
    schedule: Option<Option<String>>,
    lock: Option<Lock>,
}

/// What `list new` and `list edit` set on a block list; `None` leaves it.
#[derive(Debug, PartialEq, Default)]
struct ListChanges {
    name: Option<String>,
    filters: Option<Vec<String>>,
    allowed: Option<Vec<String>>,
    programs: Option<Vec<String>>,
    enforcers: Option<Enforcers>,
}

impl ListChanges {
    fn apply(self, list: &mut BlockList) {
        if let Some(name) = self.name {
            list.name = name;
        }
        if let Some(filters) = self.filters {
            list.filters = filters;
        }
        if let Some(allowed) = self.allowed {
            list.allowed = allowed;
        }
        if let Some(programs) = self.programs {
            list.programs = programs;
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
    // `list` and `schedule` take a second word.
    let (command, rest) = match (command.as_str(), rest.split_first()) {
        ("list" | "schedule", Some((verb, rest))) => (format!("{command} {verb}"), rest),
        ("list" | "schedule", None) => return Err(format!("`{command}` needs show, new, edit, delete{}", if command == "schedule" { " or check" } else { "" })),
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
        "start" => Command::Start { block: named(&mut options, "block")?, lock: parse_lock(&mut options)?, yes: options.flag("--yes") },
        "stop" => Command::Stop { block: named(&mut options, "block")?, password: options.value("--password")? },
        "list show" => Command::ListShow { list: named(&mut options, "block list")?, json: options.flag("--json") },
        "list new" => {
            let name = options.positional().ok_or("Give the new block list a name")?;
            Command::ListNew { name, changes: parse_list_changes(&mut options, false)? }
        }
        "list edit" => Command::ListEdit { list: named(&mut options, "block list")?, changes: parse_list_changes(&mut options, true)? },
        "list delete" => Command::ListDelete { list: named(&mut options, "block list")?, yes: options.flag("--yes") },
        "schedule show" => Command::ScheduleShow { schedule: named(&mut options, "schedule")? },
        "schedule new" => {
            let name = options.positional().ok_or("Give the new schedule a name")?;
            let times = options.positional().ok_or("Give its times, e.g. \"Mon-Fri 08:00-17:00\"")?;
            Command::ScheduleNew { name, times }
        }
        "schedule edit" => Command::ScheduleEdit {
            schedule: named(&mut options, "schedule")?,
            name: options.value("--name")?,
            times: options.value("--times")?,
        },
        "schedule delete" => Command::ScheduleDelete { schedule: named(&mut options, "schedule")?, yes: options.flag("--yes") },
        "schedule check" => Command::ScheduleCheck { times: options.positional().ok_or("Give the times to check, e.g. \"Mon-Fri 08:00-17:00\"")? },
        "unlock" => Command::Unlock { password: options.value("--password")? },
        other => return Err(format!("Unknown command \"{other}\"")),
    };
    options.finish()?;
    Ok(parsed)
}

/// `--lock-timer <duration>`, `--lock-schedule <schedule>`, `--lock-password <pw>` or `--no-lock`,
/// at most one. A lock schedule by name or id, looked up when the command runs (`find_lock_schedule`).
fn parse_lock(options: &mut Options) -> Result<Option<Lock>, String> {
    let timer = options.value("--lock-timer")?.map(|duration| Lock::Timer { duration });
    let scheduled = options.value("--lock-schedule")?.map(|schedule| Lock::Scheduled { schedule });
    let password = options.value("--lock-password")?.map(|password| Lock::Password { password: Some(password), hash: None });
    let unlocked = options.flag("--no-lock").then_some(Lock::None);
    let mut given: Vec<Lock> = [timer, scheduled, password, unlocked].into_iter().flatten().collect();
    let lock = match given.len() {
        0 => return Ok(None),
        1 => given.remove(0),
        _ => return Err("Give one of --lock-timer, --lock-schedule, --lock-password and --no-lock".into()),
    };
    // The service checks it again; this is for a clear message before anything is sent.
    lock.duration()?;
    Ok(Some(lock))
}

/// The options of `new` and `edit`. Only `edit` takes `--name`.
fn parse_block_changes(options: &mut Options, editing: bool) -> Result<BlockChanges, String> {
    let lists = options.values("--list")?;
    Ok(BlockChanges {
        name: if editing { options.value("--name")? } else { None },
        lists: (!lists.is_empty()).then_some(lists),
        schedule: options.value("--schedule")?.map(|s| (!s.eq_ignore_ascii_case("always")).then_some(s)),
        lock: parse_lock(options)?,
    })
}

/// The options of `list new` and `list edit`. Only `list edit` takes `--name` and `--clear`.
fn parse_list_changes(options: &mut Options, editing: bool) -> Result<ListChanges, String> {
    let mut changes = ListChanges::default();
    let cleared = if editing { options.values("--clear")? } else { Vec::new() };
    for list in &cleared {
        if !["sites", "allowed", "programs"].contains(&list.as_str()) {
            return Err(format!("--clear takes sites, allowed or programs, not \"{list}\""));
        }
    }
    for (option, file, cleared_as, field) in [
        ("--site", "--sites-file", "sites", &mut changes.filters),
        ("--allow", "--allowed-file", "allowed", &mut changes.allowed),
        ("--program", "--programs-file", "programs", &mut changes.programs),
    ] {
        let mut entries = options.values(option)?;
        for path in options.values(file)? {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {path}: {e}"))?;
            entries.extend(text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string));
        }
        if !entries.is_empty() || cleared.iter().any(|c| c == cleared_as) {
            *field = Some(entries);
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
            let mut block = Definition { id: new_id(), name, lists: Vec::new(), schedule: None, lock: Lock::None };
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
            let lock = lock.map(|lock| find_lock_schedule(&overview, lock)).transpose()?;
            if let Some(lock) = lock.filter(|lock| *lock != block.lock) {
                block.lock = lock;
                send(Request::SaveBlock { block: block.clone() })?;
            }
            // As in the window: a block that cannot be stopped is only started after a second yes.
            if let Some(how) = lock_warning(&overview, &block) {
                let question = format!("Start \"{}\"? {how} Only the unlock password ends it early.", block.name);
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
            // A block locked with a password stops with it: asked for when it was not given.
            let password_locked = overview.running.iter().any(|r| r.id == block.id && r.password);
            let password = match password {
                None if password_locked => Some(read_password(&format!("Password of \"{}\": ", block.name)).map_err(Failure::Failed)?),
                given => given,
            };
            send(Request::Stop { id: block.id.clone(), password })?;
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
        Command::ListNew { name, changes } => {
            let mut list = BlockList { id: new_id(), name, filters: Vec::new(), allowed: Vec::new(), programs: Vec::new(), enforcers: Enforcers::default() };
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
            if !confirm(&format!("Delete the block list \"{}\"? This cannot be undone.", list.name), yes)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteList { id: list.id.clone() })?;
            println!("Deleted the block list \"{}\".", list.name);
        }
        Command::ScheduleShow { schedule } => {
            let overview = overview()?;
            let schedule = find(&overview.schedules, &schedule, "schedule", |s| (&s.id, &s.name))?;
            println!("{}", schedule.name);
            print!("{}", per_day(&schedule.week));
            println!("Used by: {}", users(&overview, &schedule.id));
        }
        Command::ScheduleNew { name, times } => {
            let week = times_of(&times)?;
            send(Request::SaveSchedule { schedule: NamedSchedule { id: new_id(), name: name.clone(), week } })?;
            println!("Made the schedule \"{name}\". Use it with: bullseye-cli new <block> --schedule \"{name}\"");
        }
        Command::ScheduleEdit { schedule, name, times } => {
            let overview = overview()?;
            let mut schedule = find(&overview.schedules, &schedule, "schedule", |s| (&s.id, &s.name))?.clone();
            if let Some(name) = name {
                schedule.name = name;
            }
            if let Some(times) = times {
                schedule.week = times_of(&times)?;
            }
            let name = schedule.name.clone();
            send(Request::SaveSchedule { schedule })?;
            println!("Saved the schedule \"{name}\".");
        }
        Command::ScheduleDelete { schedule, yes } => {
            let overview = overview()?;
            let schedule = find(&overview.schedules, &schedule, "schedule", |s| (&s.id, &s.name))?;
            if !confirm(&format!("Delete the schedule \"{}\"? This cannot be undone.", schedule.name), yes)? {
                return Err(failed("Not deleted."));
            }
            send(Request::DeleteSchedule { id: schedule.id.clone() })?;
            println!("Deleted the schedule \"{}\".", schedule.name);
        }
        Command::ScheduleCheck { times } => match Week::parse(&times).map_err(Failure::Usage)? {
            Some(week) => print!("{}", per_day(&week)),
            None => println!("Always active: a block without a schedule"),
        },
        Command::Unlock { password } => {
            let password = match password {
                Some(password) => password,
                None => read_password("Unlock password: ").map_err(Failure::Failed)?,
            };
            send(Request::Unlock { password })?;
            // The service tells the app to quit at its next policy request, within a second.
            println!("Unlocked: every block has ended, and Bullseye closes. It starts again at your next sign-in.");
        }
    }
    Ok(())
}

/// Weekly times, as a schedule takes them; "always" is not a schedule (a block without one is).
fn times_of(text: &str) -> Result<Week, Failure> {
    match Week::parse(text).map_err(failed)? {
        Some(week) => Ok(week),
        None => Err(failed("Give the times, e.g. \"Mon-Fri 08:00-17:00\"; a block that is always active needs no schedule")),
    }
}

/// `new` and `edit`'s changes, with lists and the schedule looked up by name or id.
fn apply_block_changes(overview: &Overview, block: &mut Definition, changes: BlockChanges) -> Result<(), Failure> {
    if let Some(name) = changes.name {
        block.name = name;
    }
    if let Some(lists) = changes.lists {
        block.lists = lists.iter().map(|l| find(&overview.lists, l, "block list", |l| (&l.id, &l.name)).map(|l| l.id.clone())).collect::<Result<_, _>>()?;
    }
    if let Some(schedule) = changes.schedule {
        block.schedule = match schedule {
            Some(s) => Some(find(&overview.schedules, &s, "schedule", |s| (&s.id, &s.name))?.id.clone()),
            None => None,
        };
    }
    if let Some(lock) = changes.lock {
        block.lock = find_lock_schedule(overview, lock)?;
    }
    Ok(())
}

/// A lock with its lock schedule, given by name or id, as the schedule's id.
fn find_lock_schedule(overview: &Overview, lock: Lock) -> Result<Lock, Failure> {
    match lock {
        Lock::Scheduled { schedule } => Ok(Lock::Scheduled { schedule: find(&overview.schedules, &schedule, "schedule", |s| (&s.id, &s.name))?.id.clone() }),
        other => Ok(other),
    }
}

/// What the start of a locked block asks about, as the window's confirmation; `None`: not locked.
fn lock_warning(overview: &Overview, block: &Definition) -> Option<String> {
    match &block.lock {
        Lock::None => None,
        Lock::Timer { .. } => {
            let words = duration::words(block.lock.duration().ok()??.as_secs());
            Some(format!("It cannot be stopped, changed or deleted for {words}."))
        }
        Lock::Scheduled { schedule } => {
            let schedule = overview.schedules.iter().find(|n| &n.id == schedule)?;
            Some(format!("It cannot be stopped while its lock schedule \"{}\" is active ({}).", schedule.name, schedule.week.describe()))
        }
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
    parts.push(match (running.enforcing, &running.schedule_change_words) {
        (true, None) => "blocking now".into(),
        (true, Some(change)) => format!("blocking now, inactive in {change}"),
        (false, Some(change)) => format!("inactive by its schedule, active in {change}"),
        (false, None) => "inactive by its schedule".into(),
    });
    parts.push(match (&running.remaining_words, running.locked) {
        _ if running.password => "locked with a password".into(),
        (Some(left), _) => format!("locked, {left} left"),
        (None, true) => match &running.lock_change_words {
            Some(change) => format!("locked by its lock schedule, for another {change}"),
            None => "locked by its lock schedule".into(),
        },
        (None, false) if running.lock_schedule.is_some() => match &running.lock_change_words {
            Some(change) => format!("can be stopped, locked again in {change}"),
            None => "can be stopped".into(),
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

/// The names of the blocks that use a block list or schedule (`Library::users_of`).
fn users(overview: &Overview, id: &str) -> String {
    let library = Library { blocks: overview.blocks.clone(), ..Library::default() };
    let names: Vec<&str> = library.users_of(id).iter().map(|b| b.name.as_str()).collect();
    if names.is_empty() { "(no block)".into() } else { names.join(", ") }
}

/// Everything, as the window's pages show it: the blocks, the block lists, the schedules.
fn describe_overview(overview: &Overview) -> String {
    let blocks: Vec<(String, String)> =
        overview.blocks.iter().map(|b| (b.name.clone(), state(overview.running.iter().find(|r| r.id == b.id)))).collect();
    let lists: Vec<(String, String)> = overview.lists.iter().map(|l| (l.name.clone(), format!("used by {}", users(overview, &l.id)))).collect();
    let schedules: Vec<(String, String)> = overview.schedules.iter().map(|s| (s.name.clone(), s.week.describe())).collect();
    let mut text = String::new();
    for (title, rows, empty) in [
        ("Blocks", blocks, "  None yet. Make one with: bullseye-cli new <name> --list <block list>\n"),
        ("Block lists", lists, "  None yet. Make one with: bullseye-cli list new <name> --site <site>\n"),
        ("Schedules", schedules, "  None (blocks without one are always active).\n"),
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
    if entries.is_empty() { "(none)".to_string() } else { entries.join(", ") }
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
    let schedule = match &block.schedule {
        None => "always active".to_string(),
        Some(id) => match overview.schedules.iter().find(|s| &s.id == id) {
            Some(s) => format!("{} ({})", s.name, s.week.describe()),
            None => "(gone)".into(),
        },
    };
    let lock = match &block.lock {
        Lock::None => "none: can be stopped at any time".to_string(),
        Lock::Timer { duration } => match block.lock.duration() {
            Ok(Some(d)) => format!("timer, {} (cannot be stopped for that long once started)", duration::words(d.as_secs())),
            _ => format!("timer, {duration}"),
        },
        Lock::Scheduled { schedule } => match overview.schedules.iter().find(|s| &s.id == schedule) {
            Some(s) => format!("scheduled: cannot be stopped while \"{}\" is active ({})", s.name, s.week.describe()),
            None => "scheduled: (schedule gone)".into(),
        },
        Lock::Password { .. } => "password: can only be stopped with its password".into(),
    };
    format!(
        "Name:        {}\nId:          {}\nState:       {}\nBlock lists: {}\nSchedule:    {schedule}\nLock:        {lock}\n",
        block.name,
        block.id,
        state(running),
        entries(&lists.iter().map(|l| l.to_string()).collect::<Vec<_>>()),
    )
}

/// One block list, everything about it.
fn describe_list(overview: &Overview, list: &BlockList) -> String {
    format!(
        "Name:           {}\nId:             {}\nBlock list:     {}\nAlways allowed: {}\nPrograms:       {}\nEnforcers:      {}\nUsed by:        {}\n",
        list.name,
        list.id,
        entries(&list.filters),
        entries(&list.allowed),
        entries(&list.programs),
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
        b.schedule = Some("s1".into());
        b.lock = Lock::Scheduled { schedule: "s1".into() };
        Overview { lists: vec![social], schedules: vec![work], blocks: vec![b], running: vec![], clock_synced: true }
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
        assert_eq!(names.len(), 4);
        let page = include_str!("../../src/index.html");
        for name in names {
            assert!(page.contains(&format!("key: \"{name}\"")), "{name} is not on the page");
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
            "--enforcers",
            "chromium_tabs, exe_names",
        ]);
        let Ok(Command::ListNew { name, changes }) = parse(&line) else { panic!("not a new list") };
        assert_eq!(name, "Social");
        let mut list = block_list("x", &[]);
        changes.apply(&mut list);
        assert_eq!(list.filters, vec!["youtube.com", "reddit.com"]);
        assert_eq!(list.allowed, vec!["youtube.com/watch*"]);
        assert_eq!(list.programs, vec!["steam"]);
        assert_eq!(list.enforcers, Enforcers { chromium_tabs: true, chromium_split_view: false, obsidian_web_viewer: false, exe_names: true });
    }

    #[test]
    fn a_new_block_finds_its_lists_and_schedule_by_name() {
        let line = args(&["new", "Evening", "--list", "social", "--list", "l1", "--schedule", "WORK HOURS", "--lock-timer", "2h 30m"]);
        let Ok(Command::New { name, changes }) = parse(&line) else { panic!("not new") };
        assert_eq!(name, "Evening");
        let mut b = block("x", &[]);
        assert!(apply_block_changes(&library(), &mut b, changes).is_ok());
        assert_eq!((b.lists, b.schedule, b.lock), (vec!["l1".to_string(), "l1".to_string()], Some("s1".to_string()), Lock::Timer { duration: "2h 30m".into() }));

        // `always`, and a list that does not exist.
        let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--schedule", "always", "--no-lock"])) else { panic!("not edit") };
        let mut b = library().blocks[0].clone();
        assert!(apply_block_changes(&library(), &mut b, changes).is_ok());
        assert_eq!((b.schedule, b.lock), (None, Lock::None));
        let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--list", "Games"])) else { panic!("not edit") };
        assert!(apply_block_changes(&library(), &mut library().blocks[0].clone(), changes).is_err());
    }

    #[test]
    fn start_can_set_the_lock_as_the_tile_does() {
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-schedule", "Work hours", "--yes"])),
            Ok(Command::Start { block: "Work".into(), lock: Some(Lock::Scheduled { schedule: "Work hours".into() }), yes: true })
        );
        assert_eq!(parse(&args(&["start", "Work"])), Ok(Command::Start { block: "Work".into(), lock: None, yes: false }));
        // What starting asks, per lock.
        let overview = library();
        let warning = lock_warning(&overview, &overview.blocks[0]).unwrap();
        assert!(warning.contains("while its schedule has it active (Mon-Fri 08:00-17:00)"), "{warning}");
        let timed = Definition { lock: Lock::Timer { duration: "2h5m".into() }, ..overview.blocks[0].clone() };
        assert!(lock_warning(&overview, &timed).unwrap().contains("2 hours 5 minutes 0 seconds"));
        assert_eq!(lock_warning(&overview, &Definition { lock: Lock::None, ..timed }), None);
    }

    #[test]
    fn schedules_are_made_from_times() {
        assert_eq!(
            parse(&args(&["schedule", "new", "Work hours", "weekdays 9-17"])),
            Ok(Command::ScheduleNew { name: "Work hours".into(), times: "weekdays 9-17".into() })
        );
        assert_eq!(
            parse(&args(&["schedule", "edit", "Work hours", "--times", "mon-thu 9-17"])),
            Ok(Command::ScheduleEdit { schedule: "Work hours".into(), name: None, times: Some("mon-thu 9-17".into()) })
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
        assert_eq!(changes.filters, Some(vec!["c.com".into(), "a.com".into(), "b.com".into()]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn what_is_not_understood() {
        for bad in [
            &["frobnicate"][..],
            &["new"],
            &["new", "--list", "a"],
            &["new", "A", "--lock-timer", "soon"],
            &["new", "A", "--lock-timer", ""],
            &["new", "A", "--lock-timer", "1h", "--no-lock"],
            &["new", "A", "--name", "B"],
            &["new", "A", "--site", "a.com"],
            &["list"],
            &["list", "new"],
            &["list", "new", "A", "--site"],
            &["list", "new", "A", "--enforcers", "chromium_tabs,lasers"],
            &["list", "new", "A", "--clear", "sites"],
            &["list", "edit", "A", "--clear", "everything"],
            &["list", "frob", "A"],
            &["schedule", "new", "A"],
            &["schedule", "check"],
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
        assert_eq!(parse(&args(&["stop", "Work"])), Ok(Command::Stop { block: "Work".into(), password: None }));
        assert_eq!(parse(&args(&["stop", "Work", "--password", "pw"])), Ok(Command::Stop { block: "Work".into(), password: Some("pw".into()) }));
        assert_eq!(
            parse(&args(&["start", "Work", "--lock-password", "partner"])),
            Ok(Command::Start { block: "Work".into(), lock: Some(Lock::Password { password: Some("partner".into()), hash: None }), yes: false })
        );
        assert!(parse(&args(&["start", "Work", "--lock-password", "pw", "--no-lock"])).is_err());
        assert_eq!(parse(&args(&["list", "show", "Social"])), Ok(Command::ListShow { list: "Social".into(), json: false }));
        assert_eq!(parse(&args(&["list", "delete", "Social", "--yes"])), Ok(Command::ListDelete { list: "Social".into(), yes: true }));
        assert_eq!(parse(&args(&["schedule", "show", "Work hours"])), Ok(Command::ScheduleShow { schedule: "Work hours".into() }));
        assert_eq!(parse(&args(&["schedule", "delete", "Work hours"])), Ok(Command::ScheduleDelete { schedule: "Work hours".into(), yes: false }));
        assert_eq!(parse(&args(&["schedule", "check", "weekdays 9-5pm"])), Ok(Command::ScheduleCheck { times: "weekdays 9-5pm".into() }));
        assert_eq!(parse(&args(&["unlock", "--password", "pw"])), Ok(Command::Unlock { password: Some("pw".into()) }));
    }

    #[test]
    fn things_are_found_by_id_or_name() {
        let lists = vec![block_list("a1", &[]), BlockList { name: "Games".into(), ..block_list("b2", &[]) }, BlockList { name: "games".into(), ..block_list("c3", &[]) }];
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
        let empty = Overview { lists: vec![], schedules: vec![], blocks: vec![], running: vec![], clock_synced: true };
        assert!(describe_overview(&empty).contains("Blocks:\n  None yet"));
        let mut overview = library();
        overview.running.push(Running {
            id: "b1".into(),
            name: "Work".into(),
            lists: vec![RunningList { name: "Social".into(), filters: vec!["youtube.com".into()], allowed: vec![], programs: vec![], enforcers: Enforcers::default() }],
            remaining: None,
            lock_schedule: Week::parse("mon-fri 8-17").unwrap(),
            password: false,
            locked: false,
            schedule: Week::parse("mon-fri 8-17").unwrap(),
            enforcing: false,
            schedule_change: Some(3600),
            remaining_words: None,
            schedule_change_words: Some(duration::words(3600)),
            lock_change: Some(3600),
            lock_change_words: Some(duration::words(3600)),
        });
        overview.clock_synced = false;
        let text = describe_overview(&overview);
        assert!(text.contains("  Work  Running · inactive by its schedule, active in 1 hour 0 minutes 0 seconds · can be stopped, locked again in 1 hour 0 minutes 0 seconds"), "{text}");
        assert!(text.contains("Block lists:\n  Social  used by Work"), "{text}");
        assert!(text.contains("Schedules:\n  Work hours  Mon-Fri 08:00-17:00"), "{text}");
        assert!(text.contains("Waiting for internet time"), "{text}");

        let block = describe_block(&overview, &overview.blocks[0], overview.running.first());
        assert!(block.contains("Block lists: Social") && block.contains("Schedule:    Work hours (Mon-Fri 08:00-17:00)"), "{block}");
        assert!(block.contains("Lock:        scheduled: cannot be stopped while \"Work hours\" is active"), "{block}");
        assert_eq!(find_lock_schedule(&overview, Lock::Scheduled { schedule: "work HOURS".into() }).ok(), Some(Lock::Scheduled { schedule: "s1".into() }));
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
