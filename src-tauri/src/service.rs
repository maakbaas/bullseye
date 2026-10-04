//! The Windows service: the root of everything. It runs as SYSTEM, started by Windows at boot
//! and started again by Windows if it ever fails, and it holds what must not be in the user's
//! reach: the running block, internet time, the unlock hash.
//!
//! It cannot close tabs itself (a service has no desktop), so the app does that, and this keeps
//! the app running (`watchdog.rs`) and tells it what to enforce, over a named pipe (`pipe.rs`).
//!
//! Who may stop it is decided by its access list (`SDDL`, set by `install`): administrators can
//! look at it and start it, and only SYSTEM can stop, change or delete it. The service does not
//! even accept a stop request. It ends only by the installer asking it to over the pipe, which
//! it refuses during a block (`handle`).
//!
//! Modes, chosen by `main`: `--service` (started by Windows), `--service-console` (the same in a
//! terminal, for development, with its own data folder and no app launching).

use std::ffi::OsString;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows_service::define_windows_service;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept, ServiceErrorControl,
    ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceStatus, ServiceType, SessionChangeReason,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::block::{parse_duration, Blocklist};
use crate::filters::FilterList;
use crate::proto::{Policy, Request, Response};
use crate::state::{self, Enforcers};
use crate::watchdog::{self, note};
use crate::{clock, pipe, unlock};

/// The service's name, exactly as it appears in the services list.
pub const NAME: &str = "SiteBlocker";

/// Who may do what with the service. SYSTEM: everything (it deletes itself on uninstall).
/// Administrators: query, enumerate, start and read the list, but not stop, pause, change,
/// delete or rewrite this list. Interactive users and service accounts: query only.
const SDDL: &str = "D:(A;;CCLCSWRPWPDTLOCRSDRCWDWO;;;SY)(A;;CCLCSWRPLOCRRC;;;BA)(A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";

/// The service is started in safe mode too, when these keys exist.
const SAFE_BOOT: [&str; 2] = [
    r"HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Minimal\SiteBlocker",
    r"HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Network\SiteBlocker",
];

/// `CREATE_NO_WINDOW`: the tools below are console programs, which would flash a window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const ERROR_SERVICE_EXISTS: i32 = 1073;
const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;

/// What the installer asked the service to end for.
pub enum Shutdown {
    /// Let go of the exe so it can be replaced; the installer starts the service again.
    Update,
    /// Remove the service and everything it keeps.
    Remove,
}

/// What `handle` works with.
struct Context {
    blocklist: Blocklist,
    lockout: Mutex<Lockout>,
}

/// Wrong unlock passwords in a row, and until when the next guess is refused.
#[derive(Default)]
struct Lockout {
    failures: u32,
    until: Option<Instant>,
}

// ---------------------------------------------------------------------------------------------
// Running

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
    let quit = match start_engine(false) {
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
        remove_registration();
    }
    note("service", "ending at the installer's request");
    // Exit code 0: Windows does not start a service again that ended cleanly.
    report(&status, ServiceState::Stopped, ServiceControlAccept::empty(), 0);
    Ok(())
}

/// `--service-console`: the service in a terminal, for development. Its own data folder, no app
/// launching (the app runs from `npm run dev`), and an installer request just ends it.
pub fn run_console() {
    watchdog::in_service();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        state::use_dir(Path::new(&local).join("SiteBlocker-dev"));
    }
    match start_engine(true) {
        Ok(quit) => {
            eprintln!("service: running in the console; data in {:?}", state::dir());
            let _ = quit.recv();
        }
        Err(e) => eprintln!("service: {e}"),
    }
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

/// Starts everything the service does. Returns what tells it to end.
fn start_engine(console: bool) -> Result<mpsc::Receiver<Shutdown>, String> {
    if !console {
        lock_down_data_dir();
    }
    clock::spawn_sync();
    let blocklist = Blocklist::new();
    // Pick up a block that was running when the service or Windows last stopped.
    blocklist.restore();
    {
        // Ends a timed block whose time is up, and keeps the time on disk moving for a restart
        // while offline (`Saved::seen`), even when no app is asking.
        let blocklist = blocklist.clone();
        thread::spawn(move || loop {
            blocklist.active();
            thread::sleep(Duration::from_secs(1));
        });
    }
    if !console {
        watchdog::spawn_supervisor(blocklist.clone());
    }
    let (quit, quit_rx) = mpsc::channel();
    let context = Arc::new(Context { blocklist, lockout: Mutex::default() });
    pipe::serve(move |request| handle(&context, request), quit)?;
    Ok(quit_rx)
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

// ---------------------------------------------------------------------------------------------
// Answering the app and the installer

/// Answers one request, and says whether the service should end afterwards. Everything in the
/// request is checked here again: it comes from any local user's program, not necessarily ours.
fn handle(context: &Context, request: Request) -> (Response, Option<Shutdown>) {
    let blocklist = &context.blocklist;
    let done = |result: Result<(), String>| match result {
        Ok(()) => Response::Done,
        Err(why) => Response::Failed(why),
    };
    match request {
        Request::Status => (Response::Status(blocklist.status()), None),
        Request::Policy => {
            let policy = blocklist.active().map(|(filters, enforcers)| Policy { filters: filters.entries(), enforcers });
            (Response::Policy(policy), None)
        }
        Request::Start { filters, duration, enforcers } => (done(start(blocklist, filters, &duration, enforcers)), None),
        Request::Stop => (done(blocklist.stop().map_err(|_| "A timed block cannot be stopped early".to_string())), None),
        Request::Unlock { password } => (done(unlock(context, &password)), None),
        // Refused during a timed block, judged by internet time, like the block itself.
        Request::PrepareUninstall => match blocklist.status().remaining {
            Some(left) => (Response::Refused(format!("a timed block has {left}s left")), None),
            None => (Response::Done, Some(Shutdown::Remove)),
        },
        // Stricter: replacing the exe is also a way to end the service, so no block at all.
        Request::PrepareUpdate => match blocklist.filters() {
            Some(_) => (Response::Refused("a block is running".into()), None),
            None => (Response::Done, Some(Shutdown::Update)),
        },
    }
}

/// What `start_block` in the app used to check, now checked where it counts.
fn start(blocklist: &Blocklist, filters: Vec<String>, duration: &str, enforcers: Enforcers) -> Result<(), String> {
    let duration = parse_duration(duration)?;
    let filters = FilterList::parse(&filters).map_err(|_| "Invalid block list syntax".to_string())?;
    if !enforcers.any() {
        return Err("Choose at least one enforcer".into());
    }
    // Only the enforcers that go by the list need one; Obsidian's does not.
    if filters.is_empty() && enforcers.use_block_list() {
        return Err("Enter at least one site to block".into());
    }
    blocklist.start(filters, duration, enforcers)
}

/// Ends the block for the unlock password. After three wrong guesses in a row, guesses are
/// refused for longer each time. The lock is held while the hash is computed, so guesses cannot
/// be made in parallel either.
fn unlock(context: &Context, password: &str) -> Result<(), String> {
    let mut lockout = context.lockout.lock().unwrap();
    if let Some(left) = lockout.until.and_then(|until| until.checked_duration_since(Instant::now())) {
        return Err(format!("Too many wrong passwords; wait {}s", left.as_secs() + 1));
    }
    if !unlock::check(password) {
        lockout.failures += 1;
        if lockout.failures >= 3 {
            let seconds = 5u64 << (lockout.failures - 3).min(8);
            lockout.until = Some(Instant::now() + Duration::from_secs(seconds));
        }
        return Err("Wrong password".into());
    }
    *lockout = Lockout::default();
    context.blocklist.reset();
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Installing, starting and removing (the installer runs these as an administrator)

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager, String> {
    ServiceManager::local_computer(None::<&str>, access).map_err(|e| format!("cannot reach the service manager: {e}"))
}

fn is_os_error(error: &windows_service::Error, code: i32) -> bool {
    matches!(error, windows_service::Error::Winapi(e) if e.raw_os_error() == Some(code))
}

/// Registers the service for `exe`, locks it down, and starts it. Needs an administrator. If
/// the service is already there (an update), it is only started again: its settings stay, and
/// could not be changed by an administrator anyway.
pub fn install(exe: &Path) -> Result<(), String> {
    let manager = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: "Site Blocker".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.to_path_buf(),
        launch_arguments: vec!["--service".into()],
        dependencies: vec![],
        // No account: LocalSystem.
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS;
    let (service, created) = match manager.create_service(&info, access) {
        Ok(service) => (service, true),
        Err(e) if is_os_error(&e, ERROR_SERVICE_EXISTS) => {
            let service = manager
                .open_service(NAME, ServiceAccess::START | ServiceAccess::QUERY_STATUS)
                .map_err(|e| format!("the service exists but cannot be opened: {e}"))?;
            (service, false)
        }
        Err(e) => return Err(format!("cannot create the service: {e}")),
    };
    if created {
        let describe = |what: &str, e: windows_service::Error| format!("{what}: {e}");
        service
            .set_description("Blocks distracting sites while a block is running. Started with Windows and restarted if it fails.")
            .map_err(|e| describe("cannot set the description", e))?;
        // Windows starts it again at once, every time it fails, however often.
        service
            .update_failure_actions(ServiceFailureActions {
                reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
                reboot_msg: None,
                command: None,
                actions: Some(
                    (0..3).map(|_| ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::ZERO }).collect(),
                ),
            })
            .map_err(|e| describe("cannot set the failure actions", e))?;
        // Also when it ends with a failure exit code instead of being killed.
        service
            .set_failure_actions_on_non_crash_failures(true)
            .map_err(|e| describe("cannot set the failure flag", e))?;
        // Last: after this, an administrator can no longer change the service. The handle held
        // here keeps the rights it was opened with.
        let output = Command::new("sc.exe")
            .args(["sdset", NAME, SDDL])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("cannot run sc.exe: {e}"))?;
        if !output.status.success() {
            return Err(format!("cannot lock the service down: {}", String::from_utf8_lossy(&output.stdout).trim()));
        }
        for key in SAFE_BOOT {
            let output = Command::new("reg.exe")
                .args(["add", key, "/ve", "/d", "Service", "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("cannot run reg.exe: {e}"))?;
            if !output.status.success() {
                return Err(format!("cannot add {key}: {}", String::from_utf8_lossy(&output.stderr).trim()));
            }
        }
    }
    match service.start::<OsString>(&[]) {
        Err(e) if !is_os_error(&e, ERROR_SERVICE_ALREADY_RUNNING) => Err(format!("cannot start the service: {e}")),
        _ => Ok(()),
    }
}

/// Whether the service is registered.
pub fn exists() -> bool {
    manager(ServiceManagerAccess::CONNECT).is_ok_and(|m| m.open_service(NAME, ServiceAccess::QUERY_STATUS).is_ok())
}

/// Starts the service, if it is registered and not running. Administrators may.
pub fn start_registered() -> Result<(), String> {
    let service = manager(ServiceManagerAccess::CONNECT)?
        .open_service(NAME, ServiceAccess::START)
        .map_err(|e| format!("cannot open the service: {e}"))?;
    match service.start::<OsString>(&[]) {
        Err(e) if !is_os_error(&e, ERROR_SERVICE_ALREADY_RUNNING) => Err(format!("cannot start the service: {e}")),
        _ => Ok(()),
    }
}

/// Waits until the service has ended or is gone, so the installer can replace or delete the exe.
pub fn wait_stopped(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let stopped = manager(ServiceManagerAccess::CONNECT)
            .ok()
            .and_then(|m| m.open_service(NAME, ServiceAccess::QUERY_STATUS).ok())
            .map_or(true, |service| service.query_status().map_or(false, |s| s.current_state == ServiceState::Stopped));
        if stopped {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

/// Run by the service itself, as SYSTEM, on an uninstall: deletes the service, its safe-mode
/// keys and everything it saved.
fn remove_registration() {
    match manager(ServiceManagerAccess::CONNECT).and_then(|m| {
        m.open_service(NAME, ServiceAccess::DELETE).map_err(|e| format!("cannot open the service: {e}"))
    }) {
        Ok(service) => {
            if let Err(e) = service.delete() {
                note("service", &format!("cannot delete the service: {e}"));
            }
        }
        Err(e) => note("service", &e),
    }
    for key in SAFE_BOOT {
        let _ = Command::new("reg.exe").args(["delete", key, "/f"]).creation_flags(CREATE_NO_WINDOW).output();
    }
    if let Some(dir) = state::dir() {
        // The log is open on and off; whatever cannot go now goes with the folder's next removal.
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Store;

    fn context(name: &str) -> Context {
        let dir = std::env::temp_dir().join(format!("service-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let blocklist = Blocklist::with_store(Store::new(dir.join("state.json")));
        Context { blocklist, lockout: Mutex::default() }
    }

    fn start_request(duration: &str) -> Request {
        Request::Start { filters: vec!["a.com".into()], duration: duration.into(), enforcers: Enforcers::default() }
    }

    #[test]
    fn requests_are_checked_again_by_the_service() {
        let context = context("checked");
        // Nothing to block, nothing chosen, a bad duration, and an address that cannot match.
        let bad = [
            Request::Start { filters: vec![], duration: "1h".into(), enforcers: Enforcers::default() },
            Request::Start {
                filters: vec!["a.com".into()],
                duration: "1h".into(),
                enforcers: Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: false },
            },
            start_request("soon"),
            Request::Start { filters: vec!["has space.com".into()], duration: "1h".into(), enforcers: Enforcers::default() },
        ];
        for request in bad {
            assert!(matches!(handle(&context, request).0, Response::Failed(_)));
        }
        assert!(context.blocklist.filters().is_none());
    }

    #[test]
    fn a_timed_block_cannot_be_shortened_stopped_or_uninstalled() {
        let context = context("timed");
        assert_eq!(handle(&context, start_request("1h")).0, Response::Done);
        // Not replaced by a shorter or an untimed one, not stopped.
        assert!(matches!(handle(&context, start_request("1s")).0, Response::Failed(_)));
        assert!(matches!(handle(&context, start_request("")).0, Response::Failed(_)));
        assert!(matches!(handle(&context, Request::Stop).0, Response::Failed(_)));
        // Neither uninstalling nor updating gets through, and the service keeps running.
        let (response, shutdown) = handle(&context, Request::PrepareUninstall);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        let (response, shutdown) = handle(&context, Request::PrepareUpdate);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        // The policy is what the app enforces.
        let Response::Policy(Some(policy)) = handle(&context, Request::Policy).0 else { panic!("no policy") };
        assert_eq!(policy.filters, vec!["a.com"]);
    }

    #[test]
    fn without_a_block_the_installer_gets_through() {
        let context = context("idle");
        let (response, shutdown) = handle(&context, Request::PrepareUninstall);
        assert_eq!(response, Response::Done);
        assert!(matches!(shutdown, Some(Shutdown::Remove)));
        let (response, shutdown) = handle(&context, Request::PrepareUpdate);
        assert_eq!(response, Response::Done);
        assert!(matches!(shutdown, Some(Shutdown::Update)));
        // An untimed block does not stop an uninstall, but does stop an update.
        assert_eq!(handle(&context, start_request("")).0, Response::Done);
        assert!(matches!(handle(&context, Request::PrepareUninstall).1, Some(Shutdown::Remove)));
        assert!(matches!(handle(&context, Request::PrepareUpdate).0, Response::Refused(_)));
    }

    #[test]
    fn the_password_ends_a_timed_block_and_wrong_ones_are_slowed_down() {
        let context = context("unlock");
        assert_eq!(handle(&context, start_request("1h")).0, Response::Done);
        for _ in 0..3 {
            let wrong = Request::Unlock { password: "nope".into() };
            assert_eq!(handle(&context, wrong).0, Response::Failed("Wrong password".into()));
        }
        // After the third, even the right password waits.
        let right = Request::Unlock { password: "test1234".into() };
        assert!(matches!(handle(&context, right).0, Response::Failed(why) if why.starts_with("Too many")));
        assert!(context.blocklist.filters().is_some());
        // Once the wait is over, it works.
        context.lockout.lock().unwrap().until = None;
        let right = Request::Unlock { password: "test1234".into() };
        assert_eq!(handle(&context, right).0, Response::Done);
        assert!(context.blocklist.filters().is_none());
    }

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
        let quit = start_engine(true).unwrap();

        let Ok(Response::Status(status)) = client::call(&Request::Status) else { panic!("no status") };
        assert_eq!(status.remaining, None);
        assert_eq!(client::call(&start_request("1h")), Ok(Response::Done));
        let Ok(Response::Policy(Some(policy))) = client::call(&Request::Policy) else { panic!("no policy") };
        assert_eq!(policy.filters, vec!["a.com"]);
        assert!(matches!(client::call(&Request::Stop), Ok(Response::Failed(_))));
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

    #[test]
    fn the_access_lists_say_what_they_should() {
        // Administrators may start (RP) but not stop (WP), delete (SD), change (DC) or rewrite (WD, WO).
        let administrators = SDDL.split('(').find(|ace| ace.ends_with(";;;BA)")).unwrap();
        assert!(administrators.contains("RP"));
        for right in ["WP", "SD", "DC", "WD", "WO", "DT"] {
            assert!(!administrators.contains(right), "{right}");
        }
        // The pipe's list is separate, in `pipe.rs`.
    }
}
