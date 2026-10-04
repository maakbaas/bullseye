//! Installing and uninstalling: what the Windows installer runs (`windows/hooks.nsh`), as
//! `SiteBlocker.exe --install` after copying the app and `--uninstall` before removing it.
//!
//! Uninstalling is refused while a timed block runs, so it is not a way around the block: the
//! way out of a timed block stays waiting for it to end or the unlock password.

use std::fs;

use crate::{clock, state, task, watchdog};

/// Exit codes the installer hooks act on.
pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const REFUSED: i32 = 2;

/// Registers the login task and starts the app with its launcher, as a logon would.
pub fn install() -> i32 {
    let Ok(exe) = std::env::current_exe() else { return FAILED };
    let registered = task::install(&exe);
    match &registered {
        Ok(()) => watchdog::note("install", &format!("registered the {} task for {}", task::NAME, exe.display())),
        Err(e) => watchdog::note("install", e),
    }
    // The app runs either way; without the task it just does not come back after a reboot.
    watchdog::spawn_launcher();
    if registered.is_ok() { OK } else { FAILED }
}

/// Removes everything the app leaves outside its install folder: the login task, the saved
/// block, the log, and its running processes. Refused during a timed block.
pub fn uninstall() -> i32 {
    if let Some(left) = timed_block_left() {
        watchdog::note("uninstall", &format!("refused: a timed block has {left}s left"));
        return REFUSED;
    }
    // The saved block first: without it, the app and the launcher stop starting each other.
    if let Some(store) = state::store() {
        store.clear();
    }
    let removed = task::remove();
    watchdog::kill_all();
    if let Some(dir) = state::dir() {
        let _ = fs::remove_dir_all(dir);
    }
    match removed {
        Ok(()) => OK,
        Err(e) => {
            eprintln!("uninstall: {e}");
            FAILED
        }
    }
}

/// Seconds left of a saved timed block, by internet time like the block itself (`clock.rs`), so
/// setting the Windows clock forward does not let the uninstaller through either. Offline, the
/// time the block was last seen running at stands in for it.
fn timed_block_left() -> Option<u64> {
    let saved = state::store()?.load()?;
    let until = saved.until?;
    let clock = clock::global();
    if let Some(seen) = saved.seen {
        clock.at_least(seen);
    }
    clock::sync();
    let now = state::to_unix(clock.now())?;
    until.checked_sub(now).filter(|&left| left > 0)
}
