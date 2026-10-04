# Moving the watchdog to a SYSTEM service: implementation guide for this repo

This guide is for implementing the SYSTEM-service design in *this* repo (`blocker`, Tauri tray app, Windows only). It is written against the actual code in `src-tauri/src/`. All the Rust below was compiled and the unit tests run in a scratch copy of the repo (`cargo check --tests`, no warnings; `cargo test`: 40 passed). One extra integration test (`the_pipe_answers_like_the_app_expects`, run with `--ignored`) started the real pipe server and talked to it like the app does: it passed. Section 9 lists what could **not** be tested without installing a real service, so you must test those parts yourself.

**Read sections 1 and 8 before you touch any code.** Section 8 is about how to get out if your own test setup locks you in.

## 1. What you are building, and why it is different from today

Today everything runs as *you*: the app, a launcher process that restarts it, and a scheduled task that restarts the launcher at logon. Because they all run as your user, you (or anything you run) can kill them, edit `state.json`, or disable the task. There is nothing to stop that.

The new design has two parts:

| Part | Runs as | Where | Job |
|---|---|---|---|
| **Service** (`SiteBlocker.exe --service`) | `SYSTEM` | session 0 (no desktop), started by Windows at boot | Owns the block, the unlock hash and internet time. Restarts the app. Answers the app over a named pipe. |
| **App** (`SiteBlocker.exe`, no arguments) | the logged-on user | the user's desktop | Tray icon, UI, and closing tabs (`tab_closer.rs`). Holds **no** authoritative state: it asks the service every second what to enforce. |

Vocabulary you need:

- **SYSTEM** is the account Windows itself runs its core services as. A process running as SYSTEM cannot be killed from Task Manager by an ordinary administrator ("Access denied"), and files owned by SYSTEM can be hidden from administrators.
- **SCM** (Service Control Manager) starts services at boot, restarts them when they fail, and decides who may stop them. That decision is a security descriptor written in a string format called **SDDL**.
- **Session 0 isolation**: services run in session 0, which has no desktop. A service cannot see or click anything in the user's browser. That is why the app has to stay a user-level process: UI Automation only works on the user's own desktop. The service's job is to keep that app alive and to be the one place the truth lives.
- **Named pipe**: a local, Windows-only socket (`\\.\pipe\SiteBlocker`). Any local program can try to connect to it, so the service has to treat every request as hostile input.

How the old pieces map to the new ones:

| Old | New |
|---|---|
| `task.rs` (logon task) | **Deleted.** Windows starts the service at boot; the service starts the app at logon. |
| `watchdog.rs` launcher + app mutex dance | Rewritten: the service finds the app with a process scan and restarts it (`CreateProcessAsUserW`). Only the app's single-instance mutex stays. |
| `state.json` in `%LOCALAPPDATA%` | Moved to `%ProgramData%\SiteBlocker`, owned by SYSTEM, administrators read-only. |
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
| `src-tauri/src/service.rs` | **new** (the service) |
| `src-tauri/src/watchdog.rs` | replace the whole file |
| `src-tauri/src/setup.rs` | replace the whole file |
| `src-tauri/src/main.rs` | replace the whole file |
| `src-tauri/src/block.rs`, `state.rs`, `tab_closer.rs` | small edits (diffs below) |
| `src-tauri/src/task.rs` | **delete** |
| `src-tauri/src/clock.rs`, `unlock.rs`, `filters.rs` | **unchanged** |
| `src/index.html` | small edit (show the error when the service is unreachable) |
| `src-tauri/tauri.conf.json` | one line: `perMachine` |
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
@@ -16,8 +16,20 @@
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
pub const PIPE: &str = r"\\.\pipe\SiteBlocker";

/// Larger messages are refused, so a request cannot make the service allocate without limit.
const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
pub enum Request {
    /// What the UI shows: time left, entries, enforcers.
    Status,
    /// What the app enforces: asked every second.
    Policy,
    Start { filters: Vec<String>, duration: String, enforcers: Enforcers },
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
        let response = Response::Policy(Some(Policy { filters: vec!["a.com".into()], enforcers: Enforcers::default() }));
        let mut wire = Vec::new();
        write_message(&mut wire, &response).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(response));
        let mut wire = Vec::new();
        write_message(&mut wire, &Response::Policy(None)).unwrap();
        assert_eq!(read_message::<Response>(&mut Cursor::new(wire)), Ok(Response::Policy(None)));
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
- **The app checks who it is talking to** (`verify_server`): the pipe must be served from **session 0**, the session of services (`GetNamedPipeServerSessionId`). Only SYSTEM and other services run there; a standard user cannot start a process in it, so cannot stand in for the service. **Do not replace this by looking at the serving process** (its file name, say): a standard user is not allowed to open a SYSTEM process, not even with `PROCESS_QUERY_LIMITED_INFORMATION`. The first version did exactly that and the app then refused to talk to the real service ("cannot look at who runs the pipe"). Comparing the exe path would also be weak: anyone can run `SiteBlocker.exe --service-console` themselves.
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
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerSessionId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

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

/// Checks that the pipe is served from session 0, the session of services. Only SYSTEM and other
/// services run there: a standard user cannot start a process in it, so cannot stand in for the
/// service. Without this, anything could open the pipe first and tell the app there is no block.
///
/// This asks the pipe itself. Looking at the serving process (its file name, say) does not work:
/// a standard user may not open a SYSTEM process, not even for a look.
fn verify_server(pipe: &File) -> Result<(), String> {
    // Development only: the console service (`service::run_console`) runs in the user's own
    // session. A release build never skips the check.
    if cfg!(debug_assertions) && std::env::var_os("SITEBLOCKER_DEV").is_some() {
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::block::Status;
use crate::filters::FilterList;
use crate::pipe;
use crate::proto::{self, Policy, Request, Response};
use crate::service;
use crate::state::Enforcers;
use crate::watchdog::note;

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
}

/// The least time between two attempts to start the service.
const START_EVERY: Duration = Duration::from_secs(5);

struct Cached {
    policy: Policy,
    list: Arc<FilterList>,
}

impl Remote {
    pub fn new() -> Remote {
        Remote::default()
    }

    /// A command that answers yes or no.
    pub fn command(&self, request: Request) -> Result<(), String> {
        match call(&request)? {
            Response::Done => Ok(()),
            Response::Failed(why) | Response::Refused(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    pub fn status(&self) -> Result<Status, String> {
        match call(&Request::Status)? {
            Response::Status(status) => Ok(status),
            Response::Failed(why) => Err(why),
            other => Err(format!("unexpected answer: {other:?}")),
        }
    }

    /// The filters and enforcers to enforce right now, if any.
    pub fn active(&self) -> Option<(Arc<FilterList>, Enforcers)> {
        match call(&Request::Policy) {
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
        self.cache.lock().unwrap().as_ref().map(|c| (c.list.clone(), c.policy.enforcers))
    }

    /// Asks Windows to start the service if it is registered but not running (it only stays down
    /// if something ended it with exit code 0, or an update did not finish). Starting is the only
    /// thing the app may do to the service: the access list gives interactive users no more
    /// (`service::SDDL`). A service that is not registered at all, as in development, just fails
    /// quietly.
    fn revive_service(&self) {
        let mut last = self.last_start.lock().unwrap();
        if last.is_some_and(|at| at.elapsed() < START_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        if let Ok(true) = service::start_registered() {
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
        let mut cache = self.cache.lock().unwrap();
        let Some(policy) = policy else {
            *cache = None;
            return;
        };
        if cache.as_ref().is_some_and(|c| c.policy == policy) {
            return;
        }
        match FilterList::parse(&policy.filters) {
            Ok(list) => *cache = Some(Cached { policy, list: Arc::new(list) }),
            // Cannot happen with a service of this version; keep what is being enforced.
            Err(_) => note("app", "the service sent a block list that cannot be read"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(filters: &[&str]) -> Policy {
        Policy { filters: filters.iter().map(|f| f.to_string()).collect(), enforcers: Enforcers::default() }
    }

    fn current(remote: &Remote) -> Option<(Arc<FilterList>, Enforcers)> {
        remote.cache.lock().unwrap().as_ref().map(|c| (c.list.clone(), c.policy.enforcers))
    }

    #[test]
    fn the_policy_follows_the_service_and_is_parsed_once() {
        let remote = Remote::new();
        assert!(current(&remote).is_none());

        remote.apply(Some(policy(&["tweakers.net"])));
        let (list, _) = current(&remote).unwrap();
        assert!(list.is_blocked("https://tweakers.net/nieuws"));

        // The same policy again keeps the same parsed list.
        remote.apply(Some(policy(&["tweakers.net"])));
        assert!(Arc::ptr_eq(&list, &current(&remote).unwrap().0));

        // A different one replaces it; none at all ends the block.
        remote.apply(Some(policy(&["b.com"])));
        assert!(current(&remote).unwrap().0.is_blocked("b.com/x"));
        remote.apply(None);
        assert!(current(&remote).is_none());
    }
}
```

## 5. Small edits to existing files

### 5.1 `block.rs`

Three small things: `Status` now travels over the pipe in both directions, `reset()` is what the unlock password uses (the old code killed the process to end a block; the service must not die, so the block has to be cleared in memory *and* on disk), and a test constructor so `service.rs` can test against a temporary store.

```diff
--- a/src-tauri/src/block.rs
+++ b/src-tauri/src/block.rs
@@ -10,7 +10,7 @@
 use std::sync::{Arc, Mutex};
 use std::time::{Duration, Instant, SystemTime};
 
-use serde::Serialize;
+use serde::{Deserialize, Serialize};
 
 use crate::clock::{self, Clock};
 use crate::filters::FilterList;
@@ -38,7 +38,7 @@
     saved_at: Instant,
 }
 
-#[derive(Serialize)]
+#[derive(Serialize, Deserialize, Debug, PartialEq)]
 pub struct Status {
     blocking: bool,
     /// Seconds left of a timed block.
@@ -58,6 +58,12 @@
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
     pub fn start(&self, filters: FilterList, duration: Option<Duration>, enforcers: Enforcers) -> Result<(), String> {
@@ -155,6 +161,13 @@
         })
     }
 
+    /// Ends the block whatever its time, and forgets it. Only for the unlock password
+    /// (`service::unlock`): everything else goes through `stop`, which keeps a timed block.
+    pub fn reset(&self) {
+        *self.block.lock().unwrap() = None;
+        self.clear_saved();
+    }
+
     /// Removes the saved block: there is no longer one to resume.
     pub fn clear_saved(&self) {
         if let Some(store) = &self.store {
```

### 5.2 `state.rs`

`Enforcers` also gets `can_run(has_entries)`: a block may start when at least one enforcer that is turned on can do something, which means it does not go by the block list, or the list has an entry. So an empty list is fine as long as e.g. the Obsidian enforcer is on, and refused when only list-based enforcers are on. **To add an enforcer: add its field and one line in `Enforcers::all()`**; the checks in `service::start` and the tab closer follow from that table, and the frontend already builds its checkboxes from `data-enforcer` attributes.

`dir()` is now the service's folder (`%ProgramData%\SiteBlocker`). The app's own log goes in `user_dir()`. `use_dir` lets `--service-console` (development) use another folder, so you do not need admin rights to develop.

```diff
--- a/src-tauri/src/state.rs
+++ b/src-tauri/src/state.rs
@@ -9,6 +9,7 @@
 
 use std::fs;
 use std::path::PathBuf;
+use std::sync::OnceLock;
 use std::time::{Duration, SystemTime, UNIX_EPOCH};
 
 use serde::{Deserialize, Serialize};
@@ -43,13 +44,31 @@
 }
 
 impl Enforcers {
-    /// Whether any enforcer goes by the block list, which then needs at least one entry.
-    pub fn use_block_list(&self) -> bool {
-        self.chromium_tabs || self.chromium_split_view
+    /// Every enforcer as (turned on, goes by the block list). **The one place to list a new
+    /// enforcer**: add its field above and a line here; what may start and what runs follow from it.
+    fn all(&self) -> [(bool, bool); 3] {
+        [
+            (self.chromium_tabs, true),
+            (self.chromium_split_view, true),
+            (self.obsidian_web_viewer, false),
+        ]
     }
 
+    /// Whether any enforcer is turned on.
     pub fn any(&self) -> bool {
-        self.use_block_list() || self.obsidian_web_viewer
+        self.all().iter().any(|&(on, _)| on)
+    }
+
+    /// Whether any enforcer that is turned on goes by the block list.
+    pub fn use_block_list(&self) -> bool {
+        self.all().iter().any(|&(on, uses_list)| on && uses_list)
+    }
+
+    /// Whether a block with these enforcers has anything to do, given whether the block list has
+    /// an entry: an enforcer that goes by the list has nothing to do with an empty one, the others
+    /// do not need it. A block that cannot do anything is not worth starting.
+    pub fn can_run(&self, has_entries: bool) -> bool {
+        self.all().iter().any(|&(on, uses_list)| on && (!uses_list || has_entries))
     }
 }
 
@@ -129,15 +148,32 @@
     }
 }
 
-/// The folder the app keeps its files in: `%LOCALAPPDATA%\SiteBlocker`.
+/// A folder to use instead of the real one, for `--service-console` (`service.rs`).
+static DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
+
+pub fn use_dir(dir: PathBuf) {
+    let _ = DIR_OVERRIDE.set(dir);
+}
+
+/// The folder the service keeps its files in: `%ProgramData%\SiteBlocker`, which the service
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
+    Some(PathBuf::from(std::env::var_os("ProgramData")?).join("SiteBlocker"))
+}
+
+/// The app's own folder, for its log: `%LOCALAPPDATA%\SiteBlocker`. Nothing in it matters to a block.
+pub fn user_dir() -> Option<PathBuf> {
     Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("SiteBlocker"))
 }
 
-/// The store the app uses: `%LOCALAPPDATA%\SiteBlocker\state.json`.
+/// The store the service uses: `%ProgramData%\SiteBlocker\state.json`.
 pub fn store() -> Option<Store> {
     Some(Store::new(dir()?.join("state.json")))
 }
@@ -219,6 +255,27 @@
     }
 
     #[test]
+    fn what_can_run_depends_on_the_enforcers_and_the_list() {
+        let only = |tabs, split, obsidian| Enforcers { chromium_tabs: tabs, chromium_split_view: split, obsidian_web_viewer: obsidian };
+        // With entries, any enforcer that is on can run.
+        assert!(only(true, false, false).can_run(true));
+        assert!(only(false, true, false).can_run(true));
+        assert!(only(false, false, true).can_run(true));
+        // Without, only one that does not go by the list can: Obsidian, also next to ones that do.
+        assert!(only(false, false, true).can_run(false));
+        assert!(only(true, true, true).can_run(false));
+        assert!(Enforcers::default().can_run(false));
+        // Not the ones that go by the list alone, and not nothing at all.
+        assert!(!only(true, false, false).can_run(false));
+        assert!(!only(true, true, false).can_run(false));
+        assert!(!only(false, false, false).can_run(true));
+        assert!(!only(false, false, false).can_run(false));
+        // The checks the tab closer and the start use agree with it.
+        assert!(only(true, false, false).use_block_list() && !only(false, false, true).use_block_list());
+        assert!(only(false, false, true).any() && !only(false, false, false).any());
+    }
+
+    #[test]
     fn wall_clock_round_trips() {
         let time = from_unix(1_700_000_000);
         assert_eq!(to_unix(time), Some(1_700_000_000));
```

### 5.3 `tab_closer.rs`

Only the source of the policy changes (plus one skipped call when the list has no entries); none of the UI Automation code does.

```diff
--- a/src-tauri/src/tab_closer.rs
+++ b/src-tauri/src/tab_closer.rs
@@ -15,7 +15,7 @@
 use std::thread;
 use std::time::{Duration, Instant};
 
-use crate::block::Blocklist;
+use crate::client::Remote;
 use crate::filters::FilterList;
 use crate::state::Enforcers;
 use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
@@ -26,7 +26,7 @@
 const INTERVAL: Duration = Duration::from_secs(1);
 const RECHECK: Duration = Duration::from_secs(10);
 
-pub fn spawn(blocklist: Blocklist) {
+pub fn spawn(remote: Remote) {
     thread::spawn(move || {
         let automation = match UIAutomation::new() {
             Ok(a) => a,
@@ -35,8 +35,9 @@
         let mut closer = Closer { automation, windows: HashMap::new(), system: System::new() };
         loop {
             let start = Instant::now();
-            if let Some((filters, enforcers)) = blocklist.active() {
-                if enforcers.use_block_list() {
+            if let Some((filters, enforcers)) = remote.active() {
+                // With no entries there is nothing for these to match, so skip the UI Automation work.
+                if enforcers.use_block_list() && !filters.is_empty() {
                     if let Err(e) = closer.tick(&filters, enforcers) {
                         eprintln!("tab_closer: {e}");
                     }
```

## 6. The service, and the files that change around it

### 6.1 `src-tauri/src/service.rs`

Read the top-of-file comment first. Decisions worth knowing:

- **`controls_accepted` does not include STOP.** A service that does not accept stop cannot be stopped with `sc stop` or from the services list, by anyone, including administrators. It only accepts `SESSION_CHANGE`, which tells it when someone logs on.
- **The SDDL** gives SYSTEM everything (it needs delete to remove itself on uninstall), administrators **and interactive users** query/enumerate/**start**/read-the-list only, and service accounts query only. Administrators keep *start* on purpose: the installer has to be able to start the service again after an update. Interactive users get *start* because the app, which runs as one, starts the service again when it is not running (`client::Remote::revive_service`, at most every 5 s). Starting is harmless: starting a running service does nothing, and a stopped one only goes back to blocking. Nobody but SYSTEM gets stop, pause, change-config, delete, write-DACL or write-owner. **Never widen what interactive users may do beyond start**: the app runs as the user, so anything the app may do, the user may do. `sc sdset` is the **last** step of the install: after it, an administrator can no longer change the service (the handle the installer already holds keeps its rights).
- **Failure actions** are "restart immediately", three times over, and the last one repeats forever. `set_failure_actions_on_non_crash_failures(true)` makes that apply also when the process ends with a failure *exit code*, not just when it is killed. A clean exit code 0 is **not** restarted: the installer relies on that.
- **Every request is checked again in `handle`/`start`/`unlock`.** The code that used to validate in the Tauri command (`parse_duration`, `FilterList::parse`, "at least one enforcer", ...) is now here, because the pipe can be reached by any program.
- **Unlock rate limit**: after 3 wrong passwords in a row, the next guesses are refused for 5 s, 10 s, 20 s, ... (capped at about 21 minutes). The lock is held while the password is hashed, so guesses cannot be tried in parallel. (The hash is still in the binary, so an offline attack on a weak password is unchanged: see section 8.4.)
- **Uninstall vs update**: `PrepareUninstall` is refused during a *timed* block (same as today). `PrepareUpdate` is refused during *any* block, because replacing the exe is also a way to end the service.
- **The data folder is locked down** at start (`lock_down_data_dir`): owner SYSTEM, SYSTEM full control, administrators read-only, no inheritance from `%ProgramData%` (where every user may create files). It uses SIDs (`*S-1-5-18` is SYSTEM, `*S-1-5-32-544` is Administrators) instead of names, so it works on any language of Windows.
- **Safe mode**: the installer creates `SafeBoot\Minimal\SiteBlocker` and `SafeBoot\Network\SiteBlocker` registry keys so Windows starts the service in safe mode as well.
- **`--service-console`** runs the same engine in a terminal for development (own data folder, no launching of the app).

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
/// Administrators and interactive users: query, enumerate, read the list and start, but not stop,
/// pause, change, delete or rewrite it. Interactive users may start it because the app (which
/// runs as one) starts it again when it is not running (`client::Remote::revive_service`);
/// starting a running service does nothing, and a stopped one only goes back to blocking.
/// Service accounts: query only.
const SDDL: &str = "D:(A;;CCLCSWRPWPDTLOCRSDRCWDWO;;;SY)(A;;CCLCSWRPLOCRRC;;;BA)(A;;CCLCSWRPLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";

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
    // An enforcer that goes by the list has nothing to do with an empty one, while the others
    // (Obsidian's) do not need it. What matters is that something can run.
    if !enforcers.can_run(!filters.is_empty()) {
        return Err("Enter at least one site to block, or turn on an enforcer that does not use the list".into());
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
        // An empty list with only enforcers that go by it, nothing chosen, a bad duration, and an
        // address that cannot match.
        let only_the_list = Enforcers { chromium_tabs: true, chromium_split_view: true, obsidian_web_viewer: false };
        let bad = [
            Request::Start { filters: vec![], duration: "1h".into(), enforcers: only_the_list },
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
    fn an_empty_list_starts_a_block_when_an_enforcer_without_the_list_is_on() {
        let first = context("empty-list");
        let obsidian_only = Enforcers { chromium_tabs: false, chromium_split_view: false, obsidian_web_viewer: true };
        let request = |enforcers| Request::Start { filters: vec![], duration: "1h".into(), enforcers };
        assert_eq!(handle(&first, request(obsidian_only)).0, Response::Done);
        // The block keeps what it was started with: no entries, and Obsidian's enforcer only.
        let Response::Policy(Some(policy)) = handle(&first, Request::Policy).0 else { panic!("no policy") };
        assert!(policy.filters.is_empty());
        assert_eq!(policy.enforcers, obsidian_only);

        // With all enforcers on (the default), an empty list is fine too: Obsidian's can run.
        let other = context("empty-list-default");
        assert_eq!(handle(&other, request(Enforcers::default())).0, Response::Done);
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

        // This server runs in the user's session, not in session 0: the app refuses to talk to it.
        std::env::remove_var("SITEBLOCKER_DEV");
        let refused = client::call(&Request::Status).unwrap_err();
        assert!(refused.contains("not by the service"), "{refused}");
        // Development mode, as `--service-console` is used with `npm run dev`, skips that check.
        std::env::set_var("SITEBLOCKER_DEV", "1");

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

`watchdog.rs` is now mostly about the service looking after the app. The app finds itself with a mutex as before, but the service lives in another session, where that mutex is invisible (`Local\` is per session), so the service looks at processes instead: the same exe, started **without arguments**, in the **console session** (the one at the physical screen). `launch_app` starts the app as the logged-on user, not as SYSTEM: `WTSQueryUserToken` gets that user's token, `CreateEnvironmentBlock` their environment, `CreateProcessAsUserW` starts the process on `winsta0\default`. It runs while any block exists (timed or not), plus once after each logon (`LAUNCH_APP`). That is why quitting the app with the unlock password sticks: the block is gone, so nobody restarts it until the next logon.

```rust
//! The service's watch over the app, and the little the app itself needs from this module.
//!
//! The service (`service.rs`) runs as SYSTEM in session 0, which has no desktop, so it cannot
//! close browser tabs itself. The app does that, in the logged-on user's session, and the service
//! keeps it there: it starts the app at every logon, and while a block runs it starts the
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

/// Set at logon (and when the service starts): the app is wanted even without a block.
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

/// The service's half: keeps the app running in the logged-on user's session while a block is
/// running (timed or not), and starts it when `LAUNCH_APP` is set.
pub fn spawn_supervisor(blocklist: Blocklist) {
    thread::spawn(move || {
        let mut system = System::new();
        // How many starts in a row have not led to a running app, which slows the starting down.
        let mut missed = 0u32;
        let mut not_before = Instant::now();
        let mut last_error = String::new();
        loop {
            thread::sleep(Duration::from_secs(1));
            if !wanted(&blocklist) {
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

/// Whether the app should be running: while any block runs, and until the app has been seen after a
/// logon. With no block the app is left alone, so quitting it with the unlock password sticks.
fn wanted(blocklist: &Blocklist) -> bool {
    blocklist.filters().is_some() || LAUNCH_APP.load(Ordering::SeqCst)
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
    fn the_app_is_wanted_while_any_block_runs() {
        use crate::filters::FilterList;
        use crate::state::{Enforcers, Store};
        LAUNCH_APP.store(false, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("watchdog-{}-wanted", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let blocklist = Blocklist::with_store(Store::new(dir.join("state.json")));
        assert!(!wanted(&blocklist));
        // An untimed block as well as a timed one: ending the app must not end the blocking.
        blocklist.start(FilterList::parse(&["a.com"]).unwrap(), None, Enforcers::default()).unwrap();
        assert!(wanted(&blocklist));
        blocklist.stop().unwrap();
        assert!(!wanted(&blocklist));
        let hour = Some(Duration::from_secs(3600));
        blocklist.start(FilterList::parse(&["a.com"]).unwrap(), hour, Enforcers::default()).unwrap();
        assert!(wanted(&blocklist));
        // Right after a logon the app is wanted even with no block.
        blocklist.reset();
        assert!(!wanted(&blocklist));
        LAUNCH_APP.store(true, Ordering::SeqCst);
        assert!(wanted(&blocklist));
    }

    #[test]
    fn claiming_a_role_twice_fails() {
        assert!(claim("Local\\SiteBlocker-test-role"));
        assert!(!claim("Local\\SiteBlocker-test-role"));
    }
}
```

`setup.rs`: the installer cannot stop the service (nobody but SYSTEM can), so it asks over the pipe. If the service is not running (an update that did not finish), it starts it first: administrators are allowed to start it.

```rust
//! Installing and uninstalling: what the Windows installer runs (`windows/hooks.nsh`), as an
//! administrator, as `SiteBlocker.exe --prepare-update` before replacing the app, `--install`
//! after copying it, and `--uninstall` before removing it.
//!
//! The installer cannot stop the service itself (nobody but SYSTEM can), so it asks it, over the
//! pipe. The service refuses while a block runs, so none of this is a way around a block: the way
//! out of a timed block stays waiting for it to end or the unlock password.

use std::thread;
use std::time::Duration;

use crate::proto::{Request, Response};
use crate::watchdog::note;
use crate::{client, service};

/// Exit codes the installer hooks act on.
pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const REFUSED: i32 = 2;

/// Registers the service and starts it. The service starts the app for whoever is logged on.
pub fn install() -> i32 {
    let Ok(exe) = std::env::current_exe() else { return FAILED };
    match service::install(&exe) {
        Ok(()) => {
            note("install", &format!("registered the {} service for {}", service::NAME, exe.display()));
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
    if !service::exists() {
        return OK;
    }
    let mut answer = client::call(&request);
    if answer.is_err() {
        // Not running (an update that did not finish, say). Administrators may start it.
        let _ = service::start_registered();
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
            if service::wait_stopped(Duration::from_secs(30)) {
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

`main.rs`: the Tauri commands are now thin forwards, the old `--launch` entry point is gone, the app no longer runs a clock, a block list or a launcher guard, and there are three new arguments (`--service`, `--service-console`, `--prepare-update`).

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod block;
mod client;
mod clock;
mod filters;
mod pipe;
mod proto;
mod service;
mod setup;
mod state;
mod tab_closer;
mod unlock;
mod watchdog;

use block::Status;
use client::Remote;
use proto::Request;
use state::Enforcers;
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State, WindowEvent,
};

/// Starts blocking with the given filter lines and enforcers, for `duration` (`5s`, `10m`, `3h`,
/// `1d`) or, when empty, until stopped.
///
/// The service owns the block and checks all of this again (`service::start`); the answer here
/// is only its verdict. It goes on even if this app is ended or Windows restarts.
#[tauri::command]
fn start_block(filters: Vec<String>, duration: String, enforcers: Enforcers, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Start { filters, duration, enforcers })
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

fn main() {
    // The service itself, started by Windows (`service.rs`).
    if std::env::args_os().any(|arg| arg == "--service") {
        std::process::exit(service::run_dispatcher());
    }
    // The same in a terminal, for development.
    if std::env::args_os().any(|arg| arg == "--service-console") {
        service::run_console();
        return;
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

    // One app instance only. A second copy (started by the service, or by launching the
    // executable again) leaves the running one to do the work.
    if !watchdog::claim_app() {
        return;
    }

    let remote = Remote::new();
    tab_closer::spawn(remote.clone());

    tauri::Builder::default()
        .manage(remote)
        .invoke_handler(tauri::generate_handler![start_block, stop_block, block_status, force_exit])
        // The window's X hides to the tray; only the unlock password quits.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button_state: MouseButtonState::Up, .. } = event {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 6.3 Frontend, config and installer

`index.html`: `block_status` can now fail (service not running), so show that instead of an unhandled error every second.

```diff
--- a/src/index.html
+++ b/src/index.html
@@ -200,7 +200,16 @@
 
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
+        const { blocking, remaining, filters, enforcers: running, clock_synced } = state;
         const timed = remaining != null;
         // A block resumed from disk comes with its list, so show it rather than an empty box.
         if (blocking && filters.length && !sites.value) sites.value = filters.join("\n");
@@ -231,7 +240,13 @@
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
         await run("start_block", { filters, duration: duration.value, enforcers: enforcers() });
```

`tauri.conf.json`: the installer must run as administrator and install into `C:\Program Files`, **not** a folder the user can write to. If the user could replace the exe the service runs from, they could run anything as SYSTEM, which would defeat the whole thing.

```diff
--- a/src-tauri/tauri.conf.json
+++ b/src-tauri/tauri.conf.json
@@ -32,7 +32,7 @@
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
      MessageBox MB_ICONSTOP "A block is running, so Site Blocker cannot be updated now.$\n$\nWait for the block to end, or end it with the unlock password in the app, then run the installer again." /SD IDOK
      SetErrorLevel 2
      Quit
    ${ElseIf} $0 <> 0
      MessageBox MB_ICONSTOP "Site Blocker could not be stopped for the update. See C:\ProgramData\SiteBlocker\service.log." /SD IDOK
      SetErrorLevel 1
      Quit
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Registers the service, locks it down and starts it; it starts the app for whoever is logged on.
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --install' $0
  ${If} $0 <> 0
    MessageBox MB_ICONEXCLAMATION "Site Blocker is installed, but its service could not be set up, so it will not block anything. See %LOCALAPPDATA%\SiteBlocker\app.log." /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; An update replaces the files only (and has asked the service already, in PREINSTALL).
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall' $0
    ${If} $0 = 2
      MessageBox MB_ICONSTOP "A timed block is still running, so Site Blocker cannot be uninstalled now.$\n$\nWait for the block to end, or end it with the unlock password in the app, then uninstall again." /SD IDOK
      SetErrorLevel 2
      Quit
    ${ElseIf} $0 <> 0
      ; The service is registered but did not answer or did not end. Stopping here, rather than
      ; deleting files under a service that may still be running, avoids a half-removed install.
      ; (No service at all is exit code 0, so that case goes on.)
      MessageBox MB_ICONSTOP "Site Blocker's service did not respond, so nothing was removed.$\n$\nSee C:\ProgramData\SiteBlocker\service.log, or use the recovery steps in detailed-instructions.md." /SD IDOK
      SetErrorLevel 1
      Quit
    ${EndIf}
  ${EndIf}
!macroend
```

## 7. Update `CLAUDE.md`

The project notes describe the old design and are used by every future session, so update them in the same change. Replace these parts (wording is a suggestion; keep the style of the file):

**File list**: remove `task.rs`; add:

- `src-tauri/src/service.rs` – the SYSTEM service: owns the block, answers the app over the pipe, installs / removes itself
- `src-tauri/src/pipe.rs`, `proto.rs`, `client.rs` – the named pipe `\\.\pipe\SiteBlocker`: server, messages, and the app's side (keeps the last policy in memory)
- `src-tauri/src/watchdog.rs` – the service's watch over the app (starts it as the logged-on user, at logon and while a block runs) and the app's single-instance mutex

**Replace "Surviving a restart, a kill, and a reboot" with:**

> The `SiteBlocker` Windows service runs as SYSTEM and starts at boot. It holds the block (`%ProgramData%\SiteBlocker\state.json`, SYSTEM-owned), internet time and the unlock hash. The tray app (user session) asks it every second what to enforce and does the tab closing; it holds no state, and keeps enforcing the last answer if the service is briefly unreachable.
> - End the app in Task Manager while a block (timed or not) is running: the service starts it again within about a second.
> - End the service: Windows refuses ("Access denied") for administrators; if it fails by itself, Windows restarts it at once (failure actions).
> - Reboot mid-block: the service reads `state.json` and resumes with the time left.
> - The service starts the app at every logon, also without a block. "Unlock" clears the block and quits the app; the service keeps running idle and starts the app again at the next logon.

**Replace the "Deliberately not hidden" and "honest ceiling" paragraphs' technical content:** still nothing is hidden (service named `SiteBlocker` in the services list, app in Settings > Apps, logs in `%ProgramData%\SiteBlocker\service.log`). The honest ceiling is now: an administrator who knows how can still get around it: run a shell as SYSTEM (for example PsExec `-s`), take ownership of the service object or the data folder (administrators hold `SeTakeOwnershipPrivilege`) and rewrite its access list, delete the service from the Windows recovery environment, or boot with an offline registry edit. NTP spoofing needs admin as before. The password hash is in the binary, so a short password can be brute-forced offline. **What changed is the effort**: a normal administrator now needs one of those, instead of "End task".

**Replace "Install / remove":** per-machine NSIS installer, needs admin (UAC), installs to `C:\Program Files\Site Blocker`. Hooks: `--prepare-update` (before replacing files; refused during any block), `--install` (register + lock down + start the service), `--uninstall` (service removes itself and ends the app; refused during a timed block, exit code 2; if the service is registered but does not answer or does not end, exit code 1 also stops the uninstall; with no service registered it carries on).

**Dev workflow**: see section 8.1 below, and drop the sentence about `restore_task` / re-enabling the login task.

## 8. Development, recovery, and things to know

### 8.1 Developing without installing a service

You do not need admin rights or a real service to develop. In a terminal, from `src-tauri`:

```powershell
cargo build
# A copy, because `tauri dev` replaces tray-app.exe and Windows refuses that while it is running.
copy target\debug\tray-app.exe target\debug\tray-service.exe
$env:SITEBLOCKER_DEV = "1"      # debug builds only: skips "is the pipe served from session 0"
.\target\debug\tray-service.exe --service-console
```

and in a second terminal, from the repo root:

```powershell
$env:SITEBLOCKER_DEV = "1"
npm run dev
```

The console service uses `%LOCALAPPDATA%\SiteBlocker-dev` for its files and does not launch the app. After changing service code, rebuild and re-copy the exe, then restart the console service. Unlocking quits the app, which ends `tauri dev`, as before.

### 8.2 Recovery: when your own test locks you out (read this before testing on a real PC)

The service you install is, by design, not stoppable by you. If you install it on your own machine and start a **timed** block, you cannot uninstall until it ends. So:

- **Test installs in a Windows virtual machine** and take a snapshot first. It is the only comfortable way back.
- On a real machine only use short blocks (`5s`, `10s`), and know that the unlock password is `test1234`.
- If it still goes wrong, as an administrator, run a shell as SYSTEM, for example with Sysinternals PsExec: `psexec -s -i cmd`. In that shell:

```bat
sc delete SiteBlocker
taskkill /f /im SiteBlocker.exe
reg delete "HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Minimal\SiteBlocker" /f
reg delete "HKLM\SYSTEM\CurrentControlSet\Control\SafeBoot\Network\SiteBlocker" /f
rmdir /s /q "%ProgramData%\SiteBlocker"
```

  (`sc delete` comes first: it marks the service for deletion, so Windows does not start it again when the `taskkill` ends the process.)
- No PsExec? As administrator you can borrow Task Scheduler to get a SYSTEM shell: save the five lines above as `C:ecover.cmd`, then run `schtasks /create /tn recover /sc once /st 00:00 /ru SYSTEM /tr C:ecover.cmd /f`, `schtasks /run /tn recover`, and afterwards `schtasks /delete /tn recover /f`.
- Last resort: boot into the Windows recovery environment's command prompt (which runs as SYSTEM) and delete the registry keys / files there.

### 8.3 Behaviour changes the user will notice

- Installing and updating asks for administrator rights (UAC). The install folder is `C:\Program Files\Site Blocker`.
- **Migration:** an existing per-user install lives elsewhere and does not know `--prepare-update`: uninstall it first (Settings > Apps). Do not install the new version over it: the pre-install hook would run the *old* exe with an unknown argument, which starts the old tray app and waits for it forever.
- The login task is gone. "Unlock" no longer disables anything: it clears the block and quits the app, and the app returns at the next logon.
- `launcher.log` is replaced by `%ProgramData%\SiteBlocker\service.log` (readable by administrators) and `%LOCALAPPDATA%\SiteBlocker\app.log`.
- Updating is refused during any block (timed or not); uninstalling during a timed block (as before), and also when the service is registered but does not answer, in which case nothing is removed (see `service.log` and section 8.2).

### 8.4 Known limits (do not hide these in the UI)

- **The honest ceiling above still holds**: this makes bypassing take real effort (SYSTEM shell, ownership change, recovery environment). It does not make it impossible. Do not claim otherwise in the UI.
- **The unlock hash is in the binary** (`unlock.rs`, unchanged): a short password can be brute-forced offline, the new rate limit only slows *online* guessing through the pipe. Use a long password.
- **Only the console session gets the app.** On a PC with several users logged on (fast user switching), the app is started for whoever is at the screen; another user's session is not watched. Remote Desktop sessions are not watched either.
- **Which token the app gets.** `WTSQueryUserToken` normally returns the user's *limited* (non-elevated) token, like the old scheduled task (`LeastPrivilege`). Check once (section 9.2) that the app is not elevated.
- **Who restarts the service**: Windows (failure actions, at once) after a crash or kill, and the app (start only, every 5 s) if the service is not running. A service that ended with exit code 0 (an update or uninstall request) is not restarted by Windows; the app or the installer starts it again. While it is down the app keeps enforcing the last policy it got (in memory), but an app that is restarted meanwhile has none.
- **The access list is set once, when the service is created.** An update only starts the existing service, and administrators cannot change the list, so an install made before the interactive-users start right keeps the old list. Uninstall and install again (in the VM) to pick it up.
- **The service restarts the app while any block runs, timed or not**, and once after each logon. With no block it leaves the app alone, so quitting it with the unlock password sticks until the next logon.
- **Offline time** works as before (`clock.rs`), but now the clock lives in the service.
- **Code signing.** The service runs the exe with SYSTEM rights, so a signed build is worth having; unsigned, SmartScreen and antivirus tools may complain at install. Not part of this change.

## 9. Testing

### 9.1 What was already tested, and how to re-run it

```powershell
cd src-tauri
cargo test                               # 40 tests: block, clock, filters, state, unlock, proto, client, service, watchdog
cargo test -- --ignored the_pipe --nocapture   # starts the real pipe server in the test process, ~15 s; no service may be running
```

The ignored test talks to the real Windows named pipe: status, start, policy, stop refused, uninstall refused during a block, an oversized length, six simultaneous clients, a silent client that is cut off after 10 s, then unlock and an update request.

### 9.2 What only you can test (needs a real service; do it in a VM)

Build with `npm run build` (installer in `src-tauri\target\release\bundle\nsis\`). The NSIS script was compiled once in the scratch copy to check the hook syntax; it was **not** run.

| # | Test | Expected |
|---|---|---|
| 1 | Run the installer | UAC prompt; service `SiteBlocker` exists (`sc qc SiteBlocker`), state `RUNNING`; the tray app appears for you |
| 2 | Task Manager > Details, the app's row, "Elevated" column | `No` (see 8.4) |
| 3 | `sc sdshow SiteBlocker` | the SDDL from `service.rs` |
| 4 | As admin: `sc stop SiteBlocker`, and "End task" on the service in Task Manager | both fail |
| 5 | `icacls "%ProgramData%\SiteBlocker"` and try to edit `state.json` as admin | SYSTEM full, Administrators read only; editing fails |
| 6 | Start a `10m` block, then end the *app* in Task Manager | it returns within about a second, block continues |
| 6b | Same with an *untimed* block (empty duration) | it returns within about a second too; after "Stop block" it no longer does |
| 7 | Log off and on during the block | app returns, block continues with the remaining time |
| 8 | Reboot during the block | block resumes after boot |
| 9 | Boot into safe mode with networking during the block | service runs (check `sc query SiteBlocker`) |
| 10 | Run the installer again during the block | refused with the message from the hook; nothing replaced |
| 11 | Uninstall during the timed block | refused (exit code 2), message shown |
| 12 | Wrong unlock password three times, then the right one | the right one is refused for a few seconds, then works; block ends, app quits |
| 13 | After unlocking: log off and on | the app returns (idle) |
| 14 | End the block, update to a new build | installer succeeds; service runs the new exe |
| 15 | Uninstall without a block | service, `%ProgramData%\SiteBlocker` and the safe-mode keys are gone |
| 16 | Open the pipe with a script and send garbage / a huge length / nothing | service keeps answering the app |
| 17 | Kill the service with a SYSTEM shell (`psexec -s`) | Windows restarts it at once; app keeps enforcing meanwhile |
| 18 | Uninstall while the service is registered but not answering (in the VM, suspend the service process with a SYSTEM shell tool first) | the uninstall stops with "did not respond, so nothing was removed" (exit code 1); no files are deleted |
| 19 | Uninstall after the service was deleted by hand (`sc delete`, via the recovery steps) | carries on and removes the files (no service registered is exit code 0) |
| 20 | The app starts a stopped service. In a SYSTEM shell turn the restart off (`sc failure SiteBlocker reset= 0 actions= ""`), then `taskkill /f /im SiteBlocker.exe` the service while the app runs | within about 5 s the app log says "the service was not running; started it" and the service runs again. **Put the restart back afterwards**: `sc failure SiteBlocker reset= 86400 actions= restart/0/restart/0/restart/0` |

If test 1 fails: read `%ProgramData%\SiteBlocker\service.log` (administrators can read it) and the installer's error message; `%LOCALAPPDATA%\SiteBlocker\app.log` has the `--install` output.

## 10. Where the risk is

If you get stuck or unsure, these are the places to ask for a review before merging, in this order:

1. `service::install`: the SDDL and the failure actions. A wrong SDDL can lock out the installer itself.
2. `watchdog::launch_app`: it hands out the user's token. It must never start the app as SYSTEM.
3. `pipe.rs`: the access list and the checks. A hole here lets a standard user talk to a SYSTEM process.
4. The installer hooks: this was the one part checked only by reading the generated script, not by running it.
