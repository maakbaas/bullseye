# Moving the watchdog to a SYSTEM service: implementation guide for this repo

This guide is for implementing the SYSTEM-service design in *this* repo (`blocker`, Tauri tray app, Windows only). It is written against the actual code in `src-tauri/src/`. All the Rust below was compiled and the unit tests run in a scratch copy of the repo (`cargo check --tests`, no warnings; `cargo test`: 67 passed). One extra integration test (`the_pipe_answers_like_the_app_expects`, run with `--ignored`) started the real pipe server and talked to it like the app does: it passed. The service was also seen working on one real install (its logs showed it launching the app in the user's session and refusing an uninstall during a timed block). Section 9 lists what could **not** be tested without installing a real service, so you must test those parts yourself.

**Read sections 1 and 8 before you touch any code.** Section 8 is about how to get out if your own test setup locks you in.

## 1. What you are building, and why it is different from today

Today everything runs as *you*: the app, a launcher process that restarts it, and a scheduled task that restarts the launcher at logon. Because they all run as your user, you (or anything you run) can kill them, edit `state.json`, or disable the task. There is nothing to stop that.

The new design has two parts:

| Part | Runs as | Where | Job |
|---|---|---|---|
| **Service** (`bullseye.exe --service`) | `SYSTEM` | session 0 (no desktop), started by Windows at boot | Owns the block, the unlock hash and internet time. Restarts the app. Answers the app over a named pipe. |
| **App** (`bullseye.exe`, no arguments) | the logged-on user | the user's desktop | Tray icon, UI, and closing tabs (`tab_closer.rs`). Holds **no** authoritative state: it asks the service every second what to enforce. |

Vocabulary you need:

- **SYSTEM** is the account Windows itself runs its core services as. A process running as SYSTEM cannot be killed from Task Manager by an ordinary administrator ("Access denied"), and files owned by SYSTEM can be hidden from administrators.
- **SCM** (Service Control Manager) starts services at boot, restarts them when they fail, and decides who may stop them. That decision is a security descriptor written in a string format called **SDDL**.
- **Session 0 isolation**: services run in session 0, which has no desktop. A service cannot see or click anything in the user's browser. That is why the app has to stay a user-level process: UI Automation only works on the user's own desktop. The service's job is to keep that app alive and to be the one place the truth lives.
- **Named pipe**: a local, Windows-only socket (`\\.\pipe\bullseye`). Any local program can try to connect to it, so the service has to treat every request as hostile input.

How the old pieces map to the new ones:

| Old | New |
|---|---|
| `task.rs` (logon task) | **Deleted.** Windows starts the service at boot; the service starts the app at logon. |
| `watchdog.rs` launcher + app mutex dance | Rewritten: the service keeps a handle to the one process it takes for the app (one it started with `CreateProcessAsUserW`, or the installed exe registering itself), restarts it, and ends it if it stops asking for its policy. The single-instance mutex is gone. |
| `state.json` in `%LOCALAPPDATA%` | Moved to `%ProgramData%\bullseye`, owned by SYSTEM, administrators read-only. |
| `Blocklist`, `clock.rs`, `unlock.rs` in the app | Run in the service. **`block.rs`, `clock.rs`, `unlock.rs`, `filters.rs`, `tab_closer.rs` are almost untouched** (see the diffs). |
| Tauri commands did the work | They forward to the service over the pipe (`client.rs`). |
| `--install` / `--uninstall` registered the task | They register / remove the service, by asking the service over the pipe. New: `--prepare-update`. |
| per-user installer | **Per-machine installer, needs admin (UAC prompt).** |

Files at a glance:

| File | What to do |
|---|---|
| `src-tauri/Cargo.toml` | edit (2 dependencies; section 3) |
| `src-tauri/src/proto.rs` | **new** (the messages on the pipe) |
| `src-tauri/src/pipe.rs` | **new** (pipe server and client) |
| `src-tauri/src/client.rs` | **new** (the app's side) |
| `src-tauri/src/programs.rs` | **new** (the list of programs to close, section 6.4) |
| `src-tauri/src/service.rs` | **new** (the running service, section 6.1) |
| `src-tauri/src/handler.rs` | **new** (what the service does with each request, section 6.1) |
| `src-tauri/src/registration.rs` | **new** (installing, locking down and removing the service, section 6.1) |
| `src-tauri/src/sync.rs` | **new** (locks that survive a crash, threads that must not die quietly, section 4.4) |
| `src-tauri/src/ui.rs` | **new** (the window, made on demand and destroyed after a while, section 6.6) |
| `src-tauri/src/watchdog.rs` | replace the whole file |
| `src-tauri/src/setup.rs` | replace the whole file |
| `src-tauri/src/main.rs` | replace the whole file |
| `src-tauri/src/block.rs`, `state.rs`, `tab_closer.rs` | small edits (diffs below) |
| `src-tauri/src/task.rs` | **delete** |
| `src-tauri/src/filters.rs` | small edits (section 5.4: web pages only, the scheme fix) |
| `src-tauri/src/clock.rs`, `unlock.rs` | **unchanged** |
| `src/index.html` | small edit (show the error when the service is unreachable) |
| `src-tauri/tauri.conf.json` | `perMachine`, and no window at startup (section 6.6) |
| `src-tauri/windows/hooks.nsh` | replace the whole file |
| `CLAUDE.md` | update (section 7) |

**Before you start:** this reverses a documented decision (`CLAUDE.md` says "no admin needed"). Make sure the owner has signed off.

## 2. Suggested order of work

Each step ends in something you can compile or test. Do not move on while a step is red.

1. Cargo.toml (section 3). `cargo check`.
2. `proto.rs`, `pipe.rs`, `client.rs` (section 4). They will not compile until the modules are declared in `main.rs` in step 5; so do steps 1–5 together, then `cargo check --tests`.
3. Small edits to `block.rs` and `state.rs` (section 5).
4. `service.rs` (section 6.1).
5. `watchdog.rs`, `setup.rs`, `main.rs`, `tab_closer.rs`, delete `task.rs` (section 6.2). Now `cargo check --tests` and `cargo test` must pass.
6. Run the pipe integration test (section 9.1) and try the dev loop (section 8.1).
7. `index.html`, `tauri.conf.json`, `hooks.nsh` (section 6.3), then build the installer and test it **in a virtual machine** (section 9.2).

## 3. `Cargo.toml`

`windows-service` is the crate that talks to the SCM. `windows-sys` needs more feature flags now (pipes, security descriptors, sessions, process tokens).

```diff
--- a/src-tauri/Cargo.toml
+++ b/src-tauri/Cargo.toml
@@ -16,8 +16,21 @@
 sysinfo = "0.39.6"
 tauri = { version = "2", features = ["tray-icon"] }
 uiautomation = "0.25.1"
-# named mutexes, for the app/watchdog pair (see src/watchdog.rs)
-windows-sys = { version = "0.61", features = ["Win32_Foundation", "Win32_Security", "Win32_System_Threading"] }
+# the service (src/service.rs); install, failure actions, session notifications
+windows-service = "0.8"
+# named mutexes, the named pipe, starting the app in the user's session (see src/watchdog.rs, src/pipe.rs)
+windows-sys = { version = "0.61", features = [
+    "Win32_Foundation",
+    "Win32_Security",
+    "Win32_Security_Authorization",
+    "Win32_Storage_FileSystem",
+    "Win32_System_Environment",
+    "Win32_System_IO",
+    "Win32_System_Pipes",
+    "Win32_System_RemoteDesktop",
+    "Win32_System_Threading",
+    "Win32_UI_WindowsAndMessaging",
+] }
 
 [profile.dev]
 incremental = false       # don't accumulate a multi-GB incremental cache
```

## 4. The new modules

### 4.1 `src-tauri/src/proto.rs`: what the app and the service say to each other

One request and one answer per connection: a 4-byte little-endian length, then that many bytes of JSON. The size limit (64 KB) exists so a request cannot make the service allocate memory without bound. The tests at the bottom cover a round trip, an oversized length, truncated input and unknown commands.

```rust
//! What the app and the service say to each other over the named pipe (`pipe.rs`).
//!
//! One request and one answer per connection, each a 4-byte little-endian length followed by
//! that many bytes of JSON. The service treats every request as untrusted: any local user can
//! open the pipe and send whatever they like (`service::handle` checks it all again).

use std::io::{Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::block::Status;
use crate::state::Enforcers;

/// The pipe the service listens on.
pub const PIPE: &str = r"\\.\pipe\bullseye";

/// Larger messages are refused, so a request cannot make the service allocate without limit.
const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
pub enum Request {
    /// What the UI shows: time left, entries, enforcers.
    Status,
    /// What the app enforces: asked every second, from the thread that closes tabs, which is how
    /// the service knows the closing is going on (`watchdog::AppSlot`).
    Policy,
    /// The app, when it starts: asks the service to take it in as the app. Refused if another app
    /// is running or this is not the installed exe, and then the app ends itself.
    Register,
    Start {
        filters: Vec<String>,
        /// Sites that are always allowed, which win over `filters`.
        #[serde(default)]
        allowed: Vec<String>,
        /// Programs to close, one exe name per line (`programs.rs`).
        #[serde(default)]
        programs: Vec<String>,
        duration: String,
        enforcers: Enforcers,
    },
    Stop,
    Unlock { password: String },
    /// From the installer: end the service and remove it. Refused during a timed block.
    PrepareUninstall,
    /// From the installer: let go of the exe so it can be replaced. Refused during any block.
    PrepareUpdate,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Response {
    Done,
    Failed(String),
    /// A block is running that does not allow this (uninstall, update).
    Refused(String),
    Status(Status),
    Policy(Option<Policy>),
}

/// What the app has to enforce right now.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Policy {
    pub filters: Vec<String>,
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub programs: Vec<String>,
    pub enforcers: Enforcers,
}

pub fn write_message<T: Serialize>(to: &mut impl Write, message: &T) -> Result<(), String> {
    let body = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    if body.len() > MAX_MESSAGE {
        return Err("message too large".into());
    }
    // One write, so the length and the body never arrive apart.
    let mut frame = (body.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&body);
    to.write_all(&frame).and_then(|_| to.flush()).map_err(|e| e.to_string())
}

pub fn read_message<T: DeserializeOwned>(from: &mut impl Read) -> Result<T, String> {
    let mut length = [0u8; 4];
    from.read_exact(&mut length).map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Err("message too large".into());
    }
    let mut body = vec![0u8; length];
    from.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_request_survives_the_wire() {
        let request = Request::Start {
            filters: vec!["tweakers.net".into()],
            allowed: vec!["gathering.tweakers.net".into()],
            programs: vec!["steam*".into()],
            duration: "10m".into(),
            enforcers: Enforcers::default(),
        };
        let mut wire = Vec::new();
        write_message(&mut wire, &request).unwrap();
        assert_eq!(read_message::<Request>(&mut Cursor::new(wire)), Ok(request));

        let mut wire = Vec::new();
        write_message(&mut wire, &Request::Policy).unwrap();
        assert_eq!(read_message::<Request>(&mut Cursor::new(wire)), Ok(Request::Policy));
    }

    #[test]
    fn a_policy_survives_the_wire() {
        let response = Response::Policy(Some(Policy {
            filters: vec!["a.com".into()],
            allowed: vec!["b.com".into()],
            programs: vec!["game.exe".into()],
            enforcers: Enforcers::default(),
        }));
        let mut wire = Vec::new();
        write_message(&mut wire, &response).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(response));
        let mut wire = Vec::new();
        write_message(&mut wire, &Response::Policy(None)).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(Response::Policy(None)));
    }

    #[test]
    fn a_start_without_programs_still_reads() {
        let body = br#"{"cmd":"start","args":{"filters":["a.com"],"duration":"1h","enforcers":{}}}"#;
        let mut wire = (body.len() as u32).to_le_bytes().to_vec();
        wire.extend_from_slice(body);
        let Ok(Request::Start { programs, allowed, .. }) = read_message::<Request>(&mut Cursor::new(wire)) else { panic!("not read") };
        assert!(programs.is_empty() && allowed.is_empty());
    }

    #[test]
    fn oversized_and_broken_messages_are_refused() {
        // A length far beyond the limit is refused before anything is allocated.
        let huge = u32::MAX.to_le_bytes().to_vec();
        assert!(read_message::<Request>(&mut Cursor::new(huge)).is_err());
        // Cut short, and not JSON.
        assert!(read_message::<Request>(&mut Cursor::new(vec![9, 0, 0, 0, b'{'])).is_err());
        let mut wire = 5u32.to_le_bytes().to_vec();
        wire.extend_from_slice(b"nope!");
        assert!(read_message::<Request>(&mut Cursor::new(wire)).is_err());
        // An unknown command is not a request.
        let body = br#"{"cmd":"format_c"}"#;
        let mut wire = (body.len() as u32).to_le_bytes().to_vec();
        wire.extend_from_slice(body);
        assert!(read_message::<Request>(&mut Cursor::new(wire)).is_err());
    }
}
```

### 4.2 `src-tauri/src/pipe.rs`: the pipe, both ends

The server side needs care, so these are the decisions made in it, in case you want to change something:

- **Who may connect** is `SDDL = "D:(A;;GA;;;SY)(A;;GRGW;;;IU)"`: SYSTEM everything, "Interactive users" (anyone logged on at the machine) read and write. The app runs as such a user, so it must be allowed. `PIPE_REJECT_REMOTE_CLIENTS` blocks connections over the network.
- **`FILE_FLAG_FIRST_PIPE_INSTANCE`** on the first instance makes creating the pipe fail if somebody else already has a pipe with that name. Otherwise a local program could grab the name first and answer the app "no block is running".
- **The app checks who it is talking to** (`verify_server`): the pipe must be served from **session 0**, the session of services (`GetNamedPipeServerSessionId`). Only SYSTEM and other services run there; a standard user cannot start a process in it, so cannot stand in for the service. **Do not replace this by looking at the serving process** (its file name, say): a standard user is not allowed to open a SYSTEM process, not even with `PROCESS_QUERY_LIMITED_INFORMATION`. The first version did exactly that and the app then refused to talk to the real service ("cannot look at who runs the pipe").
- **A silent client cannot hold the service up**: every connection gets its own thread, at most 8 at once, and a *reaper* thread cancels the read of any connection older than 10 seconds (`CancelIoEx`). The bookkeeping (`LIVE`) is locked while a handle is cancelled and while a connection removes itself, so a handle is never cancelled after it was closed and possibly reused by something else.
- **After writing the answer, the server does one more read.** That read returns when the client closes its end, which it does after reading. Closing the server end straight after writing could throw the answer away before the client has read it.
- A **panic** while handling a request is caught and turned into an error answer: it must end that connection, not the service.

```rust
//! The named pipe between the service and the app: the service's server side and the app's way
//! of connecting to it.
//!
//! The pipe is open to any interactively logged-on user (`SDDL`), because the app runs as one.
//! So the service never trusts what comes in (`service::handle`), and the app never trusts who
//! answers (`verify_server`: only a pipe served from session 0, the services' session).

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
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerSessionId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::proto::{self, Request, Response, PIPE};
use crate::sync::{spawn_critical, LockExt};
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
/// `handler` answers a request, given the process id of whoever sent it (which the pipe knows and
/// the sender cannot lie about), and may ask for something afterwards (`A`, sent on `after` once
/// the answer has been delivered).
pub fn serve<A: Send + 'static>(
    handler: impl Fn(u32, Request) -> (Response, Option<A>) + Send + Sync + 'static,
    after: Sender<A>,
) -> Result<(), String> {
    let first = create_instance(true)?;
    let handler = Arc::new(handler);
    spawn_critical("pipe reaper", reaper);
    // Raw handles are not `Send`; the accept loop below owns this one from here on.
    let first = first as usize;
    spawn_critical("pipe", move || {
        // The instance a client can connect to right now.
        let mut listening = first as HANDLE;
        loop {
            // SAFETY: `listening` is a pipe instance made by `create_instance`. A client that
            // connected before this call is reported as an error, but is a success.
            let connected = unsafe { ConnectNamedPipe(listening, null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED };
            let full = LIVE.locked().len() >= MAX_CLIENTS;
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

fn serve_one<A>(mut pipe: File, handler: &dyn Fn(u32, Request) -> (Response, Option<A>), after: Sender<A>) {
    let key = pipe.as_raw_handle() as usize;
    LIVE.locked().push((key, Instant::now() + DEADLINE));

    // Who is on the other end, as Windows knows it. 0 if that cannot be told.
    let mut client = 0u32;
    // SAFETY: the handle is a connected pipe and `client` a valid place to write to.
    unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle() as HANDLE, &mut client) };

    let (response, then) = match proto::read_message::<Request>(&mut pipe) {
        // A panic while answering must end this connection, not the service.
        Ok(request) => catch_unwind(AssertUnwindSafe(|| handler(client, request)))
            .unwrap_or_else(|_| (Response::Failed("internal error".into()), None)),
        Err(e) => (Response::Failed(format!("bad request: {e}")), None),
    };
    let _ = proto::write_message(&mut pipe, &response);
    // Closing the pipe right after writing could throw the answer away before the client reads
    // it. This read returns once the client has closed its end, which it does when it has read.
    let _ = pipe.read(&mut [0u8; 1]);

    LIVE.locked().retain(|&(handle, _)| handle != key);
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
        for &(handle, deadline) in LIVE.locked().iter() {
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
            Err(e) => return Err(format!("the Bullseye service is not running ({e})")),
        }
    }
    Err("the Bullseye service is busy".into())
}

/// Checks that the pipe is served from session 0, the session of services. Only SYSTEM and other
/// services run there: a standard user cannot start a process in it, so cannot stand in for the
/// service. Without this, anything could open the pipe first and tell the app there is no block.
///
/// This asks the pipe itself. Looking at the serving process (its file name, say) does not work:
/// a standard user may not open a SYSTEM process, not even for a look.
fn verify_server(pipe: &File) -> Result<(), String> {
    // Only the pipe test, which serves from the test's own process, in the user's session. No
    // other build can skip the check.
    if cfg!(test) && std::env::var_os("BULLSEYE_DEV").is_some() {
        return Ok(());
    }
    let mut session = u32::MAX;
    // SAFETY: the handle is a valid pipe and `session` a valid place to write to.
    if unsafe { GetNamedPipeServerSessionId(pipe.as_raw_handle() as HANDLE, &mut session) } == 0 {
        return Err(format!("cannot tell where the pipe is served from (error {})", unsafe { GetLastError() }));
    }
    if session == 0 {
        Ok(())
    } else {
        Err(format!("the pipe is served from session {session}, not by the service"))
    }
}
```

### 4.3 `src-tauri/src/client.rs`: the app's side

`Remote::active()` has the same shape `Blocklist::active()` had, which is why `tab_closer.rs` hardly changes. **If the service does not answer, the app keeps enforcing the last policy it got** (held in memory, never written to disk). A gap in the service (restart, update) must not open the block.

```rust
//! The app's side of the service: asks the service what to show and what to enforce.
//!
//! The app holds no block of its own. `tab_closer.rs` asks `Remote::active` every second, which
//! has the same shape as `Blocklist::active` did, so the closing code did not have to change.
//! If the service cannot be reached (it is restarting, or being updated), the last answer keeps
//! being enforced: a gap in the service must not open the block.
//!
//! In a debug build (`npm run dev`) the service's logic can run inside the app instead
//! (`Remote::local`): no service, no pipe, no installer, and nothing that has to stay alive. Release
//! builds do not contain that.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::block::{Active, Status};
use crate::filters::FilterList;
use crate::pipe;
use crate::programs::ProgramList;
use crate::proto::{self, Policy, Request, Response};
use crate::registration;
use crate::sync::LockExt;
use crate::watchdog::note;

/// The service's logic called directly, in place of the pipe (`service::local_handler`).
pub type Local = Arc<dyn Fn(Request) -> Response + Send + Sync>;

/// One request and its answer.
pub fn call(request: &Request) -> Result<Response, String> {
    let mut pipe = pipe::connect()?;
    proto::write_message(&mut pipe, request)?;
    proto::read_message(&mut pipe)
}

#[derive(Clone, Default)]
pub struct Remote {
    /// The last policy the service gave, parsed once, not every second.
    cache: Arc<Mutex<Option<Cached>>>,
    /// Whether the last call failed, so a failing service is logged once rather than every second.
    failing: Arc<AtomicBool>,
    /// When the service was last asked to start, so that is not tried every second.
    last_start: Arc<Mutex<Option<Instant>>>,
    /// Set in development: the service's logic, in this process.
    local: Option<Local>,
}

/// The least time between two attempts to start the service.
const START_EVERY: Duration = Duration::from_secs(5);

struct Cached {
    policy: Policy,
    active: Active,
}

impl Remote {
    pub fn new() -> Remote {
        Remote::default()
    }

    /// A remote that is its own service: for `npm run dev`, and for tests.
    #[cfg(any(debug_assertions, test))]
    pub fn local(handler: Local) -> Remote {
        Remote { local: Some(handler), ..Remote::default() }
    }

    /// Whether this is its own service, so there is nobody to register with or to start.
    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    /// One request, to the service over the pipe or to the local one.
    fn send(&self, request: Request) -> Result<Response, String> {
        match &self.local {
            Some(local) => Ok(local(request)),
            None => call(&request),
        }
    }

    /// A command that answers yes or no.
    pub fn command(&self, request: Request) -> Result<(), String> {
        match self.send(request)? {
            Response::Done => Ok(()),
            Response::Failed(why) | Response::Refused(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    pub fn status(&self) -> Result<Status, String> {
        match self.send(Request::Status)? {
            Response::Status(status) => Ok(status),
            Response::Failed(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    /// What to enforce right now, if anything.
    pub fn active(&self) -> Option<Active> {
        match self.send(Request::Policy) {
            Ok(Response::Policy(policy)) => {
                self.failing.store(false, Ordering::Relaxed);
                self.apply(policy);
            }
            Ok(other) => self.fail(&format!("unexpected answer: {other:?}")),
            Err(e) => {
                self.fail(&e);
                self.revive_service();
            }
        }
        self.cache.locked().as_ref().map(|c| c.active.clone())
    }

    /// Asks Windows to start the service if it is registered but not running (it only stays down
    /// if something ended it with exit code 0, or an update did not finish). Starting is the only
    /// thing the app may do to the service: the access list gives interactive users no more
    /// (`registration::SDDL`). A service that is not registered at all, as in development, just fails
    /// quietly.
    fn revive_service(&self) {
        if self.is_local() {
            return;
        }
        let mut last = self.last_start.locked();
        if last.is_some_and(|at| at.elapsed() < START_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        if let Ok(true) = registration::start_registered() {
            note("app", "the service was not running; started it");
        }
    }

    fn fail(&self, why: &str) {
        if !self.failing.swap(true, Ordering::Relaxed) {
            note("app", &format!("the service did not answer, going on with the last block: {why}"));
        }
    }

    /// Takes in what the service said; the list is only parsed again when it changed.
    fn apply(&self, policy: Option<Policy>) {
        let mut cache = self.cache.locked();
        let Some(policy) = policy else {
            *cache = None;
            return;
        };
        if cache.as_ref().is_some_and(|c| c.policy == policy) {
            return;
        }
        match (FilterList::parse(&policy.filters), FilterList::parse(&policy.allowed), ProgramList::parse(&policy.programs)) {
            (Ok(filters), Ok(allowed), Ok(programs)) => {
                let active = Active {
                    filters: Arc::new(filters),
                    allowed: Arc::new(allowed),
                    programs: Arc::new(programs),
                    enforcers: policy.enforcers,
                };
                *cache = Some(Cached { policy, active });
            }
            // Cannot happen with a service of this version; keep what is being enforced.
            _ => note("app", "the service sent a block list that cannot be read"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service;
    use crate::state::Enforcers;

    fn policy(filters: &[&str]) -> Policy {
        Policy {
            filters: filters.iter().map(|f| f.to_string()).collect(),
            allowed: Vec::new(),
            programs: Vec::new(),
            enforcers: Enforcers::default(),
        }
    }

    fn current(remote: &Remote) -> Option<Active> {
        remote.cache.locked().as_ref().map(|c| c.active.clone())
    }

    #[test]
    fn the_policy_follows_the_service_and_is_parsed_once() {
        let remote = Remote::new();
        assert!(current(&remote).is_none());

        remote.apply(Some(policy(&["tweakers.net"])));
        let list = current(&remote).unwrap().filters;
        assert!(list.is_blocked("https://tweakers.net/nieuws"));

        // The same policy again keeps the same parsed list.
        remote.apply(Some(policy(&["tweakers.net"])));
        assert!(Arc::ptr_eq(&list, &current(&remote).unwrap().filters));

        // A different one replaces it; none at all ends the block.
        remote.apply(Some(policy(&["b.com"])));
        assert!(current(&remote).unwrap().filters.is_blocked("b.com/x"));
        remote.apply(None);
        assert!(current(&remote).is_none());
    }

    #[test]
    fn the_programs_come_along_and_are_parsed_with_the_policy() {
        let remote = Remote::new();
        let with_programs = Policy { filters: vec![], allowed: vec![], programs: vec!["Steam*".into()], enforcers: Enforcers::default() };
        remote.apply(Some(with_programs));
        let active = current(&remote).unwrap();
        assert!(active.programs.is_blocked("steamwebhelper.exe") && !active.programs.is_blocked("chrome.exe"));
        // A list that cannot be read leaves what is being enforced as it was.
        let wrong = Policy { filters: vec![], allowed: vec![], programs: vec!["*".into()], enforcers: Enforcers::default() };
        remote.apply(Some(wrong));
        assert!(current(&remote).unwrap().programs.is_blocked("steamwebhelper.exe"));
    }

    #[test]
    fn a_local_remote_is_a_service_of_its_own_without_a_pipe() {
        use crate::block::Blocklist;
        use crate::state::Store;
        let dir = std::env::temp_dir().join(format!("client-{}-local", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let remote = Remote::local(service::local_handler(Blocklist::with_store(Store::new(dir.join("state.json")))));
        assert!(remote.is_local());
        assert!(remote.active().is_none());

        let start = Request::Start {
            filters: vec!["a.com".into()],
            allowed: vec![],
            programs: vec![],
            duration: "1h".into(),
            enforcers: Enforcers::default(),
        };
        remote.command(start).unwrap();
        assert!(remote.active().unwrap().blocks_site("https://a.com/x"));
        assert!(remote.status().unwrap().remaining.is_some());
        // The same rules as with the real service: a timed block cannot be stopped early, the
        // password ends it.
        assert!(remote.command(Request::Stop).is_err());
        assert!(remote.command(Request::Unlock { password: "wrong".into() }).is_err());
        remote.command(Request::Unlock { password: "test1234".into() }).unwrap();
        assert!(remote.active().is_none());
    }
}
```

### 4.4 `src-tauri/src/sync.rs`: locks and threads that must not die quietly

Two small things, used everywhere:

- **`LockExt::locked()`** replaces every `.lock().unwrap()`. A lock is "poisoned" when a thread crashed while holding it, and `.unwrap()` then crashes every thread that touches it next. In the service those would be the ticker, the supervisor and every request, while the process stays up, so Windows would **not** restart it: a service that looks healthy and does nothing. The data behind the locks is plain (a list, an end time, some flags) and still valid after a crash, so `locked()` just takes the lock.
- **`spawn_critical(name, work)`** starts the threads that are part of keeping things working (the clock, the ticker, the supervisor, the pipe and its reaper, the tab closer). If one crashes, the **process ends**: Windows restarts the service, the service restarts the app, and the block is read back from disk. Per-connection pipe threads are plain threads: a crashed request is caught (`catch_unwind` in `pipe.rs`) and answered with an error, it does not end the service.

```rust
//! Small helpers for state shared between threads, and for threads that must not die quietly.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::watchdog::note;

/// Locking that carries on after another thread crashed while holding the lock.
///
/// Rust marks such a lock "poisoned", and the usual `.lock().unwrap()` then crashes every thread
/// that touches it next: in the service that would be the ticker, the supervisor and every request,
/// while the process itself stays up, so Windows would not restart it. What this app keeps behind
/// its locks is plain data (a list, an end time, a few flags) that is still valid after a crash,
/// so the right answer is to take the lock anyway.
pub trait LockExt<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Starts a thread that is part of what keeps the app or the service working: the ticker, the
/// supervisor, the pipe, the tab closer. If it crashes, the whole process ends. Windows then
/// starts the service again (and the service starts the app again), and the block is read back
/// from disk, which beats a process that looks healthy while one of its jobs is no longer done.
pub fn spawn_critical(name: &'static str, work: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if catch_unwind(AssertUnwindSafe(work)).is_err() {
                note("thread", &format!("the {name} thread crashed; ending the process so it is started again"));
                std::process::exit(1);
            }
        })
        .expect("cannot start a thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_lock_is_still_usable_after_a_thread_crashed_holding_it() {
        let shared = Arc::new(Mutex::new(vec![1, 2, 3]));
        let crashing = shared.clone();
        let result = thread::spawn(move || {
            let mut list = crashing.lock().unwrap();
            list.push(4);
            panic!("crashed while holding the lock");
        })
        .join();
        assert!(result.is_err());
        // The plain way now fails for everyone...
        assert!(shared.lock().is_err());
        // ...this way carries on, with the data as the crashed thread left it.
        assert_eq!(*shared.locked(), vec![1, 2, 3, 4]);
        shared.locked().push(5);
        assert_eq!(shared.locked().len(), 5);
    }
}
```

## 5. Small edits to existing files

### 5.1 `block.rs`

Three small things: `Status` now travels over the pipe in both directions, `reset()` is what the unlock password uses (the old code killed the process to end a block; the service must not die, so the block has to be cleared in memory *and* on disk), and a test constructor so `service.rs` can test against a temporary store.

```diff
--- a/src-tauri/src/block.rs
+++ b/src-tauri/src/block.rs
@@ -10,11 +10,13 @@
 use std::sync::{Arc, Mutex};
 use std::time::{Duration, Instant, SystemTime};
 
-use serde::Serialize;
+use serde::{Deserialize, Serialize};
 
 use crate::clock::{self, Clock};
 use crate::filters::FilterList;
-use crate::state::{self, Enforcers, Store};
+use crate::programs::ProgramList;
+use crate::state::{self, Enforcers, Store, Uses};
+use crate::sync::LockExt;
 
 /// How often a running timed block writes down the time it has reached (`Saved::seen`).
 const CHECKPOINT: Duration = Duration::from_secs(60);
@@ -29,8 +31,51 @@
     clock: &'static Clock,
 }
 
+/// What a block is told to block. **A new kind of list goes here**, together with a `state::Uses`
+/// variant, an arm in `has`, and a field in `Block` and `state::Saved`.
+pub struct Lists {
+    /// The sites to block.
+    pub sites: FilterList,
+    /// The sites that are always allowed, which win over `sites`. They are not something an
+    /// enforcer needs on its own (`Uses::Sites` is about the block list), they only make
+    /// exceptions to it.
+    pub allowed: FilterList,
+    pub programs: ProgramList,
+}
+
+impl Lists {
+    /// Whether the list an enforcer needs has an entry (`Enforcers::can_run`).
+    pub fn has(&self, uses: Uses) -> bool {
+        match uses {
+            Uses::Nothing => true,
+            Uses::Sites => !self.sites.is_empty(),
+            Uses::Programs => !self.programs.is_empty(),
+        }
+    }
+}
+
+/// What is to be enforced right now.
+#[derive(Clone)]
+pub struct Active {
+    pub filters: Arc<FilterList>,
+    pub allowed: Arc<FilterList>,
+    pub programs: Arc<ProgramList>,
+    pub enforcers: Enforcers,
+}
+
+impl Active {
+    /// Whether a page is to be closed: on the block list, and not on the list of allowed sites.
+    /// Allowed wins, whatever the block list says, so `*` on the block list with `google.com`
+    /// allowed blocks every site but that one.
+    pub fn blocks_site(&self, url: &str) -> bool {
+        self.filters.is_blocked(url) && !self.allowed.is_blocked(url)
+    }
+}
+
 struct Block {
     filters: Arc<FilterList>,
+    allowed: Arc<FilterList>,
+    programs: Arc<ProgramList>,
     /// When a timed block ends, in internet time; `None` blocks until stopped.
     until: Option<SystemTime>,
     enforcers: Enforcers,
@@ -38,13 +83,19 @@
     saved_at: Instant,
 }
 
-#[derive(Serialize)]
+#[derive(Serialize, Deserialize, Debug, PartialEq)]
 pub struct Status {
     blocking: bool,
     /// Seconds left of a timed block.
     pub remaining: Option<u64>,
     /// The blocked entries, so the UI can show what a resumed block is blocking.
     pub filters: Vec<String>,
+    /// The allowed sites, for the same reason.
+    #[serde(default)]
+    pub allowed: Vec<String>,
+    /// The blocked programs, for the same reason.
+    #[serde(default)]
+    pub programs: Vec<String>,
     /// The running block's enforcers, so the UI shows what a resumed block enforces.
     enforcers: Option<Enforcers>,
     /// Whether the time comes from the internet yet; until then, time with the app not running
@@ -58,15 +109,28 @@
         Blocklist { block: Arc::default(), store: state::store().map(Arc::new), clock: clock::global() }
     }
 
+    /// A block list over a store of its own and a clock of its own, for other modules' tests.
+    #[cfg(test)]
+    pub fn with_store(store: Store) -> Blocklist {
+        Blocklist { block: Arc::default(), store: Some(Arc::new(store)), clock: Box::leak(Box::default()) }
+    }
+
     /// Starts a block, unless one is running: checked here rather than only in the UI, so a
     /// timed block cannot be swapped for a shorter or untimed one.
-    pub fn start(&self, filters: FilterList, duration: Option<Duration>, enforcers: Enforcers) -> Result<(), String> {
-        let mut block = self.block.lock().unwrap();
+    pub fn start(&self, lists: Lists, duration: Option<Duration>, enforcers: Enforcers) -> Result<(), String> {
+        let mut block = self.block.locked();
         if block.as_ref().is_some_and(|b| b.until.is_none_or(|u| self.clock.now() < u)) {
             return Err("A block is already running".into());
         }
         let until = duration.map(|d| self.clock.now() + d);
-        let new = Block { filters: Arc::new(filters), until, enforcers, saved_at: Instant::now() };
+        let new = Block {
+            filters: Arc::new(lists.sites),
+            allowed: Arc::new(lists.allowed),
+            programs: Arc::new(lists.programs),
+            until,
+            enforcers,
+            saved_at: Instant::now(),
+        };
         // Written before the block goes live, so a block cannot be missed after a restart. A
         // block that cannot be written would not survive the app ending, so it does not start.
         self.save(&new).map_err(|e| format!("Cannot save the block, so it would not survive a restart: {e}"))?;
@@ -76,7 +140,7 @@
 
     /// Stops the block unless it is timed and still running; then returns the seconds left.
     pub fn stop(&self) -> Result<(), u64> {
-        let mut block = self.block.lock().unwrap();
+        let mut block = self.block.locked();
         if let Some(left) = block.as_ref().and_then(|b| remaining(b, self.clock.now())) {
             return Err(left);
         }
@@ -91,7 +155,9 @@
         let Some(store) = &self.store else { return };
         let Some(saved) = store.load() else { return };
         // A block list we cannot read is dropped rather than resumed as something unknown.
-        let Ok(filters) = FilterList::parse(&saved.filters) else {
+        let (Ok(filters), Ok(allowed), Ok(programs)) =
+            (FilterList::parse(&saved.filters), FilterList::parse(&saved.allowed), ProgramList::parse(&saved.programs))
+        else {
             eprintln!("block: dropping an unreadable saved block");
             store.clear();
             return;
@@ -107,17 +173,24 @@
             return;
         }
         let enforcers = saved.enforcers;
-        *self.block.lock().unwrap() = Some(Block { filters: Arc::new(filters), until, enforcers, saved_at: Instant::now() });
+        *self.block.locked() = Some(Block {
+            filters: Arc::new(filters),
+            allowed: Arc::new(allowed),
+            programs: Arc::new(programs),
+            until,
+            enforcers,
+            saved_at: Instant::now(),
+        });
     }
 
-    /// The active filters, ending a timed block whose time is up.
+    /// The active filters, ending a timed block whose time is up. `Some` while any block runs.
     pub fn filters(&self) -> Option<Arc<FilterList>> {
-        self.active().map(|(filters, _)| filters)
+        self.active().map(|active| active.filters)
     }
 
-    /// The active filters and enforcers, ending a timed block whose time is up.
-    pub fn active(&self) -> Option<(Arc<FilterList>, Enforcers)> {
-        let mut block = self.block.lock().unwrap();
+    /// What is to be enforced, ending a timed block whose time is up.
+    pub fn active(&self) -> Option<Active> {
+        let mut block = self.block.locked();
         if block.as_ref().is_some_and(|b| b.until.is_some_and(|u| self.clock.now() >= u)) {
             // A timed block ends by itself and leaves nothing behind to resume.
             *block = None;
@@ -130,16 +203,23 @@
                 eprintln!("block: {e}");
             }
         }
-        block.as_ref().map(|b| (b.filters.clone(), b.enforcers))
+        block.as_ref().map(|b| Active {
+            filters: b.filters.clone(),
+            allowed: b.allowed.clone(),
+            programs: b.programs.clone(),
+            enforcers: b.enforcers,
+        })
     }
 
     pub fn status(&self) -> Status {
         let blocking = self.filters().is_some();
-        let block = self.block.lock().unwrap();
+        let block = self.block.locked();
         Status {
             blocking,
             remaining: block.as_ref().and_then(|b| remaining(b, self.clock.now())),
             filters: block.as_ref().map(|b| b.filters.entries()).unwrap_or_default(),
+            allowed: block.as_ref().map(|b| b.allowed.entries()).unwrap_or_default(),
+            programs: block.as_ref().map(|b| b.programs.entries()).unwrap_or_default(),
             enforcers: block.as_ref().map(|b| b.enforcers),
             clock_synced: self.clock.synced(),
         }
@@ -152,9 +232,18 @@
             until: block.until.and_then(state::to_unix),
             seen: state::to_unix(self.clock.now()),
             enforcers: block.enforcers,
+            programs: block.programs.entries(),
+            allowed: block.allowed.entries(),
         })
     }
 
+    /// Ends the block whatever its time, and forgets it. Only for the unlock password
+    /// (`service::unlock`): everything else goes through `stop`, which keeps a timed block.
+    pub fn reset(&self) {
+        *self.block.locked() = None;
+        self.clear_saved();
+    }
+
     /// Removes the saved block: there is no longer one to resume.
     pub fn clear_saved(&self) {
         if let Some(store) = &self.store {
@@ -219,6 +308,11 @@
         blocklist_of(name, false)
     }
 
+    /// Only sites to block; the program list is empty.
+    fn sites(entries: &[&str]) -> Lists {
+        Lists { sites: FilterList::parse(entries).unwrap(), allowed: FilterList::default(), programs: ProgramList::default() }
+    }
+
     fn entries(blocklist: &Blocklist) -> Vec<String> {
         blocklist.status().filters
     }
@@ -238,14 +332,14 @@
     #[test]
     fn timed_block_cannot_stop_early_and_ends_by_itself() {
         let list = blocklist("timed");
-        list.start(FilterList::parse(&["a.com"]).unwrap(), Some(Duration::from_millis(50)), Enforcers::default()).unwrap();
+        list.start(sites(&["a.com"]), Some(Duration::from_millis(50)), Enforcers::default()).unwrap();
         assert_eq!(list.stop(), Err(1));
         assert!(list.filters().is_some());
         std::thread::sleep(Duration::from_millis(60));
         assert!(list.filters().is_none());
         assert!(list.stop().is_ok());
 
-        list.start(FilterList::parse(&["a.com"]).unwrap(), None, Enforcers::default()).unwrap();
+        list.start(sites(&["a.com"]), None, Enforcers::default()).unwrap();
         assert!(list.stop().is_ok());
         assert!(list.filters().is_none());
     }
@@ -253,18 +347,18 @@
     #[test]
     fn a_running_block_cannot_be_replaced() {
         let list = blocklist("replace");
-        list.start(FilterList::parse(&["a.com"]).unwrap(), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
+        list.start(sites(&["a.com"]), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
         // Not for a shorter one, nor for an untimed one that could then be stopped.
-        assert!(list.start(FilterList::parse(&["a.com"]).unwrap(), Some(Duration::from_secs(1)), Enforcers::default()).is_err());
-        assert!(list.start(FilterList::parse(&["a.com"]).unwrap(), None, Enforcers::default()).is_err());
+        assert!(list.start(sites(&["a.com"]), Some(Duration::from_secs(1)), Enforcers::default()).is_err());
+        assert!(list.start(sites(&["a.com"]), None, Enforcers::default()).is_err());
         assert!(list.status().remaining.is_some_and(|left| left > 3500));
 
         // An untimed block is stopped first, then a new one can start.
         let untimed = blocklist("replace-untimed");
-        untimed.start(FilterList::parse(&["a.com"]).unwrap(), None, Enforcers::default()).unwrap();
-        assert!(untimed.start(FilterList::parse(&["b.com"]).unwrap(), None, Enforcers::default()).is_err());
+        untimed.start(sites(&["a.com"]), None, Enforcers::default()).unwrap();
+        assert!(untimed.start(sites(&["b.com"]), None, Enforcers::default()).is_err());
         untimed.stop().unwrap();
-        untimed.start(FilterList::parse(&["b.com"]).unwrap(), None, Enforcers::default()).unwrap();
+        untimed.start(sites(&["b.com"]), None, Enforcers::default()).unwrap();
         assert_eq!(entries(&untimed), vec!["b.com"]);
     }
 
@@ -276,14 +370,14 @@
         std::fs::create_dir_all(&dir).unwrap();
         std::fs::write(dir.join("not-a-folder"), "").unwrap();
         let list = Blocklist { block: Arc::default(), clock: test_clock(), store: Some(Arc::new(Store::new(dir.join("not-a-folder").join("state.json")))) };
-        assert!(list.start(FilterList::parse(&["a.com"]).unwrap(), Some(Duration::from_secs(60)), Enforcers::default()).is_err());
+        assert!(list.start(sites(&["a.com"]), Some(Duration::from_secs(60)), Enforcers::default()).is_err());
         assert!(list.filters().is_none());
     }
 
     #[test]
     fn a_block_outlives_the_app_that_started_it() {
         let list = blocklist("resume");
-        list.start(FilterList::parse(&["https://tweakers.net/", "b.com"]).unwrap(), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
+        list.start(sites(&["https://tweakers.net/", "b.com"]), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
         assert_eq!(entries(&list), vec!["tweakers.net", "b.com"]);
 
         // A new run of the app over the same store: the block is still going.
@@ -295,7 +389,7 @@
 
         // An untimed block is picked up as well: it runs until it is stopped.
         let untimed = blocklist("resume-untimed");
-        untimed.start(FilterList::parse(&["b.com"]).unwrap(), None, Enforcers::default()).unwrap();
+        untimed.start(sites(&["b.com"]), None, Enforcers::default()).unwrap();
         let resumed = reopened("resume-untimed");
         resumed.restore();
         assert!(resumed.filters().is_some());
@@ -303,21 +397,137 @@
         assert!(resumed.stop().is_ok());
     }
 
+    /// A block that blocks `block` and always allows `allow`.
+    fn with_allowed(block: &[&str], allow: &[&str]) -> Active {
+        let list = Blocklist::with_store(Store::new(
+            std::env::temp_dir().join(format!("block-{}-allowed-{}", std::process::id(), block.join("_").len() + allow.len())).join("state.json"),
+        ));
+        let lists = Lists {
+            sites: FilterList::parse(block).unwrap(),
+            allowed: FilterList::parse(allow).unwrap(),
+            programs: ProgramList::default(),
+        };
+        // `Enforcers::default()` has an enforcer that needs no list, so an empty block list starts too.
+        list.start(lists, None, Enforcers::default()).unwrap();
+        list.active().unwrap()
+    }
+
+    #[test]
+    fn allowed_sites_win_over_the_block_list() {
+        // Everything is blocked, except what is allowed.
+        let active = with_allowed(&["*"], &["google.com", "*.wikipedia.org"]);
+        assert!(active.blocks_site("https://youtube.com/watch?v=1"));
+        assert!(active.blocks_site("https://tweakers.net/"));
+        assert!(!active.blocks_site("https://google.com/search?q=a"));
+        assert!(!active.blocks_site("https://www.google.com/"));
+        assert!(!active.blocks_site("https://nl.wikipedia.org/wiki/Rust"));
+        // An allowed entry is exactly as wide as it is written: not the sites around it.
+        assert!(active.blocks_site("https://accounts.google.com/"));
+        assert!(active.blocks_site("https://wikipedia.org/"));
+
+        // A specific block and an exception inside it: the exception wins.
+        let active = with_allowed(&["youtube.com"], &["youtube.com/watch*"]);
+        assert!(active.blocks_site("https://youtube.com/"));
+        assert!(active.blocks_site("https://youtube.com/shorts/abc"));
+        assert!(!active.blocks_site("https://youtube.com/watch?v=1"));
+
+        // Allowed everything is allowed everything, whatever the block list says.
+        let active = with_allowed(&["youtube.com", "*.tweakers.net"], &["*"]);
+        assert!(!active.blocks_site("https://youtube.com/") && !active.blocks_site("https://a.tweakers.net/"));
+    }
+
+    #[test]
+    fn any_combination_of_the_two_lists_is_possible() {
+        // Neither: nothing is blocked.
+        let active = with_allowed(&[], &[]);
+        assert!(!active.blocks_site("https://youtube.com/"));
+        // Allowed alone: there is nothing to make exceptions to.
+        let active = with_allowed(&[], &["google.com"]);
+        assert!(!active.blocks_site("https://youtube.com/") && !active.blocks_site("https://google.com/"));
+        // Blocked alone.
+        let active = with_allowed(&["youtube.com"], &[]);
+        assert!(active.blocks_site("https://youtube.com/") && !active.blocks_site("https://google.com/"));
+        // The browser's own pages are neither blocked nor in need of an exception.
+        let active = with_allowed(&["*"], &[]);
+        assert!(!active.blocks_site("chrome://newtab/") && !active.blocks_site("about:blank"));
+    }
+
+    #[test]
+    fn the_allowed_sites_survive_a_restart_and_older_blocks_have_none() {
+        let list = blocklist("allowed-resume");
+        let lists = Lists {
+            sites: FilterList::parse(&["*"]).unwrap(),
+            allowed: FilterList::parse(&["Google.com", "*.wikipedia.org"]).unwrap(),
+            programs: ProgramList::default(),
+        };
+        list.start(lists, Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
+        assert_eq!(list.status().allowed, vec!["google.com", "*.wikipedia.org"]);
+
+        let resumed = reopened("allowed-resume");
+        resumed.restore();
+        let active = resumed.active().unwrap();
+        assert!(active.blocks_site("https://youtube.com/") && !active.blocks_site("https://google.com/"));
+        assert_eq!(resumed.status().allowed, vec!["google.com", "*.wikipedia.org"]);
+
+        // A saved block from before the allowed list resumes with none.
+        let older = blocklist("allowed-older");
+        let store = older.store.clone().unwrap();
+        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
+        let until = state::now().unwrap().as_secs() + 3600;
+        std::fs::write(store.path(), format!(r#"{{"filters":["a.com"],"until":{until}}}"#)).unwrap();
+        let resumed = reopened("allowed-older");
+        resumed.restore();
+        assert!(resumed.status().allowed.is_empty() && resumed.active().unwrap().blocks_site("https://a.com/"));
+    }
+
+    #[test]
+    fn a_resumed_block_keeps_its_programs() {
+        let list = blocklist("programs");
+        let lists = Lists {
+            sites: FilterList::default(),
+            allowed: FilterList::default(),
+            programs: ProgramList::parse(&["Steam*", "game"]).unwrap(),
+        };
+        list.start(lists, Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
+        assert_eq!(list.status().programs, vec!["steam*.exe", "game.exe"]);
+
+        // A new run over the same store: the programs are still on the block.
+        let resumed = reopened("programs");
+        resumed.restore();
+        let active = resumed.active().unwrap();
+        assert!(active.programs.is_blocked("steamwebhelper.exe") && active.programs.is_blocked("GAME.EXE"));
+        assert!(!active.programs.is_blocked("chrome.exe"));
+        assert_eq!(resumed.status().programs, vec!["steam*.exe", "game.exe"]);
+    }
+
+    #[test]
+    fn a_block_from_before_programs_still_resumes_without_any() {
+        let list = blocklist("before-programs");
+        let store = list.store.clone().unwrap();
+        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
+        let until = state::now().unwrap().as_secs() + 3600;
+        std::fs::write(store.path(), format!(r#"{{"filters":["a.com"],"until":{until}}}"#)).unwrap();
+        let resumed = reopened("before-programs");
+        resumed.restore();
+        let active = resumed.active().unwrap();
+        assert!(active.filters.is_blocked("a.com") && active.programs.is_empty());
+    }
+
     #[test]
     fn a_resumed_block_keeps_its_enforcers() {
-        let only_obsidian = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true };
+        let only_obsidian = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true, exe_names: false };
         let list = blocklist("enforcers");
-        list.start(FilterList::parse::<&str>(&[]).unwrap(), Some(Duration::from_secs(3600)), only_obsidian).unwrap();
+        list.start(sites(&[]), Some(Duration::from_secs(3600)), only_obsidian).unwrap();
         let resumed = reopened("enforcers");
         resumed.restore();
-        assert_eq!(resumed.active().map(|(_, e)| e), Some(only_obsidian));
+        assert_eq!(resumed.active().map(|active| active.enforcers), Some(only_obsidian));
         assert_eq!(resumed.status().enforcers, Some(only_obsidian));
     }
 
     #[test]
     fn a_block_whose_time_is_up_is_not_resumed() {
         let list = blocklist("expired");
-        list.start(FilterList::parse(&["a.com"]).unwrap(), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
+        list.start(sites(&["a.com"]), Some(Duration::from_secs(3600)), Enforcers::default()).unwrap();
         // Rewritten as though it had been left behind by a run that ended an hour ago.
         let store = list.store.clone().unwrap();
         store.save(&Saved {
@@ -325,6 +535,8 @@
             until: Some(state::now().unwrap().as_secs() - 3600),
             seen: None,
             enforcers: Enforcers::default(),
+            programs: Vec::new(),
+            allowed: Vec::new(),
         }).unwrap();
 
         let resumed = reopened("expired");
@@ -339,13 +551,13 @@
     fn ending_a_block_clears_it() {
         let list = blocklist("cleared");
         let store = list.store.clone().unwrap();
-        list.start(FilterList::parse(&["a.com"]).unwrap(), None, Enforcers::default()).unwrap();
+        list.start(sites(&["a.com"]), None, Enforcers::default()).unwrap();
         assert!(store.load().is_some());
         list.stop().unwrap();
         assert_eq!(store.load(), None);
 
         // A timed block that ends by itself is cleared too, so nothing is left to resume.
-        list.start(FilterList::parse(&["a.com"]).unwrap(), Some(Duration::from_millis(50)), Enforcers::default()).unwrap();
+        list.start(sites(&["a.com"]), Some(Duration::from_millis(50)), Enforcers::default()).unwrap();
         assert!(store.load().is_some());
         std::thread::sleep(Duration::from_millis(60));
         assert!(list.filters().is_none());
@@ -359,7 +571,7 @@
         let path = dir.join("state.json");
 
         let list = Blocklist { block: Arc::default(), clock: test_clock(), store: None };
-        list.start(FilterList::parse(&["a.com"]).unwrap(), Some(Duration::from_secs(60)), Enforcers::default()).unwrap();
+        list.start(sites(&["a.com"]), Some(Duration::from_secs(60)), Enforcers::default()).unwrap();
         assert!(list.filters().is_some());
         list.restore();
         assert!(list.filters().is_some());
```

### 5.2 `state.rs`

`Enforcers` also gets `can_run(...)`: a block may start when at least one enforcer that is turned on can do something. Each enforcer says what it needs (`Uses::Nothing`, `Sites` or `Programs`), and it can run when the list it needs has an entry. So an empty site list is fine as long as e.g. the Obsidian enforcer is on, and refused when only enforcers whose list is empty are on. **To add an enforcer, see the checklist in section 6.4.**

`dir()` is now the service's folder (`%ProgramData%\bullseye`). The app's own log goes in `user_dir()`. `use_dir` lets development (the in-process service, section 8.1) and the pipe test use another folder, so you do not need admin rights to develop.

```diff
--- a/src-tauri/src/state.rs
+++ b/src-tauri/src/state.rs
@@ -9,6 +9,7 @@
 
 use std::fs;
 use std::path::PathBuf;
+use std::sync::OnceLock;
 use std::time::{Duration, SystemTime, UNIX_EPOCH};
 
 use serde::{Deserialize, Serialize};
@@ -27,6 +28,14 @@
     /// Which enforcers the block runs. Missing in files from older versions, which ran all.
     #[serde(default)]
     pub enforcers: Enforcers,
+    /// The programs to close, one entry per line, as typed in the UI. Missing in files from older
+    /// versions, which had none.
+    #[serde(default)]
+    pub programs: Vec<String>,
+    /// The sites that are always allowed, one entry per line, as typed in the UI. They win over
+    /// `filters`. Missing in files from older versions, which had none.
+    #[serde(default)]
+    pub allowed: Vec<String>,
 }
 
 /// The ways a block is enforced, chosen when it starts and fixed while it runs, so turning one
@@ -40,22 +49,55 @@
     pub chromium_split_view: bool,
     /// Closes Obsidian while it shows web content; not linked to the block list.
     pub obsidian_web_viewer: bool,
+    /// Closes the programs on the program list (`programs.rs`).
+    pub exe_names: bool,
+}
+
+/// What an enforcer needs to have something to do.
+#[derive(Clone, Copy, Debug, PartialEq)]
+pub enum Uses {
+    /// No list: it does its work on its own (Obsidian's).
+    Nothing,
+    /// The block list of sites (`filters.rs`).
+    Sites,
+    /// The list of programs (`programs.rs`).
+    Programs,
 }
 
 impl Enforcers {
-    /// Whether any enforcer goes by the block list, which then needs at least one entry.
-    pub fn use_block_list(&self) -> bool {
-        self.chromium_tabs || self.chromium_split_view
+    /// Every enforcer as (turned on, what it needs). **The one place to list a new enforcer**: add
+    /// its field above and a line here; what may start and what runs follow from it. (A new kind of
+    /// list also needs a `Uses` variant and an arm in `block::Lists::has`.)
+    fn all(&self) -> Vec<(bool, Uses)> {
+        vec![
+            (self.chromium_tabs, Uses::Sites),
+            (self.chromium_split_view, Uses::Sites),
+            (self.obsidian_web_viewer, Uses::Nothing),
+            (self.exe_names, Uses::Programs),
+        ]
     }
 
+    /// Whether any enforcer is turned on.
     pub fn any(&self) -> bool {
-        self.use_block_list() || self.obsidian_web_viewer
+        self.all().iter().any(|&(on, _)| on)
+    }
+
+    /// Whether any enforcer that is turned on needs this kind of list.
+    pub fn uses(&self, list: Uses) -> bool {
+        self.all().iter().any(|&(on, needs)| on && needs == list)
+    }
+
+    /// Whether a block with these enforcers has anything to do, given which lists have entries
+    /// (`has_entries`): an enforcer that needs a list has nothing to do with an empty one, the
+    /// others do not need one. A block that cannot do anything is not worth starting.
+    pub fn can_run(&self, has_entries: impl Fn(Uses) -> bool) -> bool {
+        self.all().iter().any(|&(on, needs)| on && has_entries(needs))
     }
 }
 
 impl Default for Enforcers {
     fn default() -> Enforcers {
-        Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: true }
+        Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: true, exe_names: true }
     }
 }
 
@@ -129,15 +171,33 @@
     }
 }
 
-/// The folder the app keeps its files in: `%LOCALAPPDATA%\bullseye`.
+/// A folder to use instead of the real one: development and the pipe test (`service.rs`).
+static DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
+
+#[cfg(any(debug_assertions, test))]
+pub fn use_dir(dir: PathBuf) {
+    let _ = DIR_OVERRIDE.set(dir);
+}
+
+/// The folder the service keeps its files in: `%ProgramData%\bullseye`, which the service
+/// locks down to SYSTEM (`service::lock_down_data_dir`). The saved block lives here, so the user
+/// whose block it is cannot edit or delete it.
 ///
-/// `None` only if there is no local app-data folder for this user, in which case the app still
-/// blocks for as long as it runs, it just does not survive a restart.
+/// `None` only if there is no such folder, in which case the service still blocks for as long as
+/// it runs, it just does not survive a restart.
 pub fn dir() -> Option<PathBuf> {
+    if let Some(dir) = DIR_OVERRIDE.get() {
+        return Some(dir.clone());
+    }
+    Some(PathBuf::from(std::env::var_os("ProgramData")?).join("bullseye"))
+}
+
+/// The app's own folder, for its log: `%LOCALAPPDATA%\bullseye`. Nothing in it matters to a block.
+pub fn user_dir() -> Option<PathBuf> {
     Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("bullseye"))
 }
 
-/// The store the app uses: `%LOCALAPPDATA%\bullseye\state.json`.
+/// The store the service uses: `%ProgramData%\bullseye\state.json`.
 pub fn store() -> Option<Store> {
     Some(Store::new(dir()?.join("state.json")))
 }
@@ -154,7 +214,7 @@
     }
 
     fn timed(until: Option<u64>) -> Saved {
-        Saved { filters: vec!["tweakers.net".to_string()], until, seen: None, enforcers: Enforcers::default() }
+        Saved { filters: vec!["tweakers.net".to_string()], until, seen: None, enforcers: Enforcers::default(), programs: Vec::new(), allowed: Vec::new() }
     }
 
     #[test]
@@ -214,11 +274,46 @@
         let store = store("old");
         fs::create_dir_all(store.path().parent().unwrap()).unwrap();
         fs::write(store.path(), r#"{"filters":["a.com"],"until":1800000000}"#).unwrap();
-        assert_eq!(store.load(), Some(Saved { filters: vec!["a.com".into()], until: Some(1_800_000_000), seen: None, enforcers: Enforcers::default() }));
+        assert_eq!(store.load(), Some(Saved { filters: vec!["a.com".into()], until: Some(1_800_000_000), seen: None, enforcers: Enforcers::default(), programs: Vec::new(), allowed: Vec::new() }));
         store.clear();
     }
 
     #[test]
+    fn what_can_run_depends_on_the_enforcers_and_the_lists() {
+        let only = |tabs, split, obsidian, exe| Enforcers {
+            chromium_tabs: tabs,
+            chromium_split_view: split,
+            obsidian_web_viewer: obsidian,
+            exe_names: exe,
+        };
+        // Which lists have entries.
+        let lists = |sites: bool, programs: bool| move |uses: Uses| match uses {
+            Uses::Nothing => true,
+            Uses::Sites => sites,
+            Uses::Programs => programs,
+        };
+        // With entries, any enforcer that is on can run.
+        assert!(only(true, false, false, false).can_run(lists(true, false)));
+        assert!(only(false, true, false, false).can_run(lists(true, false)));
+        assert!(only(false, false, true, false).can_run(lists(false, false)));
+        assert!(only(false, false, false, true).can_run(lists(false, true)));
+        // An enforcer needs its own list: sites do not help the programs one, nor the other way round.
+        assert!(!only(false, false, false, true).can_run(lists(true, false)));
+        assert!(!only(true, false, false, false).can_run(lists(false, true)));
+        // Without any entries, only one that needs no list can run, also next to ones that do.
+        assert!(only(true, true, true, true).can_run(lists(false, false)));
+        assert!(Enforcers::default().can_run(lists(false, false)));
+        assert!(!only(true, true, false, true).can_run(lists(false, false)));
+        // And with nothing turned on, nothing can.
+        assert!(!only(false, false, false, false).can_run(lists(true, true)));
+        // The checks the closing code and the start use agree with it.
+        let programs_only = only(false, false, false, true);
+        assert!(programs_only.uses(Uses::Programs) && !programs_only.uses(Uses::Sites) && programs_only.any());
+        assert!(only(true, false, false, false).uses(Uses::Sites));
+        assert!(!only(false, false, false, false).any());
+    }
+
+    #[test]
     fn wall_clock_round_trips() {
         let time = from_unix(1_700_000_000);
         assert_eq!(to_unix(time), Some(1_700_000_000));
```

### 5.3 `tab_closer.rs`

What changed here, with measurements (release build, 341 running programs, real browser windows):

- **The sweep was 20 to 40 times too expensive, in one place.** Of a 93 ms pass, **92 ms was a single UI Automation call that searched the desktop's top-level windows for browser windows**. They are now found with a plain `EnumWindows` (0.3 ms), and UI Automation is only used on the windows found (`browser_windows`, `Closer::browser`), with the cache keyed by window handle. A pass with real browser windows takes 4 to 10 ms, where the old one took 166 to 413 ms. Creating the UI Automation condition objects costs nothing (0.001 ms), so they are **not** cached: measure before optimizing.
- **One process scan per pass** (`scan_processes`, about 6 ms) shared by the Obsidian and programs checks, instead of one each.
- **Edge calls its tabs `EdgeTab`, not `Tab`.** `close_tab` only looked for class `Tab`, so with Edge the page was found and judged blocked but "no selected tab" was found and nothing was ever closed. Both names are accepted now (`TAB_CLASSES`). The class is what tells a browser tab from a tab-like widget inside a web page, so it is **not** dropped. A manual check against a real browser is the ignored test `a_real_browser_tab_is_closed`.
- A missing UI Automation now ends the app (and the service starts it again) instead of leaving the closing thread quietly dead.

```diff
--- a/src-tauri/src/tab_closer.rs
+++ b/src-tauri/src/tab_closer.rs
@@ -15,35 +15,59 @@
 use std::thread;
 use std::time::{Duration, Instant};
 
-use crate::block::Blocklist;
-use crate::filters::FilterList;
-use crate::state::Enforcers;
+use crate::client::Remote;
+use crate::block::Active;
+use crate::programs::ProgramList;
+use crate::state::{Enforcers, Uses};
+use crate::sync::spawn_critical;
+use crate::watchdog::note;
 use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
 use uiautomation::patterns::{UIInvokePattern, UIValuePattern, UIWindowPattern};
-use uiautomation::types::{ControlType, TreeScope, UIProperty};
+use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
 use uiautomation::{UIAutomation, UIElement};
+use windows_sys::Win32::Foundation::{HWND, LPARAM};
+use windows_sys::Win32::UI::WindowsAndMessaging::{
+    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
+};
 
 const INTERVAL: Duration = Duration::from_secs(1);
 const RECHECK: Duration = Duration::from_secs(10);
 
-pub fn spawn(blocklist: Blocklist) {
-    thread::spawn(move || {
+/// How long a program is given to close when asked to, before it is ended.
+const KILL_AFTER: Duration = Duration::from_secs(3);
+
+pub fn spawn(remote: Remote) {
+    spawn_critical("tab closer", move || {
         let automation = match UIAutomation::new() {
             Ok(a) => a,
-            Err(e) => return eprintln!("tab_closer: UI Automation unavailable: {e}"),
+            // Without it nothing is enforced, so say so loudly: the process ends and is started again.
+            Err(e) => {
+                note("app", &format!("UI Automation is unavailable: {e}"));
+                std::process::exit(1);
+            }
         };
-        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new() };
+        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new(), asked: HashMap::new() };
         loop {
             let start = Instant::now();
-            if let Some((filters, enforcers)) = blocklist.active() {
-                if enforcers.use_block_list() {
-                    if let Err(e) = closer.tick(&filters, enforcers) {
+            if let Some(active) = remote.active() {
+                let enforcers = active.enforcers;
+                // With no entries there is nothing for these to match, so skip the UI Automation work.
+                if enforcers.uses(Uses::Sites) && !active.filters.is_empty() {
+                    if let Err(e) = closer.tick(&active, enforcers) {
                         eprintln!("tab_closer: {e}");
                     }
                 }
+                // Obsidian and the programs both look at the running programs: one scan serves both.
+                let programs = enforcers.uses(Uses::Programs) && !active.programs.is_empty();
+                if enforcers.obsidian_web_viewer || programs {
+                    closer.scan_processes(enforcers.obsidian_web_viewer);
+                }
                 if enforcers.obsidian_web_viewer {
                     closer.close_obsidian_with_web_content();
                 }
+                if programs {
+                    closer.close_programs(&active.programs);
+                }
             }
             thread::sleep(INTERVAL.saturating_sub(start.elapsed()));
         }
@@ -52,9 +76,11 @@
 
 struct Closer {
     automation: UIAutomation,
-    /// Window runtime id -> its cached UI elements, so they are not searched for every second.
-    windows: HashMap<Vec<i32>, Window>,
+    /// Window handle -> its cached UI elements, so they are not searched for every second.
+    windows: HashMap<isize, Window>,
     system: System,
+    /// Programs that were asked to close, by process id and when, so the ones that do not are ended.
+    asked: HashMap<u32, Instant>,
 }
 
 enum Window {
@@ -65,6 +91,8 @@
 
 #[derive(Clone)]
 struct Browser {
+    /// The browser window itself.
+    window: UIElement,
     omnibox: UIElement,
     /// Parent of the visible pages (`MultiContentsView`); holds two pages in split view.
     contents: Option<UIElement>,
@@ -77,20 +105,17 @@
 }
 
 impl Closer {
-    fn tick(&mut self, filters: &FilterList, enforcers: Enforcers) -> uiautomation::Result<()> {
-        // Window class shared by all Chromium-based browsers (and Electron apps, which have
-        // no address bar and are skipped in `browser`).
-        let cond = self.automation.create_property_condition(UIProperty::ClassName, "Chrome_WidgetWin_1".into(), None)?;
-        let windows = self.automation.get_root_element()?.find_all(TreeScope::Children, &cond)?;
-
-        let mut seen = Vec::new();
-        for window in windows {
-            let Ok(id) = window.get_runtime_id() else { continue };
-            seen.push(id.clone());
-            let Some(browser) = self.browser(&window, &id) else { continue };
+    fn tick(&mut self, active: &Active, enforcers: Enforcers) -> uiautomation::Result<()> {
+        // The browser windows are found by a plain window enumeration, which takes a fraction of a
+        // millisecond. Asking UI Automation to search the desktop for them took about 90.
+        let handles = browser_windows();
+        let mut seen = Vec::with_capacity(handles.len());
+        for handle in handles {
+            seen.push(handle);
+            let Some(browser) = self.browser(handle) else { continue };
             let Ok(pages) = self.pages(&browser) else {
                 // Cached elements went stale; look them up again next tick.
-                self.windows.remove(&id);
+                self.windows.remove(&handle);
                 continue;
             };
             // Two pages at once is a split view; one is an ordinary tab.
@@ -98,26 +123,32 @@
             if !enforced {
                 continue;
             }
-            if let Some(page) = pages.iter().find(|p| filters.is_blocked(&p.url)) {
-                match close_tab(&self.automation, &window, &page.title) {
+            if let Some(page) = pages.iter().find(|p| active.blocks_site(&p.url)) {
+                match close_tab(&self.automation, &browser.window, &page.title) {
                     Ok(()) => eprintln!("tab_closer: closed {}", page.url),
                     Err(e) => eprintln!("tab_closer: failed to close {}: {e}", page.url),
                 }
             }
         }
-        self.windows.retain(|id, _| seen.contains(id));
+        self.windows.retain(|handle, _| seen.contains(handle));
         Ok(())
     }
 
+    /// Looks at the running programs, once, for the checks below (`close_obsidian_with_web_content`,
+    /// `close_programs`). Command lines are only fetched when Obsidian's check needs them.
+    fn scan_processes(&mut self, command_lines: bool) {
+        let kind = if command_lines {
+            ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet)
+        } else {
+            ProcessRefreshKind::nothing()
+        };
+        self.system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
+    }
+
     /// Closes Obsidian (like clicking its X) when it hosts web content. Obsidian's own UI runs in
     /// an unsandboxed renderer (it needs Node); web viewer pages run in sandboxed ones. Ending
     /// those renderers instead makes Obsidian quit on the next navigation, so close it cleanly.
     fn close_obsidian_with_web_content(&mut self) {
-        self.system.refresh_processes_specifics(
-            ProcessesToUpdate::All,
-            true,
-            ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet),
-        );
         let obsidian = |p: &&sysinfo::Process| p.name().eq_ignore_ascii_case("obsidian.exe");
         let has = |p: &sysinfo::Process, flag: &str| p.cmd().iter().any(|a| a.to_string_lossy() == flag);
         let processes = self.system.processes();
@@ -151,23 +182,58 @@
         }
     }
 
+    /// Closes the programs on the list: asks each to close like the X of its windows does, and ends
+    /// the ones still running `KILL_AFTER` later (or that have no window to ask). Programs Windows
+    /// needs are never touched (`ProgramList::is_blocked`), nor is this app.
+    fn close_programs(&mut self, programs: &ProgramList) {
+        let me = std::process::id();
+        let mut running = Vec::new();
+        for (pid, process) in self.system.processes() {
+            let pid = pid.as_u32();
+            if pid == me || !programs.is_blocked(&process.name().to_string_lossy()) {
+                continue;
+            }
+            running.push(pid);
+            match self.asked.get(&pid) {
+                None => {
+                    ask_to_close(pid);
+                    self.asked.insert(pid, Instant::now());
+                    eprintln!("tab_closer: asked {} ({pid}) to close", process.name().to_string_lossy());
+                }
+                Some(since) if since.elapsed() >= KILL_AFTER => {
+                    let ended = process.kill();
+                    eprintln!("tab_closer: {} {} ({pid})", if ended { "ended" } else { "could not end" }, process.name().to_string_lossy());
+                }
+                Some(_) => {}
+            }
+        }
+        // A program that has gone, whether it closed or was ended, is not remembered.
+        self.asked.retain(|pid, _| running.contains(pid));
+    }
+
     /// The window's cached browser elements, or None if it is not a browser window.
-    fn browser(&mut self, window: &UIElement, id: &Vec<i32>) -> Option<Browser> {
-        match self.windows.get(id) {
+    fn browser(&mut self, handle: isize) -> Option<Browser> {
+        match self.windows.get(&handle) {
             Some(Window::Browser(b)) => return Some(b.clone()),
             Some(Window::Other(since)) if since.elapsed() < RECHECK => return None,
             _ => {}
         }
+        let Ok(window) = self.automation.element_from_handle(Handle::from(handle)) else {
+            // Gone already, or not ours to look at.
+            self.windows.insert(handle, Window::Other(Instant::now()));
+            return None;
+        };
         let find = |class: &str| {
             let cond = self.automation.create_property_condition(UIProperty::ClassName, class.into(), None).ok()?;
             window.find_first(TreeScope::Descendants, &cond).ok()
         };
         let Some(omnibox) = find("OmniboxViewViews") else {
-            self.windows.insert(id.clone(), Window::Other(Instant::now()));
+            self.windows.insert(handle, Window::Other(Instant::now()));
             return None;
         };
-        let browser = Browser { omnibox, contents: find("MultiContentsView") };
-        self.windows.insert(id.clone(), Window::Browser(browser.clone()));
+        let contents = find("MultiContentsView");
+        let browser = Browser { window, omnibox, contents };
+        self.windows.insert(handle, Window::Browser(browser.clone()));
         Some(browser)
     }
 
@@ -195,12 +261,59 @@
     }
 }
 
+/// The visible top-level windows of the class all Chromium-based browsers share (Electron apps
+/// have it too; they have no address bar and are skipped in `Closer::browser`). Visible ones only,
+/// which is what UI Automation lists as the desktop's children too.
+fn browser_windows() -> Vec<isize> {
+    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
+        let mut name = [0u16; 32];
+        // SAFETY: the buffer is as long as the length says; `window` is one Windows just handed over.
+        let (length, visible) = unsafe { (GetClassNameW(window, name.as_mut_ptr(), 32), IsWindowVisible(window) != 0) };
+        if visible && length > 0 && String::from_utf16_lossy(&name[..length as usize]) == "Chrome_WidgetWin_1" {
+            // SAFETY: `found` is the vector below, which outlives the enumeration.
+            unsafe { (*(found as *mut Vec<isize>)).push(window as isize) };
+        }
+        1
+    }
+    let mut found: Vec<isize> = Vec::new();
+    // SAFETY: the callback is the function above, and `found` outlives the call.
+    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<isize> as LPARAM) };
+    found
+}
+
+/// Posts a close request to every top-level window of the process, as the X button does. The
+/// program may ask to save first, or not close at all; that is what `KILL_AFTER` is for.
+fn ask_to_close(pid: u32) {
+    unsafe extern "system" fn each(window: HWND, pid: LPARAM) -> i32 {
+        let mut owner = 0u32;
+        // SAFETY: `window` is one Windows just handed over; `owner` is a valid place to write to.
+        unsafe {
+            GetWindowThreadProcessId(window, &mut owner);
+            if owner == pid as u32 {
+                PostMessageW(window, WM_CLOSE, 0, 0);
+            }
+        }
+        1
+    }
+    // SAFETY: the callback is the function above, and takes no state beyond the process id.
+    unsafe { EnumWindows(Some(each), pid as LPARAM) };
+}
+
+/// Window classes of a browser's tab strip entries: Chrome and Brave call them `Tab`, Edge calls
+/// them `EdgeTab`. The class is what tells a browser tab from a tab-like widget inside a web page
+/// (an ARIA tablist), whose close button must never be pressed.
+const TAB_CLASSES: [&str; 2] = ["Tab", "EdgeTab"];
+
 /// Presses the Close button of the selected tab showing `title`. In split view both tabs are
 /// selected and named "<title> - Left view" / "<title> - Right view", so the title picks one;
 /// with a single selected tab (or no title) the first selected tab is closed.
 fn close_tab(a: &UIAutomation, window: &UIElement, title: &str) -> uiautomation::Result<()> {
+    let is_a_tab_class = a.create_or_condition(
+        a.create_property_condition(UIProperty::ClassName, TAB_CLASSES[0].into(), None)?,
+        a.create_property_condition(UIProperty::ClassName, TAB_CLASSES[1].into(), None)?,
+    )?;
     let cond = a.create_and_condition(
-        a.create_property_condition(UIProperty::ClassName, "Tab".into(), None)?,
+        is_a_tab_class,
         a.create_and_condition(
             a.create_property_condition(UIProperty::ControlType, (ControlType::TabItem as i32).into(), None)?,
             a.create_property_condition(UIProperty::SelectionItemIsSelected, true.into(), None)?,
@@ -215,3 +328,99 @@
     let button = a.create_property_condition(UIProperty::ControlType, (ControlType::Button as i32).into(), None)?;
     tab.find_first(TreeScope::Children, &button)?.get_pattern::<UIInvokePattern>()?.invoke()
 }
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use std::process::{Child, Command, Stdio};
+
+    /// A copy of a stock Windows program under a name of its own, so a test never touches a
+    /// program the person running it is using.
+    fn copy_of_ping(name: &str) -> Child {
+        let dir = std::env::temp_dir().join(format!("closer-{}", std::process::id()));
+        std::fs::create_dir_all(&dir).unwrap();
+        let path = dir.join(format!("{name}.exe"));
+        let ping = std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join("System32").join("ping.exe");
+        std::fs::copy(ping, &path).unwrap();
+        Command::new(path).args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap()
+    }
+
+    /// What the loop does for the programs: one scan, then the check.
+    fn pass(closer: &mut Closer, programs: &ProgramList) {
+        closer.scan_processes(false);
+        closer.close_programs(programs);
+    }
+
+    #[test]
+    fn a_listed_program_is_asked_to_close_then_ended_and_others_are_left_alone() {
+        let Ok(automation) = UIAutomation::new() else { return eprintln!("skipping: no UI Automation here") };
+        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new(), asked: HashMap::new() };
+        let (mut listed, mut other) = (copy_of_ping("bullseye-test-listed"), copy_of_ping("bullseye-test-other"));
+        let programs = ProgramList::parse(&["bullseye-test-l*"]).unwrap();
+
+        // The first pass asks. This program has no window to ask, so it carries on for now.
+        pass(&mut closer, &programs);
+        assert!(closer.asked.contains_key(&listed.id()));
+        assert!(!closer.asked.contains_key(&other.id()));
+        assert!(listed.try_wait().unwrap().is_none());
+
+        // Once it has had its time, the next pass ends it.
+        thread::sleep(KILL_AFTER + Duration::from_millis(500));
+        pass(&mut closer, &programs);
+        let ended = (0..50).any(|_| {
+            thread::sleep(Duration::from_millis(100));
+            listed.try_wait().unwrap().is_some()
+        });
+        assert!(ended, "the listed program should have been ended");
+
+        // A program that has gone is forgotten, and the one that was not listed never touched.
+        pass(&mut closer, &programs);
+        assert!(closer.asked.is_empty());
+        assert!(other.try_wait().unwrap().is_none());
+        let _ = other.kill();
+        let _ = other.wait();
+    }
+
+    /// A manual check against a real browser: have a Chromium browser window open on
+    /// `https://example.com` (`msedge --new-window https://example.com`), then
+    /// `cargo test -- --ignored a_real_browser_tab_is_closed --nocapture`. The tab must be closed by the
+    /// sweep, which also prints how long a pass takes. (Needs a window that shows that page itself: a
+    /// browser extension that replaces the page would hide it.)
+    #[test]
+    #[ignore]
+    fn a_real_browser_tab_is_closed() {
+        use crate::filters::FilterList;
+        let automation = UIAutomation::new().unwrap();
+        let mut closer = Closer { automation, windows: HashMap::new(), system: System::new(), asked: HashMap::new() };
+        let active = |entry: &str| crate::block::Active {
+            filters: std::sync::Arc::new(FilterList::parse(&[entry]).unwrap()),
+            allowed: std::sync::Arc::new(FilterList::default()),
+            programs: std::sync::Arc::new(ProgramList::default()),
+            enforcers: Enforcers::default(),
+        };
+        println!("chromium-class visible windows: {}", browser_windows().len());
+        let quiet = active("real-check-nothing.invalid");
+        let _ = closer.tick(&quiet, Enforcers::default());
+        let runs = 20;
+        let start = Instant::now();
+        for _ in 0..runs {
+            closer.tick(&quiet, Enforcers::default()).unwrap();
+        }
+        println!("tick, nothing to close: {:.2} ms per pass", start.elapsed().as_secs_f64() * 1000.0 / runs as f64);
+        let browsers = closer.windows.values().filter(|w| matches!(w, Window::Browser(_))).count();
+        println!("browser windows recognised: {browsers}");
+        assert!(browsers >= 1, "open a Chromium browser window first");
+
+        let block = active("example.com");
+        let before = browser_windows().len();
+        let start = Instant::now();
+        let closed = (0..60).any(|_| {
+            let _ = closer.tick(&block, Enforcers::default());
+            thread::sleep(Duration::from_millis(250));
+            browser_windows().len() < before
+        });
+        println!("blocked tab closed: {closed} (after {:?})", start.elapsed());
+        assert!(closed, "the tab on example.com should have been closed");
+    }
+
+}
```

### 5.4 `filters.rs`

The site lists (the block list, and the list of allowed sites, section 6.5) are matched here. Two fixes and one convenience:

- **A lone `*` blocks every website**, as intended: the host wildcard matches any host and the path may be anything. What it must **not** do is close the browser's own pages, so only **web pages** are matched now (`is_website`): `http`, `https`, `ftp`, `ws`, `wss`, and an address with no scheme at all (the address bar leaves `https://` off). `chrome://settings`, `edge://newtab`, `about:blank`, `view-source:`, extensions, `file://` and an empty address are never matched, by either list.
- **Scheme stripping was wrong for addresses without a scheme.** It cut at the first `://` anywhere, so `google.com/url?q=https://tweakers.net/nieuws` (a Google redirect page as the address bar shows it) was read as `tweakers.net/nieuws` and blocked by a `tweakers.net` entry. Only text that looks like a scheme (letters, digits, `+`, `-`, `.`, no `/` or `?`) before the `://` is cut off now (`split_scheme`). This only affected the address-bar fallback.
- `FilterList` has a `Default` (an empty list), for a block without allowed sites.

```diff
--- a/src-tauri/src/filters.rs
+++ b/src-tauri/src/filters.rs
@@ -1,5 +1,9 @@
-//! Blocked-site list: one address per line, `*` as wildcard.
+//! A list of sites: one address per line, `*` as wildcard. Used twice: for the sites to block, and
+//! for the sites that are always allowed, which win over the block list (`block::Active::blocks_site`).
 //!
+//! - Only web pages count. The browser's own pages (`chrome://settings`, `edge://newtab`,
+//!   `about:blank`), extensions and local files are never matched, so even a lone `*`, which
+//!   matches every site, leaves the browser itself usable.
 //! - `https://`, `www.` and letter case are ignored.
 //! - An entry blocks everything that continues it at a break (`/ ? # & :`):
 //!   `tweakers.net` blocks `tweakers.net/nieuws?x=1`, not `tweakers.network`.
@@ -14,6 +18,12 @@
     matchers: RegexSet,
 }
 
+impl Default for FilterList {
+    fn default() -> FilterList {
+        FilterList { entries: Vec::new(), matchers: RegexSet::empty() }
+    }
+}
+
 impl FilterList {
     /// Parses one entry per line, ignoring empty lines. Errors list the invalid lines.
     pub fn parse<S: AsRef<str>>(lines: &[S]) -> Result<FilterList, Vec<String>> {
@@ -37,16 +47,45 @@
         self.entries.clone()
     }
 
+    /// Whether the address is a web page that an entry of this list matches.
     pub fn is_blocked(&self, url: &str) -> bool {
-        self.matchers.is_match(&normalize(url))
+        is_website(url) && self.matchers.is_match(&normalize(url))
     }
 }
 
 /// Lowercases and strips scheme, `www.` and trailing slashes.
 fn normalize(url: &str) -> String {
     let url = url.trim().to_lowercase();
-    let url = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
-    url.strip_prefix("www.").unwrap_or(url).trim_end_matches('/').to_string()
+    let rest = split_scheme(&url).1;
+    rest.strip_prefix("www.").unwrap_or(rest).trim_end_matches('/').to_string()
+}
+
+/// The scheme of an address (`https`) and the rest of it, if it starts with a scheme. Only text that
+/// looks like one counts: an address with none (the address bar leaves it off) can still have a
+/// `://` further on, in its query, and must not be cut there.
+fn split_scheme(url: &str) -> (Option<&str>, &str) {
+    match url.split_once("://") {
+        Some((scheme, rest))
+            if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
+        {
+            (Some(scheme), rest)
+        }
+        _ => (None, url),
+    }
+}
+
+/// Whether the address is a page on the web, which is all these lists are about. Not the browser's
+/// own pages, extensions, local files or an empty address. An address without a scheme counts: the
+/// address bar leaves it off.
+fn is_website(url: &str) -> bool {
+    let url = url.trim().to_lowercase();
+    if url.is_empty() {
+        return false;
+    }
+    match split_scheme(&url) {
+        (Some(scheme), _) => matches!(scheme, "http" | "https" | "ftp" | "ws" | "wss"),
+        (None, rest) => !["about:", "data:", "javascript:", "blob:", "view-source:"].iter().any(|p| rest.starts_with(p)),
+    }
 }
 
 fn to_regex(entry: &str) -> String {
@@ -96,6 +135,52 @@
     }
 
     #[test]
+    fn a_lone_star_blocks_every_site_but_not_the_browser_itself() {
+        let everything = FilterList::parse(&["*"]).unwrap();
+        for url in [
+            "https://youtube.com/watch?v=1",
+            "http://example.org/",
+            "https://sub.domain.example.co.uk/a/b?c=d#e",
+            "https://www.tweakers.net/",
+            "localhost:3000/app",
+            "youtube.com/watch?v=1",
+            "example.com",
+            "ftp://files.example.com/pub",
+        ] {
+            assert!(everything.is_blocked(url), "{url}");
+        }
+        // The browser's own pages, extensions, local files and an empty address are not sites.
+        for url in [
+            "chrome://settings",
+            "chrome://newtab/",
+            "edge://newtab/",
+            "brave://settings/",
+            "about:blank",
+            "chrome-extension://abcdef/popup.html",
+            "devtools://devtools/bundled/inspector.html",
+            "view-source:https://example.com/",
+            "file:///C:/Users/me/page.html",
+            "data:text/html,hello",
+            "",
+            "   ",
+        ] {
+            assert!(!everything.is_blocked(url), "{url:?}");
+        }
+    }
+
+    #[test]
+    fn only_a_real_scheme_is_cut_off() {
+        // An address without a scheme can have a "://" in its query: that is not its scheme.
+        assert!(!blocks("tweakers.net", "google.com/url?q=https://tweakers.net/nieuws"));
+        assert!(blocks("google.com", "google.com/url?q=https://tweakers.net/nieuws"));
+        assert!(!blocks("tweakers.net", "https://google.com/url?q=https://tweakers.net/nieuws"));
+        // Real schemes still go, whatever they are written like.
+        assert!(blocks("tweakers.net", "HTTPS://tweakers.net/"));
+        assert!(blocks("tweakers.net", "http://tweakers.net/x"));
+        assert!(blocks("tweakers.net", "wss://tweakers.net/socket"));
+    }
+
+    #[test]
     fn invalid_and_empty() {
         assert_eq!(FilterList::parse(&["tweakers .net"]).err(), Some(vec!["tweakers .net".to_string()]));
         assert!(FilterList::parse(&["", "  "]).unwrap().is_empty());
```

## 6. The service, and the files that change around it

### 6.1 The service: `service.rs`, `handler.rs` and `registration.rs`

The service is three files, each with one job. Read the top-of-file comment of each first.

**`service.rs`: the running service** (talking to the service manager, starting everything, the data folder, and the in-process mode for development).

- **`controls_accepted` does not include STOP.** A service that does not accept stop cannot be stopped with `sc stop` or from the services list, by anyone, including administrators. It only accepts `SESSION_CHANGE`, which tells it when someone logs on.
- **The data folder is locked down** at start (`lock_down_data_dir`): owner SYSTEM, SYSTEM full control, administrators read-only, no inheritance from `%ProgramData%` (where every user may create files). It uses SIDs (`*S-1-5-18` is SYSTEM, `*S-1-5-32-544` is Administrators) instead of names, so it works on any language of Windows.
- **A crash in one of its long-lived threads ends the process** (`sync::spawn_critical`), so Windows restarts it.
- `local_handler` / `embedded`: the same request handling without a service or a pipe, for `npm run dev` (section 8.1). Compiled in only with `debug_assertions`.

```rust
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
    let context = Arc::new(Context::new(blocklist, slot));
    pipe::serve(move |caller, request| handle(&context, caller, request), quit)?;
    Ok(quit_rx)
}

/// The running block, as the service has it: internet time on, a block left behind by an earlier
/// run picked up again (so a restart does not end it), and a thread that ends a timed block whose
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
pub fn local_handler(blocklist: Blocklist) -> Local {
    let context = Context::new(blocklist, Arc::new(AppSlot::new(false)));
    Arc::new(move |request| handle(&context, 0, request).0)
}

/// For `npm run dev`: the service inside the app, with no service installed, no pipe and no
/// administrator rights. **Debug builds only**, and not used with `BULLSEYE_USE_SERVICE` set.
/// Ending the app ends the enforcing, which is fine while developing. The block is kept in
/// `%LOCALAPPDATA%\bullseye-dev`, so a timed block is still there the next time; delete that
/// folder to forget it.
#[cfg(debug_assertions)]
pub fn embedded() -> Local {
    use_dev_dir();
    local_handler(start_blocklist())
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
    use crate::proto::{Request, Response};
    use crate::state::Enforcers;
    use std::time::Duration;

    fn start_request(duration: &str) -> Request {
        Request::Start {
            programs: vec![],
            allowed: vec![],
            filters: vec!["a.com".into()],
            duration: duration.into(),
            enforcers: Enforcers::default(),
        }
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
}
```

**`handler.rs`: what the service does with each request.** The same `handle` answers over the pipe and, in a debug build, inside the app, so the rules cannot differ between the two.

- **Every request is checked again** (`handle`, `start`, `unlock`). The code that used to validate in the Tauri command (`parse_duration`, `FilterList::parse`, "at least one enforcer", ...) is here, because the pipe can be reached by any program.
- **Unlock rate limit**: after 3 wrong passwords in a row, the next guesses are refused for 5 s, 10 s, 20 s, ... (capped at about 21 minutes). The lock is held while the password is hashed, so guesses cannot be tried in parallel. (The hash is still in the binary, so an offline attack on a weak password is unchanged: see section 8.4.)
- **Uninstall vs update**: `PrepareUninstall` is refused during a *timed* block. `PrepareUpdate` is refused during *any* block, because replacing the exe is also a way to end the service.

```rust
//! What the service does with each request from the app and the installer. Everything in a request
//! is checked here again: it comes from any local user's program, not necessarily ours. The same
//! `handle` answers over the pipe (`service.rs`) and, in a debug build, inside the app
//! (`service::local_handler`), so the rules cannot differ between the two.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::block::{parse_duration, Blocklist, Lists};
use crate::filters::FilterList;
use crate::programs::ProgramList;
use crate::proto::{Policy, Request, Response};
use crate::state::Enforcers;
use crate::sync::LockExt;
use crate::unlock;
use crate::watchdog::AppSlot;

/// What the installer asked the service to end for.
pub enum Shutdown {
    /// Let go of the exe so it can be replaced; the installer starts the service again.
    Update,
    /// Remove the service and everything it keeps.
    Remove,
}

/// What `handle` works with.
pub struct Context {
    blocklist: Blocklist,
    lockout: Mutex<Lockout>,
    /// Which process is the app (`watchdog.rs`).
    slot: Arc<AppSlot>,
}

impl Context {
    pub fn new(blocklist: Blocklist, slot: Arc<AppSlot>) -> Context {
        Context { blocklist, lockout: Mutex::default(), slot }
    }
}

/// Wrong unlock passwords in a row, and until when the next guess is refused.
#[derive(Default)]
struct Lockout {
    failures: u32,
    until: Option<Instant>,
}

/// Answers one request from the process `caller`, and says whether the service should end
/// afterwards. Everything in the request is checked here again: it comes from any local user's
/// program, not necessarily ours.
pub fn handle(context: &Context, caller: u32, request: Request) -> (Response, Option<Shutdown>) {
    let blocklist = &context.blocklist;
    let done = |result: Result<(), String>| match result {
        Ok(()) => Response::Done,
        Err(why) => Response::Failed(why),
    };
    match request {
        Request::Status => (Response::Status(blocklist.status()), None),
        // The app asks to be taken in; anyone else is refused, and the app then ends itself.
        Request::Register => match context.slot.register(caller) {
            Ok(()) => (Response::Done, None),
            Err(why) => (Response::Refused(why), None),
        },
        Request::Policy => {
            // This request comes from the thread that closes tabs: hearing it is how the service
            // knows that is going on. It counts only from the process taken in as the app.
            context.slot.heard(caller);
            let policy = blocklist.active().map(|active| Policy {
                filters: active.filters.entries(),
                allowed: active.allowed.entries(),
                programs: active.programs.entries(),
                enforcers: active.enforcers,
            });
            (Response::Policy(policy), None)
        }
        Request::Start { filters, allowed, programs, duration, enforcers } => {
            (done(start(blocklist, filters, allowed, programs, &duration, enforcers)), None)
        }
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
fn start(
    blocklist: &Blocklist,
    filters: Vec<String>,
    allowed: Vec<String>,
    programs: Vec<String>,
    duration: &str,
    enforcers: Enforcers,
) -> Result<(), String> {
    let duration = parse_duration(duration)?;
    let sites = FilterList::parse(&filters).map_err(|_| "Invalid block list syntax".to_string())?;
    let allowed = FilterList::parse(&allowed).map_err(|_| "Invalid syntax in the list of allowed sites".to_string())?;
    // Also refuses what would close programs Windows needs (`programs.rs`), with the reason.
    let programs = ProgramList::parse(&programs)?;
    if !enforcers.any() {
        return Err("Choose at least one enforcer".into());
    }
    // An enforcer has nothing to do with an empty list of its own kind, while one that needs no
    // list (Obsidian's) always does. What matters is that something can run.
    let lists = Lists { sites, allowed, programs };
    if !enforcers.can_run(|uses| lists.has(uses)) {
        return Err("Enter at least one site or program to block, or turn on an enforcer that needs no list".into());
    }
    blocklist.start(lists, duration, enforcers)
}

/// Ends the block for the unlock password. After three wrong guesses in a row, guesses are
/// refused for longer each time. The lock is held while the hash is computed, so guesses cannot
/// be made in parallel either.
fn unlock(context: &Context, password: &str) -> Result<(), String> {
    let mut lockout = context.lockout.locked();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Store;
    use std::fs;

    fn context(name: &str) -> Context {
        let dir = std::env::temp_dir().join(format!("service-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let blocklist = Blocklist::with_store(Store::new(dir.join("state.json")));
        Context { blocklist, lockout: Mutex::default(), slot: Arc::new(AppSlot::new(false)) }
    }

    fn start_request(duration: &str) -> Request {
        Request::Start { programs: vec![], allowed: vec![], filters: vec!["a.com".into()], duration: duration.into(), enforcers: Enforcers::default() }
    }

    #[test]
    fn requests_are_checked_again_by_the_service() {
        let context = context("checked");
        // An empty list with only enforcers that go by it, nothing chosen, a bad duration, and an
        // address that cannot match.
        let only_the_list = Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let bad = [
            Request::Start { programs: vec![], allowed: vec![], filters: vec![], duration: "1h".into(), enforcers: only_the_list },
            Request::Start {
                filters: vec!["a.com".into()],
                programs: vec![],
                allowed: vec![],
                duration: "1h".into(),
                enforcers: Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: false, exe_names: false },
            },
            start_request("soon"),
            Request::Start { programs: vec![], allowed: vec![], filters: vec!["has space.com".into()], duration: "1h".into(), enforcers: Enforcers::default() },
        ];
        for request in bad {
            assert!(matches!(handle(&context, 0, request).0, Response::Failed(_)));
        }
        assert!(context.blocklist.filters().is_none());
    }

    #[test]
    fn an_empty_list_starts_a_block_when_an_enforcer_without_the_list_is_on() {
        let first = context("empty-list");
        let obsidian_only = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true, exe_names: false };
        let request = |enforcers| Request::Start { programs: vec![], allowed: vec![], filters: vec![], duration: "1h".into(), enforcers };
        assert_eq!(handle(&first, 0, request(obsidian_only)).0, Response::Done);
        // The block keeps what it was started with: no entries, and Obsidian's enforcer only.
        let Response::Policy(Some(policy)) = handle(&first, 0, Request::Policy).0 else { panic!("no policy") };
        assert!(policy.filters.is_empty());
        assert_eq!(policy.enforcers, obsidian_only);

        // With all enforcers on (the default), an empty list is fine too: Obsidian's can run.
        let other = context("empty-list-default");
        assert_eq!(handle(&other, 0, request(Enforcers::default())).0, Response::Done);
    }

    #[test]
    fn only_the_installed_app_is_taken_in_as_the_app() {
        use std::process::{Command, Stdio};
        let mut context = context("register");
        context.slot = Arc::new(AppSlot::new(true));
        // A process that is not the installed exe, whatever it calls itself.
        let mut stranger = Command::new("ping.exe").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();

        let (response, _) = handle(&context, stranger.id(), Request::Register);
        assert!(matches!(response, Response::Refused(why) if why.contains("not the installed")));
        assert!(matches!(handle(&context, 0, Request::Register).0, Response::Refused(_)));
        // It may ask for the policy all the same; that is just information, and counts for nothing.
        assert_eq!(handle(&context, stranger.id(), Request::Policy).0, Response::Policy(None));
        assert!(context.slot.supervise(true, false), "no app should have been taken in");

        // This test process is the installed exe as far as the service can tell. Only if it runs
        // in the console session is it taken in; elsewhere an app just runs unwatched.
        let (response, _) = handle(&context, std::process::id(), Request::Register);
        assert_eq!(response, Response::Done);
        // A second app is refused while the first one is there.
        let (response, _) = handle(&context, stranger.id(), Request::Register);
        assert!(matches!(response, Response::Refused(_)));

        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    #[test]
    fn programs_alone_can_start_a_block_and_what_windows_needs_is_refused() {
        let programs_only = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: false, exe_names: true };
        let request = |programs: &[&str]| Request::Start {
            filters: vec![],
            allowed: vec![],
            programs: programs.iter().map(|p| p.to_string()).collect(),
            duration: "1h".into(),
            enforcers: programs_only,
        };
        let context = context("programs");

        // The programs enforcer alone, with nothing listed, has nothing to do.
        assert!(matches!(handle(&context, 0, request(&[])).0, Response::Failed(_)));
        // Entries that would close what Windows or this app needs, or are not names.
        for bad in ["*", "explorer.exe", "s*", r"C:\Games\game.exe"] {
            let Response::Failed(why) = handle(&context, 0, request(&[bad])).0 else { panic!("{bad} was accepted") };
            assert!(why.contains("needs") || why.contains("Not a program name"), "{bad}: {why}");
        }
        assert!(context.blocklist.filters().is_none());

        // A real one starts, and is what the app is told to close.
        assert_eq!(handle(&context, 0, request(&["Steam*", "game"])).0, Response::Done);
        let Response::Policy(Some(policy)) = handle(&context, 0, Request::Policy).0 else { panic!("no policy") };
        assert_eq!(policy.programs, vec!["steam*.exe", "game.exe"]);
        assert!(policy.filters.is_empty());
        assert_eq!(policy.enforcers, programs_only);
    }

    #[test]
    fn allowed_sites_travel_with_the_block_and_must_be_valid() {
        let context = context("allowed");
        let request = |filters: &[&str], allowed: &[&str]| Request::Start {
            filters: filters.iter().map(|f| f.to_string()).collect(),
            allowed: allowed.iter().map(|f| f.to_string()).collect(),
            programs: vec![],
            duration: "1h".into(),
            enforcers: Enforcers::default(),
        };
        // An allowed entry that cannot match is refused like a bad block entry.
        assert!(matches!(handle(&context, 0, request(&["*"], &["has space.com"])).0, Response::Failed(_)));
        assert!(context.blocklist.filters().is_none());

        // Everything blocked but one site: the app is told about both.
        assert_eq!(handle(&context, 0, request(&["*"], &["Google.com"])).0, Response::Done);
        let Response::Policy(Some(policy)) = handle(&context, 0, Request::Policy).0 else { panic!("no policy") };
        assert_eq!(policy.filters, vec!["*"]);
        assert_eq!(policy.allowed, vec!["google.com"]);
        let Response::Status(status) = handle(&context, 0, Request::Status).0 else { panic!("no status") };
        assert_eq!(status.allowed, vec!["google.com"]);
    }

    #[test]
    fn allowed_sites_alone_are_not_something_to_block() {
        let context = context("allowed-alone");
        // Only the tab enforcers, no sites to block and only an allowed one: nothing to do.
        let tabs_only = Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: false, exe_names: false };
        let request = Request::Start {
            filters: vec![],
            allowed: vec!["google.com".into()],
            programs: vec![],
            duration: "1h".into(),
            enforcers: tabs_only,
        };
        assert!(matches!(handle(&context, 0, request).0, Response::Failed(_)));
        assert!(context.blocklist.filters().is_none());
    }

    #[test]
    fn a_timed_block_cannot_be_shortened_stopped_or_uninstalled() {
        let context = context("timed");
        assert_eq!(handle(&context, 0, start_request("1h")).0, Response::Done);
        // Not replaced by a shorter or an untimed one, not stopped.
        assert!(matches!(handle(&context, 0, start_request("1s")).0, Response::Failed(_)));
        assert!(matches!(handle(&context, 0, start_request("")).0, Response::Failed(_)));
        assert!(matches!(handle(&context, 0, Request::Stop).0, Response::Failed(_)));
        // Neither uninstalling nor updating gets through, and the service keeps running.
        let (response, shutdown) = handle(&context, 0, Request::PrepareUninstall);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        let (response, shutdown) = handle(&context, 0, Request::PrepareUpdate);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        // The policy is what the app enforces.
        let Response::Policy(Some(policy)) = handle(&context, 0, Request::Policy).0 else { panic!("no policy") };
        assert_eq!(policy.filters, vec!["a.com"]);
    }

    #[test]
    fn without_a_block_the_installer_gets_through() {
        let context = context("idle");
        let (response, shutdown) = handle(&context, 0, Request::PrepareUninstall);
        assert_eq!(response, Response::Done);
        assert!(matches!(shutdown, Some(Shutdown::Remove)));
        let (response, shutdown) = handle(&context, 0, Request::PrepareUpdate);
        assert_eq!(response, Response::Done);
        assert!(matches!(shutdown, Some(Shutdown::Update)));
        // An untimed block does not stop an uninstall, but does stop an update.
        assert_eq!(handle(&context, 0, start_request("")).0, Response::Done);
        assert!(matches!(handle(&context, 0, Request::PrepareUninstall).1, Some(Shutdown::Remove)));
        assert!(matches!(handle(&context, 0, Request::PrepareUpdate).0, Response::Refused(_)));
    }

    #[test]
    fn the_password_ends_a_timed_block_and_wrong_ones_are_slowed_down() {
        let context = context("unlock");
        assert_eq!(handle(&context, 0, start_request("1h")).0, Response::Done);
        for _ in 0..3 {
            let wrong = Request::Unlock { password: "nope".into() };
            assert_eq!(handle(&context, 0, wrong).0, Response::Failed("Wrong password".into()));
        }
        // After the third, even the right password waits.
        let right = Request::Unlock { password: "test1234".into() };
        assert!(matches!(handle(&context, 0, right).0, Response::Failed(why) if why.starts_with("Too many")));
        assert!(context.blocklist.filters().is_some());
        // Once the wait is over, it works.
        context.lockout.locked().until = None;
        let right = Request::Unlock { password: "test1234".into() };
        assert_eq!(handle(&context, 0, right).0, Response::Done);
        assert!(context.blocklist.filters().is_none());
    }
}
```

**`registration.rs`: installing, locking down and removing the service** (the installer runs these as an administrator, except `remove`, which the service runs on itself).

- **The SDDL** gives SYSTEM everything (it needs delete to remove itself on uninstall), administrators **and interactive users** query/enumerate/**start**/read-the-list only, and service accounts query only. Administrators keep *start* on purpose: the installer has to be able to start the service again after an update. Interactive users get *start* because the app, which runs as one, starts the service again when it is not running (`client::Remote::revive_service`, at most every 5 s). Starting is harmless: starting a running service does nothing, and a stopped one only goes back to blocking. Nobody but SYSTEM gets stop, pause, change-config, delete, write-DACL or write-owner. **Never widen what interactive users may do beyond start**: the app runs as the user, so anything the app may do, the user may do. `sc sdset` is the **last** step of the install: after it, an administrator can no longer change the service (the handle the installer already holds keeps its rights).
- **Failure actions** are "restart immediately", three times over, and the last one repeats forever. `set_failure_actions_on_non_crash_failures(true)` makes that apply also when the process ends with a failure *exit code*, not just when it is killed. A clean exit code 0 is **not** restarted: the installer relies on that.
- **Safe mode**: the installer creates `SafeBoot\Minimal\bullseye` and `SafeBoot\Network\bullseye` registry keys so Windows starts the service in safe mode as well.

```rust
//! The service's registration with Windows: installing it (locked down, restarting itself if it
//! fails, started in safe mode), starting it, and removing it. The installer runs these as an
//! administrator, except `remove`, which the service runs on itself.

use std::ffi::OsString;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::state;
use crate::watchdog::note;

/// The service's name, exactly as it appears in the services list.
pub const NAME: &str = "bullseye";

/// Who may do what with the service. SYSTEM: everything (it deletes itself on uninstall).
/// Administrators and interactive users: query, enumerate, read the list and start, but not stop,
/// pause, change, delete or rewrite it. Interactive users may start it because the app (which
/// runs as one) starts it again when it is not running (`client::Remote::revive_service`);
/// starting a running service does nothing, and a stopped one only goes back to blocking.
/// Service accounts: query only.
const SDDL: &str = "D:(A;;CCLCSWRPWPDTLOCRSDRCWDWO;;;SY)(A;;CCLCSWRPLOCRRC;;;BA)(A;;CCLCSWRPLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";

/// The service is started in safe mode too, when these keys exist.
const SAFE_BOOT: [&str; 2] = [
    r"HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Minimal\bullseye",
    r"HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Network\bullseye",
];

/// `CREATE_NO_WINDOW`: the tools below are console programs, which would flash a window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const ERROR_SERVICE_EXISTS: i32 = 1073;
const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;

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
        display_name: "Bullseye".into(),
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

/// Starts the service, if it is registered and not running. Administrators and interactive users
/// may. Says whether it was started just now (`false`: it was running already).
pub fn start_registered() -> Result<bool, String> {
    let service = manager(ServiceManagerAccess::CONNECT)?
        .open_service(NAME, ServiceAccess::START)
        .map_err(|e| format!("cannot open the service: {e}"))?;
    match service.start::<OsString>(&[]) {
        Ok(()) => Ok(true),
        Err(e) if is_os_error(&e, ERROR_SERVICE_ALREADY_RUNNING) => Ok(false),
        Err(e) => Err(format!("cannot start the service: {e}")),
    }
}

/// Waits until the service has ended or is gone, so the installer can replace or delete the exe.
pub fn wait_stopped(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let stopped = manager(ServiceManagerAccess::CONNECT)
            .ok()
            .and_then(|m| m.open_service(NAME, ServiceAccess::QUERY_STATUS).ok())
            .is_none_or(|service| service.query_status().is_ok_and(|s| s.current_state == ServiceState::Stopped));
        if stopped {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

/// Run by the service itself, as SYSTEM, on an uninstall: deletes the service, its safe-mode
/// keys and everything it saved.
pub fn remove() {
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

    #[test]
    fn the_access_lists_say_what_they_should() {
        // Administrators and interactive users may start (RP) but not stop (WP), delete (SD),
        // change (DC) or rewrite (WD, WO).
        for who in [";;;BA)", ";;;IU)"] {
            let ace = SDDL.split('(').find(|ace| ace.ends_with(who)).unwrap();
            assert!(ace.contains("RP"), "{who}");
            for right in ["WP", "SD", "DC", "WD", "WO", "DT"] {
                assert!(!ace.contains(right), "{who} {right}");
            }
        }
        // Service accounts only look.
        let accounts = SDDL.split('(').find(|ace| ace.ends_with(";;;SU)")).unwrap();
        assert!(!accounts.contains("RP"));
        // The pipe's list is separate, in `pipe.rs`.
    }
}
```

### 6.2 The rewritten and deleted files

**Delete `src-tauri/src/task.rs`.** Nothing refers to it any more.

`watchdog.rs` is now about the service looking after the app, and **which process counts as the app is the service's decision, never a matter of its name**. The service keeps a handle to exactly one process in an `AppSlot`: either one it started itself (`launch_app`), or one that asked to be taken in. Taking in (`AppSlot::register`, used by `Request::Register` and by the first `Policy` request) happens only if the process runs **the installed exe** (the service, as SYSTEM, can open any process and read its image path), in the **console session** (the one at the physical screen), and no other app is taken in. The service learns who is asking from the pipe itself (`GetNamedPipeClientProcessId`), so the caller cannot lie about it. Consequences:

- A copy of the exe under another name, or a look-alike, is never the app: it is refused, and the service starts the real one anyway.
- Holding the process handle means the service sees the app end at once, and the process id cannot be reused by something else meanwhile.
- A second copy of the real app, started by hand, is refused (`Register`) and ends itself. This replaces the old per-session mutex, which a user could simply hold with any program to keep the real app from starting.
- **The app has to keep asking for its policy.** That request comes from the same thread that closes the tabs (`tab_closer.rs` calls `remote.active()` at the top of each pass). While a block runs, an app that has not asked for 5 seconds (20 seconds for a freshly started one: Tauri and WebView2 take a while) is ended and started again. That covers a suspended app, and a closing thread that hangs or is suspended. A pause of the service's own loop of 3 seconds or more is taken to be the machine having slept, and gives the app a fresh start instead of ending it.

`launch_app` starts the app as the logged-on user, not as SYSTEM: `WTSQueryUserToken` gets that user's token, `CreateEnvironmentBlock` their environment, `CreateProcessAsUserW` starts the process on `winsta0\default`. It runs while any block exists (timed or not), plus once after each logon (`LAUNCH_APP`). That is why quitting the app with the unlock password sticks: the block is gone, so nobody restarts it until the next logon.

**What this still does not stop:** the app is a process of the very user it protects against. Someone who can inject code into it, or run the real exe under a debugger, can keep it asking for its policy while it enforces nothing. The heartbeat shows that the closing loop is running, not that tabs are being closed. That takes effort and skill rather than a renamed copy of Notepad, which is the level of friction this project aims for.

```rust
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

            let block_active = blocklist.filters().is_some();
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
```

`setup.rs`: the installer cannot stop the service (nobody but SYSTEM can), so it asks over the pipe. If the service is not running (an update that did not finish), it starts it first: administrators are allowed to start it.

```rust
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
```

`main.rs`: the Tauri commands are now thin forwards, the old `--launch` entry point is gone, the app no longer runs a clock, a block list or a launcher guard, and there are two new arguments (`--service`, `--prepare-update`).

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod block;
mod client;
mod clock;
mod filters;
mod handler;
mod pipe;
mod programs;
mod proto;
mod registration;
mod service;
mod setup;
mod state;
mod sync;
mod tab_closer;
mod ui;
mod unlock;
mod watchdog;

use block::Status;
use client::Remote;
use proto::{Request, Response};
use state::Enforcers;
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, RunEvent, State, WindowEvent,
};

/// Starts blocking with the given filter lines, allowed-site lines, program lines and enforcers, for `duration` (`5s`, `10m`, `3h`,
/// `1d`) or, when empty, until stopped.
///
/// The service owns the block and checks all of this again (`service::start`); the answer here
/// is only its verdict. It goes on even if this app is ended or Windows restarts.
#[tauri::command]
fn start_block(
    filters: Vec<String>,
    allowed: Vec<String>,
    programs: Vec<String>,
    duration: String,
    enforcers: Enforcers,
    remote: State<Remote>,
) -> Result<(), String> {
    remote.command(Request::Start { filters, allowed, programs, duration, enforcers })
}

#[tauri::command]
fn stop_block(remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Stop)
}

#[tauri::command]
fn block_status(remote: State<Remote>) -> Result<Status, String> {
    remote.status()
}

/// Ends the block for good, if the unlock password is right, and quits the app. The service
/// stays; with no block left it has nothing to do, and starts the app again at the next logon.
#[tauri::command]
fn force_exit(password: String, app: AppHandle, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Unlock { password })?;
    watchdog::note("app", "stopped by the unlock password");
    app.exit(0);
    Ok(())
}

/// Who the app talks to. Normally the service, over its pipe. In a debug build (`npm run dev`) the
/// service's logic runs inside the app instead, so development needs no installed service; set
/// `BULLSEYE_USE_SERVICE` to talk to a real, installed one anyway. A release build has no
/// such code.
fn remote_for_this_run() -> Remote {
    #[cfg(debug_assertions)]
    if std::env::var_os("BULLSEYE_USE_SERVICE").is_none() {
        return Remote::local(service::embedded());
    }
    Remote::new()
}

fn main() {
    // The service itself, started by Windows (`service.rs`).
    if std::env::args_os().any(|arg| arg == "--service") {
        std::process::exit(service::run_dispatcher());
    }
    // Run by the Windows installer and uninstaller (`setup.rs`); the exit code is their answer.
    if std::env::args_os().any(|arg| arg == "--install") {
        std::process::exit(setup::install());
    }
    if std::env::args_os().any(|arg| arg == "--uninstall") {
        std::process::exit(setup::uninstall());
    }
    if std::env::args_os().any(|arg| arg == "--prepare-update") {
        std::process::exit(setup::prepare_update());
    }

    let remote = remote_for_this_run();

    // Ask the service to take this process in as the app. It refuses if another app is running
    // (a second copy leaves the running one to do the work) or if this is not the installed exe.
    // If the service cannot be reached, go on: it takes the app in on its first policy request.
    // (A local service has nobody to register with.)
    if !remote.is_local() {
        if let Ok(Response::Refused(why)) = client::call(&Request::Register) {
            watchdog::note("app", &format!("not starting: {why}"));
            return;
        }
    }

    tab_closer::spawn(remote.clone());

    tauri::Builder::default()
        .manage(remote)
        .manage(ui::Ui::new())
        .invoke_handler(tauri::generate_handler![start_block, stop_block, block_status, force_exit, ui::window_ready])
        // The window's X hides it (it is destroyed after a while, `ui.rs`); only the unlock password quits.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                ui::dismiss(window);
            }
        })
        .setup(|app| {
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button_state: MouseButtonState::Up, .. } = event {
                        ui::show_in_background(tray.app_handle().clone());
                    }
                })
                .build(app)?;
            ui::spawn_reaper(app.handle().clone());
            #[cfg(debug_assertions)]
            ui::debug_aids(app.handle());

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // With no window the app has nothing to show but must not end: an exit that nobody asked
            // for (the last window going away) is refused, while the unlock password's is allowed.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
```

### 6.3 Frontend, config and installer

`index.html`: `block_status` can now fail (service not running), so show that instead of an unhandled error every second.

```diff
--- a/src/index.html
+++ b/src/index.html
@@ -57,9 +57,10 @@
         color: var(--text);
         user-select: none;
         cursor: default;
+        overflow: hidden;
       }
       textarea {
-        flex: 1;
+        flex: none;
         resize: none;
         padding: 6px 10px;
         font: inherit;
@@ -76,7 +77,6 @@
       textarea:disabled::placeholder { color: var(--text-disabled); }
       /* The enforcers, grouped by whether they go by the block list. */
       section { display: flex; flex-direction: column; gap: 8px; }
-      section.list { flex: 1; min-height: 0; }
       h2 { margin: 0; font-size: 14px; line-height: 20px; font-weight: 600; }
       label.check { display: flex; align-items: center; gap: 8px; }
       label.check:has(input:disabled) { color: var(--text-disabled); }
@@ -132,7 +132,7 @@
       input:disabled { color: var(--text-disabled); background: var(--control-disabled); border-color: var(--control-stroke); }
       /* A status bar along the bottom edge: full width, past the body's side and bottom padding. */
       #status {
-        margin: 0 -16px;
+        margin: auto -16px 0;
         padding: 4px 16px 8px;
         min-height: 16px;
         font-family: "Cascadia Mono", Consolas, monospace;
@@ -150,10 +150,17 @@
   <body>
     <section class="list">
       <h2>Block list</h2>
-      <textarea id="sites" placeholder="One site per line, e.g. youtube.com" aria-label="Block list" autofocus></textarea>
+      <textarea id="sites" rows="5" placeholder="One site per line, e.g. youtube.com" aria-label="Block list" autofocus></textarea>
+      <h2>Always allowed</h2>
+      <textarea id="allowed" rows="5" placeholder="One site per line, e.g. google.com" aria-label="Always allowed sites"></textarea>
       <label class="check"><input type="checkbox" data-enforcer="chromium_tabs" checked />Close blocked tabs in Chrome, Edge, Brave</label>
       <label class="check"><input type="checkbox" data-enforcer="chromium_split_view" checked />Close blocked pages in split view</label>
     </section>
+    <section class="list">
+      <h2>Programs</h2>
+      <textarea id="programs" rows="5" placeholder="One program per line, e.g. steam.exe" aria-label="Programs to close"></textarea>
+      <label class="check"><input type="checkbox" data-enforcer="exe_names" checked />Close these programs</label>
+    </section>
     <section>
       <h2>Without block list</h2>
       <label class="check"><input type="checkbox" data-enforcer="obsidian_web_viewer" checked />Close Obsidian when it shows web pages</label>
@@ -170,6 +177,8 @@
     <script>
       const { invoke } = window.__TAURI__.core;
       const sites = document.getElementById("sites");
+      const programBox = document.getElementById("programs");
+      const allowedBox = document.getElementById("allowed");
       const duration = document.getElementById("duration");
       const toggle = document.getElementById("toggle");
       const status = document.getElementById("status");
@@ -200,13 +209,26 @@
 
       // The backend owns the block (timed blocks end there); the UI mirrors it every second.
       async function refresh() {
-        const { blocking, remaining, filters, enforcers: running, clock_synced } = await invoke("block_status");
+        let state;
+        try {
+          state = await invoke("block_status");
+        } catch (e) {
+          // The service did not answer: say so, and leave the rest as it was.
+          status.classList.add("error");
+          status.textContent = String(e);
+          return;
+        }
+        const { blocking, remaining, filters, allowed: allowedEntries, programs: programEntries, enforcers: running, clock_synced } = state;
         const timed = remaining != null;
         // A block resumed from disk comes with its list, so show it rather than an empty box.
         if (blocking && filters.length && !sites.value) sites.value = filters.join("\n");
+        if (blocking && allowedEntries.length && !allowedBox.value) allowedBox.value = allowedEntries.join("\n");
+        if (blocking && programEntries.length && !programBox.value) programBox.value = programEntries.join("\n");
         toggle.textContent = blocking ? "Stop block" : "Start block";
         toggle.disabled = timed;
         sites.disabled = blocking;
+        allowedBox.disabled = blocking;
+        programBox.disabled = blocking;
         // Fixed while a block runs, so turning one off is not a way around it.
         for (const c of checks) {
           if (running) c.checked = running[c.dataset.enforcer];
@@ -231,10 +253,18 @@
       }
 
       toggle.addEventListener("click", async () => {
-        const { blocking } = await invoke("block_status");
+        let blocking;
+        try {
+          ({ blocking } = await invoke("block_status"));
+        } catch (e) {
+          error = String(e);
+          return refresh();
+        }
         if (blocking) return run("stop_block");
         const filters = sites.value.split(/\r?\n/);
-        await run("start_block", { filters, duration: duration.value, enforcers: enforcers() });
+        const programs = programBox.value.split(/\r?\n/);
+        const allowed = allowedBox.value.split(/\r?\n/);
+        await run("start_block", { filters, allowed, programs, duration: duration.value, enforcers: enforcers() });
       });
 
       // Enter in the field submits too.
@@ -249,6 +279,8 @@
       });
       password.addEventListener("input", () => { error = ""; force.disabled = !password.value; refresh(); });
       sites.addEventListener("input", () => { error = ""; refresh(); });
+      programBox.addEventListener("input", () => { error = ""; refresh(); });
+      allowedBox.addEventListener("input", () => { error = ""; refresh(); });
       duration.addEventListener("input", () => { error = ""; refresh(); });
       for (const c of checks) {
         c.addEventListener("change", () => {
@@ -258,8 +290,15 @@
         });
       }
 
-      refresh();
-      setInterval(refresh, 1000);
+      // The window is made when it is asked for and shown once this page has drawn (`ui::window_ready`);
+      // its X only hides it. While it is hidden nothing is looked at, so nothing is asked of the service.
+      let visible = true;
+      window.__TAURI__.event.listen("visibility", (event) => {
+        visible = event.payload;
+        if (visible) refresh();
+      });
+      setInterval(() => { if (visible && !document.hidden) refresh(); }, 1000);
+      refresh().finally(() => invoke("window_ready"));
     </script>
   </body>
 </html>
```

`tauri.conf.json`: the installer must run as administrator and install into `C:\Program Files`, **not** a folder the user can write to. If the user could replace the exe the service runs from, they could run anything as SYSTEM, which would defeat the whole thing.

```diff
--- a/src-tauri/tauri.conf.json
+++ b/src-tauri/tauri.conf.json
@@ -9,18 +9,7 @@
   },
   "app": {
     "withGlobalTauri": true,
-    "windows": [
-      {
-        "label": "main",
-        "title": "Bullseye",
-        "width": 340,
-        "height": 560,
-        "visible": false,
-        "resizable": false,
-        "transparent": true,
-        "windowEffects": { "effects": ["mica"] }
-      }
-    ],
+    "windows": [],
     "security": {
       "csp": null
     }
@@ -32,7 +21,7 @@
     "shortDescription": "Blocks distracting sites in Chromium browsers and Obsidian",
     "windows": {
       "nsis": {
-        "installMode": "currentUser",
+        "installMode": "perMachine",
         "installerHooks": "./windows/hooks.nsh"
       }
     },
```

`windows/hooks.nsh` (replace the whole file). Tauri's NSIS template runs its own "is the app running?" check (and tries to kill the process) right *after* the pre-install and pre-uninstall hooks. Our hooks run first and make the service end the app, so that check finds nothing. This order was read from the generated `installer.nsi` of the existing build (hook at the `Section Install` start, the check on the next line; same in `Section Uninstall`). An update (`/UPDATE`) goes straight into `Section Install`, so `--prepare-update` belongs in `PREINSTALL`.

The exit codes of `--uninstall`: `0` done (or no service registered), `1` the service is registered but did not answer or did not end in time, `2` it answered that a timed block is running. **Both 1 and 2 stop the uninstall** and nothing is removed: continuing after a failure would delete files under a service that may still be running and leave a half-removed install. If a broken service blocks the uninstall, use the recovery steps in section 8.2.

```nsis
; Hooks into Tauri's NSIS installer (tauri.conf.json > bundle > windows > nsis > installerHooks).
; The installer runs as an administrator (installMode "perMachine"). The work itself is in the
; app (src/setup.rs); these only run it and act on its exit code:
; 0 done, 1 failed, 2 refused because a block is running.
;
; The service cannot be stopped by anyone but itself, so each hook asks it to let go over its
; pipe. Both hooks run before the template's own "is the app running" check, which is why that
; check finds nothing: the service has ended and has ended the app with it.

!macro NSIS_HOOK_PREINSTALL
  ; An update replaces the exe the service runs from. Nothing to ask on a first install.
  ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --prepare-update' $0
    ${If} $0 = 2
      MessageBox MB_ICONSTOP "A block is running, so Bullseye cannot be updated now.$\n$\nWait for the block to end, or end it with the unlock password in the app, then run the installer again." /SD IDOK
      SetErrorLevel 2
      Quit
    ${ElseIf} $0 <> 0
      MessageBox MB_ICONSTOP "Bullseye could not be stopped for the update. See C:\ProgramData\bullseye\service.log." /SD IDOK
      SetErrorLevel 1
      Quit
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Registers the service, locks it down and starts it; it starts the app for whoever is logged on.
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --install' $0
  ${If} $0 <> 0
    MessageBox MB_ICONEXCLAMATION "Bullseye is installed, but its service could not be set up, so it will not block anything. See %LOCALAPPDATA%\bullseye\app.log." /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; An update replaces the files only (and has asked the service already, in PREINSTALL).
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall' $0
    ${If} $0 = 2
      MessageBox MB_ICONSTOP "A timed block is still running, so Bullseye cannot be uninstalled now.$\n$\nWait for the block to end, or end it with the unlock password in the app, then uninstall again." /SD IDOK
      SetErrorLevel 2
      Quit
    ${ElseIf} $0 <> 0
      ; The service is registered but did not answer or did not end. Stopping here, rather than
      ; deleting files under a service that may still be running, avoids a half-removed install.
      ; (No service at all is exit code 0, so that case goes on.)
      MessageBox MB_ICONSTOP "Bullseye's service did not respond, so nothing was removed.$\n$\nSee C:\ProgramData\bullseye\service.log, or use the recovery steps in detailed-instructions.md." /SD IDOK
      SetErrorLevel 1
      Quit
    ${EndIf}
  ${EndIf}
!macroend
```

### 6.4 The programs enforcer, and how to add the next enforcer

The new enforcer closes programs by the name of their exe. It has its own list and its own textarea, next to the site list. Decisions worth knowing:

- **Matching** (`programs.rs`): one entry per line, compared with the file name of the running program, never a path. Case is ignored, `*` stands for any characters, and `.exe` is added when missing: `discord` is `discord.exe`, `steam*` is `steam*.exe`, `*launcher.exe` covers `epiclauncher.exe`. Names with spaces are fine; paths and characters a file name cannot have are refused.
- **A protected list** (`PROTECTED` in `programs.rs`) holds what must never be closed: Windows' own programs (`explorer.exe`, `svchost.exe`, `csrss.exe`, ...), this app's tray app and its WebView2, and the service. An entry that would match **any** of them is refused when the block starts, with the reason. That also refuses `*`, `*.exe` and `s*`, so a typo cannot take the desktop down. `is_blocked` also never says yes to a protected name, as a last line of defence for a list that was read back from disk.
- **How a program is closed** (`tab_closer.rs`, `close_programs`): once a second the app lists the running programs. A listed one is first **asked to close** (a `WM_CLOSE` to each of its top-level windows, like the X button, so it can offer to save), and if it is still running **3 seconds later** it is ended. A program with no window to ask is therefore ended after those 3 seconds. The app never touches its own process.
- **The app, not the service, does this**, like the tab closing: it runs as the user, so it can close the user's programs and cannot touch SYSTEM's. It also means the heartbeat in section 6.2 covers this enforcer too.
- **The block remembers its programs**: they are saved in `state.json` (`Saved::programs`, which an older file simply lacks) and come back after a restart; the UI shows them again for a running block.
- **Start rules**: an enforcer needs its own list. The programs enforcer on with an empty program list has nothing to do, so a block with only that enforcer and no programs is refused; a block with Obsidian on and both lists empty is fine.

```rust
//! Blocked programs: one exe name per line, `*` as wildcard.
//!
//! - Letter case is ignored, and only the file name counts, never a path: `steam.exe`.
//! - `.exe` is added when it is missing: `discord` is `discord.exe`, `steam*` is `steam*.exe`.
//! - `*` matches any characters: `*launcher.exe` covers `epiclauncher.exe` and `launcher.exe`.
//!
//! Some programs must never be closed, because closing them breaks Windows or this app: they are
//! listed in `PROTECTED`. An entry that would match any of them is refused when the block starts,
//! which also refuses `*` and `*.exe` and the like, so a typo cannot take the desktop down.

use regex::RegexSet;

/// Programs that are never closed: Windows' own, and this app's (the tray app, its WebView2 and
/// the service). Lower case, as the names are compared.
const PROTECTED: [&str; 25] = [
    "explorer.exe",
    "winlogon.exe",
    "wininit.exe",
    "csrss.exe",
    "smss.exe",
    "services.exe",
    "lsass.exe",
    "svchost.exe",
    "dwm.exe",
    "fontdrvhost.exe",
    "sihost.exe",
    "taskhostw.exe",
    "ctfmon.exe",
    "conhost.exe",
    "runtimebroker.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "msedgewebview2.exe",
    "bullseye.exe",
    "tray-app.exe",
    "system",
    "registry",
    "memory compression",
    "secure system",
];

/// Characters a file name cannot contain, which also keeps paths out.
const NOT_IN_A_NAME: [char; 8] = ['\\', '/', ':', '"', '<', '>', '|', '?'];

pub struct ProgramList {
    /// The normalized entries, kept so a block can be written to disk and resumed (`state.rs`).
    entries: Vec<String>,
    matchers: RegexSet,
}

impl Default for ProgramList {
    fn default() -> ProgramList {
        ProgramList { entries: Vec::new(), matchers: RegexSet::empty() }
    }
}

impl ProgramList {
    /// Parses one entry per line, ignoring empty lines. The error says in words what is wrong.
    pub fn parse<S: AsRef<str>>(lines: &[S]) -> Result<ProgramList, String> {
        let trimmed: Vec<&str> = lines.iter().map(|l| l.as_ref().trim()).filter(|l| !l.is_empty()).collect();
        let invalid: Vec<&str> = trimmed.iter().copied().filter(|e| e.contains(NOT_IN_A_NAME)).collect();
        if !invalid.is_empty() {
            return Err(format!("Not a program name (give the name only, e.g. steam.exe): {}", invalid.join(", ")));
        }
        let entries: Vec<String> = trimmed.iter().map(|e| normalize(e)).collect();
        // Entry by entry, so the message says which ones.
        let too_wide: Vec<&str> = entries
            .iter()
            .filter(|entry| {
                let one = RegexSet::new([to_regex(entry)]).unwrap();
                PROTECTED.iter().any(|name| one.is_match(name))
            })
            .map(String::as_str)
            .collect();
        if !too_wide.is_empty() {
            return Err(format!("Would close programs Windows or this app needs: {}", too_wide.join(", ")));
        }
        Ok(ProgramList { matchers: RegexSet::new(entries.iter().map(|e| to_regex(e))).unwrap(), entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries as written down, which are normalized and so match exactly what they matched.
    pub fn entries(&self) -> Vec<String> {
        self.entries.clone()
    }

    /// Whether a program with this file name (`chrome.exe`) is on the list. Never one of
    /// `PROTECTED`, whatever the list says.
    pub fn is_blocked(&self, file_name: &str) -> bool {
        let name = file_name.to_lowercase();
        !PROTECTED.contains(&name.as_str()) && self.matchers.is_match(&name)
    }
}

/// Lower case, with `.exe` added when missing.
fn normalize(entry: &str) -> String {
    let entry = entry.trim().to_lowercase();
    if entry.ends_with(".exe") {
        entry
    } else {
        format!("{entry}.exe")
    }
}

/// `*` becomes "anything", everything else is literal, and the whole name has to match.
fn to_regex(entry: &str) -> String {
    let parts: Vec<String> = entry.split('*').map(regex::escape).collect();
    format!("^{}$", parts.join(".*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(entries: &[&str]) -> ProgramList {
        ProgramList::parse(entries).unwrap()
    }

    #[test]
    fn names_match_whole_and_ignore_case() {
        let programs = list(&["Steam.exe", "discord"]);
        assert!(programs.is_blocked("steam.exe"));
        assert!(programs.is_blocked("STEAM.EXE"));
        assert!(programs.is_blocked("Discord.exe"));
        // The whole name, not a part of it.
        assert!(!programs.is_blocked("steamwebhelper.exe"));
        assert!(!programs.is_blocked("notsteam.exe"));
        assert!(!programs.is_blocked("steam.exe.bak"));
        assert_eq!(programs.entries(), vec!["steam.exe", "discord.exe"]);
    }

    #[test]
    fn wildcards_stand_for_any_characters() {
        let programs = list(&["steam*", "*launcher.exe", "a*b*c.exe"]);
        assert!(programs.is_blocked("steam.exe"));
        assert!(programs.is_blocked("steamwebhelper.exe"));
        assert!(programs.is_blocked("epicgameslauncher.exe"));
        assert!(programs.is_blocked("launcher.exe"));
        assert!(programs.is_blocked("axxbyyc.exe"));
        assert!(programs.is_blocked("abc.exe"));
        assert!(!programs.is_blocked("steam.dll"));
        assert!(!programs.is_blocked("axxc.exe"));
        // Characters that mean something in a regular expression are just characters.
        let dots = list(&["a.b+c(1).exe"]);
        assert!(dots.is_blocked("a.b+c(1).exe"));
        assert!(!dots.is_blocked("axb+c(1).exe"));
    }

    #[test]
    fn empty_lines_are_skipped_and_an_empty_list_blocks_nothing() {
        let programs = list(&["", "  ", "game.exe", ""]);
        assert_eq!(programs.entries(), vec!["game.exe"]);
        let none = list(&["", " "]);
        assert!(none.is_empty());
        assert!(!none.is_blocked("game.exe"));
        assert!(!ProgramList::default().is_blocked("game.exe"));
    }

    #[test]
    fn paths_and_odd_characters_are_not_names() {
        for bad in [r"C:\Games\game.exe", "games/game.exe", r"..\game.exe", "game?.exe", "a|b.exe"] {
            assert!(ProgramList::parse(&[bad]).is_err(), "{bad}");
        }
        // Spaces inside a name are fine: plenty of programs have them.
        assert!(list(&["Some Game.exe"]).is_blocked("some game.exe"));
    }

    #[test]
    fn what_windows_needs_cannot_be_listed() {
        // Directly, in any case, and by wildcards that reach one of them.
        for bad in ["explorer.exe", "Explorer", "svchost", "*", "*.exe", "s*", "*host*", "bullseye", "msedgewebview2.exe"] {
            let error = ProgramList::parse(&[bad]).err().unwrap_or_default();
            assert!(error.contains("needs"), "{bad}: {error:?}");
        }
        // The message says which entry it is, and one bad entry refuses the lot.
        let error = ProgramList::parse(&["steam.exe", "*"]).err().unwrap();
        assert!(error.contains('*') && !error.contains("steam"), "{error}");
        // Ordinary wildcards are fine.
        assert!(ProgramList::parse(&["steam*", "*game*", "epic*launcher"]).is_ok());
    }

    #[test]
    fn a_protected_program_is_never_blocked_even_if_a_list_somehow_has_it() {
        // A list read back from disk is parsed again, but this is the last line of defence.
        let programs = ProgramList { entries: vec!["explorer.exe".into()], matchers: RegexSet::new(["^explorer\\.exe$"]).unwrap() };
        assert!(!programs.is_blocked("explorer.exe"));
        assert!(!programs.is_blocked("Explorer.EXE"));
    }
}
```

**Checklist for another enforcer** (the code was arranged so this stays small):

1. `state.rs`: add a `bool` field to `Enforcers` (and to `Default`), and **one line in `Enforcers::all()`** saying what it needs (`Uses::...`).
2. If it needs a **new kind of list**: a `Uses` variant in `state.rs`, a field in `block::Lists`, in `block::Block` and in `state::Saved` (with `#[serde(default)]`), an arm in `Lists::has`, and the list in `proto::Request::Start`, `proto::Policy` and `service::start`/`Policy`. A new list type like `programs.rs` carries its own parsing and tests.
3. `tab_closer.rs`: do the work, behind `enforcers.uses(Uses::...)` or the field.
4. `index.html`: a checkbox with `data-enforcer="<field name>"` (and a textarea if it has a list). The checkboxes are read, saved and disabled by their attribute, nothing else to wire.
5. Tests: `state::tests::what_can_run_...` covers the combinations; add the new enforcer to it.

### 6.5 The list of allowed sites (the whitelist)

Under the block list there is a second, shorter list: sites that are **always allowed** and **win over the block list**. It uses the same syntax and the same matcher (`FilterList`), so `*`, `*.example.org` and `example.com/path*` all work in both.

- **The rule** (`block::Active::blocks_site`): a page is closed when it is on the block list **and not** on the allowed list. So `*` on the block list with `google.com` allowed blocks every site but that one; an allowed entry is exactly as wide as it is written (`google.com` does not allow `accounts.google.com`, `*.google.com` does).
- **Any combination works**: both empty (nothing blocked), only a block list, a block list with exceptions, `*` with exceptions, and allowed `*` (nothing is blocked whatever the block list says). **Allowed sites alone are not something to block**: with only the tab enforcers on and an empty block list, the block is refused like before, because there is nothing to make exceptions to. The block list is what the enforcer needs (`Uses::Sites`); the allowed list is not an enforcer's own need.
- **It travels everywhere the block list does**: `Request::Start.allowed`, `Policy.allowed` and `Status.allowed` (all `#[serde(default)]`, so older messages and files read as empty), `state::Saved.allowed` (saved with the block and back after a restart), `block::Lists`/`Active`, and the app's cached policy. `service::start` refuses an allowed entry that cannot match, like a bad block entry.
- **UI**: a textarea under the block list ("Always allowed"; all three lists are 5 lines tall), shown again for a running block, locked while a block runs.
- Not covered by the allowed list: programs (`programs.rs` has its own list and its own protected list) and the Obsidian enforcer.

### 6.6 Speed, weight and the window

Measured on a release build of this branch (341 running programs, 2 browser windows):

| | Before | After |
|---|---|---|
| Tab sweep, per second while a site block runs | 93 to 105 ms (up to 413 ms with 3 windows) | **4 to 10 ms** |
| Process scans per second (Obsidian + programs) | 2 x 6 ms | 1 x 6 ms |
| The app with its window closed, idle | 372 MB across 7 processes (6 WebView2) | **20 MB, 1 process** |
| Opening the window after it was destroyed | n/a | about half a second |
| Pipe round trip, the app's per-second request | 0.1 ms | 0.1 ms |
| Password check (PBKDF2) | 68 ms | 68 ms |

**The window** (`ui.rs`): the web view behind it is almost all of the app's memory, and the blocking never needs it. So the app starts in the tray **with no window** (`"windows": []` in `tauri.conf.json`). A click on the tray icon makes the window (from a thread of its own, because making a window inside the event handler that asked for it can deadlock); it is **shown once the page has drawn** (the page calls `window_ready`), so there is no blank window. The X only **hides** it (it reopens at once); after **5 minutes hidden** it is destroyed and the memory goes back. `RunEvent::ExitRequested` without an exit code is refused, so that the last window going away does not end the app, while the unlock password's `app.exit(0)` is allowed. The page learns of hiding and showing through a `visibility` event and stops polling the service while hidden.

**Trying it**: debug builds have aids, `BULLSEYE_SHOW=1` (open the window at start), `BULLSEYE_REOPEN_AFTER_SECS=n` (open it again after n seconds, like another tray click) and `BULLSEYE_UI_TIMEOUT_SECS=n` (a shorter keep time). With the last two set to a few seconds you can watch the web view processes appear, disappear and come back in Task Manager. This was checked with the real app: no web view at start; window visible, hidden by the X with the web view kept, destroyed after the timeout, recreated and visible within two seconds.

```rust
//! The window: made when it is asked for, hidden (not closed) when it is dismissed so that it
//! comes back at once, and destroyed once it has stayed hidden for a while.
//!
//! The web view behind the window costs about 300 MB and half a dozen extra processes, and the
//! app is only looked at now and then; the blocking never needs the window. So the app starts in
//! the tray with no window at all, and gives the memory back after a quiet spell. A window that is
//! looked at in bursts (check, start a block, look again a minute later) reopens instantly; the
//! first opening after a long pause takes about half a second.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window};

use crate::sync::{spawn_critical, LockExt};
use crate::watchdog::note;

const LABEL: &str = "main";
const WIDTH: f64 = 340.0;
const HEIGHT: f64 = 850.0;

/// How long a dismissed window is kept before it is destroyed.
const KEEP_HIDDEN: Duration = Duration::from_secs(5 * 60);

/// Whether, and since when, the window is hidden.
pub struct Ui {
    hidden_since: Mutex<Option<Instant>>,
    keep_hidden: Duration,
}

impl Ui {
    pub fn new() -> Ui {
        // Debug builds can shorten the wait, to try the destroying and reopening.
        #[cfg(debug_assertions)]
        let keep_hidden = env_seconds("BULLSEYE_UI_TIMEOUT_SECS").unwrap_or(KEEP_HIDDEN);
        #[cfg(not(debug_assertions))]
        let keep_hidden = KEEP_HIDDEN;
        Ui { hidden_since: Mutex::new(None), keep_hidden }
    }

    fn hidden(&self, now: Instant) {
        *self.hidden_since.locked() = Some(now);
    }

    fn shown(&self) {
        *self.hidden_since.locked() = None;
    }

    /// Whether the window has been hidden long enough to be destroyed.
    fn expired(&self, now: Instant) -> bool {
        self.hidden_since.locked().is_some_and(|since| now.saturating_duration_since(since) >= self.keep_hidden)
    }
}

/// Opens the window: shows the one there is, or makes a new one. A new one is shown by the page
/// itself, once it has drawn (`window_ready`), so there is no blank window in between.
pub fn show(app: &AppHandle) {
    match app.get_webview_window(LABEL) {
        Some(window) => reveal(&app.state::<Ui>(), &window),
        None => {
            if let Err(e) = create(app) {
                note("app", &format!("cannot open the window: {e}"));
            }
        }
    }
}

/// `show`, from a thread of its own: making a window from inside the event handler that asked for
/// it (a click on the tray icon) can deadlock.
pub fn show_in_background(app: AppHandle) {
    thread::spawn(move || show(&app));
}

fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Bullseye")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .transparent(true)
        .effects(EffectsBuilder::new().effect(Effect::Mica).build())
        .visible(false)
        .build()
}

fn reveal(ui: &Ui, window: &WebviewWindow) {
    ui.shown();
    let _ = window.show();
    let _ = window.set_focus();
    // Tells the page to look at the service again (it stops while it is hidden).
    let _ = window.emit("visibility", true);
}

/// Called by the page once it has drawn for the first time.
#[tauri::command]
pub fn window_ready(window: WebviewWindow, ui: State<Ui>) {
    reveal(&ui, &window);
}

/// The window's X: hides it, and starts the clock after which it is destroyed.
pub fn dismiss(window: &Window) {
    let _ = window.hide();
    let _ = window.emit("visibility", false);
    window.app_handle().state::<Ui>().hidden(Instant::now());
}

/// Destroys the window once it has been hidden long enough.
pub fn spawn_reaper(app: AppHandle) {
    let ui = app.state::<Ui>();
    let every = (ui.keep_hidden / 4).clamp(Duration::from_secs(1), Duration::from_secs(10));
    spawn_critical("window reaper", move || loop {
        thread::sleep(every);
        let ui = app.state::<Ui>();
        if ui.expired(Instant::now()) {
            if let Some(window) = app.get_webview_window(LABEL) {
                let _ = window.destroy();
            }
            ui.shown();
        }
    });
}

#[cfg(debug_assertions)]
fn env_seconds(name: &str) -> Option<Duration> {
    std::env::var(name).ok()?.parse().ok().map(Duration::from_secs)
}

/// Development aids, debug builds only. `BULLSEYE_SHOW=1` opens the window at start (otherwise
/// click the tray icon), `BULLSEYE_REOPEN_AFTER_SECS=n` opens it again n seconds later, as another
/// click on the tray icon would, and `BULLSEYE_UI_TIMEOUT_SECS=n` shortens how long a hidden
/// window is kept.
#[cfg(debug_assertions)]
pub fn debug_aids(app: &AppHandle) {
    if std::env::var_os("BULLSEYE_SHOW").is_some() {
        show_in_background(app.clone());
    }
    if let Some(after) = env_seconds("BULLSEYE_REOPEN_AFTER_SECS") {
        let app = app.clone();
        thread::spawn(move || {
            thread::sleep(after);
            show(&app);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_window_is_destroyed_only_after_its_time() {
        let ui = Ui { hidden_since: Mutex::new(None), keep_hidden: Duration::from_secs(300) };
        let start = Instant::now();
        // Never hidden (or shown): nothing to destroy.
        assert!(!ui.expired(start + Duration::from_secs(1000)));

        ui.hidden(start);
        assert!(!ui.expired(start + Duration::from_secs(299)), "reopening within the time is instant");
        assert!(ui.expired(start + Duration::from_secs(300)));
        assert!(ui.expired(start + Duration::from_secs(3000)));

        // Opened again: the clock is gone, and starts over the next time it is dismissed.
        ui.shown();
        assert!(!ui.expired(start + Duration::from_secs(3000)));
        ui.hidden(start + Duration::from_secs(3000));
        assert!(!ui.expired(start + Duration::from_secs(3100)));
        assert!(ui.expired(start + Duration::from_secs(3300)));
    }
}
```

## 7. Update `CLAUDE.md`

The project notes describe the old design and are used by every future session, so update them in the same change. Replace these parts (wording is a suggestion; keep the style of the file):

**File list**: remove `task.rs`; add:

- `src-tauri/src/service.rs` - the running SYSTEM service (service manager, start-up, data folder); `handler.rs` - what it does with each request (same code over the pipe and, in debug builds, inside the app); `registration.rs` - installing, locking down and removing it
- `src-tauri/src/sync.rs` - `locked()` (locks that survive a crash) and `spawn_critical` (a crash in a long-lived thread ends the process, so Windows restarts it)
- `src-tauri/src/ui.rs` - the window: made on demand, hidden by the X, destroyed after 5 minutes hidden (the web view is almost all of the memory)
- `src-tauri/src/filters.rs` – site lists: plain address prefixes plus `*`, used for the block list and for the allowed list (which wins over it); only web pages are ever matched, never the browser's own pages
- `src-tauri/src/programs.rs` – the programs to close: exe names with `*`, a protected list that is never closed
- `src-tauri/src/pipe.rs`, `proto.rs`, `client.rs` – the named pipe `\\.\pipe\bullseye`: server, messages, and the app's side (keeps the last policy in memory)
- `src-tauri/src/watchdog.rs` – the service's watch over the app (starts it as the logged-on user, at logon and while a block runs; decides which process is the app and ends one that stops asking for its policy)

**Replace "Surviving a restart, a kill, and a reboot" with:**

> The `bullseye` Windows service runs as SYSTEM and starts at boot. It holds the block (`%ProgramData%\bullseye\state.json`, SYSTEM-owned), internet time and the unlock hash. The tray app (user session) asks it every second what to enforce and does the tab closing; it holds no state, and keeps enforcing the last answer if the service is briefly unreachable.
> - End the app in Task Manager while a block (timed or not) is running: the service starts it again within about a second. Suspend it, or stop its closing loop, and it is ended and started again within about 5 seconds.
> - The service, not the process name, decides which process is the app: the installed exe, in the console session, that it started or that registered itself. A copy under another name is not the app.
> - End the service: Windows refuses ("Access denied") for administrators; if it fails by itself, Windows restarts it at once (failure actions).
> - Reboot mid-block: the service reads `state.json` and resumes with the time left.
> - The service starts the app at every logon, also without a block. "Unlock" clears the block and quits the app; the service keeps running idle and starts the app again at the next logon.

**Replace the "Deliberately not hidden" and "honest ceiling" paragraphs' technical content:** still nothing is hidden (service named `bullseye` in the services list, app in Settings > Apps, logs in `%ProgramData%\bullseye\service.log`). The honest ceiling is now: an administrator who knows how can still get around it: run a shell as SYSTEM (for example PsExec `-s`), take ownership of the service object or the data folder (administrators hold `SeTakeOwnershipPrivilege`) and rewrite its access list, delete the service from the Windows recovery environment, or boot with an offline registry edit. NTP spoofing needs admin as before. The password hash is in the binary, so a short password can be brute-forced offline. **What changed is the effort**: a normal administrator now needs one of those, instead of "End task".

**Replace "Install / remove":** per-machine NSIS installer, needs admin (UAC), installs to `C:\Program Files\Bullseye`. Hooks: `--prepare-update` (before replacing files; refused during any block), `--install` (register + lock down + start the service), `--uninstall` (service removes itself and ends the app; refused during a timed block, exit code 2; if the service is registered but does not answer or does not end, exit code 1 also stops the uninstall; with no service registered it carries on).

**Dev workflow**: see section 8.1 below, and drop the sentence about `restore_task` / re-enabling the login task.

## 8. Development, recovery, and things to know

### 8.1 Developing without installing a service

**`npm run dev` works on its own.** In a debug build the app runs the service's logic inside itself (`main::remote_for_this_run` gives it `Remote::local(service::embedded())`): no installed service, no pipe, no administrator rights, and nothing that has to stay alive. Kill the app whenever you like; it is only enforcing for as long as it runs.

- The same rules apply as with the real service: the same `handle` function answers every request, so a timed block cannot be stopped early, the unlock password ends it, and starting is validated the same way. Only the transport differs (`Remote::send`).
- What is *not* there: the SYSTEM service itself, the watchdog that restarts the app (and the heartbeat, `AppSlot`), the locked-down data folder, the pipe and its access list. Test those with a real install (section 9.2), or the ignored pipe test for the pipe itself (section 9.1).
- The block is kept in `%LOCALAPPDATA%\bullseye-dev`, apart from a real install's files. A timed block is therefore still there the next time you start the app; delete that folder to forget it.
- **Release builds do not contain any of this.** It is compiled in only with `debug_assertions` (`service::embedded`, `Remote::local`), and `cargo check --release` is warning-free. Keep it that way: never make a release build able to be its own service.
- Unlocking quits the app, which ends `tauri dev`, as before.
- **There is no window until you click the tray icon** (or set `BULLSEYE_SHOW=1`, section 6.6). The page itself is baked in at build time: a change to `src/index.html` alone does not relaunch `tauri dev`; touch `src-tauri/src/main.rs` to force it.

To talk to a real, installed service from a debug build (in a virtual machine), set `BULLSEYE_USE_SERVICE=1` before `npm run dev`. The app then uses the pipe like a release build does, including the session-0 check on who serves it. There is no separate "console service" to run: the pipe itself is covered by the ignored test `the_pipe_answers_like_the_app_expects` (`cargo test -- --ignored the_pipe`, with no service installed), which serves from the test's own process and is the only place the session-0 check can be skipped (`cfg!(test)` in `pipe::verify_server`).

### 8.2 Recovery: when your own test locks you out (read this before testing on a real PC)

The service you install is, by design, not stoppable by you. If you install it on your own machine and start a **timed** block, you cannot uninstall until it ends. So:

- **Test installs in a Windows virtual machine** and take a snapshot first. It is the only comfortable way back.
- On a real machine only use short blocks (`5s`, `10s`), and know that the unlock password is `test1234`.
- If it still goes wrong, as an administrator, run a shell as SYSTEM, for example with Sysinternals PsExec: `psexec -s -i cmd`. In that shell:

```bat
sc delete bullseye
taskkill /f /im bullseye.exe
reg delete "HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Minimal\bullseye" /f
reg delete "HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Network\bullseye" /f
rmdir /s /q "%ProgramData%\bullseye"
```

  (`sc delete` comes first: it marks the service for deletion, so Windows does not start it again when the `taskkill` ends the process.)
- No PsExec? As administrator you can borrow Task Scheduler to get a SYSTEM shell: save the five lines above as `C:
ecover.cmd`, then run `schtasks /create /tn recover /sc once /st 00:00 /ru SYSTEM /tr C:
ecover.cmd /f`, `schtasks /run /tn recover`, and afterwards `schtasks /delete /tn recover /f`.
- Last resort: boot into the Windows recovery environment's command prompt (which runs as SYSTEM) and delete the registry keys / files there.

### 8.3 Behaviour changes the user will notice

- Installing and updating asks for administrator rights (UAC). The install folder is `C:\Program Files\Bullseye`.
- **Migration:** an existing per-user install lives elsewhere and does not know `--prepare-update`: uninstall it first (Settings > Apps). Do not install the new version over it: the pre-install hook would run the *old* exe with an unknown argument, which starts the old tray app and waits for it forever.
- The login task is gone. "Unlock" no longer disables anything: it clears the block and quits the app, and the app returns at the next logon.
- `launcher.log` is replaced by `%ProgramData%\bullseye\service.log` (readable by administrators) and `%LOCALAPPDATA%\bullseye\app.log`.
- Updating is refused during any block (timed or not); uninstalling during a timed block (as before), and also when the service is registered but does not answer, in which case nothing is removed (see `service.log` and section 8.2).

### 8.4 Known limits (do not hide these in the UI)

- **The honest ceiling above still holds**: this makes bypassing take real effort (SYSTEM shell, ownership change, recovery environment). It does not make it impossible. Do not claim otherwise in the UI.
- **The unlock hash is in the binary** (`unlock.rs`, unchanged): a short password can be brute-forced offline, the new rate limit only slows *online* guessing through the pipe. Use a long password.
- **Only the console session gets the app.** On a PC with several users logged on (fast user switching), the app is started for whoever is at the screen; another user's session is not watched. Remote Desktop sessions are not watched either.
- **Which token the app gets.** `WTSQueryUserToken` normally returns the user's *limited* (non-elevated) token, like the old scheduled task (`LeastPrivilege`). Check once (section 9.2) that the app is not elevated.
- **Who restarts the service**: Windows (failure actions, at once) after a crash or kill, and the app (start only, every 5 s) if the service is not running. A service that ended with exit code 0 (an update or uninstall request) is not restarted by Windows; the app or the installer starts it again. While it is down the app keeps enforcing the last policy it got (in memory), but an app that is restarted meanwhile has none.
- **The access list is set once, when the service is created.** An update only starts the existing service, and administrators cannot change the list, so an install made before the interactive-users start right keeps the old list. Uninstall and install again (in the VM) to pick it up.
- **The service restarts the app while any block runs, timed or not**, and once after each logon. With no block it leaves the app alone, so quitting it with the unlock password sticks until the next logon.
- **Browser updates can break the tab closing.** It relies on UI Automation details of the browsers: the window class `Chrome_WidgetWin_1`, the address bar class `OmniboxViewViews`, the page container `MultiContentsView`, and the tab classes (`Tab` in Chrome and Brave, `EdgeTab` in Edge). Edge's tab class was found to differ only by testing with a real window. After a major browser update, run the manual check (`a_real_browser_tab_is_closed`, section 9.2).
- **Offline time** works as before (`clock.rs`), but now the clock lives in the service.
- **Code signing.** The service runs the exe with SYSTEM rights, so a signed build is worth having; unsigned, SmartScreen and antivirus tools may complain at install. Not part of this change.

## 9. Testing

### 9.1 What was already tested, and how to re-run it

```powershell
cd src-tauri
cargo test                               # 67 tests: block, clock, filters, programs, state, unlock, proto, client, service, watchdog, tab_closer
cargo test -- --ignored the_pipe --nocapture   # starts the real pipe server in the test process, ~15 s; no service may be running
```

The ignored test talks to the real Windows named pipe: status, start, policy, stop refused, uninstall refused during a block, an oversized length, six simultaneous clients, a silent client that is cut off after 10 s, then unlock and an update request.

### 9.2 What only you can test (needs a real service; do it in a VM)

Build with `npm run build` (installer in `src-tauri\target\release\bundle\nsis\`). The NSIS script was compiled once in the scratch copy to check the hook syntax; it was **not** run.

| # | Test | Expected |
|---|---|---|
| 1 | Run the installer | UAC prompt; service `bullseye` exists (`sc qc bullseye`), state `RUNNING`; the tray app appears for you |
| 2 | Task Manager > Details, the app's row, "Elevated" column | `No` (see 8.4) |
| 3 | `sc sdshow bullseye` | the SDDL from `service.rs` |
| 4 | As admin: `sc stop bullseye`, and "End task" on the service in Task Manager | both fail |
| 5 | `icacls "%ProgramData%\bullseye"` and try to edit `state.json` as admin | SYSTEM full, Administrators read only; editing fails |
| 6 | Start a `10m` block, then end the *app* in Task Manager | it returns within about a second, block continues |
| 6b | Same with an *untimed* block (empty duration) | it returns within about a second too; after "Stop block" it no longer does |
| 7 | Log off and on during the block | app returns, block continues with the remaining time |
| 8 | Reboot during the block | block resumes after boot |
| 9 | Boot into safe mode with networking during the block | service runs (check `sc query bullseye`) |
| 10 | (Removed: there is no update path. Bullseye is only installed on a clean machine; to try a new build, uninstall first.) | |
| 11 | Uninstall during the timed block | refused (exit code 2), message shown |
| 12 | Wrong unlock password, then the right one | the wrong one is refused with "Wrong password" at once (there is no slowdown); the right one ends every block and the app quits |
| 13 | After unlocking: log off and on | the app returns (idle) |
| 14 | End the block, uninstall, install a new build | installer succeeds; service runs the new exe |
| 15 | Uninstall without a block | service, `%ProgramData%\bullseye` and the safe-mode keys are gone |
| 16 | Open the pipe with a script and send garbage / a huge length / nothing | service keeps answering the app |
| 17 | Kill the service with a SYSTEM shell (`psexec -s`) | Windows restarts it at once; app keeps enforcing meanwhile |
| 18 | Uninstall while the service is registered but not answering (in the VM, suspend the service process with a SYSTEM shell tool first) | the uninstall stops with "did not respond, so nothing was removed" (exit code 1); no files are deleted |
| 19 | Uninstall after the service was deleted by hand (`sc delete`, via the recovery steps) | carries on and removes the files (no service registered is exit code 0) |
| 20 | The app starts a stopped service. In a SYSTEM shell turn the restart off (`sc failure bullseye reset= 0 actions= ""`), then `taskkill /f /im bullseye.exe` the service while the app runs | within about 5 s the app log says "the service was not running; started it" and the service runs again. **Put the restart back afterwards**: `sc failure bullseye reset= 86400 actions= restart/0/restart/0/restart/0` |
| 21 | Copy `bullseye.exe` to another folder under another name and run it during a block, then end the real app in Task Manager | the service still starts the real app within about a second (the copy was never taken in) |
| 22 | During a block, suspend the real app (Process Explorer, or `pssuspend` from Sysinternals) | within about 5 to 6 seconds `service.log` says "the app stopped asking for its policy; ending it", and a new app starts |
| 23 | Start `bullseye.exe` by hand while the app runs | the second one exits at once; `app.log` says "not starting: Bullseye is already running" |
| 24 | During a block, put the machine to sleep for a minute and wake it | the app is not ended because of the sleep (no "stopped asking" line in `service.log`) |
| 25 | Put `steam*` (or a program you have open, say `notepad`) in the Programs box and start a block | the program gets a close request, and is ended about 3 seconds later if it is still running; `app.log` is not written, but the program is gone |
| 26 | Put `*` or `explorer.exe` in the Programs box and start | refused with "Would close programs Windows or this app needs" |
| 27 | Start a block with only Obsidian ticked and both lists empty | it starts; with only the programs box ticked and no programs it is refused |
| 28 | Restart the PC during a block with programs | after the restart the block is back and the Programs box shows them |
| 29 | Block list `*`, allowed `google.com`; start | every tab is closed except google.com pages; the new-tab page, `chrome://settings` and the like stay open |
| 30 | Block list `youtube.com`, allowed `youtube.com/watch*` | the YouTube front page and shorts close, a watch page stays |
| 31 | Allowed list filled, block list empty, only the tab options ticked | refused (nothing to make exceptions to) |
| 32 | Open a Google redirect page (a result link that goes through `google.com/url?q=https://...`) with a block entry for the target site | the redirect page itself is not closed because of the target in its query |
| 33 | Open an Edge window on a blocked site (block entry `example.com`, `msedge --new-window https://example.com`) | the tab is closed within a second or two (this was broken before: Edge's tabs are `EdgeTab`). Chrome and Brave too |
| 34 | Start the app and look at Task Manager > Details | only `bullseye.exe`, about 20 MB, and no `msedgewebview2.exe` of its own; click the tray icon: the window opens in about half a second and the web view processes appear |
| 35 | Close the window with the X, wait 5 minutes | the `msedgewebview2.exe` processes are gone; clicking the tray icon makes the window again; closing and reopening within 5 minutes is instant |
| 36 | During a site block, watch `bullseye.exe` CPU in Task Manager | a few percent of one core at most (it was about 10% with two browser windows) |
| 37 | Make a block list "Social" (`example.com`), a schedule with the current hour marked and the next not, and a block of both with Lock: Duration `2h` on its tile; Start | the start asks first, in words; the tab closes now; once the marked hour is over the tile says "Inactive by schedule", `example.com` stays open, Stop stays greyed out; after 2 hours the block is gone |
| 38 | During test 37, change Windows' time zone (Settings > Time & language) by several hours | nothing changes: the running block keeps the time zone it started with |
| 39 | During test 37, restart the PC | the block is back with its schedule and the same time left |
| 40 | During test 37, open "Social" and its schedule from their pages | both editors are read-only with "“…” uses it and is running"; their tiles show a lock icon and "In use by a running block"; Delete is refused |
| 41 | The same block with Lock: Schedule, "Locked during" its own schedule (which marks the current hour); Start | it cannot be stopped during the marked hour ("locked by its lock schedule, for another …"); after it, Stop works and the tile says when it locks again; uninstall is refused during the marked hour only |
| 42 | A block with schedule "Always" and Lock: Schedule with another schedule (current hour marked); Start | it blocks all the time but cannot be stopped during the marked hour only; that schedule's tile says it is in use, and it cannot be edited or deleted while the block runs |
| 43 | In the schedule editor, drag from Monday 08:00 to Friday 16:30, then start a drag on a marked cell | the drag marks the rectangle and the line under the grid says `Active Mon-Fri 08:00-17:00`; the second drag clears what it covers |
| 44 | Lock: Password on a tile; type a password and a different one under Repeat password; then the same | with different ones "The passwords differ" and Start says the lock is not saved; with the same the lock is saved and the fields empty. Start, then Stop: a modal asks for the password; a wrong one is refused, the right one stops it |
| 45 | In a terminal: `bullseye-cli list new T --site example.com`, `bullseye-cli new T --list T`, `bullseye-cli start T --lock-duration 10m` (answer y), `bullseye-cli status`, `bullseye-cli stop T` | everything appears in the window; status shows "locked, 9 minutes … left"; stop is refused with the time left (exit code 1) |
| 46 | `bullseye-cli unlock`, type the password | not echoed; "Unlocked: every block has ended, and Bullseye closes." (exit code 0); within about a second the tray icon is gone and `app.log` says "stopped by the unlock password"; it comes back at the next sign-in |
| 47 | `bullseye-cli list new X --program explorer` | refused with "Would close programs Windows or this app needs" (exit code 1); `bullseye-cli new X --colour red` is exit code 2 |
| 48 | Lock: Until, today a few minutes from now; Start | the start says the time written out and how far away; Stop is greyed out; the block ends at that minute. A time that has passed is refused in the form, and at Start |
| 49 | Lock: Until restart; Start; then sign out and in, and end the service with a SYSTEM shell | the block stays and stays locked; uninstall is refused |
| 50 | During test 49, Restart Windows | after the restart the block is gone (Shut down with Fast Startup on keeps it: Windows hibernates) |

If test 1 fails: read `%ProgramData%\bullseye\service.log` (administrators can read it) and the installer's error message; `%LOCALAPPDATA%\bullseye\app.log` has the `--install` output.

## 10. Where the risk is

If you get stuck or unsure, these are the places to ask for a review before merging, in this order:

1. `registration::install`: the SDDL and the failure actions. A wrong SDDL can lock out the installer itself.
2. `watchdog::launch_app`: it hands out the user's token. It must never start the app as SYSTEM.
3. `pipe.rs`: the access list and the checks. A hole here lets a standard user talk to a SYSTEM process.
4. The installer hooks: this was the one part checked only by reading the generated script, not by running it.

## 11. Several blocks, and the Carbon UI (later change)

Sections 1 to 10 describe one block at a time. Since then the app keeps any number of named block
**definitions** and runs each on its own; the code is the reference, this is the summary.

- `definitions.rs`: `Definition { id, name, filters, allowed, programs, enforcers, schedule }`, kept by
  the service in `%ProgramData%\bullseye\blocks.json`. `Schedule` is a tagged enum
  (`{"kind":"duration","duration":"1h"}`), so other kinds of schedule can be added without changing
  the file format. A definition is checked when saved (name, id, lists, schedule) and again when
  started (something to do).
- `block.rs`: `Blocklist` holds a list of running blocks, keyed by the definition's id. Starting copies
  the definition; a running block cannot be started again, and its definition cannot be saved or
  deleted until it ends. Each timed block ends by itself; `stop` refuses a timed one. The unlock
  password ends them all. `Active` is every running block's part; `blocks_page`, `blocks_program`,
  `closes_obsidian` combine them, each block going by its own enforcers and its own allowed sites.
- `state.json` is now `{"blocks":[Saved…]}`; `Saved` has `id` and `name`. A file from before (one
  block, no id) still loads, as a block with an empty id that the UI shows as "Earlier block".
- The pipe protocol: `Save { block }`, `Delete { id }`, `Start { id }`, `Stop { id }`; `Status` answers
  an `Overview { blocks, running, clock_synced }`; `Policy` answers one policy per running block.
  The installer rules are unchanged in spirit: uninstall is refused while any timed block runs,
  update while any block runs.
- The UI is Carbon web components (see `CLAUDE.md`, "Design system"), vendored in `src/vendor/`; the
  window is resizable and the page responsive. Unlock moved to a Settings page.

## 12. Schedules, lock methods and the command line (later change)

The code is the reference; this is the reasoning.

**Three layers.** A block is now *what* it enforces (lists and enforcers), *when* it is active (its schedule, `definitions::Schedule`: `always` or `weekly`) and *how* it is locked (its lock method, `lock::Lock`: `none` or `timer`). The old duration "schedule" was really a lock: it kept a block from being stopped and ended it when the time was up. It is now `Lock::Timer`; a definition that still has `{"kind":"duration"}` (an older `blocks.json`, or an older client) is turned into a timer lock as it is read or saved (`Definition::upgraded`). The schedule is independent of the lock, so any lock works with any schedule, and both are fixed while the block runs, like the enforcers, because changing either would be a way around a lock.

**What a schedule does.** Outside its schedule a running block is still there: still locked, the service still keeps the app alive (`Blocklist::is_running`), and an update is still refused. Only its part of the policy is left out (`Blocklist::active` filters by `Block::in_schedule`), so other blocks go on. A timer lock that ends outside the schedule ends the block as before. The user's example: schedule 08:00-17:00, block started at 16:00 with a 2-hour timer: blocking until 17:00, everything accessible from 17:00, block gone at 18:00 (a unit test does exactly this).

**Time.** Schedules are evaluated on the service's internet clock, converted to local time with `SystemTimeToTzSpecificLocalTimeEx` in the time zone Windows had when the block started. That zone is stored by its key name (`state.json` `zone`) and looked up again after a restart (`EnumDynamicTimeZoneInformation`). Following Windows' current zone would let a standard user, who may change the time zone, move the schedule out of the way. If local time cannot be computed the block enforces (it errs on the side of blocking). Tests use a fixed UTC zone and a test clock set with `Clock::at_least`.

**Format.** A week is per weekday, Monday first, spans `[start, end)` in minutes since midnight, merged and sorted by `Week::normalized` (which also refuses a week with no time in it). The text form (`Mon-Fri 08:00-17:00; Sat 10:00-12:00`) is parsed and written in Rust only; the page asks for it through two local commands (`parse_schedule`, `describe_schedule`), so the week grid, the Times field, the tile summaries and the CLI cannot disagree. The grid works in half hours; dragging rewrites only the days it covers, so a typed schedule with finer minutes survives on the other days. Carbon has no calendar or time-grid component, so the grid is the one hand-made control (layout in Carbon tokens, `aria-hidden`; the Times text field is the accessible equivalent).

**Durations and words.** Input stays short and may be combined (`2h 5m`, `1d12h`, or words). Everything shown, in the window, in the service's messages and in the CLI, is written out (`2 hours and 5 minutes`; units that are zero are left out, at most the three largest from the first that is not) by `duration::words` alone: the running blocks' times come in the status already in words (`remaining_words`, `schedule_change_words`), and a lock's typed length is described by the local command `describe_duration`. The page has no duration code of its own.

**One implementation for the window and the CLI.** Nothing a block does is decided in either: both send the same `proto::Request`s through the same `client::Remote` (the CLI's is `Remote::for_cli`, which also waits for a service it had to start), the service checks and does everything (`handler.rs`), and texts the backend knows (schedules, durations) come from the same Rust functions. What each has of its own is only presentation: the page's tiles and editor, the CLI's argument parsing and plain-text output, and each asking before starting a locked block or deleting one.

**The CLI.** `bullseye-cli.exe` is a second binary of the package (console subsystem; the tray exe is a GUI-subsystem program and cannot print to a terminal). To share the code, the crate became a library (`lib.rs`, named `bullseye_core` because a library and a binary of the same name collide on Windows) with two thin binaries. Tauri's bundler installs every binary of the package next to the main one, and `default-run` keeps `tauri dev` on the tray app. The CLI talks to the service over the pipe like the app does, so the service's checks apply unchanged; it may start a registered service that is not running, like the app. `cli::GUI_COMMANDS` maps every command in `generate_handler!` to the CLI commands that do the same, and a test reads `lib.rs` and fails when the window gains a command the CLI does not have, or when the help or `CLI.md` miss one. See `CLI.md` for its use.

**Unlock from the CLI closes the app too**, for consistency with the window. The CLI cannot end the app itself (and should not need to know which process it is), so the service does it: a successful unlock, from anywhere, sets a short window (10 s) in which the app's policy request is answered with `Response::Quit`, and the app exits through Tauri as after the window's Unlock. The window is short so that an app started at the next sign-in is not sent away, and it is cleared when a block starts, so a block started right after an unlock gets its app.

## 13. Block lists, schedules and blocks as three lists (later change)

The three layers of section 12 became three kinds of things with lists and pages of their own, so that they can be reused: a block list ("Social media") or a schedule ("Work hours") is defined once and used by any number of blocks, and a block is a combination of one or more block lists, a schedule (or always) and a lock. The block is what is started.

**Locks have no page.** A lock is either none, a duration or "while active", so it is part of the block and set on its tile, next to Start (a `cds-select` and, for a timer, a duration field; saved with the block as soon as it is chosen). `start` in the CLI takes the same lock options. **Locked while active** (`Lock::WhileActive`) is locked exactly while the block's schedule has it active and can be stopped outside those times, so a weekly block can run indefinitely and still not be stoppable during work hours. It needs a weekly schedule (with "always" only the password would end it); `definitions` and `handler::start` both refuse it otherwise, and the tile greys the option out.

**What is fixed while a block runs.** A running block is a snapshot of its block lists and schedule (`block::Part`s, the week), so editing them could not weaken it, but the window would then show something else than what runs. So the service refuses changes to a running block and to every block list and schedule a running block uses (`handler::edit`, which names the block), and refuses deleting a list or schedule that any block uses (`Definitions::delete_list`). A block that is not running may still change which lists it uses. `edit` and `start` share one lock, so a list cannot change while a block using it is being started.

**Several lists per block.** Each list keeps its own enforcers and allowed sites in the running block (`Part`), and `Active` simply holds all parts of all active blocks, so the app's enforcing code is unchanged: an allowed site of one list is not an exception to another list's sites.

**Migration.** `blocks.json` was a list of all-in-one blocks. It is read as such when it is not today's `Library` object (`OnDisk`, untagged) and turned into a block list and a block per old block (same id and name), one named schedule per different weekly schedule, and a timer lock for an old duration. `state.json` (running blocks) keeps reading the one-list form of older versions (`Saved::parts`).

**UI.** One generic page (tiles) and one generic editor serve the three kinds, described in `KINDS`; the forms are `blockForm`, `listForm` and `scheduleForm`. The header has four icon actions; Carbon's side nav would be the next step if there are ever more pages.

## 14. Lean again: no old formats, password and scheduled locks (later change)

**No old formats and no updates.** Bullseye is only ever installed on a clean machine, so everything that read older files or carried a block across an update is gone: `blocks.json` is read only as today's `Library`, `state.json` only as today's `{"blocks": [...]}`, there are no serde defaults for fields older versions lacked, and `--prepare-update`, `Request::PrepareUpdate` and the installer's PREINSTALL hook are removed. Running the installer over an installed copy is not supported; uninstall first.

**No slowdown on wrong passwords.** The unlock password (and a password lock's) is checked as given; the lockout after three wrong guesses is gone. A PBKDF2 check of 600 000 rounds already costs a moment per guess, and offline guessing never went through the service anyway (the hash is in the binary).

**Password lock** (`Lock::Password`). Meant for someone else to set: the block runs until stopped with that password, or with the unlock password. The client sends the password once (`password`), the service turns it into a salted PBKDF2 hash at once (`unlock::new_hash`, `salt$hash` in hex, a random salt per lock) and keeps only that in `blocks.json`, and the running block keeps the hash in `state.json`. Neither the password nor the hash is ever sent back (`Library::redacted`); a password lock saved without a new password keeps its hash. It is asked for twice (Password and Repeat password), as nothing ever shows it again: a typo would leave a lock only the unlock password opens; it is saved only when both are the same. Stop on such a block opens the modal with a password field (`ask({password: true})`); the CLI's `stop` asks for it when it is not given with `--password`.

**Scheduled lock** (`Lock::Scheduled { schedule }`) replaces "while active". The block cannot be stopped while a schedule of its own choosing is active, and can outside those times; otherwise it runs until stopped. With the block's own schedule it is exactly the old "locked while active"; with another one, a block can for instance block all the time but be stoppable only in the evenings. The lock's schedule is resolved when the block starts (`Resolved::lock_week`) and fixed in the running block (`lock_schedule`, read in the block's pinned time zone), and it counts as used by the block (`Library::users_of`): it cannot be deleted while a block uses it, nor changed while that block runs. The status gives `lock_change` / `lock_change_words`, the time until it locks or frees the block. In the window it is a `cds-select` of the schedules ("Locked during"), defaulting to the block's own schedule; the CLI option is `--lock-schedule <schedule>`.

**Lock settings in the modal.** The tile only holds the lock's method (a `cds-select`), next to Start; the current lock's settings are changed from the tile's ⋮ menu ("Lock settings": not "Change lock", which would mean another method), as Edit and Delete are. An edit icon beside the select looked lost between the select and Start. A method with settings (a timer's length, a lock schedule, a password twice) asks for them in the one modal (`ask` with `fields` and `read`; `LOCKS[method].form`), which stays open while they are wrong and marks the field with Carbon's invalid state; Cancel keeps the lock the block had. Side by side on a tile the fields were cramped, and a lock is set rarely. A password lock that has a password shows 8 dots in both fields, as settings pages usually do, so it is plain that one is set; they are a stand-in value (`KEPT`), not the password, which the window never has. Entering a field clears them, leaving both empty restores them, and saving them as they are sends the lock without a password, so the service keeps the hash.

**The window's texts in one file.** `src/strings.js` holds every text the page shows, grouped by page (`window.STRINGS`), so the copy can be fine-tuned without reading the script. It is a script rather than JSON so it loads before the page's own script without a fetch (which also works from disk, for checking the page in a browser) and can carry comments saying what each placeholder stands for. `t(path, values)` fills `{placeholders}` and picks `one` / `other` by `{n}`; a missing text shows as its path and is logged. The tables (`LISTS`, `SECTIONS`, `LOCKS`, `KINDS`) take their texts from it. Texts the backend writes stay in Rust, so the window and the CLI keep saying the same.

## 15. Duration, until and until-restart locks; icons on the block tiles (later change)

**Duration** is the timer lock renamed (`{"method":"duration"}`, `--lock-duration`): the name says how it is set, next to the new **until** lock, which locks the same way but is set by its end. An until lock keeps the local date and time as typed (`2026-10-06 18:00`, `datetime.rs`); the block's time zone is pinned when it starts (as for schedules), and the end is computed then (`localtime::Zone::instant`, `TzSpecificLocalTimeToSystemTimeEx`) and kept as an instant on internet time, exactly like a duration lock's end. A time that has passed by then does not start. Keeping it local until the start means the definition reads as the user typed it; `describe_until` (window and CLI, `localtime::until`) writes it out and says how far away it is. The window's form is Carbon's date picker and time picker.

**Until restart** (`{"method":"until_restart"}`, `--lock-until-restart`): the block cannot be stopped and ends when Windows restarts. The service knows a restart by a random id in a volatile registry key (`boot.rs`, `HKLM\SOFTWARE\bullseye-boot`), which Windows forgets when it restarts; a running block keeps the id it started under (`state.json` `boot`), and is not resumed under another. Uptime and boot time were not used: Fast Startup keeps the uptime, and the boot time moves with the Windows clock. A shut down with Fast Startup is not a restart either (Windows hibernates), which the confirmation says. The debug build's embedded service runs as the user and cannot write HKLM, so it uses HKCU.

**Icons on a block tile's lines**: block lists, schedule, lock (closed or open) and, while running, when its schedule turns it on or off, as 16 px Carbon icons (`summaryOf` gives `[icon, text]`).

**Lock settings are kept per method.** A block remembers the last settings of each lock method it has had (`Definition::kept_locks`): choosing Duration again after No lock starts from its 5m, a lock schedule from the one chosen before, a password from the hash it had (shown as the 8 dots, and saved unchanged it is that password again). The service keeps them, not the window, so they survive the window and a client cannot put a hash in: `save_block` builds `kept_locks` from what it had and the block's previous lock, and ignores the client's. A kept lock schedule does not count as a use of that schedule: it can be deleted, and is then forgotten by every block that kept it (`delete_schedule`), so the form starts from the block's own schedule, or the first there is.

**The schedule editor is the week grid alone.** The Times text field above it confused the user and is gone, with the `parse_schedule` command it used; the schedule is written out under the grid instead (`describe_schedule`), and the CLI is where a schedule is typed. The hour labels are centred on the lines they name, from 00:00 on the grid's top edge to 24:00 on its bottom edge: before, each label sat at the top of its two hours and read as half a slot off.

**No browser menu.** Right-click in WebView2 shows the browser's menu (Back, Refresh, Print, …), which has no place in an app; the page cancels the `contextmenu` event except in text fields, where cut, copy and paste are useful.

