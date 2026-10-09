//! What a command line says: the commands and their options, read into a `Command`, and `HELP`.

use super::*;

/// Where the descriptions start in `HELP`, and how wide a line of it may be.
pub(super) const HELP_COLUMN: usize = 36;
pub(super) const HELP_WIDTH: usize = 100;

pub(super) const HELP: &str = "\
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
pub(super) enum Command {
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
pub(super) enum RuleChoice {
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
pub(super) struct SessionChanges {
    pub(super) name: Option<String>,
    pub(super) targets: Option<Vec<String>>,
    /// `Some(None)`: always.
    pub(super) condition: Option<Option<String>>,
    /// What `--allowance` and its options set; `None`: they were not given.
    pub(super) allowance: Option<AllowanceChanges>,
    pub(super) lock: Option<Lock>,
}

/// What `--allowance` and its options of `new` and `edit` set on a session's allowance; `None` leaves
/// it (with no allowance yet, the default). Read when the command runs (`build_terms`).
#[derive(Debug, PartialEq, Default)]
pub(super) struct AllowanceChanges {
    /// `budget`, `pomodoro`, `delayed` or `none`.
    pub(super) kind: Option<String>,
    pub(super) allowed: Option<String>,
    pub(super) scope: Option<Scope>,
    pub(super) reset: Option<Reset>,
    /// With `Reset::Every`: how often, as typed (`1d`).
    pub(super) every: Option<String>,
    pub(super) blocked: Option<String>,
    pub(super) anchor: Option<Anchor>,
    /// A delayed start: how long after the session starts, or the date and time, enforcing begins.
    pub(super) after: Option<String>,
    pub(super) from: Option<String>,
}

/// What `target new` and `target edit` set on a target; `None` leaves it.
#[derive(Debug, PartialEq, Default)]
pub(super) struct TargetChanges {
    pub(super) name: Option<String>,
    /// The lists to set, by their field of `Entries`.
    pub(super) lists: BTreeMap<&'static str, Vec<String>>,
    pub(super) actions: Option<Actions>,
}

/// The options that set a list, one per list: the option (its items; `<option>-file` reads them
/// from a file, one per line, and `--clear` names it without the dashes) and its field of
/// `Entries`. **A new list is a line here.**
pub(super) const LIST_OPTIONS: [(&str, &str); 6] = [
    ("--sites", "filters"),
    ("--allowed-sites", "allowed"),
    ("--programs", "programs"),
    ("--store-apps", "store_apps"),
    ("--titles", "titles"),
    ("--allowed-titles", "allowed_titles"),
];

impl TargetChanges {
    pub(super) fn apply(self, list: &mut Target) {
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

pub(super) fn parse(args: &[String]) -> Result<Command, String> {
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
        other => return Err(format!("Unknown command “{other}”")),
    };
    options.finish()?;
    Ok(parsed)
}

/// `--lock-duration <duration>`, `--lock-until <date time>`, `--lock-until-restart`,
/// `--lock-condition <condition>`, `--lock-password <pw>`, `--lock-text <text>` or `--no-lock`, at most one. A lock
/// condition by name or id, looked up when the command runs (`resolve_lock`).
pub(super) fn parse_lock(options: &mut Options) -> Result<Option<Lock>, String> {
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
pub(super) fn parse_way(options: &mut Options) -> Result<Option<Option<Lock>>, String> {
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
pub(super) fn parse_session_changes(options: &mut Options, editing: bool) -> Result<SessionChanges, String> {
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
pub(super) fn parse_allowance_changes(options: &mut Options) -> Result<Option<AllowanceChanges>, String> {
    let kind = options.value("--allowance")?.map(|kind| kind.to_lowercase());
    if let Some(kind) = &kind {
        if !["budget", "pomodoro", "delayed", "none"].contains(&kind.as_str()) {
            return Err(format!("--allowance takes budget, pomodoro, delayed or none, not “{kind}”"));
        }
    }
    let allowed = options.value("--allowed")?;
    let scope = options
        .value("--scope")?
        .map(|scope| match scope.to_lowercase().as_str() {
            "total" => Ok(Scope::Total),
            "each" => Ok(Scope::Each),
            other => Err(format!("--scope takes total or each, not “{other}”")),
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
                _ => return Err(format!("--reset takes never, condition or a duration such as 1d, not “{reset}”")),
            },
        },
    };
    let blocked = options.value("--blocked")?;
    let anchor = options
        .value("--anchor")?
        .map(|anchor| match anchor.to_lowercase().as_str() {
            "start" => Ok(Anchor::Start),
            "clock" => Ok(Anchor::Clock),
            other => Err(format!("--anchor takes start or clock, not “{other}”")),
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
pub(super) fn build_terms(changes: AllowanceChanges, old: Option<&Terms>) -> Result<Terms, String> {
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
pub(super) fn parse_rule(options: &mut Options) -> Result<Option<RuleChoice>, String> {
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
pub(super) fn parse_webhook(options: &mut Options) -> Result<Option<Hook>, String> {
    let Some(url) = options.value("--webhook")? else { return Ok(None) };
    let method = match options.value("--method")?.map(|m| m.to_lowercase()).as_deref() {
        None | Some("get") => Method::Get,
        Some("post") => Method::Post,
        Some(other) => return Err(format!("--method is get or post, not “{other}”")),
    };
    let interval = match options.value("--every")? {
        Some(text) => duration::parse(&text)?.ok_or("Give the time between requests, e.g. 5m")?.as_secs(),
        None => 60,
    };
    let fallback = match options.value("--fallback")?.map(|f| f.to_lowercase()).as_deref() {
        None | Some("active") => true,
        Some("inactive") => false,
        Some(other) => return Err(format!("--fallback is active or inactive, not “{other}”")),
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
pub(super) fn parse_target_changes(options: &mut Options, editing: bool) -> Result<TargetChanges, String> {
    let mut changes = TargetChanges::default();
    let cleared = if editing { options.items("--clear")?.unwrap_or_default() } else { Vec::new() };
    let names: Vec<&str> = LIST_OPTIONS.iter().map(|(option, _)| &option[2..]).collect();
    for list in &cleared {
        if !names.contains(&list.as_str()) {
            return Err(format!("--clear takes {}, not “{list}”", names.join(", ")));
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
pub(super) fn parse_list_entries(options: &mut Options, cleared: &[String]) -> Result<BTreeMap<&'static str, Vec<String>>, String> {
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
pub(super) fn help() -> String {
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

pub(super) fn action_names() -> Vec<String> {
    match serde_json::to_value(Actions::default()) {
        Ok(Value::Object(fields)) => fields.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// The actions named (`chromium_tabs firefox_tabs`), or `all`: every action as a new target has them (windows by
/// title closed, not minimized: `window_titles_minimize` says to minimize them).
pub(super) fn parse_actions(given: &[String]) -> Result<Actions, String> {
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
            return Err(format!("Unknown action “{name}”; the actions are {}", names.join(", ")));
        }
    }
    serde_json::from_value(Value::Object(chosen)).map_err(|e| e.to_string())
}

/// The arguments after the command, taken out one kind at a time; whatever is left is an error.
pub(super) struct Options {
    pub(super) args: Vec<String>,
}

impl Options {
    pub(super) fn new(args: &[String]) -> Options {
        Options { args: args.to_vec() }
    }

    pub(super) fn flag(&mut self, name: &str) -> bool {
        let before = self.args.len();
        self.args.retain(|arg| arg != name);
        self.args.len() != before
    }

    /// Every `--name value` and `--name=value`.
    pub(super) fn values(&mut self, name: &str) -> Result<Vec<String>, String> {
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
    pub(super) fn items(&mut self, name: &str) -> Result<Option<Vec<String>>, String> {
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
    pub(super) fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        let mut values = self.values(name)?;
        if values.len() > 1 {
            return Err(format!("{name} is given more than once"));
        }
        Ok(values.pop())
    }

    /// The first argument that is not an option. Taken before the options are, so it must come
    /// first: `edit Work --list Social`.
    pub(super) fn positional(&mut self) -> Option<String> {
        match self.args.first() {
            Some(arg) if !arg.starts_with("--") => Some(self.args.remove(0)),
            _ => None,
        }
    }

    pub(super) fn finish(self) -> Result<(), String> {
        match self.args.first() {
            None => Ok(()),
            Some(arg) => Err(format!("Not understood: “{arg}”")),
        }
    }
}
