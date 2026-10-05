//! Installing and uninstalling: what the Windows installer runs (`windows/hooks.nsh`), as an
//! administrator, as `bullseye.exe --prepare-update` before replacing the app, `--install`
//! after copying it, and `--uninstall` before removing it.
//!
//! The installer cannot stop the service itself (nobody but SYSTEM can), so it asks it, over the
//! pipe. The service refuses while a block runs, so none of this is a way around a block: the way
//! out of a timed block stays waiting for it to end or the unlock password.

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

/// Removes the service and everything it kept, and ends the app. Refused during a timed block.
pub fn uninstall() -> i32 {
    ask("uninstall", Request::PrepareUninstall)
}

/// Lets go of the exe so the installer can replace it. Refused during any block.
pub fn prepare_update() -> i32 {
    ask("update", Request::PrepareUpdate)
}

fn ask(what: &str, request: Request) -> i32 {
    // Nothing to ask: never installed, or already removed.
    if !registration::exists() {
        return OK;
    }
    let mut answer = client::call(&request);
    if answer.is_err() {
        // Not running (an update that did not finish, say). Administrators may start it.
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
