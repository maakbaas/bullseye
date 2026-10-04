//! The named pipe between the service and the app: the service's server side and the app's way
//! of connecting to it.
//!
//! The pipe is open to any interactively logged-on user (`SDDL`), because the app runs as one.
//! So the service never trusts what comes in (`service::handle`), and the app never trusts who
//! answers (`verify_server`).

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};

use crate::proto::{self, Request, Response, PIPE};
use crate::watchdog::{note, wide};

/// SYSTEM: everything. Interactive users: read and write, which is all a client needs.
const SDDL: &str = "D:(A;;GA;;;SY)(A;;GRGW;;;IU)";

/// More connections than this at once are dropped.
const MAX_CLIENTS: usize = 8;

/// A client that has not finished its request this long after connecting is cut off.
const DEADLINE: Duration = Duration::from_secs(10);

/// Connections being served, by raw handle, and when each is cut off. Held while the reaper
/// cancels and while a connection removes itself, so a handle is never cancelled after it has
/// been closed and possibly reused.
static LIVE: Mutex<Vec<(usize, Instant)>> = Mutex::new(Vec::new());

/// Starts serving. The first pipe instance is made here, before this returns, so that a pipe
/// name already taken by something else fails the service start rather than going unnoticed.
///
/// `handler` answers a request and may ask for something afterwards (`A`, sent on `after` once
/// the answer has been delivered).
pub fn serve<A: Send + 'static>(
    handler: impl Fn(Request) -> (Response, Option<A>) + Send + Sync + 'static,
    after: Sender<A>,
) -> Result<(), String> {
    let first = create_instance(true)?;
    let handler = Arc::new(handler);
    thread::spawn(reaper);
    // Raw handles are not `Send`; the accept loop below owns this one from here on.
    let first = first as usize;
    thread::spawn(move || {
        // The instance a client can connect to right now.
        let mut listening = first as HANDLE;
        loop {
            // SAFETY: `listening` is a pipe instance made by `create_instance`. A client that
            // connected before this call is reported as an error, but is a success.
            let connected = unsafe { ConnectNamedPipe(listening, null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED };
            let full = LIVE.lock().unwrap().len() >= MAX_CLIENTS;
            // The next instance exists before this one is handed off. A client that arrives in
            // between is told the pipe is busy and tries again (`connect`).
            let next = match create_instance(false) {
                Ok(handle) => handle,
                Err(e) => {
                    note("service", &format!("cannot create the pipe: {e}"));
                    // The service manager starts the service again.
                    std::process::exit(1);
                }
            };
            let current = std::mem::replace(&mut listening, next);
            if !connected || full {
                // SAFETY: `current` is ours and is not used again.
                unsafe { CloseHandle(current) };
                continue;
            }
            let handler = handler.clone();
            let after = after.clone();
            let raw = current as usize;
            thread::spawn(move || {
                // SAFETY: a connected pipe instance that nothing else owns.
                let pipe = unsafe { File::from_raw_handle(raw as HANDLE) };
                serve_one(pipe, &*handler, after);
            });
        }
    });
    Ok(())
}

fn serve_one<A>(mut pipe: File, handler: &dyn Fn(Request) -> (Response, Option<A>), after: Sender<A>) {
    let key = pipe.as_raw_handle() as usize;
    LIVE.lock().unwrap().push((key, Instant::now() + DEADLINE));

    let (response, then) = match proto::read_message::<Request>(&mut pipe) {
        // A panic while answering must end this connection, not the service.
        Ok(request) => catch_unwind(AssertUnwindSafe(|| handler(request)))
            .unwrap_or_else(|_| (Response::Failed("internal error".into()), None)),
        Err(e) => (Response::Failed(format!("bad request: {e}")), None),
    };
    let _ = proto::write_message(&mut pipe, &response);
    // Closing the pipe right after writing could throw the answer away before the client reads
    // it. This read returns once the client has closed its end, which it does when it has read.
    let _ = pipe.read(&mut [0u8; 1]);

    LIVE.lock().unwrap().retain(|&(handle, _)| handle != key);
    drop(pipe);
    if let Some(then) = then {
        let _ = after.send(then);
    }
}

/// Cancels the reads of connections that took too long, so one client cannot hold a slot for ever.
fn reaper() {
    loop {
        thread::sleep(Duration::from_secs(1));
        let now = Instant::now();
        for &(handle, deadline) in LIVE.lock().unwrap().iter() {
            if deadline <= now {
                // SAFETY: the handle is still open: its connection removes itself from `LIVE`
                // under the same lock before closing it.
                unsafe { CancelIoEx(handle as HANDLE, null()) };
            }
        }
    }
}

fn create_instance(first: bool) -> Result<HANDLE, String> {
    let sddl = wide(SDDL);
    let name = wide(PIPE);
    // SAFETY: the strings are null-terminated; the descriptor is freed after the pipe has
    // copied it; the attributes outlive the call.
    unsafe {
        let mut descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, null_mut()) == 0 {
            return Err(format!("cannot read the pipe's access list (error {})", GetLastError()));
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // FIRST_PIPE_INSTANCE fails if anyone already has a pipe of this name, so the name
        // cannot be taken before the service gets to it.
        let mode = if first { PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_DUPLEX };
        let handle = CreateNamedPipeW(
            name.as_ptr(),
            mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        );
        let error = GetLastError();
        LocalFree(descriptor);
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("cannot create {PIPE} (error {error})"));
        }
        Ok(handle)
    }
}

/// Opens the service's pipe, waiting briefly while the service is between two instances.
pub fn connect() -> Result<File, String> {
    for _ in 0..40 {
        match OpenOptions::new().read(true).write(true).open(PIPE) {
            Ok(pipe) => {
                verify_server(&pipe)?;
                return Ok(pipe);
            }
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("the Site Blocker service is not running ({e})")),
        }
    }
    Err("the Site Blocker service is busy".into())
}

/// Checks that whoever serves the pipe is this very executable. The service is the installed exe,
/// started by Windows as SYSTEM; a standard user cannot write to that path, so cannot run a
/// look-alike from it. Without this, anything could open the pipe first and tell the app there is
/// no block.
fn verify_server(pipe: &File) -> Result<(), String> {
    // Development only: `tauri dev` replaces the exe while it is running, which Windows refuses if
    // the console service is running from that same file, so the console service runs from a copy.
    // A release build never skips the check.
    if cfg!(debug_assertions) && std::env::var_os("SITEBLOCKER_DEV").is_some() {
        return Ok(());
    }
    // SAFETY: the handle is a valid pipe; the process handle is closed again; the buffer is as
    // long as the length says.
    let theirs = unsafe {
        let mut pid = 0u32;
        if GetNamedPipeServerProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) == 0 {
            return Err("cannot tell who runs the pipe".into());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err("cannot look at who runs the pipe".into());
        }
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if ok == 0 {
            return Err("cannot read the name of whoever runs the pipe".into());
        }
        String::from_utf16_lossy(&buffer[..length as usize])
    };
    let mine = std::env::current_exe().map_err(|e| e.to_string())?;
    if theirs.eq_ignore_ascii_case(&mine.to_string_lossy()) {
        Ok(())
    } else {
        Err(format!("the pipe is served by {theirs}, not by this installation"))
    }
}
