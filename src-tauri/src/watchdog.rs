//! The service's watch over the app and the worker.
//!
//! The service (`service.rs`) runs as SYSTEM in session 0, which has no desktop, so it cannot
//! close browser tabs itself. Two processes in the logged-on user's session do the rest:
//! - **the app** (`bullseye-app.exe`): the tray and the window, as that user;
//! - **the worker** (`bullseye-app.exe --worker`): the closing (`tab_closer/`), as SYSTEM, on that
//!   user's desktop (`launch_worker`). Windows lets no process close, minimize or end what runs
//!   with more rights than its own: a worker with the user's rights could not touch a program run
//!   as administrator, whether with their own administrator rights or another account's (a
//!   standard user's UAC asks for another account's password), nor one started as SYSTEM. As
//!   SYSTEM it reaches them all. It has no window, so there is nothing on the desktop to send
//!   it messages through. The window stays at the user's plain rights: it has no need of more.
//!
//! The service keeps both there: it starts them at every logon and, while a session runs, starts
//! one again within about a second of it ending. With no session they are left alone.
//!
//! **Which process is the app, and which the worker, is the service's decision, not a matter of
//! their names.** The service keeps a handle to each (`AppSlot`):
//! - the app: one it started itself (`launch_app`), or one that asked to be taken in
//!   (`Request::Register`), and only if it runs the installed exe, in the console session, with no
//!   other app taken in. The service learns who is asking from the pipe itself, and as SYSTEM it
//!   can look at any process.
//! - the worker: only one it started itself. Any other `--worker` is refused
//!   (`Request::RegisterWorker`) and ends itself, so a worker started by hand with fewer rights
//!   cannot stand in for the real one.
//!
//! A copy of the exe under another name, or a look-alike, is therefore neither: it is never taken
//! in, so the service starts the real ones anyway. And the worker has to keep asking for its
//! policy: that request comes from the same thread that closes the tabs (`tab_closer/`), so a
//! worker that was suspended, or whose closing thread hangs or is suspended, stops asking, and the
//! service ends it and starts a new one (`AppSlot::supervise`). An administrator could end it from an
//! elevated Task Manager: it is started again at once, as the app is.
//!
//! Nothing here is hidden or protected beyond that, on purpose: the service is named
//! `bullseye` in the services list, the app and the worker run under their own name in Task
//! Manager (the worker with `--worker` on its command line), and
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
use windows_sys::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, SetTokenInformation, TokenPrimary, TokenSessionId, SECURITY_ATTRIBUTES, TOKEN_ALL_ACCESS,
    TOKEN_DUPLICATE,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::RemoteDesktop::{ProcessIdToSessionId, WTSGetActiveConsoleSessionId, WTSQueryUserToken};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, ResumeThread, TerminateProcess,
    WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, STARTUPINFOW,
};

use crate::session::Sessions;
use crate::state;
use crate::sync::{spawn_critical, LockExt};

/// What the worker is started with, and how it knows to be the worker (`lib.rs`).
pub const WORKER_ARG: &str = "--worker";

/// Cap, in seconds, on how long the app is left alone after failing to start, so a broken build
/// cannot turn into a process being started over and over.
const MAX_BACKOFF: u32 = 10;

/// A worker that has asked for its policy before must ask again within this time, while a session
/// runs, or it is ended and started again.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// A freshly started worker has this long to ask for the first time.
const STARTUP_GRACE: Duration = Duration::from_secs(20);

/// A pause of the service's own loop this long or longer is taken to be the machine having slept,
/// not the app having stalled.
const RESUME_GAP: Duration = Duration::from_secs(3);

/// Set at logon (and when the service starts): the app is wanted even without a session.
/// Cleared once an app is there: without a session, one that has ended is not started again.
static LAUNCH_APP: AtomicBool = AtomicBool::new(true);

/// The same for the worker.
static LAUNCH_WORKER: AtomicBool = AtomicBool::new(true);

/// Someone logged on, or came back to their session: the app and the worker are wanted there.
pub fn logged_on() {
    LAUNCH_APP.store(true, Ordering::SeqCst);
    LAUNCH_WORKER.store(true, Ordering::SeqCst);
}

/// Whether this process is the service, which only decides where `note` writes.
static IN_SERVICE: AtomicBool = AtomicBool::new(false);

/// Whether this process is the worker, which only decides what `note` calls it.
static IN_WORKER: AtomicBool = AtomicBool::new(false);

pub fn in_service() {
    IN_SERVICE.store(true, Ordering::Relaxed);
}

pub fn in_worker() {
    IN_WORKER.store(true, Ordering::Relaxed);
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
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
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

/// The app and the worker the service looks after.
pub struct AppSlot {
    /// False where there is nothing to look after: development (the service's logic runs inside the
    /// app, `service::local_handler`) and the pipe test.
    supervised: bool,
    current: Mutex<Option<Adopted>>,
    worker: Mutex<Option<Adopted>>,
}

struct Adopted {
    process: Process,
    /// When the service took it in.
    since: Instant,
    /// When it last asked for its policy; `None` until it has.
    heard: Option<Instant>,
}

/// Which of the two is not there now (`AppSlot::supervise`), so the supervisor starts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub app: bool,
    pub worker: bool,
}

impl AppSlot {
    pub fn new(supervised: bool) -> AppSlot {
        AppSlot { supervised, current: Mutex::new(None), worker: Mutex::new(None) }
    }

    /// The service started this process itself, as the app.
    fn launched(&self, process: Process) {
        *self.current.locked() = Some(Adopted { process, since: Instant::now(), heard: None });
    }

    /// The service started this process itself, as the worker.
    fn launched_worker(&self, process: Process) {
        *self.worker.locked() = Some(Adopted { process, since: Instant::now(), heard: None });
    }

    /// The app asks to be taken in when it starts. Refused if another app is running, or if it is
    /// not the installed exe: the app then ends itself.
    pub fn register(&self, pid: u32) -> Result<(), String> {
        if !self.supervised {
            return Ok(());
        }
        adopt(&mut self.current.locked(), pid)
    }

    /// The worker asks whether it is the one the service started, when it starts. Any other one is
    /// refused and ends itself: only the service starts the worker, with the rights it chose.
    pub fn register_worker(&self, pid: u32) -> Result<(), String> {
        if !self.supervised {
            return Ok(());
        }
        match self.worker.locked().as_ref() {
            Some(worker) if worker.process.pid == pid => Ok(()),
            _ => Err("not the worker the service started".into()),
        }
    }

    /// The worker asked for its policy, which it does every second from the thread that closes tabs.
    /// This is what shows the closing is going on. Refused to anyone but the worker, which then ends
    /// itself: a worker from before the service restarted, say, would otherwise close beside the new one.
    pub fn heard(&self, pid: u32) -> Result<(), String> {
        if !self.supervised {
            return Ok(());
        }
        match self.worker.locked().as_mut() {
            Some(worker) if worker.process.pid == pid => {
                worker.heard = Some(Instant::now());
                Ok(())
            }
            _ => Err("not the worker the service started".into()),
        }
    }

    /// Looks after the app and the worker once a second: forgets one that has ended and, while a
    /// session runs, ends a worker that has stopped asking. `resumed` is set when the machine seems
    /// to have slept, which is no fault of the worker's. Returns which of them is not there now.
    pub fn supervise(&self, session_active: bool, resumed: bool) -> Missing {
        let app = look_after(&mut self.current.locked(), "the app", None);
        let heartbeat = Heartbeat { session_active, resumed };
        let worker = look_after(&mut self.worker.locked(), "the worker", Some(heartbeat));
        Missing { app, worker }
    }

    /// A slot holding `process` as its worker as if started at `since` and last heard at `heard`.
    #[cfg(test)]
    fn set_for_test(&self, process: Process, since: Instant, heard: Option<Instant>) {
        *self.worker.locked() = Some(Adopted { process, since, heard });
    }
}

/// What judges whether the worker has stayed quiet for too long.
#[derive(Clone, Copy)]
struct Heartbeat {
    session_active: bool,
    resumed: bool,
}

/// Forgets the process held if it has ended, and ends it if it was to keep asking (`heartbeat`)
/// and stopped. Returns whether nothing is held now.
fn look_after(current: &mut Option<Adopted>, name: &str, heartbeat: Option<Heartbeat>) -> bool {
    if let Some(held) = current.as_mut() {
        let now = Instant::now();
        if held.process.exited() {
            note("service", &format!("{name} has ended"));
            *current = None;
        } else if let Some(Heartbeat { session_active, resumed }) = heartbeat {
            if resumed {
                held.since = now;
                held.heard = held.heard.map(|_| now);
            } else if session_active && is_stale(held.since, held.heard, now) {
                note("service", &format!("{name} stopped asking for its policy; ending it"));
                held.process.terminate();
                *current = None;
            }
        }
    }
    current.is_none()
}

/// Takes the process `pid` in as the app, or says why not. Already being the app is fine.
fn adopt(current: &mut Option<Adopted>, pid: u32) -> Result<(), String> {
    if current.as_ref().is_some_and(|app| app.process.exited()) {
        *current = None;
    }
    match current {
        Some(app) if app.process.pid == pid => return Ok(()),
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

/// Whether a worker started at `since` and last heard at `heard` has stayed quiet for too long.
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

/// One of the two the supervisor keeps there: how it is started, and how its starting went.
struct Kept {
    name: &'static str,
    /// Whether it is wanted even without a session (`LAUNCH_APP`, `LAUNCH_WORKER`).
    wanted: &'static AtomicBool,
    /// How many starts in a row have not led to a running process, which slows the starting down.
    missed: u32,
    not_before: Instant,
    last_error: String,
}

impl Kept {
    fn new(name: &'static str, wanted: &'static AtomicBool) -> Kept {
        Kept { name, wanted, missed: 0, not_before: Instant::now(), last_error: String::new() }
    }

    /// One round: starts it with `launch` if it is wanted and not there. The process started, if any.
    fn round<T>(&mut self, session_active: bool, missing: bool, launch: impl FnOnce() -> Result<T, String>) -> Option<T> {
        if !session_active && !self.wanted.load(Ordering::SeqCst) {
            self.missed = 0;
            return None;
        }
        if !missing {
            self.wanted.store(false, Ordering::SeqCst);
            self.missed = 0;
            return None;
        }
        if Instant::now() < self.not_before {
            return None;
        }
        let started = match launch() {
            Ok(process) => {
                self.wanted.store(false, Ordering::SeqCst);
                self.last_error.clear();
                note("service", &format!("started {}", self.name));
                Some(process)
            }
            // Nobody is logged on yet, say: expected, and not worth a line every second.
            Err(e) => {
                if e != self.last_error {
                    note("service", &format!("cannot start {}: {e}", self.name));
                    self.last_error = e;
                }
                None
            }
        };
        self.missed += 1;
        self.not_before = Instant::now() + Duration::from_secs(u64::from(self.missed.min(MAX_BACKOFF)));
        started
    }
}

/// The service's half: starts the app and the worker when they are wanted and not there, and
/// looks after them.
pub fn spawn_supervisor(sessions: Sessions, slot: Arc<AppSlot>) {
    spawn_critical("supervisor", move || {
        let mut app = Kept::new("the app", &LAUNCH_APP);
        let mut worker = Kept::new("the worker", &LAUNCH_WORKER);
        let mut last_round = Instant::now();
        loop {
            thread::sleep(Duration::from_secs(1));
            let resumed = last_round.elapsed() >= RESUME_GAP;
            last_round = Instant::now();

            let session_active = sessions.is_running();
            let missing = slot.supervise(session_active, resumed);
            // Each is held before it runs, so its first request finds it known.
            if let Some(Started { process, thread }) = app.round(session_active, missing.app, launch_app) {
                slot.launched(process);
                thread.resume();
            }
            if let Some(Started { process, thread }) = worker.round(session_active, missing.worker, launch_worker) {
                slot.launched_worker(process);
                thread.resume();
            }
        }
    });
}

/// A process just started, its first thread not yet running (`launch`).
struct Started {
    process: Process,
    thread: Suspended,
}

/// The first thread of a process started suspended. Dropping it lets it run too, so a process is
/// never left hanging.
struct Suspended(usize);

impl Suspended {
    fn resume(self) {}
}

impl Drop for Suspended {
    fn drop(&mut self) {
        // SAFETY: the handle is ours, from `CreateProcessAsUserW`, and is not used again.
        unsafe {
            ResumeThread(self.0 as HANDLE);
            CloseHandle(self.0 as HANDLE);
        }
    }
}

/// Starts the app on the desktop of whoever is at the screen, as that user and not as SYSTEM: its
/// window has no business being SYSTEM, nor having their administrator rights.
fn launch_app() -> Result<Started, String> {
    launch("", Rights::Plain)
}

/// Starts the worker there, as SYSTEM (see the top of this file for why).
fn launch_worker() -> Result<Started, String> {
    launch(WORKER_ARG, Rights::System)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rights {
    /// The token Windows gives the user's programs.
    Plain,
    /// The service's own, moved into the user's session.
    System,
}

/// A copy of the service's own token (SYSTEM) in `session`, to start a process with there. Moving a
/// token to another session takes the privilege (SeTcb) only SYSTEM has.
///
/// # Safety
/// Plain calls; the token returned is the caller's to close.
unsafe fn system_token(session: u32) -> Result<HANDLE, String> {
    // SAFETY: every pointer is a valid place to write to; the handles are closed on every way out.
    unsafe {
        let mut own: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE, &mut own) == 0 {
            return Err(format!("cannot open the service's token (error {})", GetLastError()));
        }
        let mut token: HANDLE = null_mut();
        let copied = DuplicateTokenEx(own, TOKEN_ALL_ACCESS, null(), SecurityImpersonation, TokenPrimary, &mut token);
        let error = GetLastError();
        CloseHandle(own);
        if copied == 0 {
            return Err(format!("cannot copy the service's token (error {error})"));
        }
        let size = std::mem::size_of::<u32>() as u32;
        if SetTokenInformation(token, TokenSessionId, (&raw const session).cast(), size) == 0 {
            let error = GetLastError();
            CloseHandle(token);
            return Err(format!("cannot move the service's token to session {session} (error {error})"));
        }
        Ok(token)
    }
}

/// Starts this exe with `arguments` on the desktop of whoever is at the screen, as that user or as
/// SYSTEM (`rights`). Its first thread waits until the `Started` is let go of.
fn launch(arguments: &str, rights: Rights) -> Result<Started, String> {
    let exe = exe().ok_or("no executable to start")?;
    let mut command = wide(format!("\"{}\" {arguments}", exe.display()).trim_end());
    let mut desktop = wide(r"winsta0\default");
    // SAFETY: every pointer passed is valid for the call; the tokens, the environment block and
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
        // Someone has to be logged on either way; the worker then takes SYSTEM's token instead.
        if rights == Rights::System {
            CloseHandle(token);
            token = system_token(session)?;
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
            CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
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
        Ok(Started {
            process: Process { handle: process.hProcess as usize, pid: process.dwProcessId },
            thread: Suspended(process.hThread as usize),
        })
    }
}

/// Ends the app and the worker, when the service goes away for an uninstall. Every other process
/// of this executable that was started without arguments or as the worker; the service itself and
/// the installer's calls are left alone.
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
        let same = process.exe().is_some_and(|other| file_name(other) == name) && is_app_or_worker(process.cmd());
        if same && pid.as_u32() != std::process::id() {
            let ended = process.kill();
            note("service", &format!("{} the app or worker ({pid})", if ended { "ended" } else { "could not end" }));
        }
    }
}

/// Whether a process of this exe started with `command` (its exe first) is the app or the worker,
/// not the service or an installer's call.
fn is_app_or_worker(command: &[std::ffi::OsString]) -> bool {
    match command {
        [] | [_] => true,
        [_, argument] => argument == WORKER_ARG,
        _ => false,
    }
}

/// This executable, for starting and recognising the app.
fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Writes a line to a log file as well as to stderr: `service.log` next to the saved session for
/// the service, `logs\worker.log` there for the worker (`log_place`), `app.log` in
/// `%LOCALAPPDATA%\bullseye` for the app. The closing is the same code in the worker and, in
/// development, the app: its lines say `worker` in the worker.
///
/// Neither runs with a window, so stderr goes nowhere in a release build; the file is how the
/// user can see what they actually did.
pub fn note(who: &str, message: &str) {
    let who = if who == "app" && IN_WORKER.load(Ordering::Relaxed) { "worker" } else { who };
    eprintln!("{who}: {message}");
    // The tests must not write into the log of a real install on the machine they run on.
    if cfg!(test) {
        return;
    }
    let Some((dir, file)) =
        log_place(IN_SERVICE.load(Ordering::Relaxed), IN_WORKER.load(Ordering::Relaxed), state::dir(), state::user_dir())
    else {
        return;
    };
    // The folder only exists once something has been written to it.
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(file);
    start_over_if_big(&path, LOG_LIMIT);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {who}: {message}", timestamp());
    }
}

/// Where a process writes its log, from `%ProgramData%\bullseye` (`data`) and `%LOCALAPPDATA%\bullseye`
/// (`own`): the service `service.log` beside its files; the worker, SYSTEM too, `worker.log` in
/// their `logs` folder, which the service lets users read (`service::lock_down_data_dir`), and
/// never anywhere the user can write to, where a link could send SYSTEM's writes elsewhere; the app
/// `app.log` in its own folder.
fn log_place(service: bool, worker: bool, data: Option<PathBuf>, own: Option<PathBuf>) -> Option<(PathBuf, &'static str)> {
    match (service, worker) {
        (true, _) => Some((data?, "service.log")),
        (false, true) => Some((data?.join(LOGS), "worker.log")),
        (false, false) => Some((own?, "app.log")),
    }
}

/// The folder in `%ProgramData%\bullseye` that users may read: the worker's log.
pub const LOGS: &str = "logs";

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
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
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
        assert_eq!(file_name(std::path::Path::new(r"C:\a\B\bullseye-app.EXE")), "bullseye-app.exe");
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
        assert_eq!(slot.register_worker(0), Ok(()));
        assert_eq!(slot.heard(0), Ok(()));
    }

    #[test]
    fn only_the_worker_the_service_started_is_the_worker() {
        let (mut started, mut stranger) = (sleeper(), sleeper());
        let slot = AppSlot::new(true);
        let refused = Err("not the worker the service started".to_string());
        // Before the service started one, no worker is taken.
        assert_eq!(slot.register_worker(started.id()), refused);
        assert_eq!(slot.supervise(true, false), Missing { app: true, worker: true });

        slot.launched_worker(Process::open(started.id()).unwrap());
        assert_eq!(slot.register_worker(started.id()), Ok(()));
        assert_eq!(slot.register_worker(stranger.id()), refused);
        // Started as a worker, not as the app: the app is still missing.
        assert_eq!(slot.supervise(true, false), Missing { app: true, worker: false });
        // The stranger asking for the policy is refused and keeps nobody alive; the worker asking does.
        assert_eq!(slot.heard(stranger.id()), refused);
        assert!(slot.worker.locked().as_ref().unwrap().heard.is_none());
        assert_eq!(slot.heard(started.id()), Ok(()));
        assert!(slot.worker.locked().as_ref().unwrap().heard.is_some());

        // Once it has ended it is missing again.
        let _ = started.kill();
        let _ = started.wait();
        assert!(slot.supervise(true, false).worker);
        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    #[test]
    fn the_app_is_kept_by_being_there_not_by_asking() {
        let mut app = sleeper();
        let slot = AppSlot::new(true);
        let now = Instant::now();
        // An app that never asked for anything, long ago, while a session runs: left alone.
        *slot.current.locked() =
            Some(Adopted { process: Process::open(app.id()).unwrap(), since: now - Duration::from_secs(600), heard: None });
        assert!(!slot.supervise(true, false).app);
        assert!(!Process::open(app.id()).unwrap().exited());
        let _ = app.kill();
        let _ = app.wait();
    }

    #[test]
    fn each_process_logs_where_it_should() {
        let (data, own) = (PathBuf::from(r"C:\ProgramData\bullseye"), PathBuf::from(r"C:\Users\a\AppData\Local\bullseye"));
        let place = |service, worker| log_place(service, worker, Some(data.clone()), Some(own.clone()));
        assert_eq!(place(true, false), Some((data.clone(), "service.log")));
        // The worker is SYSTEM: never in the user's own folder, where they could put a link.
        assert_eq!(place(false, true), Some((data.join("logs"), "worker.log")));
        assert_eq!(place(false, false), Some((own.clone(), "app.log")));
        // Without the folder, no log.
        assert_eq!(log_place(false, true, None, Some(own.clone())), None);
        assert_eq!(log_place(false, false, Some(data.clone()), None), None);
    }

    #[test]
    fn the_app_and_the_worker_are_ended_on_uninstall_but_not_the_service_or_the_installer() {
        let command = |args: &[&str]| args.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert!(is_app_or_worker(&command(&[])));
        assert!(is_app_or_worker(&command(&["bullseye-app.exe"])));
        assert!(is_app_or_worker(&command(&["bullseye-app.exe", "--worker"])));
        assert!(!is_app_or_worker(&command(&["bullseye-app.exe", "--service"])));
        assert!(!is_app_or_worker(&command(&["bullseye-app.exe", "--uninstall"])));
        assert!(!is_app_or_worker(&command(&["bullseye-app.exe", "--worker", "x"])));
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
        assert!(slot.supervise(true, false).app, "nothing should have been taken in");

        // The installed exe (this test is it) is taken in, and then no other one is.
        assert_eq!(slot.register(me), Ok(()));
        assert!(!slot.supervise(true, false).app);
        assert_eq!(slot.register(me), Ok(()));
        assert_eq!(slot.register(child.id()), Err("Bullseye is already running".into()));

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_worker_that_stops_asking_is_ended_but_not_by_a_sleep_or_without_a_session() {
        let now = Instant::now();
        let ago = |seconds| now - Duration::from_secs(seconds);
        let quiet = |slot: &AppSlot, child: &Child| {
            slot.set_for_test(Process::open(child.id()).unwrap(), ago(60), Some(ago(30)));
        };
        let (mut stale, mut slept, mut idle) = (sleeper(), sleeper(), sleeper());
        let slot = AppSlot::new(true);

        // Quiet for 30 seconds while a session runs: ended, and the slot is empty for a new one.
        quiet(&slot, &stale);
        let held = Process::open(stale.id()).unwrap();
        assert!(slot.supervise(true, false).worker);
        assert!(wait_until_exited(&held));

        // The same after the machine slept: the worker is given a fresh start instead.
        quiet(&slot, &slept);
        assert!(!slot.supervise(true, true).worker);
        assert!(!Process::open(slept.id()).unwrap().exited());
        assert!(!slot.supervise(true, false).worker, "the grace after a sleep should still be running");

        // And with no session there is nothing to enforce, so nothing to end it for.
        quiet(&slot, &idle);
        assert!(!slot.supervise(false, false).worker);
        assert!(!Process::open(idle.id()).unwrap().exited());

        for child in [&mut stale, &mut slept, &mut idle] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
