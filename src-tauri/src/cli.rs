//! The command line: everything the window can do, from a terminal or a script
//! (`bullseye-cli.exe`, explained in `CLI.md`). It talks to the service over the pipe, like the app
//! does, so the service checks everything done here exactly as it checks the window.
//!
//! **Keep it level with the window.** Every command the page can invoke (the list in
//! `generate_handler!` in `lib.rs`) is in `GUI_COMMANDS` with the CLI commands that do the same, and a
//! test fails when one is missing, or missing from `HELP` or `CLI.md`. Enforcer names come from
//! `Enforcers` itself, so a new enforcer is here without any change. A new list, schedule kind or
//! lock method needs an option in `parse_changes`, a line in `HELP` and in `CLI.md`.

use std::io::{BufRead, IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::block::Running;
use crate::client::Remote;
use crate::definitions::{Definition, Schedule};
use crate::duration;
use crate::lock::Lock;
use crate::proto::{Overview, Request};
use crate::schedule::Week;
use crate::state::Enforcers;

/// Each command the window's page can invoke, and the CLI commands that do the same. Empty: it is
/// about the window itself.
pub const GUI_COMMANDS: [(&str, &[&str]); 10] = [
    ("block_status", &["status", "show"]),
    ("save_block", &["new", "edit"]),
    ("delete_block", &["delete"]),
    ("start_block", &["start"]),
    ("stop_block", &["stop"]),
    ("force_exit", &["unlock"]),
    ("parse_schedule", &["schedule"]),
    ("describe_schedule", &["schedule"]),
    // A lock's length in words: what `show` says of a timer lock.
    ("describe_duration", &["show"]),
    ("window_ready", &[]),
];

const HELP: &str = "\
Bullseye command line: everything the window can do.

Usage: bullseye-cli <command> [options]
A <block> is a block's name (any case) or its id.

Commands:
  status [--json]          Every block: running or not, active by its schedule or not, time left.
  show <block> [--json]    One block's lists, enforcers, schedule and lock, and its state.
  new <name> [options]     Make a block. Options:
      --site <entry>           A site to block (repeatable). Address prefix, * as a wildcard.
      --allow <entry>          A site that is always allowed, also when blocked (repeatable).
      --program <name>         A program to close, by exe name (repeatable), e.g. steam.exe.
      --sites-file <file>      Sites to block, one per line. Likewise --allowed-file and
                               --programs-file.
      --enforcers <list>       Comma-separated, or 'all' (the default). Names:
                               {enforcers}
      --schedule <text>        When the block is active once started: 'always' (the default) or
                               a weekly schedule, e.g. \"Mon-Fri 08:00-17:00; Sat 10:00-12:00\".
      --lock-timer <duration>  Lock the block for this long once started, e.g. 2h 30m: it cannot
                               be stopped and ends by itself. --no-lock: not locked (the default).
  edit <block> [options]   Change a block that is not running: the options of new, which replace
                           what they name (a list option replaces that whole list), and
                           --name <name>, --clear <sites|allowed|programs> (repeatable).
  delete <block> [--yes]   Delete a block that is not running. Asks first, unless --yes.
  start <block> [--yes]    Start a block. A locked one asks first, unless --yes.
  stop <block>             Stop a running block that is not locked.
  unlock [--password <pw>] End every running block with the unlock password, and close the app,
                           as the window's Unlock does. Asks for the password when not given (or
                           reads standard input when that is not a terminal).
  schedule <text>          Check a weekly schedule and show it per day, as --schedule takes it.
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
    New { name: String, changes: Changes },
    Edit { block: String, changes: Changes },
    Delete { block: String, yes: bool },
    Start { block: String, yes: bool },
    Stop { block: String },
    Unlock { password: Option<String> },
    Schedule { text: String },
}

/// What `new` and `edit` set; `None` leaves it as it is (or as a new block has it).
#[derive(Debug, PartialEq, Default)]
struct Changes {
    name: Option<String>,
    filters: Option<Vec<String>>,
    allowed: Option<Vec<String>>,
    programs: Option<Vec<String>>,
    enforcers: Option<Enforcers>,
    schedule: Option<Schedule>,
    lock: Option<Lock>,
}

impl Changes {
    fn apply(self, block: &mut Definition) {
        if let Some(name) = self.name {
            block.name = name;
        }
        if let Some(filters) = self.filters {
            block.filters = filters;
        }
        if let Some(allowed) = self.allowed {
            block.allowed = allowed;
        }
        if let Some(programs) = self.programs {
            block.programs = programs;
        }
        if let Some(enforcers) = self.enforcers {
            block.enforcers = enforcers;
        }
        if let Some(schedule) = self.schedule {
            block.schedule = schedule;
        }
        if let Some(lock) = self.lock {
            block.lock = lock;
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
    let mut options = Options::new(rest);
    let block = |options: &mut Options| options.positional().ok_or_else(|| format!("Say which block to {command}: its name or its id"));
    let parsed = match command.as_str() {
        "help" | "--help" | "-h" | "/?" => Command::Help,
        "version" | "--version" | "-V" => Command::Version,
        "status" => Command::Status { json: options.flag("--json") },
        "show" => Command::Show { block: block(&mut options)?, json: options.flag("--json") },
        "new" => {
            let name = options.positional().ok_or("Give the new block a name")?;
            Command::New { name, changes: parse_changes(&mut options, false)? }
        }
        "edit" => Command::Edit { block: block(&mut options)?, changes: parse_changes(&mut options, true)? },
        "delete" => Command::Delete { block: block(&mut options)?, yes: options.flag("--yes") },
        "start" => Command::Start { block: block(&mut options)?, yes: options.flag("--yes") },
        "stop" => Command::Stop { block: block(&mut options)? },
        "unlock" => Command::Unlock { password: options.value("--password")? },
        "schedule" => Command::Schedule { text: options.positional().ok_or("Give the schedule to check, e.g. \"Mon-Fri 08:00-17:00\"")? },
        other => return Err(format!("Unknown command \"{other}\"")),
    };
    options.finish()?;
    Ok(parsed)
}

/// The options of `new` and `edit`. Only `edit` takes `--name` and `--clear`.
fn parse_changes(options: &mut Options, editing: bool) -> Result<Changes, String> {
    let mut changes = Changes::default();
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
    changes.schedule = match options.value("--schedule")? {
        Some(text) => Some(match Week::parse(&text)? {
            Some(week) => Schedule::Weekly(week),
            None => Schedule::Always,
        }),
        None => None,
    };
    let timer = options.value("--lock-timer")?;
    let unlocked = options.flag("--no-lock");
    changes.lock = match (timer, unlocked) {
        (Some(_), true) => return Err("Give --lock-timer or --no-lock, not both".into()),
        (Some(duration), false) => Some(Lock::Timer { duration }),
        (None, true) => Some(Lock::None),
        (None, false) => None,
    };
    // The service checks it again; this is for a clear message before anything is sent.
    if let Some(lock) = &changes.lock {
        lock.duration()?;
    }
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
    /// first: `edit Work --site a.com`.
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
        Command::Schedule { text } => match Week::parse(&text).map_err(Failure::Usage)? {
            None => println!("Always active"),
            Some(week) => print!("{}", per_day(&week)),
        },
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
            let definition = find(&overview, &block)?;
            let running = overview.running.iter().find(|r| r.id == definition.id);
            if json {
                let both = serde_json::json!({ "block": definition, "running": running });
                println!("{}", serde_json::to_string_pretty(&both).map_err(|e| failed(e.to_string()))?);
            } else {
                print!("{}", describe_block(definition, running));
            }
        }
        Command::New { name, changes } => {
            let mut block = Definition {
                id: new_id(),
                name,
                filters: Vec::new(),
                allowed: Vec::new(),
                programs: Vec::new(),
                enforcers: Enforcers::default(),
                schedule: Schedule::Always,
                lock: Lock::None,
            };
            changes.apply(&mut block);
            let (id, name) = (block.id.clone(), block.name.clone());
            send(Request::Save { block })?;
            println!("Made \"{name}\" (id {id}). Start it with: bullseye-cli start \"{name}\"");
        }
        Command::Edit { block, changes } => {
            let overview = overview()?;
            let mut block = find(&overview, &block)?.clone();
            changes.apply(&mut block);
            let name = block.name.clone();
            send(Request::Save { block })?;
            println!("Saved \"{name}\".");
        }
        Command::Delete { block, yes } => {
            let overview = overview()?;
            let block = find(&overview, &block)?;
            if !confirm(&format!("Delete \"{}\"? This cannot be undone.", block.name), yes)? {
                return Err(failed("Not deleted."));
            }
            send(Request::Delete { id: block.id.clone() })?;
            println!("Deleted \"{}\".", block.name);
        }
        Command::Start { block, yes } => {
            let overview = overview()?;
            let block = find(&overview, &block)?;
            // As in the window: a block that cannot be stopped is only started after a second yes.
            if let Ok(Some(locked)) = block.lock.duration() {
                let words = duration::words(locked.as_secs());
                let question = format!(
                    "Start \"{}\" for {words}? It cannot be stopped, changed or deleted for {words}; only the unlock password ends it early.",
                    block.name
                );
                if !confirm(&question, yes)? {
                    return Err(failed("Not started."));
                }
            }
            send(Request::Start { id: block.id.clone() })?;
            println!("Started \"{}\".", block.name);
        }
        Command::Stop { block } => {
            let overview = overview()?;
            let (id, name) = match find(&overview, &block) {
                Ok(definition) => (definition.id.clone(), definition.name.clone()),
                // A running block whose definition is gone can still be stopped, by its id.
                Err(_) => match overview.running.iter().find(|r| r.id == block) {
                    Some(running) => (running.id.clone(), running.name.clone()),
                    None => return Err(failed(format!("No block called \"{block}\""))),
                },
            };
            send(Request::Stop { id })?;
            println!("Stopped \"{name}\".");
        }
        Command::Unlock { password } => {
            let password = match password {
                Some(password) => password,
                None => read_password().map_err(Failure::Failed)?,
            };
            send(Request::Unlock { password })?;
            // The service tells the app to quit at its next policy request, within a second.
            println!("Unlocked: every block has ended, and Bullseye closes. It starts again at your next sign-in.");
        }
    }
    Ok(())
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

/// A block by its id, or else by its name, ignoring case.
fn find<'a>(overview: &'a Overview, wanted: &str) -> Result<&'a Definition, Failure> {
    if let Some(block) = overview.blocks.iter().find(|b| b.id == wanted) {
        return Ok(block);
    }
    let named: Vec<&Definition> = overview.blocks.iter().filter(|b| b.name.trim().eq_ignore_ascii_case(wanted.trim())).collect();
    match named.as_slice() {
        [block] => Ok(block),
        [] => Err(failed(format!("No block called \"{wanted}\"; `bullseye-cli status` lists them"))),
        _ => Err(failed(format!(
            "Several blocks are called \"{wanted}\"; use the id instead: {}",
            named.iter().map(|b| b.id.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// An id for a new block: letters, digits and `-`, as `Definition::check` wants, and different
/// from any other.
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

/// The unlock password, typed without showing it; from standard input as it is when that is not a
/// terminal (`echo pw | bullseye-cli unlock`).
fn read_password() -> Result<String, String> {
    use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_ECHO_INPUT, STD_INPUT_HANDLE};

    let terminal = std::io::stdin().is_terminal();
    let mut mode = 0;
    // SAFETY: plain console calls on this process's own input handle, with a valid place to write.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let hidden = terminal && unsafe { GetConsoleMode(handle, &mut mode) } != 0;
    if terminal {
        eprint!("Unlock password: ");
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
    parts.push(match &running.remaining_words {
        Some(left) => format!("locked, {left} left"),
        None => "not locked".into(),
    });
    parts.join(" · ")
}

/// Every block, one per line, as the blocks page shows them.
fn describe_overview(overview: &Overview) -> String {
    let mut rows: Vec<(String, String)> = overview
        .blocks
        .iter()
        .map(|b| (b.name.clone(), state(overview.running.iter().find(|r| r.id == b.id))))
        .collect();
    // A running block whose definition is gone (from an older version) is shown too.
    for running in overview.running.iter().filter(|r| !overview.blocks.iter().any(|b| b.id == r.id)) {
        let name = if running.name.is_empty() { format!("Earlier block ({})", running.id) } else { running.name.clone() };
        rows.push((name, state(Some(running))));
    }
    if rows.is_empty() {
        return "No blocks yet. Make one with: bullseye-cli new <name> --site <site>\n".into();
    }
    let width = rows.iter().map(|(name, _)| name.chars().count()).max().unwrap_or(0);
    let mut text: String = rows.iter().map(|(name, state)| format!("{name:width$}  {state}\n")).collect();
    if !overview.clock_synced {
        text.push_str("(Waiting for internet time: until then, time with the service not running does not count.)\n");
    }
    text
}

/// One block, everything about it.
fn describe_block(block: &Definition, running: Option<&Running>) -> String {
    let list = |entries: &[String]| if entries.is_empty() { "(none)".to_string() } else { entries.join(", ") };
    let enforcers = match serde_json::to_value(block.enforcers) {
        Ok(Value::Object(fields)) => fields.iter().filter(|(_, on)| **on == Value::Bool(true)).map(|(name, _)| name.clone()).collect(),
        _ => Vec::new(),
    };
    let schedule = match block.schedule.week() {
        Ok(Some(week)) => week.describe(),
        Ok(None) => "always active".into(),
        Err(why) => why,
    };
    let lock = match &block.lock {
        Lock::None => "none: can be stopped at any time".to_string(),
        Lock::Timer { duration } => match block.lock.duration() {
            Ok(Some(d)) => format!("timer, {} (cannot be stopped for that long once started)", duration::words(d.as_secs())),
            _ => format!("timer, {duration}"),
        },
    };
    format!(
        "Name:           {}\nId:             {}\nState:          {}\nBlock list:     {}\nAlways allowed: {}\nPrograms:       {}\nEnforcers:      {}\nSchedule:       {schedule}\nLock:           {lock}\n",
        block.name,
        block.id,
        state(running),
        list(&block.filters),
        list(&block.allowed),
        list(&block.programs),
        list(&enforcers),
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

    fn args(line: &[&str]) -> Vec<String> {
        line.iter().map(|arg| arg.to_string()).collect()
    }

    fn defined(id: &str, name: &str) -> Definition {
        Definition { name: name.into(), ..crate::definitions::tests::definition(id, &["a.com"]) }
    }

    /// Every command the window can invoke has CLI commands that do the same, in the help and in
    /// `CLI.md`. When this fails, the window has learned something the CLI cannot do yet.
    #[test]
    fn the_cli_can_do_whatever_the_window_can() {
        let lib = include_str!("lib.rs");
        let start = lib.find("generate_handler![").expect("no generate_handler in lib.rs") + "generate_handler![".len();
        let list = &lib[start..start + lib[start..].find(']').unwrap()];
        let commands: Vec<&str> = list.split(',').map(|c| c.trim().rsplit("::").next().unwrap()).filter(|c| !c.is_empty()).collect();
        assert!(commands.len() >= 8, "{commands:?}");
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
    fn a_new_block_with_everything() {
        let line = args(&[
            "new",
            "Work hours",
            "--site",
            "youtube.com",
            "--site=reddit.com",
            "--allow",
            "youtube.com/watch*",
            "--program",
            "steam",
            "--enforcers",
            "chromium_tabs, exe_names",
            "--schedule",
            "mon-fri 8-17",
            "--lock-timer",
            "2h 30m",
        ]);
        let Ok(Command::New { name, changes }) = parse(&line) else { panic!("not a new block") };
        assert_eq!(name, "Work hours");
        let mut block = defined("x", "x");
        changes.apply(&mut block);
        assert_eq!(block.name, "x", "new takes the name as its argument, not --name");
        assert_eq!(block.filters, vec!["youtube.com", "reddit.com"]);
        assert_eq!(block.allowed, vec!["youtube.com/watch*"]);
        assert_eq!(block.programs, vec!["steam"]);
        assert_eq!(block.enforcers, Enforcers { chromium_tabs: true, chromium_split_view: false, obsidian_web_viewer: false, exe_names: true });
        assert_eq!(block.schedule.week().unwrap().unwrap().describe(), "Mon-Fri 08:00-17:00");
        assert_eq!(block.lock, Lock::Timer { duration: "2h 30m".into() });
    }

    #[test]
    fn a_new_block_has_every_enforcer_and_an_edit_changes_only_what_it_names() {
        let Ok(Command::New { changes, .. }) = parse(&args(&["new", "A"])) else { panic!("not new") };
        assert_eq!(changes.enforcers, Some(parse_enforcers("all").unwrap()));
        assert!(changes.filters.is_none() && changes.schedule.is_none() && changes.lock.is_none());

        let Ok(Command::Edit { block, changes }) = parse(&args(&["edit", "Work", "--name", "Work days", "--clear", "allowed", "--schedule", "always", "--no-lock"]))
        else {
            panic!("not an edit")
        };
        assert_eq!(block, "Work");
        let mut work = defined("w", "Work");
        work.allowed = vec!["google.com".into()];
        work.lock = Lock::Timer { duration: "1h".into() };
        work.schedule = Schedule::Weekly(Week::parse("weekends").unwrap().unwrap());
        changes.apply(&mut work);
        assert_eq!(work.name, "Work days");
        assert_eq!(work.filters, vec!["a.com"], "not named, so kept");
        assert!(work.allowed.is_empty());
        assert_eq!((work.schedule, work.lock), (Schedule::Always, Lock::None));
    }

    #[test]
    fn lists_come_from_files_too() {
        let path = std::env::temp_dir().join(format!("cli-{}-sites.txt", std::process::id()));
        std::fs::write(&path, "a.com\r\n\r\nb.com\n").unwrap();
        let line = args(&["new", "A", "--sites-file", path.to_str().unwrap(), "--site", "c.com"]);
        let Ok(Command::New { changes, .. }) = parse(&line) else { panic!("not new") };
        assert_eq!(changes.filters, Some(vec!["c.com".into(), "a.com".into(), "b.com".into()]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn what_is_not_understood() {
        for bad in [
            &["frobnicate"][..],
            &["new"],
            &["new", "--site", "a.com"],
            &["new", "A", "--site"],
            &["new", "A", "--lock-timer", "soon"],
            &["new", "A", "--lock-timer", ""],
            &["new", "A", "--lock-timer", "1h", "--no-lock"],
            &["new", "A", "--schedule", "someday"],
            &["new", "A", "--enforcers", "chromium_tabs,lasers"],
            &["new", "A", "--name", "B"],
            &["new", "A", "--clear", "sites"],
            &["edit", "A", "--clear", "everything"],
            &["edit"],
            &["start"],
            &["start", "A", "B"],
            &["stop", "A", "now"],
            &["schedule"],
            &["new", "A", "--sites-file", "Z:/no/such/file.txt"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_other_commands() {
        assert_eq!(parse(&[]), Ok(Command::Help));
        assert_eq!(parse(&args(&["help"])), Ok(Command::Help));
        assert_eq!(parse(&args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse(&args(&["status", "--json"])), Ok(Command::Status { json: true }));
        assert_eq!(parse(&args(&["show", "Work", "--json"])), Ok(Command::Show { block: "Work".into(), json: true }));
        assert_eq!(parse(&args(&["start", "Work"])), Ok(Command::Start { block: "Work".into(), yes: false }));
        assert_eq!(parse(&args(&["start", "Work", "--yes"])), Ok(Command::Start { block: "Work".into(), yes: true }));
        assert_eq!(parse(&args(&["delete", "Work", "--yes"])), Ok(Command::Delete { block: "Work".into(), yes: true }));
        assert_eq!(parse(&args(&["stop", "Work"])), Ok(Command::Stop { block: "Work".into() }));
        assert_eq!(parse(&args(&["unlock"])), Ok(Command::Unlock { password: None }));
        assert_eq!(parse(&args(&["unlock", "--password", "pw"])), Ok(Command::Unlock { password: Some("pw".into()) }));
        assert_eq!(parse(&args(&["schedule", "weekdays 9-5pm"])), Ok(Command::Schedule { text: "weekdays 9-5pm".into() }));
    }

    #[test]
    fn blocks_are_found_by_id_or_name() {
        let overview = Overview { blocks: vec![defined("a1", "Work"), defined("b2", "Games"), defined("c3", "games")], running: vec![], clock_synced: true };
        assert_eq!(find(&overview, "a1").ok().map(|b| b.id.as_str()), Some("a1"));
        assert_eq!(find(&overview, "work").ok().map(|b| b.id.as_str()), Some("a1"));
        assert!(matches!(find(&overview, "Games"), Err(Failure::Failed(why)) if why.contains("b2, c3")));
        assert!(find(&overview, "Nope").is_err());
        assert!(new_id().chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && new_id().len() <= 64);
    }

    #[test]
    fn the_status_reads_like_the_blocks_page() {
        let empty = Overview { blocks: vec![], running: vec![], clock_synced: true };
        assert!(describe_overview(&empty).starts_with("No blocks yet"));
        let running = Running {
            id: "a1".into(),
            name: "Work".into(),
            remaining: Some(2 * 3600 + 61),
            filters: vec!["a.com".into()],
            allowed: vec![],
            programs: vec![],
            enforcers: Enforcers::default(),
            schedule: Week::parse("mon-fri 8-17").unwrap(),
            enforcing: false,
            schedule_change: Some(3600),
            remaining_words: Some(duration::words(2 * 3600 + 61)),
            schedule_change_words: Some(duration::words(3600)),
        };
        let overview = Overview { blocks: vec![defined("a1", "Work"), defined("b2", "Evenings")], running: vec![running.clone()], clock_synced: false };
        let text = describe_overview(&overview);
        assert!(text.contains("Work      Running · inactive by its schedule, active in 1 hour 0 minutes 0 seconds · locked, 2 hours 1 minute 1 second left"), "{text}");
        assert!(text.contains("Evenings  Not running"), "{text}");
        assert!(text.contains("Waiting for internet time"), "{text}");

        let mut work = defined("a1", "Work");
        work.schedule = Schedule::Weekly(Week::parse("mon-fri 8-17").unwrap().unwrap());
        work.lock = Lock::Timer { duration: "2h5m".into() };
        let text = describe_block(&work, Some(&running));
        assert!(text.contains("Schedule:       Mon-Fri 08:00-17:00"), "{text}");
        assert!(text.contains("Lock:           timer, 2 hours 5 minutes 0 seconds"), "{text}");
        assert!(text.contains("Always allowed: (none)"), "{text}");
    }

    #[test]
    fn a_schedule_is_shown_per_day() {
        let text = per_day(&Week::parse("mon-fri 8-12, 13-17; sat 10-12").unwrap().unwrap());
        assert!(text.starts_with("Mon-Fri 08:00-12:00, 13:00-17:00; Sat 10:00-12:00\n"), "{text}");
        assert!(text.contains("  Tue  08:00-12:00, 13:00-17:00\n"), "{text}");
        assert!(text.contains("  Sun  inactive\n"), "{text}");
    }
}
