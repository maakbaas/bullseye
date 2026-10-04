//! The entry point: one command that starts the app and keeps it alive.
//!
//! Running the executable with `--launch` starts the app and then watches over it. While a
//! timed block is saved on disk, the app is started again within about half a second of it
//! ending, so ending the app does not end the block. The app in turn watches the launcher and
//! starts a new one within about a second of it ending (`guard_launcher`), so ending either half
//! of the pair is undone by the other. The login task (`task.rs`, registered by the installer) runs this same
//! `--launch` command at every logon, which brings the pair back after both were ended.
//!
//! Nothing here is hidden or protected, on purpose: every process runs under the app's own
//! name and shows up in Task Manager, the login task is named and can be disabled or deleted
//! in Task Scheduler, and uninstalling the app removes it. The mechanism is here to add
//! friction at a weak moment, not to be unknowable.
//!
//! Each role is claimed through a named mutex, so a second app or launcher finds its role taken
//! and exits instead of doubling up. Windows releases a mutex when the process holding it ends,
//! however it ends, which is how the launcher notices the app is gone.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenMutexW, ReleaseMutex, WaitForSingleObject, MUTEX_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE,
};

use crate::{state, task};

/// How often the launcher looks at the saved block and at the app.
const POLL: Duration = Duration::from_millis(500);

/// Cap on how fast the app is started again when it never gets as far as running at all, so a
/// broken build cannot turn into a process being started over and over.
const MAX_BACKOFF: u32 = 10;

/// The argument that makes the executable the launcher instead of the app.
const LAUNCH: &str = "--launch";

/// What the launcher and the app write to, next to the saved block, so the user can see what
/// they did.
const LOG: &str = "launcher.log";

/// Session-scoped mutex names. `Local\` keeps them out of other sessions and needs no privilege
/// to create.
const APP_MUTEX: &str = r"Local\SiteBlocker-app";
const LAUNCHER_MUTEX: &str = r"Local\SiteBlocker-launcher";

/// How often the app looks for its launcher.
const LAUNCHER_POLL: Duration = Duration::from_secs(1);

/// `--launch`: the one entry point. Starts the app, then keeps it alive while a timed block is
/// saved on disk. Returns when a launcher is already running (or there is no executable to
/// start); otherwise it ends the process once there is no timed block left to keep alive.
pub fn launch_main() {
    // A launcher is already doing this.
    if !claim(LAUNCHER_MUTEX) {
        return;
    }
    let Some(exe) = exe() else { return };
    note("launch", "started");

    // The app is what closes tabs and shows the tray; the launcher only watches over it, so
    // the app starts even when no block is running. First, because schtasks below can take
    // seconds.
    spawn(&exe, &[]);

    // The app turns the login task back on as it starts (`restore_task`).
    supervise(&exe);
}

/// Keeps the app alive for as long as a timed block is saved on disk.
fn supervise(exe: &PathBuf) {
    note("launch", "watching for the app to end while a timed block runs");
    // How many polls the app has been missing without ever being seen running, which slows the
    // restarting down if the app is not surviving its own startup.
    let mut missed = 0;
    loop {
        if !timed_block_saved() {
            break;
        }
        let wait = if app_is_running() {
            missed = 0;
            POLL
        } else {
            spawn(exe, &[]);
            missed += 1;
            POLL * missed.min(MAX_BACKOFF)
        };
        thread::sleep(wait);
    }
    note("launch", "no timed block left to watch; standing down");
    std::process::exit(0);
}

/// Whether this process is the one app instance, claiming the app's mutex if it is free.
///
/// A second copy (started by the launcher, or by launching the executable again) leaves the
/// running one to do the work rather than closing tabs twice over.
pub fn claim_app() -> bool {
    claim(APP_MUTEX)
}

/// Starts a launcher, which starts the app. A second one finds the mutex taken and exits.
pub fn spawn_launcher() {
    let Some(exe) = exe() else { return };
    spawn(&exe, &[LAUNCH]);
}

/// The app's half of the pair: while a timed block is saved, starts a launcher whenever none is
/// running. The launcher brings the app back and this brings the launcher back, so ending one
/// of them is undone by the other; only ending both at once stops the pair until the next logon.
pub fn guard_launcher() {
    thread::spawn(|| loop {
        if timed_block_saved() && !is_held(LAUNCHER_MUTEX) {
            note("app", "no launcher is watching the timed block; starting one");
            spawn_launcher();
        }
        // Long enough for a new launcher to have claimed its mutex by the next look; should
        // two be started anyway, the second finds the mutex taken and exits.
        thread::sleep(LAUNCHER_POLL);
    });
}

/// Undoes the forced stop's turning off of the login task, so starting the app, however it is
/// started, puts everything back as it was. In the background, since schtasks can take seconds.
///
/// Best effort: a task installed from an administrator shell can only be turned on from an
/// elevated process. Installed without (the intended setup), this always works.
pub fn restore_task() {
    thread::spawn(|| {
        if let Err(e) = task::set_enabled(true) {
            note("app", &e);
        }
    });
}

/// Ends every other process of this app: the launcher first, then the app instances, so that
/// none is left to start the others again. Returns the process ids that were ended.
pub fn kill_all() -> Vec<u32> {
    let Some(exe) = exe() else { return Vec::new() };
    let name = file_name(&exe);
    let me = std::process::id();
    let mut system = System::new();
    let mut ended = Vec::new();
    for launchers_first in [true, false] {
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            // The executable path is how this app's processes are recognised; sysinfo leaves it
            // empty unless asked for.
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet).with_exe(UpdateKind::OnlyIfNotSet),
        );
        for (pid, process) in system.processes() {
            let same = process.exe().is_some_and(|other| file_name(other) == name);
            if pid.as_u32() == me || !same {
                continue;
            }
            let is_launcher = process.cmd().iter().any(|arg| arg.to_string_lossy() == LAUNCH);
            if is_launcher != launchers_first {
                continue;
            }
            let label = format!("{} ({})", process.name().to_string_lossy(), pid);
            if process.kill() {
                ended.push(pid.as_u32());
                note("stop", &format!("ended {label}"));
            } else {
                note("stop", &format!("could not end {label}"));
            }
        }
    }
    ended
}

/// Whether a timed block is saved on disk: the thing to keep the app alive for.
///
/// Whether its time is up is left to the app, which goes by internet time (`clock.rs`) and
/// deletes the file when the block ends. Judging it here by the Windows clock would let moving
/// that clock forward stand the launcher down.
fn timed_block_saved() -> bool {
    state::store().and_then(|store| store.load()).is_some_and(|saved| saved.until.is_some())
}

/// Whether some app instance is running, i.e. the app's mutex is held.
fn app_is_running() -> bool {
    is_held(APP_MUTEX)
}

/// Whether another thread, normally in another process, holds the mutex for `role`.
fn is_held(role: &str) -> bool {
    let name = wide(role);
    // SAFETY: `name` is null-terminated, a mutex taken here is released again, and the handle
    // is closed before returning.
    unsafe {
        let handle = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE, 0, name.as_ptr());
        if handle.is_null() {
            return false;
        }
        // While its holder runs, the wait times out. A mutex that is free, or abandoned by a
        // process that was ended, is taken by the wait instead, so give it straight back.
        let result = WaitForSingleObject(handle, 0);
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
            ReleaseMutex(handle);
        }
        CloseHandle(handle);
        result == WAIT_TIMEOUT
    }
}

/// Claims a mutex for this process, returning false when another process already holds it.
fn claim(role: &str) -> bool {
    let name = wide(role);
    // SAFETY: a null security descriptor and a null-terminated name are both valid here. The
    // handle is deliberately never closed: it must stay open for as long as this process runs,
    // which is what keeps the role claimed, so not closing it is the point.
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        // Read before anything else can overwrite the thread's last error.
        let taken = GetLastError() == windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS;
        if handle.is_null() {
            // Nothing to hold the mutex with, so do not risk running a second copy.
            note("launch", &format!("cannot claim {role}"));
            return false;
        }
        !taken
    }
}

/// This executable, for starting the other half of the pair.
fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Writes a line to `%LOCALAPPDATA%\SiteBlocker\launcher.log` as well as to stderr.
///
/// The launcher runs without a window, so stderr goes nowhere in a release build; the file is
/// how the user can see what the launcher actually did.
pub fn note(who: &str, message: &str) {
    eprintln!("{who}: {message}");
    let Some(dir) = state::dir() else { return };
    // The folder only exists once a block has been saved, and this runs before that.
    let _ = fs::create_dir_all(&dir);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(dir.join(LOG)) {
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

/// Starts `exe` detached, so it outlives this process.
fn spawn(exe: &PathBuf, args: &[&str]) {
    let started = Command::new(exe).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    if let Err(e) = started {
        note("launch", &format!("cannot start {}: {e}", exe.display()));
    }
}

/// Lowercased file name, so processes are matched on this one build whatever its path.
fn file_name(path: &std::path::Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().to_lowercase()
}

/// A null-terminated UTF-16 name, as the Win32 calls want it.
fn wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_null_terminated() {
        assert_eq!(wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(*wide(APP_MUTEX).last().unwrap(), 0);
        assert_eq!(*wide(LAUNCHER_MUTEX).last().unwrap(), 0);
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

    /// One test, because the app mutex is per-session and the tests run in parallel.
    #[test]
    fn the_app_mutex_reports_whether_the_app_runs() {
        // A real app running while the tests run holds this same mutex, so the test only means
        // anything when nothing is up.
        if app_is_running() {
            eprintln!("skipping: the app mutex is already held, so an app is running");
            return;
        }
        // The app holds the mutex on a thread of its own and the launcher asks from another
        // process. A thread may take a mutex it already owns, so asking from the claiming
        // thread would always say "free"; another thread is the closest a test gets.
        let (claimed_tx, claimed) = std::sync::mpsc::channel();
        let (end, ended) = std::sync::mpsc::channel::<()>();
        let app = thread::spawn(move || {
            claimed_tx.send(claim_app()).unwrap();
            let _ = ended.recv();
        });
        assert!(claimed.recv().unwrap());
        assert!(app_is_running());
        // Asking does not take the mutex away from the app.
        assert!(app_is_running());
        assert!(!claim_app());

        // The holder ending abandons the mutex, as ending the app in Task Manager does.
        drop(end);
        app.join().unwrap();
        assert!(!app_is_running());
    }
}