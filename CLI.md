# Bullseye command line

`bullseye-cli.exe` does everything the window does, from a terminal or a script: make, change and delete block lists, schedules and blocks, start and stop blocks, and end them all with the unlock password. The installer puts it next to the app, in `C:\Program Files\Bullseye\`. To call it by name from anywhere, add that folder to your `PATH` (Settings > System > About > Advanced system settings > Environment Variables), or call it by its full path:

```powershell
& "C:\Program Files\Bullseye\bullseye-cli.exe" status
```

It talks to the Bullseye service exactly as the window does (the same requests, through the same code), so the service checks everything done here as it checks the window, and what you do here shows up in the window within a second (and the other way round). If the service is installed but not running, the CLI starts it first, as the app would. It needs no administrator rights.

A `<block>`, `<list>` or `<schedule>` is its name (any case) or its id; `bullseye-cli status` lists the names, and `show` gives the id. When two have the same name, use the id.

## Three kinds of things

As in the window, which has a page for each:

- **Block lists**: what to block. Sites, always-allowed sites, programs, and the enforcers that act on them.
- **Schedules**: when, as a weekly schedule. A block without one is always active.
- **Blocks**: what you start. One or more block lists, a schedule (or always) and a lock:
  - `none`: not locked; it can be stopped at any time.
  - `timer`: locked for a while. It cannot be stopped, changed or deleted, and it ends by itself when the time is up.
  - `while active`: locked while its schedule has it active. Outside those times it can be stopped; otherwise it runs until stopped. Needs a weekly schedule.

Outside its schedule a running block keeps running, but enforces nothing. For example, with a schedule of `daily 08:00-17:00`, a block started at 16:00 with a 2-hour timer blocks until 17:00; from then on everything is accessible again, and at 18:00 the timer ends the block. With "while active" instead, it blocks every day from 8 to 5 and cannot be stopped then, until you stop it some evening.

A running block keeps what it was started with: while it runs, it cannot be changed or deleted, and neither can the block lists and the schedule it uses. A block list or schedule that any block uses cannot be deleted either; the message says which blocks.

## Commands

| Command | Does what the window does with |
|---|---|
| `bullseye-cli status [--json]` | the three pages: every block (running or not, active or not, locked or not), every block list, every schedule |
| `bullseye-cli show <block> [--json]` | a block's tile and its editor |
| `bullseye-cli new <name> [options]` | **New block**, then **Save** |
| `bullseye-cli edit <block> [options]` | **Edit**, then **Save** |
| `bullseye-cli delete <block> [--yes]` | **Delete** (asks first, unless `--yes`) |
| `bullseye-cli start <block> [lock option] [--yes]` | the lock on the block's tile, then **Start** (a locked block asks first, unless `--yes`) |
| `bullseye-cli stop <block>` | **Stop** |
| `bullseye-cli list show <list> [--json]` | a block list's tile and its editor |
| `bullseye-cli list new <name> [options]` | **New block list**, then **Save** |
| `bullseye-cli list edit <list> [options]` | **Edit** a block list, then **Save** |
| `bullseye-cli list delete <list> [--yes]` | **Delete** a block list |
| `bullseye-cli schedule show <schedule>` | a schedule's tile and its editor |
| `bullseye-cli schedule new <name> <times>` | **New schedule**, then **Save** |
| `bullseye-cli schedule edit <schedule> [--name <name>] [--times <times>]` | **Edit** a schedule, then **Save** |
| `bullseye-cli schedule delete <schedule> [--yes]` | **Delete** a schedule |
| `bullseye-cli schedule check <times>` | the Times field of the schedule editor |
| `bullseye-cli unlock [--password <pw>]` | **Unlock** in Settings (also closes the app) |
| `bullseye-cli help` | |
| `bullseye-cli version` | |

Exit codes: `0` done, `1` failed, refused or cancelled (the reason is printed), `2` not understood. Every option can also be written as `--option=value`. The name or id comes right after the command.

### Blocks: `new`, `edit`, `start`

| Option | Meaning | New block's default |
|---|---|---|
| `--list <list>` | A block list it enforces (repeatable). | none (it cannot start without one) |
| `--schedule <schedule>` | When it is active: a schedule, or `always`. | `always` |
| `--lock-timer <duration>` | Lock: timer, for this long once started. | |
| `--lock-while-active` | Lock: while its schedule has it active. | |
| `--no-lock` | Not locked. | yes |
| `--name <name>` | (`edit` only) A new name. | |

With `edit`, an option replaces only what it names: `--list` replaces all its block lists with the ones given, and what is not named stays. `start` takes the three lock options too: they set the block's lock before it starts, as the lock on its tile in the window does.

### Block lists: `list new`, `list edit`

| Option | Meaning | New list's default |
|---|---|---|
| `--site <entry>` | A site to block (repeatable). An address prefix with `*` as a wildcard, as in the window: `youtube.com`, `*.reddit.com`, `*` (every site). | none |
| `--allow <entry>` | A site that is always allowed, also when this list's sites match it (repeatable). | none |
| `--program <name>` | A program to close, by exe name (repeatable): `steam`, `steam*.exe`. | none |
| `--sites-file <file>`, `--allowed-file <file>`, `--programs-file <file>` | The same, one entry per line, from a text file. Can be combined with the options above. | |
| `--enforcers <list>` | Which enforcers act on this list, comma-separated: `chromium_tabs` (close blocked tabs in Chrome, Edge, Brave), `chromium_split_view` (close blocked pages in split view), `exe_names` (close the programs), `obsidian_web_viewer` (close Obsidian when it shows web pages), or `all`. `bullseye-cli help` lists the current names. | `all` |
| `--name <name>` | (`list edit` only) A new name. | |
| `--clear <list>` | (`list edit` only) Empty a list: `sites`, `allowed` or `programs` (repeatable). | |

### Examples

```powershell
# What to block.
bullseye-cli list new "Social media" --site youtube.com --site reddit.com
bullseye-cli list new "Games" --programs-file games.txt --enforcers exe_names

# When.
bullseye-cli schedule new "Work hours" "Mon-Fri 08:00-17:00"

# Blocks: work hours that cannot be stopped during work hours, and an evening one for 2 hours.
bullseye-cli new "Work" --list "Social media" --list Games --schedule "Work hours" --lock-while-active
bullseye-cli new "Evening" --list Games
bullseye-cli start Work --yes
bullseye-cli start Evening --lock-timer 2h

# Later: Saturdays too (once Work is stopped, outside work hours).
bullseye-cli stop Work
bullseye-cli schedule edit "Work hours" --times "Mon-Fri 08:00-17:00; Sat 10:00-14:00"
```

### `status` and `show`

```
> bullseye-cli status
Blocks:
  Work     Running · inactive by its schedule, active in 15 hours 0 minutes 0 seconds · can be stopped until it is active again
  Evening  Running · blocking now · locked, 1 hour 2 minutes 3 seconds left
Block lists:
  Social media  used by Work
  Games         used by Work, Evening
Schedules:
  Work hours  Mon-Fri 08:00-17:00
```

`show <block>`, `list show <list>` and `schedule show <schedule>` give everything about one. With `--json`, `status` prints what the service says, for scripts: `lists`, `schedules` and `blocks` (as defined; a schedule's `days` are Monday first, each a list of `[start, end)` spans in minutes since midnight; a block's `schedule` is a schedule's id or `null` for always, and its `lock` is `{"method":"none"}`, `{"method":"timer","duration":"2h 30m"}` or `{"method":"while_active"}`), `running` (each running block with its `lists` as started; `remaining`, the seconds left of a timer, and `remaining_words`; `while_active`; `locked`, whether it cannot be stopped right now; `schedule`; `enforcing`, whether it blocks anything right now; `schedule_change`, the seconds until its schedule turns it on or off, and `schedule_change_words`) and `clock_synced`.

### `stop`, `delete`, `list delete`, `schedule delete`

`stop <block>` stops a running block that is not locked right now; a locked one is refused, and the message says why and for how long. Deleting asks first (`[y/N]`), or goes ahead with `--yes`, which is also needed when there is no terminal to ask in (a script). A running block, and a block list or schedule that a block uses, cannot be deleted.

### `unlock`

Ends every running block, locked or not, with the unlock password, and closes the tray app, exactly as the window's Unlock does: the block lists, schedules and blocks stay, and the service stays installed and starts the app again at your next sign-in. Wrong guesses are slowed down after three, as in the window. Without `--password` it asks for the password without showing it, or reads it from standard input when that is not a terminal:

```powershell
bullseye-cli unlock
"my password" | bullseye-cli unlock
```

(A password given with `--password` ends up in your shell history.) The app is closed by the service: after an unlock it answers the app's next request for what to enforce, within a second, with "quit", for 10 seconds or until a block is started again.

### `schedule check`

Checks times written as text and shows them per day, the way a schedule and the window's Times field read them:

```
> bullseye-cli schedule check "weekdays 8-12, 13-17; sat 10am-noon"
In "sat 10am-noon": "noon" is not a time such as 08:00, 17:30 or 5pm
> bullseye-cli schedule check "weekdays 8-12, 13-17; sat 10am-12pm"
Mon-Fri 08:00-12:00, 13:00-17:00; Sat 10:00-12:00
  Mon  08:00-12:00, 13:00-17:00
  ...
  Sat  10:00-12:00
  Sun  inactive
```

## Weekly times

A schedule's times are rules separated by `;` (or new lines). Each rule is a set of days, then one or more time spans separated by commas or spaces:

- **Days**: `mon` (or `monday`, `mo`), a range `mon-fri` (also round the end of the week: `fri-mon`), a list `mon,wed,fri`, or `daily`, `weekdays`, `weekends`. A rule without days is every day.
- **Times**: `8`, `8:30`, `08:30`, `17:00`, `24:00`, `8am`, `5:30pm`. A span ends just before its end time. One that ends before it starts runs past midnight into the next day: `fri 22:00-02:00` is Friday 22:00 to Saturday 02:00. A rule without times is the whole day.

Examples: `Mon-Fri 08:00-17:00`, `weekdays 9-12, 13-17; sat 10-12`, `daily 21:00-07:00`, `weekends`. Times with nothing in them are refused: a block would never be active.

A running block reads its schedule in the time zone Windows had when the block started, and keeps that zone until it ends (also after a restart), so changing Windows' time zone does not move it. Daylight saving time is followed. Like the lock, it is measured on internet time, not on the Windows clock.

## Durations

`30s`, `10m`, `2h`, `1d`, or several parts together: `2h 30m`, `2h30m`, `1d 12h`. Words work too: `2 hours 30 minutes`. Everything the CLI and the window show is written out: `2 hours 30 minutes 0 seconds`.

## For developers

- The CLI shares everything with the window but its presentation: it sends the same `proto::Request`s through the same `client::Remote` (`Remote::for_cli`), so the service decides and checks everything for both, and texts such as schedules and durations come from the same Rust functions (`schedule.rs`, `duration.rs`).
- The CLI is `src-tauri/src/cli.rs`, a console exe (`src-tauri/src/bin/bullseye-cli.rs`) on the same library as the app (`src-tauri/src/lib.rs`). Tauri's installer bundles every binary of the package, so it ships next to `bullseye.exe`.
- **Keep it level with the window.** Every command the window can invoke (the `generate_handler!` list in `lib.rs`) must be in `cli::GUI_COMMANDS` with the CLI commands that do the same; `cargo test` fails otherwise, and also when such a command is missing from the help text or from this file. Enforcer names come from `state::Enforcers` itself, so a new enforcer needs no CLI change. A new list or lock method needs an option in `cli.rs`, the help and this file.
- `npm run dev` keeps the service inside the app, where the CLI cannot reach it; use an installed service (in a VM) to try the CLI.
