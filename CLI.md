# Bullseye command line

`bullseye-cli.exe` does everything the window does, from a terminal or a script: list the blocks, make, change and delete them, start and stop them, and end them all with the unlock password. The installer puts it next to the app, in `C:\Program Files\Bullseye\`. To call it by name from anywhere, add that folder to your `PATH` (Settings > System > About > Advanced system settings > Environment Variables), or call it by its full path:

```powershell
& "C:\Program Files\Bullseye\bullseye-cli.exe" status
```

It talks to the Bullseye service over the same pipe the app uses, so the service checks everything done here exactly as it checks the window, and what you do here shows up in the window within a second (and the other way round). If the service is installed but not running, the CLI starts it first, as the app would. It needs no administrator rights.

A `<block>` is a block's name (any case) or its id; `bullseye-cli status` lists the names and `bullseye-cli show` gives the id. When two blocks have the same name, use the id.

## Commands

| Command | Does what the window does with |
|---|---|
| `bullseye-cli status [--json]` | the blocks page: every block, running or not, and how |
| `bullseye-cli show <block> [--json]` | a block's tile and its editor: everything about one block |
| `bullseye-cli new <name> [options]` | **New block**, then **Save** |
| `bullseye-cli edit <block> [options]` | **Edit**, then **Save** |
| `bullseye-cli delete <block> [--yes]` | **Delete** (asks first, unless `--yes`) |
| `bullseye-cli start <block> [--yes]` | **Start** (a locked block asks first, unless `--yes`) |
| `bullseye-cli stop <block>` | **Stop** |
| `bullseye-cli unlock [--password <pw>]` | **Unlock** in Settings |
| `bullseye-cli schedule <text>` | the Times field of a weekly schedule |
| `bullseye-cli help` | |
| `bullseye-cli version` | |

Exit codes: `0` done, `1` failed, refused or cancelled (the reason is printed), `2` not understood.

## A block's three layers

A block is, in the same order as in the window's editor:

1. **What it enforces**: its lists (sites to block, always-allowed sites, programs) and its enforcers.
2. **When it is active**: its schedule, `always` or a weekly schedule.
3. **How it is locked**: its lock method, none or a timer.

Outside its schedule a running block keeps running and stays locked, but enforces nothing. For example, with a schedule of `daily 08:00-17:00`, a block started at 16:00 with a 2-hour timer lock blocks until 17:00; from then on everything is accessible again, and at 18:00 the lock ends the block. With a lock of 20 hours instead, it would block again the next morning from 08:00 until the lock ends at 12:00.

### `new` and `edit`

`new <name>` makes a block; `edit <block>` changes one that is not running (a running block keeps what it was started with, as in the window). Options:

| Option | Meaning | New block's default |
|---|---|---|
| `--site <entry>` | A site to block (repeatable). An address prefix with `*` as a wildcard, as in the window: `youtube.com`, `*.reddit.com`, `*` (every site). | none |
| `--allow <entry>` | A site that is always allowed, also when the block list matches it (repeatable). | none |
| `--program <name>` | A program to close, by exe name (repeatable): `steam`, `steam*.exe`. | none |
| `--sites-file <file>`, `--allowed-file <file>`, `--programs-file <file>` | The same, one entry per line, from a text file. Can be combined with the options above. | |
| `--enforcers <list>` | Which enforcers run, comma-separated: `chromium_tabs` (close blocked tabs in Chrome, Edge, Brave), `chromium_split_view` (close blocked pages in split view), `exe_names` (close the programs), `obsidian_web_viewer` (close Obsidian when it shows web pages), or `all`. `bullseye-cli help` lists the current names. | `all` |
| `--schedule <text>` | When the block is active once started: `always`, or a weekly schedule (see below). | `always` |
| `--lock-timer <duration>` | Lock the block for this long once started: it cannot be stopped, changed or deleted, and it ends by itself when the time is up. | |
| `--no-lock` | Not locked: it can be stopped at any time. | yes |

Only `edit` takes these two:

| Option | Meaning |
|---|---|
| `--name <name>` | A new name. |
| `--clear <list>` | Empty a list: `sites`, `allowed` or `programs` (repeatable). |

With `edit`, an option replaces only what it names: `--site` replaces the whole block list with the sites given, and lists, enforcers, schedule and lock that are not named stay as they are. Every option can also be written as `--option=value`. The block's name or id comes right after the command.

```powershell
# Block YouTube and Reddit; not locked, always active.
bullseye-cli new "Distractions" --site youtube.com --site reddit.com

# Everything but Google and Wikipedia, on weekdays from 8 to 5, locked for 2 hours 30 minutes once started.
bullseye-cli new "Work hours" --site "*" --allow google.com --allow "*.wikipedia.org" `
    --schedule "Mon-Fri 08:00-17:00" --lock-timer "2h 30m"

# Close Steam in the evenings, from a list in a file, with only the programs enforcer.
bullseye-cli new "Evenings" --programs-file games.txt --enforcers exe_names --schedule "daily 19:00-23:00"

# Later: no more lock, and Saturdays too.
bullseye-cli edit "Work hours" --no-lock --schedule "Mon-Fri 08:00-17:00; Sat 10:00-14:00"
```

### `start`, `stop`, `delete`

`start <block>` starts a block as it is defined now. The same rules as in the window apply, checked by the service: at least one enforcer, something for it to do, valid entries, and nothing that would close programs Windows or Bullseye needs. A block with a timer lock cannot be stopped once started, so, like the window, the CLI asks first (`[y/N]`); `--yes` answers yes beforehand, which is also needed when there is no terminal to ask in (a script).

`stop <block>` stops a running block that is not locked; a locked one is refused, and the message says how long it is still locked. `delete <block>` deletes a block that is not running, after asking (or with `--yes`).

### `status` and `show`

```
> bullseye-cli status
Work hours  Running · inactive by its schedule, active in 15 hours 0 minutes 0 seconds · locked, 1 hour 2 minutes 3 seconds left
Games       Not running
```

```
> bullseye-cli show "Work hours"
Name:           Work hours
Id:             0f6c3c1e-…
State:          Running · inactive by its schedule, active in 15 hours 0 minutes 0 seconds · locked, 1 hour 2 minutes 3 seconds left
Block list:     *
Always allowed: google.com, *.wikipedia.org
Programs:       (none)
Enforcers:      chromium_split_view, chromium_tabs, exe_names, obsidian_web_viewer
Schedule:       Mon-Fri 08:00-17:00
Lock:           timer, 2 hours 30 minutes 0 seconds (cannot be stopped for that long once started)
```

With `--json`, `status` prints what the service says, for scripts: `blocks` (every definition, with `schedule` as `{"kind":"always"}` or `{"kind":"weekly","days":[…]}` and `lock` as `{"method":"none"}` or `{"method":"timer","duration":"2h 30m"}`), `running` (each running block with `remaining`, the seconds left of its lock or `null`; `schedule`; `enforcing`, whether it blocks anything right now; and `schedule_change`, the seconds until its schedule turns it on or off) and `clock_synced`. `show --json` prints `{"block": …, "running": … or null}`. A weekly schedule's `days` are Monday first, each a list of `[start, end)` spans in minutes since midnight.

### `unlock`

Ends every running block, locked or not, with the unlock password; the definitions stay. Wrong guesses are slowed down after three, as in the window. Without `--password` it asks for the password without showing it, or reads it from standard input when that is not a terminal:

```powershell
bullseye-cli unlock
"my password" | bullseye-cli unlock
```

Unlike the window's Unlock, this does not close the tray app: it stays, with nothing to enforce. (A password given with `--password` ends up in your shell history.)

### `schedule`

Checks a weekly schedule written as text and shows it per day, the way `--schedule` and the window's Times field read it:

```
> bullseye-cli schedule "weekdays 8-12, 13-17; sat 10am-noon"
In "sat 10am-noon": "noon" is not a time such as 08:00, 17:30 or 5pm
> bullseye-cli schedule "weekdays 8-12, 13-17; sat 10am-12pm"
Mon-Fri 08:00-12:00, 13:00-17:00; Sat 10:00-12:00
  Mon  08:00-12:00, 13:00-17:00
  ...
  Sat  10:00-12:00
  Sun  inactive
```

## Weekly schedules

A schedule is a list of rules separated by `;` (or new lines). Each rule is a set of days, then one or more time spans separated by commas or spaces:

- **Days**: `mon` (or `monday`, `mo`), a range `mon-fri` (also round the end of the week: `fri-mon`), a list `mon,wed,fri`, or `daily`, `weekdays`, `weekends`. A rule without days is every day.
- **Times**: `8`, `8:30`, `08:30`, `17:00`, `24:00`, `8am`, `5:30pm`. A span ends just before its end time. One that ends before it starts runs past midnight into the next day: `fri 22:00-02:00` is Friday 22:00 to Saturday 02:00. A rule without times is the whole day.

Examples: `Mon-Fri 08:00-17:00`, `weekdays 9-12, 13-17; sat 10-12`, `daily 21:00-07:00`, `weekends`. A schedule with no time in it is refused: the block would never be active.

A running block reads its schedule in the time zone Windows had when the block started, and keeps that zone until it ends (also after a restart), so changing Windows' time zone does not move it. Daylight saving time is followed. Like the lock, it is measured on internet time, not on the Windows clock.

## Durations

`30s`, `10m`, `2h`, `1d`, or several parts together: `2h 30m`, `2h30m`, `1d 12h`. Words work too: `2 hours 30 minutes`. Everything the CLI and the window show is written out: `2 hours 30 minutes 0 seconds`.

## For developers

- The CLI is `src-tauri/src/cli.rs`, a console exe (`src-tauri/src/bin/bullseye-cli.rs`) on the same library as the app (`src-tauri/src/lib.rs`). Tauri's installer bundles every binary of the package, so it ships next to `bullseye.exe`.
- **Keep it level with the window.** Every command the window can invoke (the `generate_handler!` list in `lib.rs`) must be in `cli::GUI_COMMANDS` with the CLI commands that do the same; `cargo test` fails otherwise, and also when such a command is missing from the help text or from this file. Enforcer names come from `state::Enforcers` itself, so a new enforcer needs no CLI change. A new list, schedule kind or lock method needs an option in `cli::parse_changes`, the help and this file.
- `npm run dev` keeps the service inside the app, where the CLI cannot reach it; use an installed service (in a VM) to try the CLI.
