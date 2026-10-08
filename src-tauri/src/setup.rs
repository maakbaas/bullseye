//! Installing and uninstalling: what the Windows installer runs (`windows/hooks.nsh`), as an
//! administrator, as `bullseye-app.exe --install` after copying the app and `--uninstall` before
//! removing it. Installing is from a clean machine only; there is no update path. Both also put the
//! installation folder on the system `PATH` and take it off again (`system_path.rs`), so the CLI,
//! `bullseye-app.exe`, runs from any terminal.
//!
//! The installer cannot stop the service itself (nobody but SYSTEM can), so it asks it, over the
//! pipe. The service refuses while a block is locked, so this is no way around a block: the way out
//! of a locked block stays waiting for it to end or the unlock password.

use std::thread;
use std::time::Duration;

use crate::proto::{Request, Response};
use crate::watchdog::note;
use crate::{client, registration, system_path};

/// Exit codes the installer hooks act on.
pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const REFUSED: i32 = 2;

/// The folder the app (and the CLI next to it) is installed in, as `PATH` names it.
fn folder() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.to_string_lossy().into_owned())
}

/// Registers the service and starts it. The service starts the app for whoever is logged on. The
/// CLI not being on `PATH` is noted, not a failure: the service is what matters.
pub fn install() -> i32 {
    let Ok(exe) = std::env::current_exe() else { return FAILED };
    match registration::install(&exe) {
        Ok(()) => {
            note("install", &format!("registered the {} service for {}", registration::NAME, exe.display()));
            if let Some(dir) = folder() {
                if let Err(e) = system_path::add(&dir) {
                    note("install", &format!("could not put {dir} on PATH: {e}"));
                }
            }
            OK
        }
        Err(e) => {
            note("install", &e);
            FAILED
        }
    }
}

/// Removes the service and everything it kept, ends the app and takes its folder off `PATH`.
/// Refused while a block is locked, and then `PATH` stays as it is.
pub fn uninstall() -> i32 {
    let done = remove_service();
    if done == OK {
        if let Some(dir) = folder() {
            if let Err(e) = system_path::remove(&dir) {
                note("uninstall", &format!("could not take {dir} off PATH: {e}"));
            }
        }
    }
    done
}

fn remove_service() -> i32 {
    let (what, request) = ("uninstall", Request::PrepareUninstall);
    // Nothing to ask: never installed, or already removed.
    if !registration::exists() {
        return OK;
    }
    let mut answer = client::call(&request);
    if answer.is_err() {
        // Not running. Administrators may start it.
        let _ = registration::start_registered();
        for _ in 0..10 {
            thread::sleep(Duration::from_secs(1));
            answer = client::call(&request);
            if answer.is_ok() {
                break;
            }
        }
    }
    match answer {
        Ok(Response::Done) => {
            // The exe is in use until the service has ended.
            if registration::wait_stopped(Duration::from_secs(30)) {
                OK
            } else {
                note(what, "the service did not end in time");
                FAILED
            }
        }
        Ok(Response::Refused(why)) => {
            note(what, &format!("refused: {why}"));
            REFUSED
        }
        Ok(other) => {
            note(what, &format!("unexpected answer: {other:?}"));
            FAILED
        }
        Err(e) => {
            note(what, &e);
            FAILED
        }
    }
}
