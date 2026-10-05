//! Installing and uninstalling: what the Windows installer runs (`windows/hooks.nsh`), as an
//! administrator, as `bullseye.exe --install` after copying the app and `--uninstall` before
//! removing it. Installing is from a clean machine only; there is no update path.
//!
//! The installer cannot stop the service itself (nobody but SYSTEM can), so it asks it, over the
//! pipe. The service refuses while a block is locked, so this is no way around a block: the way out
//! of a locked block stays waiting for it to end or the unlock password.

use std::thread;
use std::time::Duration;

use crate::proto::{Request, Response};
use crate::watchdog::note;
use crate::{client, registration};

/// Exit codes the installer hooks act on.
pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const REFUSED: i32 = 2;

/// Registers the service and starts it. The service starts the app for whoever is logged on.
pub fn install() -> i32 {
    let Ok(exe) = std::env::current_exe() else { return FAILED };
    match registration::install(&exe) {
        Ok(()) => {
            note("install", &format!("registered the {} service for {}", registration::NAME, exe.display()));
            OK
        }
        Err(e) => {
            note("install", &e);
            FAILED
        }
    }
}

/// Removes the service and everything it kept, and ends the app. Refused while a block is locked.
pub fn uninstall() -> i32 {
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
