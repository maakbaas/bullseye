# Bullseye command line

`bullseye.exe` does everything the window does, from a terminal or a script. The installer puts it on the `PATH`, so any terminal opened after installing runs it by name:

```powershell
bullseye status
```

It sends the same requests to the Bullseye service as the window, so the service checks both alike and each shows the other's changes within a second. It needs no administrator rights. A `<session>`, `<target>` or `<condition>` is its name (any case) or its id (`show` gives the id). Sessions, targets, conditions, locks and allowances are explained in the [README](README.md).

## Commands

| Command | Does what the window does with |
|---|---|
| `bullseye status [--json]` | the three pages: every session (running or not, active or not, locked or not), every target, every condition |
| `bullseye show <session> [--json]` | a session's tile and its editor |
| `bullseye new <name> [options]` | **New session**, then **Save** |
| `bullseye edit <session> [options]` | **Edit**, then **Save** |
| `bullseye add <session> --targets <targets>` | ticking more targets in a session's editor, then **Save** |
| `bullseye remove <session> --targets <targets>` | unticking targets in a session's editor, then **Save** |
| `bullseye delete <session> [--force]` | **Delete** (asks first, unless `--force`) |
| `bullseye duplicate <session>` | **Duplicate** |
| `bullseye start <session> [lock option] [--force]` | the lock on the session's tile, then **Start** (a locked session asks first, unless `--force`) |
| `bullseye stop <session> [--password <pw>] [--abort]` | **Stop** (a session locked with a password asks for it, as the window does; one locked with a delay is asked to stop first, `--abort` ends the wait) |
| `bullseye target show <target> [--json]` | a target's tile and its editor |
| `bullseye target apps [--json]` | the installed Store apps the editor offers to tick, by name and package family name |
| `bullseye target new <name> [options]` | **New target**, then **Save** |
| `bullseye target edit <target> [options]` | **Edit** a target, then **Save** |
| `bullseye target add <target> [options]` | adding entries to a target (and ticking enforcers), then **Save** |
| `bullseye target remove <target> [options]` | taking entries out of a target (and unticking enforcers), then **Save** |
| `bullseye target delete <target> [--force]` | **Delete** a target |
| `bullseye target duplicate <target>` | **Duplicate** a target |
| `bullseye condition show <condition>` | a condition's tile and its editor |
| `bullseye condition new <name> (--times <times> \| --drive <drive> \| --webhook <url> [webhook options] \| --any <conditions> \| --all <conditions>)` | **New condition**, of the type Weekly schedule, Flash drive or Combination, then **Save** |
| `bullseye condition edit <condition> [--name <name>] [--times <times> \| --drive <drive> \| --webhook <url> [webhook options] \| --any <conditions> \| --all <conditions>]` | **Edit** a condition, then **Save** |
| `bullseye condition add <condition> --conditions <conditions>` | ticking more conditions in a combination's editor, then **Save** |
| `bullseye condition remove <condition> --conditions <conditions>` | unticking conditions in a combination's editor, then **Save** |
| `bullseye condition delete <condition> [--force]` | **Delete** a condition |
| `bullseye condition duplicate <condition>` | **Duplicate** a condition |
| `bullseye condition test <condition>` | **Test** in a webhook condition's editor: asks it once and shows the status, the start of the body and what the match makes of it |
| `bullseye condition check <times>` | the schedule written out under the week grid in the condition editor |
| `bullseye export <file>` | **Export** in Settings |
| `bullseye import <file> [--force]` | **Import** in Settings |
| `bullseye drives [--json]` | the flash drives the **Flash drive** type of a condition offers, by name and serial number |
| `bullseye unlock [--abort] [--password <pw>]` | **Request unlock**, **Unlock** and **Keep locked** / **Lock again** in Settings; `--password` unlocks at once with the password of the other way to unlock |
| `bullseye unlock-way [--condition <condition> \| --password <pw> \| --text <text> \| --none]` | **Another way to unlock** in Settings: show it, set it or remove it |
| `bullseye unlock-delay [<time>]` | **Unlock delay** in Settings: show it, or set it |
| `bullseye help` | |
| `bullseye version` | |

Exit codes: `0` done, `1` failed, refused or cancelled (the reason is printed), `2` not understood. Every option can also be written as `--option=value`. The name or id comes right after the command.

**Several items** (targets, sites, programs, titles, conditions of a combination, enforcers, lists to clear) are the words after their option, up to the next option, separated by spaces: `--sites youtube.com reddit.com`. An item with a space in it goes in double quotes, as the terminal has it: `--titles "Team chat" *YouTube*`. Each such option is given once.

### Sessions: `new`, `edit`, `start`

| Option | Meaning | New session's default |
|---|---|---|
| `--targets <targets>` | The targets it enforces. | none (it cannot start without one) |
| `--condition <condition>` | When it is active: a condition, or `always`. | `always` |
| `--allowance <budget\|pomodoro\|none>` | Its allowance, with the options below (see [Allowances](#allowances---allowance-and-its-options)). | `none` |
| `--allowed`, `--scope`, `--reset`, `--blocked`, `--anchor` | The allowance's terms. | |
| `--lock-duration <duration>` | Lock: for this long once started. | |
| `--lock-until <date time>` | Lock: until then, e.g. `"2026-10-06 18:00"` (local time). A time that has passed when the session starts is refused. | |
| `--lock-until-restart` | Lock: until Windows restarts. | |
| `--lock-condition <condition>` | Lock: while this condition is active. A schedule locks at its times; a flash drive locks while it is not connected. | |
| `--lock-password <pw>` | Lock: with this password. Only a hash of it is kept. | |
| `--lock-delay <time>` | Lock: stopping is asked for with `stop`, counts down this long (at most 24 hours) and can be aborted with `stop --abort`; when it is over, `stop` stops the session. Like unlocking. | |
| `--lock-text <text>` | Lock: until this text is typed in exactly. The window shows it above an input that cannot be pasted into; that is the only place to stop such a session (or by unlocking). The text is no secret: anyone who can see the session sees it. | |
| `--no-lock` | Not locked. | yes |
| `--name <name>` | (`edit` only) A new name. | |

With `edit`, an option replaces only what it names: `--targets` replaces all its targets with the ones given, and what is not named stays. A running session takes the change at once (its lock aside, which is for the next start); while it is locked, only its name and a stricter allowance. `start` takes the lock options too: they set the session's lock before it starts, as the lock on its tile in the window does.

### Targets: `target new`, `target edit`

| Option | Meaning | New list's default |
|---|---|---|
| `--sites <sites>` | The sites to block: each site, its subdomains and everything under it (`youtube.com` is `www.youtube.com` and `music.youtube.com` too), with `*` as a wildcard as in the window: `reddit.com/r/*`, `*` (every site). `*.reddit.com` is the subdomains only. An exception for one subdomain goes in `--allowed-sites`. | none |
| `--allowed-sites <sites>` | Sites that are always allowed, also when this list's sites match them. | none |
| `--programs <names>` | Programs to close, by exe name: `steam`, `steam*.exe`. | none |
| `--store-apps <families>` | Store apps to close, by their exact package family names, as `target apps` shows the installed ones: `Microsoft.WindowsCalculator_8wekyb3d8bbwe`. No `*`, as in the window, where they are ticked. Windows' own apps are never closed. | none |
| `--titles <titles>` | Windows to close, by their title, the whole title with `*` as a wildcard and letter case ignored: `*YouTube*`. The whole window closes, or is minimized (`--enforcers`), a browser's with all its tabs. A lone `*` matches every window but Bullseye's own and Windows' shell. | none |
| `--allowed-titles <titles>` | Window titles that are always left alone, also when this list's titles match them, written as in `--titles`: `*Visual Studio Code`. | none |
| `--sites-file <file>`, `--allowed-sites-file <file>`, `--programs-file <file>`, `--store-apps-file <file>`, `--titles-file <file>`, `--allowed-titles-file <file>` | The same, one entry per line, from a text file. Can be combined with the options above. | |
| `--enforcers <names>` | Which enforcers act on this list: `chromium_tabs` (close blocked tabs in Chrome, Edge, Brave), `chromium_split_view` (close blocked pages in split view), `chromium_app_windows` (close app windows of blocked pages, e.g. `chrome --app=<url>`), `firefox_tabs` (close blocked tabs in Firefox), `firefox_split_view` (close blocked pages in Firefox split view), `window_titles_minimize` (minimize the windows by title instead of closing them: nothing is lost, and a window that cannot be minimized is left alone), `obsidian_web_viewer` (close Obsidian when its web viewer is active), or `all` (every enforcer but minimizing windows, which goes instead of closing them). Programs, Store apps and windows by title need no enforcer: a list with entries acts, an empty one does nothing. `bullseye help` lists the current names. A locked session's list may go from minimizing to closing windows, not back. | `all` |
| `--name <name>` | (`target edit` only) A new name. | |
| `--clear <lists>` | (`target edit` only) Empty lists: `sites`, `allowed-sites`, `programs`, `store-apps`, `titles`, `allowed-titles`, e.g. `--clear sites titles`. | |

### Allowances: `--allowance` and its options

An allowance lets a session go quiet for a while. A session has at most one, set on the session itself with the options of `new` and `edit` (as in the window, where it is a section of the session's editor). Two kinds:

- A **budget** is a time of use. It is for the session as a whole (`--scope total`) or for each of its items on its own (`--scope each`: each site, program, Store app and window title in its targets). While time is left, what is covered is not closed; it is only watched, and the time it is in use is counted. When the time is used up it is closed again, until the budget is reset.
- A **Pomodoro** is a fixed cycle: blocked for `--blocked`, then everything allowed for `--allowed`, over and over. While it allows, the session enforces nothing, Obsidian's web viewer included.

**Use** is what the app sees: a page shown in a visible tab (a page in a background tab is not seen), an open window or app, a program that is running. The service counts it, on internet time.

| Option | Meaning | Default |
|---|---|---|
| `--allowance <budget\|pomodoro\|none>` | Which kind, or `none` to take it away. Without it, `--blocked` makes a Pomodoro, `--scope` or `--reset` a budget, and `--allowed` or `--anchor` alone keep the kind the session has (a budget if none). Another kind starts from its defaults. | `budget` |
| `--allowed <duration>` | Budget: the time allowed, e.g. `30m`. Pomodoro: how long everything is allowed each cycle. | required |
| `--scope <total\|each>` | Budget: the session as a whole, or each item on its own. | `total` |
| `--reset <never\|condition\|duration>` | Budget: when what was used is given back: `never` while the session runs, each time the session's `condition` turns it on (a time block of a schedule, a flash drive being unplugged; a session with no condition is always on, so never resets), or every so long, a duration such as `1d`, `12h` or `90m`, counted as `--anchor` says. | `never` |
| `--blocked <duration>` | Pomodoro: how long it is blocked each cycle, e.g. `50m`. | required |
| `--anchor <start\|clock>` | What a Pomodoro, or a budget's reset every so long, is counted from: the start of the session (a cycle is blocked first), or midnight on the clock, so a cycle or reset of an hour comes at every xx:00 and `1d` at midnight. On the clock it has to fit a day evenly: 30 minutes, 1 hour, 1 day. | `start` |

With `edit`, an option replaces only what it names. While the session is locked, its allowance can only get stricter: a budget's time shorter or reset less often (every so long only longer and counted from the same point, or never; a reset by condition stays), its scope and a Pomodoro's cycle stay as they are, and `--allowance none` takes it away; a session without one cannot be given one.

Obsidian's web viewer enforcer has no items to count, so a budget (total or for each item) never lets it through: Obsidian is closed on web content all the time. Only a Pomodoro's allowed time lets it be.

```powershell
bullseye new "Social time" --targets "Social media" --allowance budget --allowed 30m --scope total --reset 1d --anchor clock
bullseye edit "Social time" --allowed 10m --scope each --reset 2h
bullseye new "Focus" --targets "Social media" --allowance pomodoro --blocked 50m --allowed 10m --anchor clock
bullseye edit "Focus" --allowance none
```

### Examples

```powershell
# What to block.
bullseye target new "Social media" --sites youtube.com reddit.com --titles "Team chat" *Shorts*
bullseye target new "Games" --programs-file games.txt

# When.
bullseye condition new "Work hours" --times "Mon-Fri 08:00-17:00"

# A flash drive as the key: active (and, as a lock, locked) while it is not connected.
bullseye drives
bullseye condition new "Key" --drive "Kingston DataTraveler 3.0"

# A combination of two or more conditions (not of other combinations): active when any of them
# is (--any), or when all of them are (--all). Give the conditions separated by spaces.
bullseye condition new "Room" --webhook "https://example.com/state?room=1" --match-body off --every 5m
bullseye condition new "Door" --webhook https://example.com/door --method post --fallback inactive --match-status 200
bullseye condition test "Room"
bullseye condition new "Work or weekend" --any "Work hours" Weekend
bullseye condition new "Work hours with the key out" --all "Work hours" Key

# Sessions: work hours that cannot be stopped during work hours, and an evening one for 2 hours.
bullseye new "Work" --targets "Social media" Games --condition "Work hours" --lock-condition "Work hours"
bullseye new "Evening" --targets Games
bullseye start Work --force
bullseye start Evening --lock-duration 2h
bullseye start Evening --lock-until "2026-10-06 18:00"
bullseye start Evening --lock-until-restart

# Changing one thing at a time: add and remove keep the rest.
bullseye target add "Social media" --sites x.com tiktok.com
bullseye target remove "Social media" --sites reddit.com
bullseye add Evening --targets "Social media"
bullseye condition add "Work or weekend" --conditions Key

# Later: Saturdays too (once Work is stopped, outside work hours).
bullseye stop Work
bullseye condition edit "Work hours" --times "Mon-Fri 08:00-17:00; Sat 10:00-14:00"
```

### `status` and `show`

```
> bullseye status
Sessions:
  Work     Running · inactive for the next 15 hours · unlocked for the next 15 hours
  Evening  Running · active · locked for the next 1 hour and 2 minutes
Targets:
  Social media  used by Work
  Games         used by Work, Evening
Conditions:
  Work hours  Mon-Fri 08:00-17:00
```

A running session with an allowance says what it is doing at the end of its line: `allowed for the next 4 minutes` or `blocked for the next 12 minutes` (a Pomodoro), `12 minutes left of 30 minutes, reset in 3 hours` (a budget for the session), or `10 minutes for each item, reset in 40 minutes (youtube.com 4 minutes left)`.

`show <session>`, `target show <target>` and `condition show <condition>` give everything about one. With `--json`, `status`, `show` and `target show` print what the service says, for scripts: the same fields as the export file, plus `running` with each running session's state (`locked`, `enforcing`, the times left in seconds and in words).

### `stop`, `delete`, `target delete`, `condition delete`

`stop <session>` stops a running session that is not locked right now; a locked one is refused, and the message says why and for how long. A session locked with a text is stopped in the window, by typing the text; one locked with a password takes `--password` (or asks for it). Deleting asks first (`[y/N]`); `--force` skips the question, also where there is no terminal to ask in. Deleting a target or condition says first what it does to the sessions that use it.

### `add` and `remove`

`edit` replaces a whole set (`edit Work --targets Games` leaves Games as its only target); `add` and `remove` change only the items named and keep the rest: a session's targets (`--targets`), a target's entries (its list options, `--sites`, `--programs`, …, and `--enforcers` to turn on or off) and a combination's conditions (`--conditions`). Adding what is there already changes nothing. Taking out what is not there is refused, as a typo, and then nothing is saved; an entry is found however it is written (`Steam` is `steam.exe`, `YouTube.com` is `youtube.com`). The service checks the result like any other save: while a locked session uses a target, adding entries to block goes through and taking them out is refused; a combination keeps at least two conditions, and a session at least one target.

### `duplicate`, `target duplicate`, `condition duplicate`

Make a copy, as **Duplicate** in a tile's menu does, named after the original with the next free number (`Work (2)`). A session's copy has the same targets, condition, allowance and lock; it is not running.

### `export`, `import`

To move to another computer: `export <file>` writes every target, condition and session to a settings file, as **Export** in Settings does; `import <file>` replaces them all, after asking, and is refused while a session is locked. Password locks are in the file as a hash only, so keep it private. Store apps that are not installed on this PC are left out, and the output says which.

```powershell
bullseye export D:\bullseye.json
bullseye import D:\bullseye.json
```

### `unlock`

Ends every running session, locked or not, exactly as the window's Unlock does: the targets, conditions and sessions stay, and so does the app, with nothing to enforce. **Unlocking is free until you set an unlock delay** (see `unlock-delay` below), so on a clean install `bullseye unlock` ends everything at once. With a delay it is a countdown, in steps you run again:

```powershell
bullseye unlock           # starts the countdown: "ready in 1 hour"
bullseye unlock           # while it counts down: says how long is left
bullseye unlock           # once it is over: ends every session
bullseye unlock --abort   # at any time: stop the countdown, everything stays locked
```

Ready, you can still `--abort` instead. Unlocking uses the request up: the next time starts again.

### `unlock-delay`

Shows how long unlocking takes, or sets it: `0` (none, the default) to `24h`, typed like a duration (`30m`, `2h 30m`). A longer delay can be set at any time; a shorter one only once unlocking is ready or while no session is locked, because setting it low would be a way around the wait.

```powershell
bullseye unlock-delay
bullseye unlock-delay 2h
bullseye unlock-delay 0
```

### The other way to unlock

Besides the delay, which is always there, one other way to unlock can be set (`unlock-way`), chosen like a session's lock but only a condition, a custom text or a password: unlocking is ready when the delay is over **or** this way lets go, one is enough. `--condition <condition>` is locked while the condition is active (as a session's lock) and open when it is not, `--text <text>` opens when the text is typed in (in the window: a terminal cannot refuse a paste) and `--password <pw>` with the password (`unlock --password`, asked for when not given; only a salted hash is kept). `--none` removes it. Setting one makes unlocking easier, so it needs unlocking to be ready, or no session to be locked; removing it is always free.

```
bullseye unlock-way
bullseye unlock-way --password secret
bullseye unlock-way --condition "Work hours"
bullseye unlock-way --none
```

### Webhook conditions

`--webhook <url>` makes a condition that asks one web address (it can hold GET parameters; no tokens or headers) every `--every <time>` (at least `1m`, the default) with `--method get` or `post` (default get), and is active or not by the answer. `--fallback active|inactive` (default active) is what the condition is by default, also when no answer comes or the last one is older than three intervals; one match turns it into the other: `--match-status <code>` (the HTTP status code) or `--match-body <text>` (the whole body, white space around it aside). So with the default fallback, `--match-body off` makes it not active while the address answers `off`, and active otherwise. `condition test <condition>` asks it once and says what came back and what the match makes of it. A locked session's webhook condition cannot change.

### `condition check`

Checks times written as text and shows them per day, the way a schedule reads them, and as the window writes them out:

```
> bullseye condition check "weekdays 8-12, 13-17; sat 10am-noon"
In "sat 10am-noon": "noon" is not a time such as 08:00, 17:30 or 5pm
> bullseye condition check "weekdays 8-12, 13-17; sat 10am-12pm"
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

Examples: `Mon-Fri 08:00-17:00`, `weekdays 9-12, 13-17; sat 10-12`, `daily 21:00-07:00`, `weekends`. Times with nothing in them are refused: a session would never be active.

A running session reads its schedule in the time zone Windows had when the session started, and keeps that zone until it ends (also after a restart), so changing Windows' time zone does not move it. Daylight saving time is followed. Like the lock, it is measured on internet time, not on the Windows clock.

## Durations

`30s`, `10m`, `2h`, `1d`, or several parts together: `2h 30m`, `2h30m`, `1d 12h`. Words work too: `2 hours 30 minutes`. Everything the CLI and the window show is written out, leaving out units that are zero: `2 hours and 30 minutes`, `2 hours and 5 seconds`, `1 day, 2 hours and 1 minute`.
