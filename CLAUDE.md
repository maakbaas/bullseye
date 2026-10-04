# Site Blocker (Tauri tray app, Windows only)

Blocks sites listed in the UI: closes matching Chromium-browser tabs and closes Obsidian when it shows web content.

- `src/index.html` – the whole frontend (no bundler), Fluent styling, Mica via `tauri.conf.json`
- `src-tauri/src/main.rs` – tray (every start is hidden in the tray; click opens the window), X hides to tray, `start_block` / `stop_block` / `block_status` / `force_exit` commands, and the two entry modes
- `src-tauri/src/block.rs` – the running block; timed blocks (`5s`/`10m`/`3h`/`1d`) are enforced here, not in the UI
- `src-tauri/src/clock.rs` – internet time (SNTP) that timed blocks are measured against, so moving the Windows clock does nothing
- `src-tauri/src/unlock.rs` – the unlock password, kept only as a salted PBKDF2-SHA256 hash
- `src-tauri/src/filters.rs` – blocked-site entries: plain address prefixes plus `*` (user rejected full uBlock syntax as too complex)
- `src-tauri/src/tab_closer.rs` – 1-second worker thread doing the blocking, as three enforcers (`state::Enforcers`, checkboxes in the UI, fixed while a block runs): with the block list, Chromium tabs and Chromium split view (a window showing two pages); without it, Obsidian's web viewer
- `src-tauri/src/state.rs` – the block on disk (`%LOCALAPPDATA%\SiteBlocker\state.json`), so it survives a restart
- `src-tauri/src/watchdog.rs` – the launcher process and the app/launcher pair
- `src-tauri/src/task.rs` – the `SiteBlocker` scheduled task that runs the entry point at logon
- `src-tauri/src/setup.rs` + `src-tauri/windows/hooks.nsh` – `--install` / `--uninstall`, run by the NSIS installer

## Surviving a restart, a kill, and a reboot

One command does everything: `SiteBlocker.exe --launch` (`tray-app.exe` in dev builds). It starts the app, then watches over it
and starts it again within ~0.5 s of it ending for as long as a timed block is saved on disk.
The app in turn starts a new launcher within ~1 s whenever none is running during a timed
block, and the login task runs `--launch` at every logon, so the pair comes back after a reboot.

- End the app in Task Manager during a timed block: the launcher brings it back and the block
  continues.
- End the launcher: the app brings it back. (The task's *restart on failure* does not help
  here: it only covers the task failing to start, not exit codes.)
- End both, then log off and on: the login task starts the pair again.
- Restart Windows mid-block: the block is read back from `state.json` and resumes, with the time
  it had left.

Roles are held through named mutexes (`Local\SiteBlocker-app`, `Local\SiteBlocker-launcher`), so
a second copy exits instead of doubling up.

**"Unlock"** in the UI (`force_exit`, inline password field + button at the bottom of the window; password `test1234`, stored only as a hash in `unlock.rs`, which says how to change it)
disables the login task, clears the saved block and ends every process, so nothing restarts by
itself. It disables the task rather than deleting it: starting the exe again (plain, or
`--launch`) re-enables it as the app starts (`watchdog::restore_task`), so everything is back to
how it was before. The task is registered without elevation, so the app can always do this.

### Deliberately not hidden

Per the user's decision: nothing here is obscured or anti-tamper. The task is named
`SiteBlocker` and appears in Task Scheduler; both processes run under the app's own name in
Task Manager; the app is listed under Settings > Apps and uninstalls from there;
`%LOCALAPPDATA%\SiteBlocker\launcher.log` records what the launcher did. The mechanism is
friction, not secrecy.

**The honest ceiling** (do not claim otherwise in the UI): an administrator can kill both
processes at once, delete `state.json`, disable or delete the task, or boot into safe mode.
The deadline is in NTP time; offline, time only advances while the app runs (from `seen` in
`state.json`), so time with the PC off counts once internet time is back. Spoofing NTP (hosts
file, firewall redirect) needs admin. The password hash is in the binary, so a short password can
be brute-forced offline.

### Install / remove

`npm run build` makes a per-user NSIS installer (`src-tauri\target\release\bundle\nsis\`), no
admin needed; the MSI target is off, since it always installs machine-wide. The installer's hooks
(`windows/hooks.nsh`) run the app with `--install` (register the task for this user,
`LeastPrivilege`, from XML via `schtasks /Create /XML`, then start the pair) and `--uninstall`
(remove the task, the saved block, `%LOCALAPPDATA%\SiteBlocker` and the processes). Per the
user's decision, uninstall is **refused during a timed block** (exit code 2, judged by internet
time), so it is not a way around the block. Updates (`/UPDATE`) skip the uninstall hook.

Mutex check: the app holds its mutex on its own thread, so another process's zero-timeout wait
*times out* while the app runs (`watchdog::is_held`). A test that claims and checks on the same
thread proves nothing, since a thread may take a mutex it already owns.

## Dev workflow

- The window starts hidden (`visible: false`); click the tray icon to see it, also in dev.
- Run `npm run dev` as a background task; it rebuilds and relaunches on changes in `src-tauri/`. Changes in `src/` are not picked up automatically – `touch src-tauri/src/main.rs` to force a relaunch.
- Unlocking with the password quits the app, which also ends `tauri dev`; restart it afterwards.
- Dev log lines contain ANSI colour codes (`Running\e[0m \`target\...`), so patterns like `'Running \`'` never match. Wait for a relaunch by matching `tray-app.exe`, and check the real log text before writing any wait loop.
- Edit JS/HTML containing escapes (`\n`, regexes) with the Write/Edit tools, not heredocs or Python string replacement – escapes got mangled into literal newlines twice. Syntax-check the page script with `node --check` after edits.
- `cargo test` (in `src-tauri/`) covers URL matching.

## User constraints

- Windows only for now.
- No browser extensions, no Obsidian plugins, no launch flags for Chrome/Obsidian; everything must live in the Tauri app.
- Never switch tabs visibly to inspect them.
- Reading browser profile data (Chrome `Sessions/`) and enabling Obsidian's Node inspector (`process._debugProcess`) were blocked by the permission classifier; don't retry without the user's explicit OK.

## Findings (don't re-investigate)

- Chromium browsers via UI Automation: every tab's title and Close button are visible, but only the visible pages' URLs. Each visible page's root `Document` has its URL as Value; documents are grandchildren of the `MultiContentsView` pane (two of them in split view, where the address bar only shows one). Split-view tabs are both selected and named "<title> - Left view" / "- Right view". The address bar (class `OmniboxViewViews`) is the fallback. Background tabs have no URL anywhere in UIA. Tabs are class `Tab`; windows are class `Chrome_WidgetWin_1` (shared with Electron apps, which have no omnibox).
- Obsidian (Electron) exposes no accessibility tree: UIA, MSAA, Chromium's screen-reader honeypot and `SPI_SETSCREENREADER` all failed. Its window title is stale (shows the web viewer's first page).
- Obsidian web viewer content runs in renderers with `--type=renderer --enable-sandbox`; its own UI renderer has `--no-sandbox`. Killing the sandboxed renderer greys the view, but Obsidian then quits cleanly on the next navigation – so the app closes Obsidian via UIA WindowPattern instead.
