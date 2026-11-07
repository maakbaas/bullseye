//! What the closer does besides pages: closing the programs on a session's list and the programs of its
//! Store apps, Obsidian when it shows web content, and windows (or Steam's browser) by their title.

use std::time::Instant;

use super::win::{ask_to_close, top_level_windows, SHELL_CLASSES};
use super::{explain, Closer, KILL_AFTER, MINIMIZE_AFTER};
use crate::programs;
use crate::session::{Active, TitleAction};
use crate::store_apps;
use crate::watchdog::note;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
use uiautomation::patterns::UIWindowPattern;
use uiautomation::types::TreeScope;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ACCESS_DENIED, HWND};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, ShowWindowAsync, SW_MINIMIZE, WM_CLOSE};

impl Closer {
    /// Looks at the running programs, once, for the checks below (`close_obsidian_with_web_content`,
    /// `close_processes`, `close_windows_by_title`). Command lines are only fetched when Obsidian's
    /// check needs them, and where each program is only when Store apps are to be closed.
    pub(super) fn scan_processes(&mut self, command_lines: bool, locations: bool) {
        let mut kind = ProcessRefreshKind::nothing();
        if command_lines {
            kind = kind.with_cmd(UpdateKind::OnlyIfNotSet);
        }
        if locations {
            kind = kind.with_exe(UpdateKind::OnlyIfNotSet);
        }
        self.system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    }

    /// Closes Obsidian (like clicking its X) when it hosts web content. Obsidian's own UI runs in
    /// an unsandboxed renderer (it needs Node); web viewer pages run in sandboxed ones. Ending
    /// those renderers instead makes Obsidian quit on the next navigation, so close it cleanly.
    pub(super) fn close_obsidian_with_web_content(&mut self) {
        let obsidian = |p: &&sysinfo::Process| p.name().eq_ignore_ascii_case("obsidian.exe");
        let has = |p: &sysinfo::Process, flag: &str| p.cmd().iter().any(|a| a.to_string_lossy() == flag);
        let processes = self.system.processes();
        let web_content = processes.values().filter(obsidian).any(|p| has(p, "--type=renderer") && has(p, "--enable-sandbox"));
        if !web_content {
            return;
        }
        // Main process: the one without a --type= switch. Its top-level windows are the vault windows.
        let main_pids: Vec<u32> = processes
            .values()
            .filter(obsidian)
            .filter(|p| !p.cmd().iter().any(|a| a.to_string_lossy().starts_with("--type=")))
            .map(|p| p.pid().as_u32())
            .collect();
        let close = || -> uiautomation::Result<()> {
            for window in self.automation.get_root_element()?.find_all(TreeScope::Children, &self.automation.create_true_condition()?)? {
                if main_pids.contains(&window.get_process_id()?) {
                    if let Ok(pattern) = window.get_pattern::<UIWindowPattern>() {
                        pattern.close()?;
                        note("app", &format!("closed Obsidian, which showed web content ({})", self.woken.describe()));
                    }
                }
            }
            Ok(())
        };
        if let Err(e) = close() {
            self.fail(&format!("could not close Obsidian: {}", explain(&e)));
        }
    }

    /// Closes the programs on the running sessions' lists, and the programs of the Store apps on
    /// them (a packaged app's program runs from its own folder, which names the app:
    /// `store_apps::family_of`): asks each to close like the X of its windows does, and ends the
    /// ones still running `KILL_AFTER` later (or that have no window to ask). Programs Windows
    /// needs are never touched (`ProgramList::is_blocked`), nor Windows' own apps, nor is this app.
    pub(super) fn close_processes(&mut self, active: &Active) {
        let me = std::process::id();
        let mut running = Vec::new();
        for (pid, process) in self.system.processes() {
            let pid = pid.as_u32();
            let in_a_blocked_app = || process.exe().and_then(store_apps::family_of).is_some_and(|family| active.blocks_store_app(&family));
            if pid == me || !(active.blocks_program(&process.name().to_string_lossy()) || in_a_blocked_app()) {
                continue;
            }
            running.push(pid);
            match self.asked.get(&pid) {
                None => {
                    ask_to_close(pid);
                    self.asked.insert(pid, Instant::now());
                    note("app", &format!("asked {} ({pid}) to close ({})", process.name().to_string_lossy(), self.woken.describe()));
                }
                Some(since) if since.elapsed() >= KILL_AFTER => {
                    let ended = process.kill();
                    note("app", &format!("{} {} ({pid})", if ended { "ended" } else { "could not end" }, process.name().to_string_lossy()));
                }
                Some(_) => {}
            }
        }
        // A program that has gone, whether it closed or was ended, is not remembered.
        self.asked.retain(|pid, _| running.contains(pid));
    }

    /// Closes or minimizes the windows whose title is on the running sessions' lists
    /// (`Active::title_action`). Closing is as the X does, and again every `KILL_AFTER` for one that
    /// stays; only the window, never its program, so a browser window goes with all its tabs and
    /// nothing else is ended. Minimizing is asked once; a window still up `MINIMIZE_AFTER` later
    /// cannot be minimized and is left alone (never closed instead), and dialogs and tool windows
    /// are not minimized, as they go with the window they belong to. Not windows of programs Windows
    /// needs (`programs::is_protected`; File Explorer's windows can go, but not the desktop and the
    /// taskbar, `SHELL_CLASSES`), nor this app's.
    pub(super) fn windows_by_title(&mut self, active: &Active) {
        let me = std::process::id();
        let windows = top_level_windows();
        let steam_browser = active.closes_steam_browser();
        let mut closing = Vec::new();
        let mut minimizing = Vec::new();
        for window in &windows {
            if window.pid == me || SHELL_CLASSES.contains(&window.class.as_str()) {
                continue;
            }
            let action = active.title_action(&window.title, window.minimized);
            if action.is_none() && !(steam_browser && window.title == programs::STEAM_BROWSER_TITLE) {
                continue;
            }
            if action == Some(TitleAction::Minimize) && (window.owned || window.tool || self.unminimizable.contains(&window.handle)) {
                continue;
            }
            // Without knowing the program (it may have just started) the window is left for the next pass.
            let Some(process) = self.system.process(sysinfo::Pid::from_u32(window.pid)) else { continue };
            let program = process.name().to_string_lossy();
            let action = match action {
                Some(action) => action,
                None if programs::is_steam_browser(&window.title, &window.class, &program) => TitleAction::Close,
                None => continue,
            };
            if programs::is_protected(&program) && !program.eq_ignore_ascii_case("explorer.exe") {
                continue;
            }
            match action {
                TitleAction::Close => {
                    closing.push(window.handle);
                    let due = match self.titles_asked.get(&window.handle) {
                        Some(since) => since.elapsed() >= KILL_AFTER,
                        None => true,
                    };
                    if due {
                        // SAFETY: a window handle Windows handed over a moment ago; a stale one only fails.
                        let posted = unsafe { PostMessageW(window.handle as HWND, WM_CLOSE, 0, 0) } != 0;
                        let error = last_error();
                        self.titles_asked.insert(window.handle, Instant::now());
                        let what = format!("the window “{}” of {program}", window.title);
                        note("app", &asked(posted, &format!("asked {what} to close ({})", self.woken.describe()), &what, error));
                    }
                }
                TitleAction::Minimize => {
                    minimizing.push(window.handle);
                    match self.minimize_asked.get(&window.handle) {
                        Some(since) if since.elapsed() >= MINIMIZE_AFTER => {
                            self.unminimizable.insert(window.handle);
                            note("app", &format!("the window “{}” of {program} does not minimize; it is left alone", window.title));
                        }
                        Some(_) => {}
                        None => {
                            // SAFETY: as above; asynchronous, so a window that does not answer holds nothing up.
                            let started = unsafe { ShowWindowAsync(window.handle as HWND, SW_MINIMIZE) } != 0;
                            let error = last_error();
                            self.minimize_asked.insert(window.handle, Instant::now());
                            let what = format!("the window “{}” of {program}", window.title);
                            note("app", &asked(started, &format!("minimized {what} ({})", self.woken.describe()), &what, error));
                        }
                    }
                }
            }
        }
        // A window that has gone, or went down, is not remembered (one brought up again is asked again);
        // one that would not minimize is remembered for as long as it is open.
        self.titles_asked.retain(|handle, _| closing.contains(handle));
        self.minimize_asked.retain(|handle, _| minimizing.contains(handle));
        self.unminimizable.retain(|handle| windows.iter().any(|window| window.handle == *handle));
    }
}

/// Windows' last error on this thread, read right after the call that failed.
fn last_error() -> u32 {
    // SAFETY: plain query with no arguments.
    unsafe { GetLastError() }
}

/// The log line for asking `what` to close or minimize: `done` when Windows took the request, else
/// why not. Refused access is a program with more rights than this worker's.
fn asked(taken: bool, done: &str, what: &str, error: u32) -> String {
    match (taken, error) {
        (true, _) => done.to_string(),
        (false, ERROR_ACCESS_DENIED) => {
            format!("Windows refused to pass the request to {what}: it runs with more rights than Bullseye's worker")
        }
        (false, error) => format!("could not reach {what} (error {error})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_windows_refused_is_logged_as_refused() {
        let what = "the window “x” of y.exe";
        assert_eq!(asked(true, "asked it", what, ERROR_ACCESS_DENIED), "asked it");
        assert_eq!(
            asked(false, "asked it", what, ERROR_ACCESS_DENIED),
            "Windows refused to pass the request to the window “x” of y.exe: it runs with more rights than Bullseye's worker"
        );
        assert_eq!(asked(false, "asked it", what, 1400), "could not reach the window “x” of y.exe (error 1400)");
    }
}
