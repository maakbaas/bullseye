//! The Windows service: the root of everything. It runs as SYSTEM, started by Windows at boot
//! and started again by Windows if it ever fails, and it holds what must not be in the user's
//! reach: the block definitions, the running blocks, internet time, the unlock hash.
//!
//! It cannot close tabs itself (a service has no desktop), so the app does that, and this keeps
//! the app running (`watchdog.rs`) and tells it what to enforce, over a named pipe (`pipe.rs`).
//!
//! Who may stop it is decided by its access list (`SDDL`, set by `install`): administrators can
//! look at it and start it, and only SYSTEM can stop, change or delete it. The service does not
//! even accept a stop request. It ends only by the installer asking it to over the pipe, which
//! it refuses during a block (`handle`).
//!
//! `main` starts it with `--service`, which only Windows does. Development has no service at all:
//! a debug build runs the same request handling inside the app (`embedded`).
//!
//! This file is the running service: talking to the service manager, starting everything, and the
//! data folder. What it does with a request is `handler.rs`; how it is installed, locked down and
//! removed is `registration.rs`.

use std::ffi::OsString;
use std::fs;
use std::os::windows::process::CommandExt;
#[cfg(debug_assertions)]
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use windows_service::define_windows_service;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType, SessionChangeReason,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle};
use windows_service::service_dispatcher;

use crate::block::Blocklist;
use crate::definitions::Definitions;
#[cfg(any(debug_assertions, test))]
use crate::client::Local;
use crate::handler::{handle, Context, Shutdown};
use crate::registration::{self, CREATE_NO_WINDOW, NAME};
use crate::state;
use crate::sync::spawn_critical;
use crate::watchdog::{self, note, AppSlot};
use crate::{clock, pipe};

define_windows_service!(ffi_service_main, service_main);

/// `--service`: hands this process to the service manager. Returns the exit code.
pub fn run_dispatcher() -> i32 {
    watchdog::in_service();
    match service_dispatcher::start(NAME, ffi_service_main) {
        Ok(()) => 0,
        Err(e) => {
            note("service", &format!("cannot reach the service manager (was this started by hand?): {e}"));
            1
        }
    }
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        note("service", &e);
    }
}

fn run_service() -> Result<(), String> {
    let status = service_control_handler::register(NAME, |control| match control {
        // A user logging on, or coming back to their session: the app is wanted there.
        ServiceControl::SessionChange(change) => {
            if matches!(change.reason, SessionChangeReason::SessionLogon | SessionChangeReason::ConsoleConnect) {
                watchdog::LAUNCH_APP.store(true, Ordering::SeqCst);
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        // Stop is not accepted at all (see `report`), so it never arrives.
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(|e| format!("cannot register with the service manager: {e}"))?;

    report(&status, ServiceState::StartPending, ServiceControlAccept::empty(), 0);
    let quit = match start_engine(true) {
        Ok(quit) => quit,
        Err(e) => {
            // A failure exit code, so Windows starts the service again.
            report(&status, ServiceState::Stopped, ServiceControlAccept::empty(), 1);
            return Err(e);
        }
    };
    // Only session changes are accepted. Not STOP: a service that does not accept it cannot be
    // stopped with `sc stop` or the services list, by anyone.
    report(&status, ServiceState::Running, ServiceControlAccept::SESSION_CHANGE, 0);
    note("service", "running");

    let Ok(shutdown) = quit.recv() else {
        report(&status, ServiceState::Stopped, ServiceControlAccept::empty(), 1);
        return Err("the pipe thread ended".into());
    };
    watchdog::kill_apps();
    if matches!(shutdown, Shutdown::Remove) {
        registration::remove();
    }
    note("service", "ending at the installer's request");
    // Exit code 0: Windows does not start a service again that ended cleanly.
    report(&status, ServiceState::Stopped, ServiceControlAccept::empty(), 0);
    Ok(())
}

fn report(handle: &ServiceStatusHandle, state: ServiceState, accepted: ServiceControlAccept, code: u32) {
    let _ = handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accepted,
        exit_code: ServiceExitCode::Win32(code),
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    });
}

/// Starts everything the service does. Returns what tells it to end. `for_real` is the service
/// itself: it locks its data folder down and looks after the app. Without it (the pipe test) it
/// only serves requests.
fn start_engine(for_real: bool) -> Result<mpsc::Receiver<Shutdown>, String> {
    if for_real {
        lock_down_data_dir();
    }
    let blocklist = start_blocklist();
    let slot = Arc::new(AppSlot::new(for_real));
    if for_real {
        watchdog::spawn_supervisor(blocklist.clone(), slot.clone());
    }
    let (quit, quit_rx) = mpsc::channel();
    let context = Arc::new(Context::new(blocklist, Definitions::new(), slot));
    pipe::serve(move |caller, request| handle(&context, caller, request), quit)?;
    Ok(quit_rx)
}

/// The running blocks, as the service has them: internet time on, blocks left behind by an earlier
/// run picked up again (so a restart does not end them), and a thread that ends a timed block whose
/// time is up and keeps the time on disk moving for a restart while offline (`Saved::seen`), even
/// when no app is asking.
fn start_blocklist() -> Blocklist {
    clock::spawn_sync();
    let blocklist = Blocklist::new();
    blocklist.restore();
    let ticking = blocklist.clone();
    spawn_critical("ticker", move || loop {
        ticking.active();
        thread::sleep(Duration::from_secs(1));
    });
    blocklist
}

/// The service's logic without a service: every request answered in this process, for
/// `Remote::local`. Nobody can talk to it but the app that holds it, and nothing looks after the
/// app (`AppSlot::new(false)`).
#[cfg(any(debug_assertions, test))]
pub fn local_handler(blocklist: Blocklist, definitions: Definitions) -> Local {
    let context = Context::new(blocklist, definitions, Arc::new(AppSlot::new(false)));
    Arc::new(move |request| handle(&context, 0, request).0)
}

/// For `npm run dev`: the service inside the app, with no service installed, no pipe and no
/// administrator rights. **Debug builds only**, and not used with `BULLSEYE_USE_SERVICE` set.
/// Ending the app ends the enforcing, which is fine while developing. The blocks are kept in
/// `%LOCALAPPDATA%\bullseye-dev`, so a timed block is still there the next time; delete that
/// folder to forget it.
#[cfg(debug_assertions)]
pub fn embedded() -> Local {
    use_dev_dir();
    local_handler(start_blocklist(), Definitions::new())
}

/// Development keeps its files apart from a real install's: `%LOCALAPPDATA%\bullseye-dev`.
#[cfg(debug_assertions)]
fn use_dev_dir() {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        state::use_dir(Path::new(&local).join("bullseye-dev"));
    }
}

/// Makes the data folder SYSTEM's: owned by SYSTEM, full control for SYSTEM, read-only for
/// administrators (so the log can be read), nothing for anyone else, and no inheriting from
/// `%ProgramData%`, where every user may create files. SIDs rather than names, so this works on
/// any language of Windows.
fn lock_down_data_dir() {
    let Some(dir) = state::dir() else { return };
    if let Err(e) = fs::create_dir_all(&dir) {
        return note("service", &format!("cannot create {}: {e}", dir.display()));
    }
    for args in [
        vec!["/setowner", "*S-1-5-18"],
        vec!["/inheritance:r", "/grant:r", "*S-1-5-18:(OI)(CI)F", "/grant:r", "*S-1-5-32-544:(OI)(CI)RX"],
    ] {
        match Command::new("icacls.exe").arg(&dir).args(&args).creation_flags(CREATE_NO_WINDOW).output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => note("service", &format!("icacls {args:?}: {}", String::from_utf8_lossy(&output.stdout).trim())),
            Err(e) => note("service", &format!("cannot run icacls: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tests::definition;
    use crate::definitions::Definition;
    use crate::lock::Lock;
    use crate::proto::{Request, Response};
    use std::time::Duration;

    /// Runs the real pipe server in this process and talks to it like the app and the installer
    /// do. Needs no service installed, but no service may be running either (it would hold the
    /// pipe): `cargo test -- --ignored the_pipe`. Takes about 15 seconds.
    #[test]
    #[ignore]
    fn the_pipe_answers_like_the_app_expects() {
        use crate::{client, proto};
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!("service-pipe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        state::use_dir(dir);
        let quit = start_engine(false).unwrap();

        // This server runs in the user's session, not in session 0: the app refuses to talk to it.
        std::env::remove_var("BULLSEYE_DEV");
        let refused = client::call(&Request::Status).unwrap_err();
        assert!(refused.contains("not by the service"), "{refused}");
        // The test lets the app talk to it anyway, which only a test build allows (`pipe::verify_server`).
        std::env::set_var("BULLSEYE_DEV", "1");

        // This engine looks after no app, so taking one in is always fine.
        assert_eq!(client::call(&Request::Register), Ok(Response::Done));
        let Ok(Response::Status(status)) = client::call(&Request::Status) else { panic!("no status") };
        assert!(status.running.is_empty());
        let block = Definition { lock: Lock::Timer { duration: "1h".into() }, ..definition("a", &["a.com"]) };
        assert_eq!(client::call(&Request::Save { block }), Ok(Response::Done));
        assert_eq!(client::call(&Request::Start { id: "a".into() }), Ok(Response::Done));
        let Ok(Response::Policy(policies)) = client::call(&Request::Policy) else { panic!("no policy") };
        assert_eq!(policies[0].filters, vec!["a.com"]);
        assert!(matches!(client::call(&Request::Stop { id: "a".into() }), Ok(Response::Failed(_))));
        assert!(matches!(client::call(&Request::PrepareUninstall), Ok(Response::Refused(_))));

        // A length far beyond the limit gets an error back, not an allocation.
        let mut pipe = crate::pipe::connect().unwrap();
        pipe.write_all(&u32::MAX.to_le_bytes()).unwrap();
        assert!(matches!(proto::read_message::<Response>(&mut pipe), Ok(Response::Failed(_))));
        drop(pipe);

        // Several requests at once, as the app's ticks and the UI's polls make.
        let threads: Vec<_> = (0..6)
            .map(|_| std::thread::spawn(|| assert!(matches!(client::call(&Request::Status), Ok(Response::Status(_))))))
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        // A client that connects and says nothing is cut off, and does not hold the service up.
        let mut silent = crate::pipe::connect().unwrap();
        assert!(matches!(client::call(&Request::Status), Ok(Response::Status(_))));
        std::thread::sleep(Duration::from_secs(13));
        // What it gets is a refusal, or a closed pipe; never an answer to a request it did not make.
        assert!(matches!(proto::read_message::<Response>(&mut silent), Ok(Response::Failed(_)) | Err(_)));

        // The password ends the block; then the installer may update, and the service says so.
        let unlock = Request::Unlock { password: "test1234".into() };
        assert_eq!(client::call(&unlock), Ok(Response::Done));
        assert_eq!(client::call(&Request::PrepareUpdate), Ok(Response::Done));
        assert!(matches!(quit.recv_timeout(Duration::from_secs(5)), Ok(Shutdown::Update)));
    }
}
