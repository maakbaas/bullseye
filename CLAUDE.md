# Site Blocker (Tauri tray app, Windows only)

Blocks sites listed in the UI (closes matching Chromium-browser tabs), closes programs by exe name, and closes Obsidian when it shows web content. The block is held by a **SYSTEM Windows service**; the tray app does the closing. `detailed-instructions.md` is the long design guide with the reasoning, measurements and a test table; this file is the short version.

- `src/index.html` – the whole frontend (no bundler), Fluent styling, Mica; three lists (block list, always-allowed sites, programs) of 5 lines each, no page scroll
- `src-tauri/src/main.rs` – entry modes (`--service`, `--install`, `--uninstall`, `--prepare-update`, otherwise the app), tray, and the `start_block` / `stop_block` / `block_status` / `force_exit` commands, which only forward to the service
- `src-tauri/src/ui.rs` – the window: none at startup, made on a tray click (shown once the page has drawn), its X only hides it, destroyed after 5 minutes hidden (the web view is almost all of the app's memory)
- `src-tauri/src/service.rs` – the running SYSTEM service (service manager, start-up, data folder lock-down); `handler.rs` – what it does with each request (the same code over the pipe and, in debug builds, inside the app); `registration.rs` – installing, locking down (SDDL), starting and removing it
- `src-tauri/src/pipe.rs`, `proto.rs`, `client.rs` – the named pipe `\\.\pipe\SiteBlocker`: server, messages, and the app's side (keeps the last policy in memory; `Remote::local` is the in-process variant)
- `src-tauri/src/block.rs` – the running block (lists, end time, enforcers); timed blocks (`5s`/`10m`/`3h`/`1d`) are enforced here, never in the UI
- `src-tauri/src/clock.rs` – internet time (SNTP) that timed blocks are measured against, so moving the Windows clock does nothing
- `src-tauri/src/unlock.rs` – the unlock password, kept only as a salted PBKDF2-SHA256 hash
- `src-tauri/src/filters.rs` – site lists: plain address prefixes plus `*` (user rejected full uBlock syntax as too complex). Used for the block list and the always-allowed list, which **wins**. Only web pages are matched, never the browser's own pages, so a lone `*` blocks every site and leaves the browser usable
- `src-tauri/src/programs.rs` – programs to close: exe names with `*`, `.exe` added when missing, and a protected list (Windows' own, this app, WebView2) that is never closed and refuses entries that would match it
- `src-tauri/src/tab_closer.rs` – 1-second worker in the app doing the closing, as enforcers (`state::Enforcers`, checkboxes in the UI, fixed while a block runs): with the site list, Chromium tabs and split view; with the program list, programs (asked to close, ended after 3 s); without a list, Obsidian's web viewer
- `src-tauri/src/state.rs` – the block on disk (`%ProgramData%\SiteBlocker\state.json`, SYSTEM-owned) and the `Enforcers` table (one place to add an enforcer)
- `src-tauri/src/watchdog.rs` – the service's watch over the app (`AppSlot`) and the log writer
- `src-tauri/src/sync.rs` – `locked()` (locks that survive a crash) and `spawn_critical` (a crash in a long-lived thread ends the process, so Windows restarts it)
- `src-tauri/src/setup.rs` + `src-tauri/windows/hooks.nsh` – `--install` / `--uninstall` / `--prepare-update`, run by the NSIS installer

## How the pieces keep each other alive

The `SiteBlocker` service runs as SYSTEM, starts at boot, and holds the block, internet time and the unlock hash. The tray app (user session) asks it every second what to enforce, does the closing, and keeps no state; it keeps enforcing the last answer if the service is briefly unreachable.

- The service does not accept a stop request, and its access list (`registration.rs`) gives only SYSTEM stop / change / delete. If it fails by itself, Windows restarts it at once (failure actions).
- The service starts the app at every logon and, **while any block runs**, starts it again within ~1 s of it ending. Which process is "the app" is the service's decision: the one it launched, or the installed exe in the console session that registered; never a matter of the process name. An app that has not asked for its policy for 5 s (20 s after starting) during a block is ended and started again, which covers a suspended app. After a sleep the app gets a fresh start instead.
- If the service is not running, the app asks Windows to start it (interactive users may *start* it, nothing more).
- Reboot mid-block: the service reads `state.json` back and resumes with the time left.

**"Unlock"** in the UI (`force_exit`, password `test1234`, stored only as a hash in `unlock.rs`, which says how to change it) is checked by the service, rate-limited after 3 wrong guesses. It clears the block and the app quits; the service keeps running idle and starts the app again at the next logon. It is the only way to end a timed block early.

### Deliberately not hidden

Per the user's decision: nothing here is obscured. The service is named `SiteBlocker` in the services list, the app runs under its own name in Task Manager, it is listed under Settings > Apps, and `%ProgramData%\SiteBlocker\service.log` (administrators can read it) and `%LOCALAPPDATA%\SiteBlocker\app.log` say what happened. The mechanism is friction, not secrecy.

**The honest ceiling** (do not claim otherwise in the UI): an administrator who knows how can run a shell as SYSTEM (for example PsExec `-s`), take ownership of the service object or its data folder and rewrite them, delete the service from the Windows recovery environment, or edit the registry offline. The app is a process of the user it protects against, so someone who can inject code into it can keep it asking for its policy while it enforces nothing. The deadline is in NTP time; offline, time only advances while the service runs (from `seen` in `state.json`). Spoofing NTP needs admin. The password hash is in the binary, so a short password can be brute-forced offline. What changed from the old per-user design is the effort: "End task" no longer works.

### Install / remove

`npm run build` makes a **per-machine** NSIS installer (`src-tauri\target\release\bundle\nsis\`), which needs admin and installs to `C:\Program Files\Site Blocker` (never a user-writable folder: the exe runs as SYSTEM). The hooks (`windows/hooks.nsh`) run the app with `--prepare-update` (before replacing files; refused during any block), `--install` (create the service, lock it down, safe-mode keys, start it) and `--uninstall` (the service removes itself, its data and the app). Uninstall is **refused during a timed block** (exit code 2, internet time). If a service is registered but does not answer, the uninstall also stops (exit code 1, nothing removed); with no service registered it carries on. The installer cannot stop the service itself, so it asks over the pipe. `.github/workflows/build.yml` builds the installer as an artifact for every push.

Recovery when your own test locks you out (commands in `detailed-instructions.md`, section 8.2): test installs in a VM; a SYSTEM shell can `sc delete SiteBlocker` and end the process.

## Dev workflow

- `npm run dev` needs no service: in a debug build the app runs the service's logic inside itself (`service::embedded`), with its files in `%LOCALAPPDATA%\SiteBlocker-dev`. Kill it at will. `SITEBLOCKER_USE_SERVICE=1` talks to a real installed service instead. Release builds contain none of this (`debug_assertions`); `cargo check --release` must stay warning-free.
- There is **no window until you click the tray icon**. Debug aids: `SITEBLOCKER_SHOW=1` (open at start), `SITEBLOCKER_REOPEN_AFTER_SECS=n`, `SITEBLOCKER_UI_TIMEOUT_SECS=n` (shorter keep time).
- Run `npm run dev` as a background task; it rebuilds and relaunches on changes in `src-tauri/`. Changes in `src/` are not picked up automatically – `touch src-tauri/src/main.rs` to force a relaunch.
- Unlocking with the password quits the app, which also ends `tauri dev`; restart it afterwards.
- Dev log lines contain ANSI colour codes (`Running\e[0m \`target\...`), so patterns like `'Running \`'` never match. Wait for a relaunch by matching `tray-app.exe`, and check the real log text before writing any wait loop.
- Edit JS/HTML containing escapes (`\n`, regexes) with the Write/Edit tools, not heredocs or Python string replacement – escapes got mangled into literal newlines several times. Syntax-check the page script with `node --check` after edits.
- `cargo test` (in `src-tauri/`) runs 67 tests. Ignored on purpose: the network clock, printing a new unlock hash, `the_pipe_answers_like_the_app_expects` (`-- --ignored the_pipe`; no service may be installed, it takes the pipe name) and `a_real_browser_tab_is_closed` (needs a Chromium browser window open on example.com). Also run `cargo clippy --all-targets` (clean) and `cargo check --release`.

## User constraints

- Windows only for now.
- No browser extensions, no Obsidian plugins, no launch flags for Chrome/Obsidian; everything must live in the Tauri app.
- Never switch tabs visibly to inspect them.
- Reading browser profile data (Chrome `Sessions/`) and enabling Obsidian's Node inspector (`process._debugProcess`) were blocked by the permission classifier; don't retry without the user's explicit OK.
- Never push to a remote unless asked in that message.

## Findings (don't re-investigate)

- Chromium browsers via UI Automation: every tab's title and Close button are visible, but only the visible pages' URLs. Each visible page's root `Document` has its URL as Value; documents are grandchildren of the `MultiContentsView` pane (two of them in split view, where the address bar only shows one). Split-view tabs are both selected and named "<title> - Left view" / "- Right view". The address bar (class `OmniboxViewViews`) is the fallback. Background tabs have no URL anywhere in UIA. **Tabs are class `Tab` in Chrome/Brave and `EdgeTab` in Edge** (Edge was silently never closing tabs before this was found); windows are class `Chrome_WidgetWin_1` (shared with Electron apps, which have no omnibox).
- **Never search for browser windows with a UI Automation `find_all` on the root element**: it cost 92 ms per pass. A plain `EnumWindows` + class name costs 0.3 ms; then use UI Automation only on the windows found. Creating UI Automation conditions is free, so caching them gains nothing.
- A standard user cannot open a SYSTEM process, not even with `PROCESS_QUERY_LIMITED_INFORMATION`, so the app cannot check who serves the pipe by looking at the process; it asks the pipe for the server's session instead (session 0 = a service). `WTSQueryUserToken` from the service gives the user's limited token, which is what the app should run with.
- A WebView2 app's browser process is shared by every instance using the same user-data folder, so measuring a new instance's processes needs `WEBVIEW2_USER_DATA_FOLDER` set to a fresh folder.
- Obsidian (Electron) exposes no accessibility tree: UIA, MSAA, Chromium's screen-reader honeypot and `SPI_SETSCREENREADER` all failed. Its window title is stale (shows the web viewer's first page).
- Obsidian web viewer content runs in renderers with `--type=renderer --enable-sandbox`; its own UI renderer has `--no-sandbox`. Killing the sandboxed renderer greys the view, but Obsidian then quits cleanly on the next navigation – so the app closes Obsidian via UIA WindowPattern instead.
