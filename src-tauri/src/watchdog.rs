//! The service's watch over the app, and the little the app itself needs from this module.
//!
//! The service (`service.rs`) runs as SYSTEM in session 0, which has no desktop, so it cannot
//! close browser tabs itself. The app does that, in the logged-on user's session, and the service
//! keeps it there: it starts the app at every logon, and while a timed block runs it starts the
//! app again within about a second of it ending. Ending the app does not end the block, and
//! ending the service is up to Windows (`service::install` sets it to restart on failure and
//! takes the right to stop it away from everyone but SYSTEM).
//!
//! Nothing here is hidden or protected beyond that, on purpose: the service is named
//! `SiteBlocker` in the services list, the app runs under its own name in Task Manager, and
//! uninstalling the app (Settings > Apps) removes the service. The mechanism is here to add
//! friction at a weak moment, not to be unknowable.
//!
//! The app claims `Local\SiteBlocker-app`, so a second copy of it exits instead of doubling up.
//! That mutex is per session and the service is in another one, which is why the service finds
//! the app by looking at processes instead.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::RemoteDesktop::{ProcessIdToSessionId, WTSGetActiveConsoleSessionId, WTSQueryUserToken};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, CreateProcessAsUserW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

use crate::block::Blocklist;
use crate::state;

/// Cap, in seconds, on how long the app is left alone after failing to start, so a broken build
/// cannot turn into a process being started over and over.
const MAX_BACKOFF: u32 = 10;

/// Session-scoped mutex name. `Local\` keeps it out of other sessions and needs no privilege.
const APP_MUTEX: &str = r"Local\SiteBlocker-app";

/// Set at logon (and when the service starts): the app is wanted even without a timed block.
/// Cleared once the app is seen running, so quitting it with the unlock password sticks.
pub static LAUNCH_APP: AtomicBool = AtomicBool::new(true);

/// Whether this process is the service, which only decides where `note` writes.
static IN_SERVICE: AtomicBool = AtomicBool::new(false);

pub fn in_service() {
    IN_SERVICE.store(true, Ordering::Relaxed);
}

/// Whether this process is the one app instance, claiming the app's mutex if it is free.
///
/// A second copy (started by the service, or by launching the executable again) leaves the
/// running one to do the work rather than closing tabs twice over.
pub fn claim_app() -> bool {
    claim(APP_MUTEX)
}

/// The service's half: keeps the app running in the logged-on user's session while a timed block
/// is running, and starts it when `LAUNCH_APP` is set.
pub fn spawn_supervisor(blocklist: Blocklist) {
    thread::spawn(move || {
        let mut system = System::new();
        // How many starts in a row have not led to a running app, which slows the starting down.
        let mut missed = 0u32;
        let mut not_before = Instant::now();
        let mut last_error = String::new();
        loop {
            thread::sleep(Duration::from_secs(1));
            if blocklist.status().remaining.is_none() && !LAUNCH_APP.load(Ordering::SeqCst) {
                missed = 0;
                continue;
            }
            if app_is_running(&mut system) {
                LAUNCH_APP.store(false, Ordering::SeqCst);
                missed = 0;
                continue;
            }
            if Instant::now() < not_before {
                continue;
            }
            match launch_app() {
                Ok(()) => {
                    LAUNCH_APP.store(false, Ordering::SeqCst);
                    last_error.clear();
                    note("service", "started the app");
                }
                // Nobody is logged on yet, say: expected, and not worth a line every second.
                Err(e) if e != last_error => {
                    note("service", &format!("cannot start the app: {e}"));
                    last_error = e;
                }
                Err(_) => {}
            }
            missed += 1;
            not_before = Instant::now() + Duration::from_secs(u64::from(missed.min(MAX_BACKOFF)));
        }
    });
}

/// Whether the app is running in the session of whoever is at the screen. It is the same
/// executable without arguments: the service, the installer's calls and the like all have some.
fn app_is_running(system: &mut System) -> bool {
    let Some(exe) = exe() else { return false };
    let name = file_name(&exe);
    // SAFETY: plain query with no arguments.
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet).with_exe(UpdateKind::OnlyIfNotSet),
    );
    system.processes().iter().any(|(pid, process)| {
        let same = process.exe().is_some_and(|other| file_name(other) == name) && process.cmd().len() <= 1;
        if !same || pid.as_u32() == std::process::id() {
            return false;
        }
        let mut session = 0u32;
        // SAFETY: `session` is a valid place to write to.
        let ok = unsafe { ProcessIdToSessionId(pid.as_u32(), &mut session) };
        ok != 0 && session == console
    })
}

/// Starts the app on the desktop of whoever is at the screen, as that user and not as SYSTEM:
/// closing their browser tabs goes through UI Automation, which needs their desktop, and the
/// app has no business being SYSTEM.
fn launch_app() -> Result<(), String> {
    let exe = exe().ok_or("no executable to start")?;
    let mut command = wide(&format!("\"{}\"", exe.display()));
    let mut desktop = wide(r"winsta0\default");
    // SAFETY: every pointer passed is valid for the call; the token, the environment block and
    // the two process handles are released again whatever happens.
    unsafe {
        let session = WTSGetActiveConsoleSessionId();
        if session == u32::MAX {
            return Err("nobody is at the screen".into());
        }
        let mut token: HANDLE = null_mut();
        if WTSQueryUserToken(session, &mut token) == 0 {
            return Err(format!("no user is logged on yet (error {})", GetLastError()));
        }
        let mut environment = null_mut();
        let have_environment = CreateEnvironmentBlock(&mut environment, token, 0) != 0;

        let mut startup: STARTUPINFOW = std::mem::zeroed();
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        startup.lpDesktop = desktop.as_mut_ptr();
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        let none: *const SECURITY_ATTRIBUTES = null();
        let started = CreateProcessAsUserW(
            token,
            null(),
            command.as_mut_ptr(),
            none,
            none,
            0,
            CREATE_UNICODE_ENVIRONMENT,
            if have_environment { environment } else { null() },
            null(),
            &startup,
            &mut process,
        );
        let error = GetLastError();
        if have_environment {
            DestroyEnvironmentBlock(environment);
        }
        CloseHandle(token);
        if started == 0 {
            return Err(format!("CreateProcessAsUserW failed (error {error})"));
        }
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

/// Ends the app, when the service goes away for an update or an uninstall. Every other process
/// of this executable that was started without arguments; the service itself and the installer's
/// calls are left alone.
pub fn kill_apps() {
    let Some(exe) = exe() else { return };
    let name = file_name(&exe);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet).with_exe(UpdateKind::OnlyIfNotSet),
    );
    for (pid, process) in system.processes() {
        let same = process.exe().is_some_and(|other| file_name(other) == name) && process.cmd().len() <= 1;
        if same && pid.as_u32() != std::process::id() {
            let ended = process.kill();
            note("service", &format!("{} the app ({pid})", if ended { "ended" } else { "could not end" }));
        }
    }
}

/// Claims a mutex for this process, returning false when another process already holds it.
fn claim(role: &str) -> bool {
    let name = wide(role);
    // SAFETY: a null security descriptor and a null-terminated name are both valid here. The
    // handle is deliberately never closed: it must stay open for as long as this process runs,
    // which is what keeps the role claimed, so not closing it is the point.
    unsafe {
        let handle = CreateMutexW(null(), 1, name.as_ptr());
        // Read before anything else can overwrite the thread's last error.
        let taken = GetLastError() == ERROR_ALREADY_EXISTS;
        if handle.is_null() {
            // Nothing to hold the mutex with, so do not risk running a second copy.
            note("app", &format!("cannot claim {role}"));
            return false;
        }
        !taken
    }
}

/// This executable, for starting and recognising the app.
fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Writes a line to a log file as well as to stderr: `service.log` next to the saved block for
/// the service, `app.log` in `%LOCALAPPDATA%\SiteBlocker` for the app.
///
/// Neither runs with a window, so stderr goes nowhere in a release build; the file is how the
/// user can see what they actually did.
pub fn note(who: &str, message: &str) {
    eprintln!("{who}: {message}");
    let (dir, file) = if IN_SERVICE.load(Ordering::Relaxed) { (state::dir(), "service.log") } else { (state::user_dir(), "app.log") };
    let Some(dir) = dir else { return };
    // The folder only exists once something has been written to it.
    let _ = fs::create_dir_all(&dir);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(dir.join(file)) {
        let _ = writeln!(file, "{} {who}: {message}", timestamp());
    }
}

/// `2026-10-02 13:45:06`, without pulling in a date/time crate.
fn timestamp() -> String {
    let secs = state::now().unwrap_or_default().as_secs();
    let days = secs / 86400;
    let (hour, minute, second) = (secs % 86400 / 3600, secs % 3600 / 60, secs % 60);
    // Days since 1970-01-01, turned into a date by the usual civil-from-days count.
    let (year, month, day) = civil(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// A day count since 1970-01-01 as a year, month and day (Howard Hinnant's algorithm): the
/// day is shifted so that the leap day lands at the end of the era, then counted back.
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    // March-based month number, so the leap day is the last day of the year.
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = month_from_march + 3 - 12 * (month_from_march / 10);
    // Counting from March, January and February are the first months of the *next* year.
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year as u64, month as u64, day as u64)
}

/// Lowercased file name, so processes are matched on this one build whatever its path.
fn file_name(path: &std::path::Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().to_lowercase()
}

/// A null-terminated UTF-16 name, as the Win32 calls want it.
pub fn wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_null_terminated() {
        assert_eq!(wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(*wide(APP_MUTEX).last().unwrap(), 0);
    }

    #[test]
    fn file_names_match_regardless_of_path_and_case() {
        assert_eq!(file_name(std::path::Path::new(r"C:\a\B\tray-app.EXE")), "tray-app.exe");
    }

    #[test]
    fn log_timestamps_are_real_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        // A leap day, the day before it, and a date after it.
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(civil(19_781), (2024, 2, 28));
        assert_eq!(civil(19_841), (2024, 4, 28));
        // Every month boundary of a leap year, which is where a month count goes wrong.
        for (day, date) in [(19_753, (2024, 1, 31)), (19_813, (2024, 3, 31)), (19_843, (2024, 4, 30))] {
            assert_eq!(civil(day), date);
        }
        assert_eq!(civil(1_700_000_000 / 86400), (2023, 11, 14));
        assert_eq!(timestamp().len(), 19);
    }

    #[test]
    fn claiming_a_role_twice_fails() {
        assert!(claim("Local\\SiteBlocker-test-role"));
        assert!(!claim("Local\\SiteBlocker-test-role"));
    }
}
