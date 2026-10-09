# Bullseye: developer guide

Bullseye is a Windows tray app (Tauri 2, Rust, a Carbon web page) that blocks distractions: it closes browser tabs of blocked sites, programs, Store apps and windows by title. A SYSTEM service holds everything and decides; the tray app only closes what it is told to. `README.md` is for users, `CLI.md` for the command line, this file for whoever changes the code.

## Concepts

- **Target**: what to block. Lists of sites (with always-allowed sites), programs, Store apps and window titles (with always-allowed titles), plus **actions**: the ways it is closed (Chromium tabs, split view, app windows, Firefox tabs and split view, Obsidian's web viewer; titles closed or minimized).
- **Condition**: when. A weekly schedule, a flash drive that is unplugged, a webhook, or a combination (any / all) of those. "Always" is built in.
- **Session**: what is started. One or more targets, a condition, at most one **allowance** (a time budget, a Pomodoro cycle or a delayed start that lets it go quiet) and a **lock** (none, timed (for a while, or until a date and time), until restart, while a condition is active, a delay, a text to type, a password). Sessions run side by side; outside its condition a session still runs and stays locked, but enforces nothing. A session has no editor page: it is set up on its tile (`fields.js`).
- **Unlock** (Settings): ends every session. The escape hatch, behind an optional countdown and one optional other way (a condition, text or password).

**The sharing rule.** A running session that is locked cannot be changed (only renamed, its allowance made stricter, and its lock too when it is timed or a delay: ending later, or longer: `Sessions::relock` via `stricter`, `handler::lock_running`), and the targets and conditions it uses can only get stricter. Anything else can change, and running sessions follow at once. A running session that is not locked can also be given a lock, as if it started with it now (`Request::Lock`, `Sessions::relock`): it could be stopped and started again with it anyway. Deleting is refused while a locked session is involved; otherwise what used the deleted item follows (`Library::fallout` says how, and the delete question shows it).

## Architecture

- **Service** (`bullseye`, SYSTEM, starts at boot): definitions (`%ProgramData%\bullseye\sessions.json`), running sessions (`state.json`), unlock settings (`unlock.json`), internet time. It cannot be stopped by users, restarts itself on failure, starts the app at every logon and keeps it running while any session runs (an app that stops asking for 5 s is restarted). Sessions resume after a reboot.
- **App** (`bullseye-app.exe`, the user): the tray, the window, and the closing worker. It asks the service for its policy every second and keeps no state; if the service is away it keeps enforcing the last answer and asks Windows to start it.
- **CLI** (`bullseye.exe`, on `PATH`): everything the window can do, as the same requests.
- One pipe, `\\.\pipe\bullseye`; every action is a `proto::Request`, handled by `handler.rs` (in debug builds in-process: `npm run dev` needs no service).

## Code map (`src-tauri/src`)

| File | What |
|---|---|
| `lib.rs`, `main.rs`, `bin/bullseye.rs`, `ui.rs` | entry modes (`--service`, `--install`, `--uninstall`, app), tray, the page's commands; the window (made on a tray click, destroyed 5 min after hiding) |
| `service.rs`, `handler.rs`, `registration.rs`, `setup.rs`, `system_path.rs` | the service, what it does per request (the sharing rule lives here), its registration and lock-down, install / uninstall and `PATH` |
| `pipe.rs`, `proto.rs`, `client.rs` | the pipe, its messages, the app's side (`Remote`) |
| `definitions.rs` | the `Library` (targets, conditions, sessions), saving, duplicating, export / import, fallout of deletes |
| `session.rs`, `state.rs` | running sessions (`Sessions`), what is active now (`Active`), locks and allowances at run time; the files on disk and `Actions` |
| `allowance.rs`, `lock.rs`, `condition.rs`, `schedule.rs`, `webhook.rs`, `drives.rs` | allowances, lock methods, condition rules, weekly times, webhook polling, flash drives by serial number |
| `lists.rs`, `filters.rs`, `patterns.rs`, `programs.rs`, `store_apps.rs` | a target's lists: sites, `*` patterns, programs (with a protected list), Store apps by package family name |
| `tab_closer.rs`, `wake.rs` | the closing worker (UI Automation) and what wakes it early |
| `unlock.rs` | the escape hatch and password hashing |
| `clock.rs`, `localtime.rs`, `datetime.rs`, `duration.rs`, `boot.rs` | internet time, time zones, dates, durations, which start of Windows this is |
| `watchdog.rs`, `sync.rs` | the service's watch over the app, logging; crash-safe locks and threads |

Frontend (`src/`, no build step): `index.html`, `app.css` (layout only), `strings.js` (every visible text), `js/main.js` (router, tabs, Settings), `core.js` (commands, texts, status, `ask`), `kinds.js` (`KINDS`: the three kinds, generic pages), `forms.js` (the target and condition editors, and an allowance's form), `duration.js` (`durationField`: a number and a unit; the window never takes typed durations, the CLI does), `timing.js` (`timingFields`: a duration or a local date and time, shared by the timed lock and the delayed start), `locks.js` (`LOCKS`), `tiles.js` (the tiles), `fields.js` (a session's rows on its tile and their dialogs, the New session and Rename modals), `editor.js` (targets and conditions), `detour.js` (a target or condition made from a session's tile). `src/vendor/` is Carbon, generated by `npm run vendor` from `scripts/carbon/entry.js`; never edit it.

## Rules

- **The service decides, the app enforces.** Locks, schedules, allowances and what is let through are decided in Rust; the page has no logic of its own for what the backend knows, and every word about times, schedules and errors comes from Rust (so the window and the CLI say the same).
- **Every visible text is in `strings.js`**, looked up with `t("path")`; never inline.
- **Time left, and every time in a tile's status, is one unit, rounded** (`duration::left`: `2 minutes`, `89 seconds`; a date `18:00` today, `October 6` otherwise); elsewhere a set duration is exact (`duration::countdown`).
- **The CLI stays level with the window**: a command in `generate_handler!` needs an entry in `cli::GUI_COMMANDS`, a CLI command, a help line and a line in `CLI.md`; a test fails otherwise.
- **Faults open the hatch, never close it.** A broken `unlock.json` means no unlock delay; an unconfigured app can always unlock. The unlock tests are many on purpose; keep them passing.
- **Err towards blocking elsewhere**: a condition that cannot be told (an unreadable drive, no webhook answer) counts as its fallback or as active.
- **Only today's file formats are read.** Bullseye is installed on clean machines: no update path, no migration code.
- **Nothing is hidden.** The service, app and logs (`%ProgramData%\bullseye\service.log`, `%LOCALAPPDATA%\bullseye\app.log`) go by their own names. The mechanism is friction, not secrecy.
- **Naming**: the product is "Bullseye"; everything a process or the system goes by is lowercase `bullseye` (service, pipe, data folders, CLI `bullseye.exe`). The app is `bullseye-app.exe` and so is the Cargo package: Tauri takes the binary named like the package as the main one.

## User decisions (do not undo without asking)

- Allowances and locks are set on the session, never pages of their own. A budget resets never, every so long (any duration, from the start or on the clock) or when the condition turns on.
- Site lists are address prefixes with `*`, a domain covering its subdomains; no uBlock syntax. Always-allowed entries win.
- Program, Store app and title lists have no on/off switch: entries act, an empty list does nothing. Titles are closed or minimized (a window that cannot be minimized is left alone). A lone `*` title is allowed; Bullseye's own and Windows' windows are spared. Programs refuse anything that would match a protected one.
- Store apps are exact package family names; on import, apps not installed here are dropped with a message, never matched by name.
- Exports keep password locks as hashes only.
- A webhook has a fallback state and one match (HTTP code or body) that turns it around.
- CLI: lists are the words after an option (`--sites a.com b.com`, quotes for an item with a space), never repeated options; `add` / `remove` change items, `edit` replaces; `--force` skips questions.
- Unlock keeps the app running.

## Extending

The UI is built from tables, so these need no new markup:

- **Action**: a field in `state::Actions` and a line in `Actions::all`; an entry in a `SECTIONS` group (`forms.js`); a method on `session::Active` if it acts per session.
- **List** in a target: a field of `lists::Entries` and `lists::Lists` (with `parse`, `entries`, `has`, `Entries::field`), a `Uses` variant and an `Active` method; an entry in `LISTS` and a `SECTIONS` group; a line in `cli::LIST_OPTIONS`.
- **Condition kind**: a variant of `condition::Rule` (`active`, `next_change`, `tightened_by`), an editor in `conditionForm`, an option in `condition new|edit`.
- **Lock method**: a variant of `lock::Lock` (timed is one method, JSON `{"method":"timed","kind":"for|until","value":"..."}`, with `--lock-duration` and `--lock-until` as its CLI options), its `StartLock` / `RunLock` and meaning in `Session::locked` (and in `stricter` if it can be made stricter); an entry in `LOCKS` (form, describe, confirm); an option in `cli::parse_lock`.
- **Allowance kind**: a variant of `allowance::Terms` (a delayed start: `type: delayed`, kind `after` / `from`, `--allowance delayed` with `--after` or `--from`; on a locked session it can only start sooner), its meaning in `Run` / `Session::enforced`, fields in `allowanceSection` (durations and dates through `durationField` / `timingFields`), a CLI option.

## Development

- `npm run dev` runs the app with the service inside it (files in `%LOCALAPPDATA%\bullseye-dev`); `BULLSEYE_USE_SERVICE=1` uses an installed one. There is no window until the tray icon is clicked (`BULLSEYE_SHOW=1` opens it at start). Changes in `src/` need `touch src-tauri/src/main.rs` to relaunch.
- The CLI needs the real service: `cargo run --bin bullseye -- status`.
- Check UI changes in a browser: serve `src/` (`python3 -m http.server`) and open `index.html` in Playwright's Chromium with a stand-in `window.__TAURI__`; check 456 px and wide, light and dark. `node --check` the scripts after edits.
- From Linux: `scripts/linux-tests/clippy-windows.sh` lints the Windows build as CI does, and `cargo test` in `scripts/linux-tests` runs every platform-free module's tests (built from `src-tauri/src` with stand-ins).
- CI (`.github/workflows/build.yml`): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, then the installer as an artifact. `cargo check --release` must stay warning-free.

**Always pursue full test coverage.** Every change comes with tests for what it does: each branch, each error, each message a person reads. Measure with `cargo llvm-cov --manifest-path scripts/linux-tests/Cargo.toml --summary-only -- --skip only_the_installed_app --skip the_app_of_a_program`. What cannot be unit-tested (Win32, UI Automation, the window, the network) is kept thin, with its decisions pulled into tested functions. Tests use no internet, no real service and no files outside the temp folder, and set time with the session's clock instead of waiting.

## UI (Carbon)

All UI is IBM's Carbon design system through `@carbon/web-components`; a missing component or icon is added to `scripts/carbon/entry.js` and vendored.

- Follow Carbon's paradigms; when a request cannot be met the Carbon way, say so and propose the Carbon way. Never restyle a component's shadow DOM (the one exception: side-by-side text areas share a height, `sameHeight`). Only Carbon components, tokens (`--cds-*` colours and spacing) and type classes; `app.css` is layout only.
- Themes g10 (light) and g100 (dark), following Windows.
- Layout: one full-width column per view in `main`, capped at 66rem and centred; the window is at least 456 × 480 px (Delete, Cancel and Save fit on one row); no horizontal scrolling. Only `#scroller` scrolls, with the scrollbar's room kept on both sides so content stays centred; the heading and tabs are fixed above it.
- Navigation: line `cds-tabs` (icon-only below 672 px) and a breadcrumb on every view; no header actions, hamburger or side nav.
- Buttons: `size="md"`, every button has an icon (except the modal's footer), ghost only as a row's action; `tertiary` for minor actions, `danger-tertiary` for destructive ones.
- Tiles: a `cds-tile` with its name, a tag and the ⋮ menu, then rows in a `cds-contained-list` (every contained list is `size="md"`): icon, label, value; target and condition names link to their editors. Target and condition tiles are read-only.
- Sessions: rows Targets, Condition, Allowance, Lock, then Start / Stop on the right. Targets and Condition open a list in the dialog (`ask()` without buttons; the last target cannot be unmarked; New target / New condition go on a detour); Allowance and Lock are an inline `cds-dropdown` with a settings button. Condition, Allowance and Lock have a few words of status (`sessions.now`), beside the value, under it in a narrow window. Each choice is saved as made. Locked: only the allowance and a timed or delay lock's settings stay open (stricter only). `#/edit/<id>` shows that tile alone (Sessions / name, with Show all sessions).
- Editors (targets, conditions): a name first, then sections with a heading and a line of text, spaced apart, no divider lines but the one above the buttons; the target editor uses an accordion. Every choice from a list is a `cds-dropdown` with `autoalign`; a duration is the `durationField` picker, never typed text. Forms are built anew each time (Carbon elements must not be re-attached).
- Errors: a field's own (`invalid`); a group's or form's an error notification under it (`errorNote`); a refused action a notification at the top. What settings come to is an info notification under them (`infoNote`).
- Anything that cannot be undone asks first in the one `cds-modal` (`ask()`), which also holds short forms (lock settings, passwords). A session's lock is its tile's fourth row: for the next start while it is stopped, at once after a yes while it runs unlocked (`lock_session`), read-only while it is locked.
- The weekly schedule is the one hand-made control (`weekEditor`): Carbon has no time grid.

## Install

`npm run build` makes a per-machine NSIS installer that installs to `C:\Program Files\Bullseye`, registers and locks down the service and puts the folder on `PATH`. Uninstalling is refused while a session is locked. **Develop with `npm run dev`**, which can always be ended; an installed Bullseye is ended only through Unlock in Settings, whose delay is at most 24 hours.

## Constraints

- Windows only. No browser extensions, Obsidian plugins or launch flags: everything lives in the app. Never switch tabs visibly to inspect them.
- Reading browser profile data and Obsidian's Node inspector were refused by the permission classifier; don't retry without the user's explicit OK.
- Never push to a remote unless asked in that message.

## Findings (don't re-investigate)

- Chromium via UI Automation: every tab's title and Close button are visible, but only visible pages' URLs (a page's root `Document` value; the address bar `OmniboxViewViews` as fallback). Tabs are class `Tab` in Chrome/Brave and `EdgeTab` in Edge; windows are `Chrome_WidgetWin_1`, shared with Electron apps. Only a browser's own window is ever closed as an app window.
- Find browser windows with `EnumWindows` + class name (0.3 ms), never a UI Automation search from the root (92 ms).
- A standard user cannot open a SYSTEM process; the app asks the pipe for the server's session instead.
- Obsidian exposes no accessibility tree and a stale title; it is closed via UI Automation's WindowPattern (killing its web renderer greys the view but it quits on the next navigation).
- WebView2 instances share a browser process per user-data folder (`WEBVIEW2_USER_DATA_FOLDER` to measure one alone).
