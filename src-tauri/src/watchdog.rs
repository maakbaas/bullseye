//! The service's watch over the app.
//!
//! The service (`service.rs`) runs as SYSTEM in session 0, which has no desktop, so it cannot
//! close browser tabs itself. The app does that, in the logged-on user's session, and the service
//! keeps it there: it starts the app at every logon and, while a block runs, starts it again
//! within about a second of it ending. With no block the app is left alone, so quitting it with
//! the unlock password sticks.
//!
//! **Which process is "the app" is the service's decision, not a matter of its name.** The service
//! keeps a handle to one process, the app (`AppSlot`):
//! - one it started itself (`launch_app`), or
//! - one that asked to be taken in (`Request::Register`, or its first policy request), and only
//!   if it runs the installed exe, in the console session, with no other app taken in. The
//!   service learns who is asking from the pipe itself, and as SYSTEM it can look at any process.
//!
//! A copy of the exe under another name, or a look-alike, is therefore not the app: it is never
//! taken in, so the service starts the real one anyway. And the app has to keep asking for its
//! policy: that request comes from the same thread that closes the tabs (`tab_closer.rs`), so an
//! app that was suspended, or whose closing thread hangs or is suspended, stops asking, and the
//! service ends it and starts a new one (`AppSlot::supervise`).
//!
//! What this does not stop: the app is still a process of the user it protects against. Someone
//! who can inject code into it can keep it asking while it enforces nothing.
//!
//! Nothing here is hidden or protected beyond that, on purpose: the service is named
//! `bullseye` in the services list, the app runs under its own name in Task Manager, and
//! uninstalling the app (Settings > Apps) removes the service. The mechanism is here to add
//! friction at a weak moment, not to be unknowable.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::RemoteDesktop::{ProcessIdToSessionId, WTSGetActiveConsoleSessionId, WTSQueryUserToken};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, STARTUPINFOW,
};

use crate::block::Blocklist;
use crate::state;
use crate::sync::{spawn_critical, LockExt};

/// Cap, in seconds, on how long the app is left alone after failing to start, so a broken build
/// cannot turn into a process being started over and over.
const MAX_BACKOFF: u32 = 10;

/// An app that has asked for its policy before must ask again within this time, while a block
/// runs, or it is ended and started again.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// A freshly started app has this long to ask for the first time (Tauri and WebView2 start up).
const STARTUP_GRACE: Duration = Duration::from_secs(20);

/// A pause of the service's own loop this long or longer is taken to be the machine having slept,
/// not the app having stalled.
const RESUME_GAP: Duration = Duration::from_secs(3);

/// Set at logon (and when the service starts): the app is wanted even without a block.
/// Cleared once an app is there, so quitting it with the unlock password sticks.
pub static LAUNCH_APP: AtomicBool = AtomicBool::new(true);

/// Whether this process is the service, which only decides where `note` writes.
static IN_SERVICE: AtomicBool = AtomicBool::new(false);

pub fn in_service() {
    IN_SERVICE.store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------------
// The service's hold on the app

/// A handle to a process, held by the service. Holding it means the process id cannot be given to
/// another process while the service still takes it for the app.
pub struct Process {
    handle: usize,
    pub pid: u32,
}

impl Process {
    /// Opens a process by id. The service is SYSTEM and may open any.
    pub fn open(pid: u32) -> Option<Process> {
        // SAFETY: plain call; the handle is closed in `drop`.
        let handle = unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid)
        };
        (!handle.is_null()).then_some(Process { handle: handle as usize, pid })
    }

    pub fn exited(&self) -> bool {
        // SAFETY: the handle is ours and still open.
        unsafe { WaitForSingleObject(self.handle as HANDLE, 0) != WAIT_TIMEOUT }
    }

    pub fn terminate(&self) {
        // SAFETY: the handle is ours and still open.
        unsafe { TerminateProcess(self.handle as HANDLE, 1) };
    }

    /// The path of the exe it runs.
    fn image_path(&self) -> Option<String> {
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        // SAFETY: the buffer is as long as `length` says.
        let ok = unsafe { QueryFullProcessImageNameW(self.handle as HANDLE, 0, buffer.as_mut_ptr(), &mut length) };
        (ok != 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and is not used again.
        unsafe { CloseHandle(self.handle as HANDLE) };
    }
}

/// The one app the service looks after.
pub struct AppSlot {
    /// False where there is nothing to look after: development (the service's logic runs inside the
    /// app, `service::local_handler`) and the pipe test.
    supervised: bool,
    current: Mutex<Option<Adopted>>,
}

struct Adopted {
    process: Process,
    /// When the service took it in.
    since: Instant,
    /// When it last asked for its policy; `None` until it has.
    heard: Option<Instant>,
}

impl AppSlot {
    pub fn new(supervised: bool) -> AppSlot {
        AppSlot { supervised, current: Mutex::new(None) }
    }

    /// The service started this process itself.
    fn launched(&self, process: Process) {
        *self.current.locked() = Some(Adopted { process, since: Instant::now(), heard: None });
    }

    /// The app asks to be taken in when it starts. Refused if another app is running, or if it is
    /// not the installed exe: the app then ends itself.
    pub fn register(&self, pid: u32) -> Result<(), String> {
        if !self.supervised {
            return Ok(());
        }
        adopt(&mut self.current.locked(), pid)
    }

    /// The app asked for its policy, which it does every second from the thread that closes tabs.
    /// This is what shows the closing is going on. From anyone but the app, it counts for nothing.
    pub fn heard(&self, pid: u32) {
        if self.supervised {
            // A refusal here is not an error: it only means this caller is not the app.
            let _ = adopt(&mut self.current.locked(), pid);
        }
    }

    /// Looks after the app once a second: forgets one that has ended and, while a block runs, ends
    /// one that has stopped asking. `resumed` is set when the machine seems to have slept, which
    /// is no fault of the app's. Returns whether there is no app now.
    pub fn supervise(&self, block_active: bool, resumed: bool) -> bool {
        let mut current = self.current.locked();
        if let Some(app) = current.as_mut() {
            let now = Instant::now();
            if app.process.exited() {
                note("service", "the app has ended");
                *current = None;
            } else if resumed {
                app.since = now;
                app.heard = app.heard.map(|_| now);
            } else if block_active && is_stale(app.since, app.heard, now) {
                note("service", "the app stopped asking for its policy; ending it");
                app.process.terminate();
                *current = None;
            }
        }
        current.is_none()
    }

    /// A slot holding `process` as if taken in at `since` and last heard at `heard`.
    #[cfg(test)]
    fn set_for_test(&self, process: Process, since: Instant, heard: Option<Instant>) {
        *self.current.locked() = Some(Adopted { process, since, heard });
    }
}

/// Takes the process `pid` in as the app, or says why not. Already being the app is fine and
/// counts as having been heard.
fn adopt(current: &mut Option<Adopted>, pid: u32) -> Result<(), String> {
    if current.as_ref().is_some_and(|app| app.process.exited()) {
        *current = None;
    }
    match current {
        Some(app) if app.process.pid == pid => {
            app.heard = Some(Instant::now());
            return Ok(());
        }
        Some(_) => return Err("Bullseye is already running".into()),
        None => {}
    }
    let process = Process::open(pid).ok_or("cannot look at the process that asked")?;
    // The name does not matter, the file does: it has to be the installed exe, which a standard
    // user cannot write to or replace.
    let installed = exe().ok_or("the service does not know its own exe")?;
    let same = process.image_path().is_some_and(|theirs| theirs.eq_ignore_ascii_case(&installed.to_string_lossy()));
    if !same {
        return Err("not the installed Bullseye".into());
    }
    // Only the session at the screen is looked after; an app elsewhere just runs.
    if session_of(pid) != console_session() {
        return Ok(());
    }
    let now = Instant::now();
    *current = Some(Adopted { process, since: now, heard: Some(now) });
    Ok(())
}

/// Whether an app taken in at `since` and last heard at `heard` has stayed quiet for too long.
fn is_stale(since: Instant, heard: Option<Instant>, now: Instant) -> bool {
    let (from, limit) = match heard {
        Some(heard) => (heard, HEARTBEAT_TIMEOUT),
        None => (since, STARTUP_GRACE),
    };
    now.saturating_duration_since(from) > limit
}

fn console_session() -> u32 {
    // SAFETY: plain query with no arguments.
    unsafe { WTSGetActiveConsoleSessionId() }
}

fn session_of(pid: u32) -> u32 {
    let mut session = u32::MAX;
    // SAFETY: `session` is a valid place to write to.
    unsafe { ProcessIdToSessionId(pid, &mut session) };
    session
}

/// The service's half: starts the app when it is wanted and not there, and looks after it.
pub fn spawn_supervisor(blocklist: Blocklist, slot: Arc<AppSlot>) {
    spawn_critical("supervisor", move || {
        // How many starts in a row have not led to a running app, which slows the starting down.
        let mut missed = 0u32;
        let mut not_before = Instant::now();
        let mut last_error = String::new();
        let mut last_round = Instant::now();
        loop {
            thread::sleep(Duration::from_secs(1));
            let resumed = last_round.elapsed() >= RESUME_GAP;
            last_round = Instant::now();

            let block_active = blocklist.is_running();
            let no_app = slot.supervise(block_active, resumed);
            if !block_active && !LAUNCH_APP.load(Ordering::SeqCst) {
                missed = 0;
                continue;
            }
            if !no_app {
                LAUNCH_APP.store(false, Ordering::SeqCst);
                missed = 0;
                continue;
            }
            if Instant::now() < not_before {
                continue;
            }
            match launch_app() {
                Ok(process) => {
                    slot.launched(process);
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

/// Starts the app on the desktop of whoever is at the screen, as that user and not as SYSTEM:
/// closing their browser tabs goes through UI Automation, which needs their desktop, and the
/// app has no business being SYSTEM.
fn launch_app() -> Result<Process, String> {
    let exe = exe().ok_or("no executable to start")?;
    let mut command = wide(&format!("\"{}\"", exe.display()));
    let mut desktop = wide(r"winsta0\default");
    // SAFETY: every pointer passed is valid for the call; the token, the environment block and
    // the thread handle are released again whatever happens, and the process handle is handed on.
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
        Ok(Process { handle: process.hProcess as usize, pid: process.dwProcessId })
    }
}

/// Ends the app, when the service goes away for an uninstall. Every other process
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

/// This executable, for starting and recognising the app.
fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Writes a line to a log file as well as to stderr: `service.log` next to the saved block for
/// the service, `app.log` in `%LOCALAPPDATA%\bullseye` for the app.
///
/// Neither runs with a window, so stderr goes nowhere in a release build; the file is how the
/// user can see what they actually did.
pub fn note(who: &str, message: &str) {
    eprintln!("{who}: {message}");
    // The tests must not write into the log of a real install on the machine they run on.
    if cfg!(test) {
        return;
    }
    let (dir, file) = if IN_SERVICE.load(Ordering::Relaxed) { (state::dir(), "service.log") } else { (state::user_dir(), "app.log") };
    let Some(dir) = dir else { return };
    // The folder only exists once something has been written to it.
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(file);
    start_over_if_big(&path, LOG_LIMIT);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {who}: {message}", timestamp());
    }
}

/// A log that grows past this is moved to `<name>.old`, replacing the one before, and started again:
/// the last half megabyte or so of history is kept, and the files never take over the disk.
const LOG_LIMIT: u64 = 512 * 1024;

fn start_over_if_big(path: &std::path::Path, limit: u64) {
    if fs::metadata(path).is_ok_and(|meta| meta.len() > limit) {
        let _ = fs::rename(path, path.with_extension("log.old"));
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
    use std::process::{Child, Command, Stdio};

    #[test]
    fn a_log_that_got_big_is_moved_aside_and_started_again() {
        let dir = std::env::temp_dir().join(format!("watchdog-{}-log", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("service.log");

        fs::write(&log, "x".repeat(100)).unwrap();
        start_over_if_big(&log, 1000);
        assert_eq!(fs::metadata(&log).unwrap().len(), 100, "a small log stays");

        fs::write(&log, "x".repeat(2000)).unwrap();
        start_over_if_big(&log, 1000);
        assert!(!log.exists(), "the big one is moved aside");
        assert_eq!(fs::metadata(dir.join("service.log.old")).unwrap().len(), 2000);

        // The next big one replaces the old one rather than piling up.
        fs::write(&log, "y".repeat(3000)).unwrap();
        start_over_if_big(&log, 1000);
        assert_eq!(fs::metadata(dir.join("service.log.old")).unwrap().len(), 3000);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_are_null_terminated() {
        assert_eq!(wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
    }

    #[test]
    fn file_names_match_regardless_of_path_and_case() {
        assert_eq!(file_name(std::path::Path::new(r"C:\a\B\bullseye.EXE")), "bullseye.exe");
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

    /// A process that lives for a while and is not this exe.
    fn sleeper() -> Child {
        Command::new("ping.exe").args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap()
    }

    fn wait_until_exited(process: &Process) -> bool {
        (0..50).any(|_| {
            thread::sleep(Duration::from_millis(100));
            process.exited()
        })
    }

    #[test]
    fn a_held_process_shows_when_it_ends() {
        let mut child = sleeper();
        let process = Process::open(child.id()).unwrap();
        assert!(!process.exited());
        process.terminate();
        assert!(wait_until_exited(&process));
        let _ = child.wait();
    }

    #[test]
    fn staying_quiet_is_judged_by_what_was_heard_last() {
        let now = Instant::now();
        let ago = |seconds| now - Duration::from_secs(seconds);
        // Never heard: the start-up allowance applies. Heard: the short one.
        assert!(!is_stale(ago(10), None, now));
        assert!(is_stale(ago(30), None, now));
        assert!(!is_stale(ago(60), Some(ago(2)), now));
        assert!(is_stale(ago(60), Some(ago(8)), now));
    }

    #[test]
    fn a_slot_that_is_not_supervised_takes_everyone() {
        let slot = AppSlot::new(false);
        assert_eq!(slot.register(0), Ok(()));
        slot.heard(0);
    }

    #[test]
    fn only_the_installed_exe_is_the_app_and_only_one_at_a_time() {
        let me = std::process::id();
        if session_of(me) != console_session() {
            return eprintln!("skipping: this test does not run in the console session");
        }
        let mut child = sleeper();

        // Not the installed exe, whatever it is called: refused, and nothing is taken in.
        let slot = AppSlot::new(true);
        assert_eq!(slot.register(child.id()), Err("not the installed Bullseye".into()));
        slot.heard(child.id());
        assert!(slot.supervise(true, false), "nothing should have been taken in");

        // The installed exe (this test is it) is taken in, and then no other one is.
        assert_eq!(slot.register(me), Ok(()));
        assert!(!slot.supervise(true, false));
        assert_eq!(slot.register(me), Ok(()));
        assert_eq!(slot.register(child.id()), Err("Bullseye is already running".into()));

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn an_app_that_stops_asking_is_ended_but_not_by_a_sleep_or_without_a_block() {
        let now = Instant::now();
        let ago = |seconds| now - Duration::from_secs(seconds);
        let quiet = |slot: &AppSlot, child: &Child| {
            slot.set_for_test(Process::open(child.id()).unwrap(), ago(60), Some(ago(30)));
        };
        let (mut stale, mut slept, mut idle) = (sleeper(), sleeper(), sleeper());
        let slot = AppSlot::new(true);

        // Quiet for 30 seconds while a block runs: ended, and the slot is empty for a new one.
        quiet(&slot, &stale);
        let held = Process::open(stale.id()).unwrap();
        assert!(slot.supervise(true, false));
        assert!(wait_until_exited(&held));

        // The same after the machine slept: the app is given a fresh start instead.
        quiet(&slot, &slept);
        assert!(!slot.supervise(true, true));
        assert!(!Process::open(slept.id()).unwrap().exited());
        assert!(!slot.supervise(true, false), "the grace after a sleep should still be running");

        // And with no block there is nothing to enforce, so nothing to end it for.
        quiet(&slot, &idle);
        assert!(!slot.supervise(false, false));
        assert!(!Process::open(idle.id()).unwrap().exited());

        for child in [&mut stale, &mut slept, &mut idle] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
