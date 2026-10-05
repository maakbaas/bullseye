//! What makes the closing worker (`tab_closer.rs`) look again at once, instead of at its next
//! second, so a blocked page or program is closed sooner. The pass every second stays: an event
//! can be missed, and the app's policy request each second is how the service knows it is alive.
//!
//! Two sources, both costing nothing while nothing happens:
//! - **In the app**, a WinEvent hook (`spawn_hook`): a Chromium window's title changing (a tab
//!   navigated or was switched to), a value changing in the window itself (its address bar), or a
//!   Chromium window coming to the foreground. Out of context, so no code goes into other
//!   processes; the hook's thread only checks the window's class and sets an event.
//! - **From the service**, a program starting (`spawn_process_watch`) that a running block would
//!   close: one in its program list, or Obsidian while its enforcer is on. WMI's process start
//!   trace needs administrator rights, which the app does not have, so the service listens and
//!   sets a named event (`SERVICE_EVENT`) that the app waits on. Only SYSTEM can create or set it
//!   (`SDDL`; creating anything in `Global\` also needs a privilege users do not have), so no one
//!   else can set it off or take its name first; interactive users may only wait on it.

use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use windows_sys::Win32::Foundation::{GetLastError, LocalFree, HANDLE, HWND, WAIT_FAILED, WAIT_OBJECT_0};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Threading::{
    CreateEventW, OpenEventW, SetEvent, WaitForMultipleObjects, INFINITE, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetAncestor, GetClassNameW, GetMessageW, TranslateMessage, CHILDID_SELF, EVENT_OBJECT_NAMECHANGE,
    EVENT_OBJECT_VALUECHANGE, EVENT_SYSTEM_FOREGROUND, GA_ROOT, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::block::Blocklist;
use crate::sync::spawn_critical;
use crate::watchdog::{note, wide};

/// What the service sets when a program starts that a running block closes.
const SERVICE_EVENT: &str = r"Global\bullseye-wake";
/// Everything for SYSTEM; interactive users may only wait on it (`SYNCHRONIZE`).
const SDDL: &str = "D:(A;;GA;;;SY)(A;;0x00100000;;;IU)";
/// The window class of Chromium browsers (and of Electron apps, Obsidian among them).
const CHROMIUM: &str = "Chrome_WidgetWin_1";

/// A handle kept in a static: an event, which any thread may set or wait on.
struct Event(HANDLE);
// SAFETY: an event handle may be used from any thread.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

/// The app's own event, set by the hook: auto-reset, so one wait answers any number of events.
fn local() -> HANDLE {
    static LOCAL: OnceLock<Event> = OnceLock::new();
    // SAFETY: an unnamed event with default security; never closed, as it lives as long as the app.
    LOCAL.get_or_init(|| Event(unsafe { CreateEventW(null(), 0, 0, null()) })).0
}

/// The service's event, opened once it is there (the service may start after the app).
fn service() -> Option<HANDLE> {
    static SERVICE: Mutex<Option<Event>> = Mutex::new(None);
    let mut service = SERVICE.lock().unwrap_or_else(|e| e.into_inner());
    if service.is_none() {
        let name = wide(SERVICE_EVENT);
        // SAFETY: the name is null-terminated; a null handle is "not there yet".
        let handle = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
        if !handle.is_null() {
            *service = Some(Event(handle));
        }
    }
    service.as_ref().map(|event| event.0)
}

/// What ended a wait, so the log can say what a close came from (and so whether the events work).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Woken {
    /// The pass every second.
    Second,
    /// A window event (`spawn_hook`).
    Window,
    /// A program starting, from the service (`spawn_process_watch`).
    ProgramStart,
}

impl Woken {
    pub fn describe(self) -> &'static str {
        match self {
            Woken::Second => "on the pass every second",
            Woken::Window => "on a window event",
            Woken::ProgramStart => "on a program starting",
        }
    }
}

/// Waits until `timeout` is up, or until the hook or the service says to look again. The worker's
/// sleep between passes.
pub fn wait(timeout: Duration) -> Woken {
    let mut handles = vec![local()];
    handles.extend(service());
    let millis = u32::try_from(timeout.as_millis()).unwrap_or(INFINITE - 1);
    // SAFETY: the handles are open events, and stay open.
    match unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, millis) } {
        WAIT_OBJECT_0 => Woken::Window,
        1 => Woken::ProgramStart,
        WAIT_FAILED => {
            thread::sleep(timeout);
            Woken::Second
        }
        _ => Woken::Second,
    }
}

/// The app's hook on window events, on a thread of its own: an out-of-context hook is called
/// through that thread's message loop.
pub fn spawn_hook() {
    spawn_critical("event hook", || {
        // SAFETY: the callback is a plain function; no module, every process and thread.
        let hooks: Vec<HWINEVENTHOOK> =
            [(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND), (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_VALUECHANGE)]
                .into_iter()
                .map(|(first, last)| unsafe {
                    SetWinEventHook(first, last, null_mut(), Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)
                })
                .collect();
        if hooks.iter().any(|hook| hook.is_null()) {
            note("app", "window events are not heard; blocked pages are closed at the next second");
            return;
        }
        let mut message: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: a plain message loop for this thread, which the hooks are called through.
        while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    });
}

/// A window event: worth a look when it comes from a Chromium window itself, not from a page in it
/// (a page's own events come from its child window, so typing in a form does not count): the
/// window's title changing (a tab navigated or was switched to), a value changing (its address
/// bar), or the window coming to the foreground.
unsafe extern "system" fn on_event(_hook: HWINEVENTHOOK, event: u32, window: HWND, object: i32, child: i32, _thread: u32, _time: u32) {
    let wanted = match event {
        EVENT_OBJECT_NAMECHANGE => object == OBJID_WINDOW && child == CHILDID_SELF as i32,
        EVENT_OBJECT_VALUECHANGE | EVENT_SYSTEM_FOREGROUND => true,
        _ => false,
    };
    // SAFETY: a window handle from Windows; a stale one only makes these calls fail.
    if wanted && !window.is_null() && unsafe { GetAncestor(window, GA_ROOT) } == window && class_of(window) == CHROMIUM {
        // SAFETY: the app's own event, open for as long as the app runs.
        unsafe { SetEvent(local()) };
    }
}

fn class_of(window: HWND) -> String {
    let mut class = [0u16; 64];
    // SAFETY: the buffer's length is passed with it.
    let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    String::from_utf16_lossy(&class[..length.max(0) as usize])
}

/// A program starting, as WMI's process start trace reports it.
#[derive(Deserialize)]
#[serde(rename = "Win32_ProcessStartTrace", rename_all = "PascalCase")]
struct ProcessStart {
    process_name: String,
}

/// The service's watch on programs starting: sets `SERVICE_EVENT` when a running block would close
/// the one that started. Without WMI (it can fail to start, or the service runs without rights in
/// a test) it says so and stops; the pass every second still closes them.
pub fn spawn_process_watch(blocklist: Blocklist) {
    thread::spawn(move || {
        let Some(event) = create_service_event() else { return };
        let not_heard =
            |e: wmi::WMIError| note("service", &format!("programs starting are not heard ({e}); they are closed at the next second"));
        // The connection stays for as long as the notifications are read.
        let connection = match wmi::WMIConnection::new() {
            Ok(connection) => connection,
            Err(e) => return not_heard(e),
        };
        let starts = match connection.raw_notification::<ProcessStart>("SELECT ProcessName FROM Win32_ProcessStartTrace") {
            Ok(starts) => starts,
            Err(e) => return not_heard(e),
        };
        for start in starts.flatten() {
            let Some(active) = blocklist.active() else { continue };
            let name = start.process_name;
            if active.blocks_program(&name) || (active.closes_obsidian() && name.eq_ignore_ascii_case("obsidian.exe")) {
                // SAFETY: the event made above, open for as long as the service runs.
                unsafe { SetEvent(event.0) };
            }
        }
        note("service", "the watch on programs starting ended; they are closed at the next second");
    });
}

/// `SERVICE_EVENT`, made by the service, with its access list.
fn create_service_event() -> Option<Event> {
    let sddl = wide(SDDL);
    let name = wide(SERVICE_EVENT);
    // SAFETY: the strings are null-terminated; the descriptor is freed once the event has copied it.
    unsafe {
        let mut descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, null_mut()) == 0 {
            note("service", &format!("cannot read the wake event's access list (error {})", GetLastError()));
            return None;
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = CreateEventW(&attributes, 0, 0, name.as_ptr());
        let error = GetLastError();
        LocalFree(descriptor);
        if handle.is_null() {
            note("service", &format!("cannot make the wake event (error {error})"));
            return None;
        }
        Some(Event(handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_wait_ends_early_when_an_event_says_so_and_is_one_wake_for_many() {
        // Nothing set: the whole timeout.
        let start = Instant::now();
        assert_eq!(wait(Duration::from_millis(200)), Woken::Second);
        assert!(start.elapsed() >= Duration::from_millis(150));
        // Set while waiting (as the hook does): over at once.
        let start = Instant::now();
        thread::spawn(|| {
            thread::sleep(Duration::from_millis(50));
            // SAFETY: the app's own event.
            unsafe { SetEvent(local()) };
        });
        assert_eq!(wait(Duration::from_secs(5)), Woken::Window);
        assert!(start.elapsed() < Duration::from_secs(2));
        // Set twice before a wait: one early end, then the timeout again.
        unsafe { SetEvent(local()) };
        unsafe { SetEvent(local()) };
        assert_eq!(wait(Duration::from_secs(5)), Woken::Window);
        assert_eq!(wait(Duration::from_millis(100)), Woken::Second);
    }
}
