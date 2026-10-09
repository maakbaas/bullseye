//! The plain Windows calls of the closer: finding windows and the program of a window, closing a
//! window, asking a program to close, and clicking a tab with messages.

use crate::wake;
use uiautomation::patterns::UIWindowPattern;
use uiautomation::UIElement;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW, GWL_EXSTYLE, GW_OWNER, WM_CLOSE, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEMOVE, WS_EX_TOOLWINDOW,
};

/// The programs whose windows without tabs are app windows (`chrome --app=<url>`, installed web apps,
/// popups). Lower case file names. A Chromium window of any other program with no tab strip is some
/// app's own (Spotify, Electron and CEF apps with an address bar), and is never closed as a page.
const BROWSERS: [&str; 12] = [
    "chrome.exe",
    "msedge.exe",
    "brave.exe",
    "vivaldi.exe",
    "opera.exe",
    "chromium.exe",
    "browser.exe",
    "whale.exe",
    "thorium.exe",
    "iron.exe",
    "arc.exe",
    "comet.exe",
];

/// Whether the window belongs to a browser (`BROWSERS`), by the file name of its program. If that cannot be
/// told the window is left alone: closing an app's window by mistake costs more than missing an app window.
pub(super) fn is_browser(handle: isize) -> bool {
    program_file(handle).is_some_and(|file| is_browser_program(&file))
}

/// Whether a program (its lower case file name) is one of `BROWSERS`.
fn is_browser_program(file: &str) -> bool {
    BROWSERS.contains(&file)
}

/// The file name (lower case) of the program a window belongs to, if that can be told.
pub(super) fn program_file(handle: isize) -> Option<String> {
    let mut pid = 0u32;
    // SAFETY: a window handle Windows handed over a moment ago; a stale one only gives no process.
    unsafe { GetWindowThreadProcessId(handle as HWND, &mut pid) };
    if pid == 0 {
        return None;
    }
    // SAFETY: the process handle is closed below, and the buffer is as long as its length says.
    let name = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = [0u16; 520];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        String::from_utf16_lossy(&buffer[..length as usize])
    };
    Some(name.rsplit(['\\', '/']).next().unwrap_or_default().to_lowercase())
}

/// The browsers whose tabs have no Close button to press (it is drawn on hover only, or not exposed at
/// all): a tab is closed there by clicking it. Lower case file names. For every other browser a tab
/// without a button is an error, said in `app.log`, not a click on a guess.
const CLICKED: [&str; 3] = ["opera.exe", "vivaldi.exe", "zen.exe"];

/// Whether this window's browser may have its tabs closed by a middle click (`CLICKED`).
pub(super) fn may_middle_click(handle: isize) -> bool {
    program_file(handle).is_some_and(|file| is_clicked_program(&file))
}

/// Whether a program (its lower case file name) is one of `CLICKED`.
fn is_clicked_program(file: &str) -> bool {
    CLICKED.contains(&file)
}

/// Which family a browser window is of, by its window class.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Chromium,
    Firefox,
}

/// The visible top-level windows of the classes browsers have: all Chromium-based ones share one
/// (Electron apps have it too; they have no address bar and are skipped in `Closer::browser`),
/// and Firefox has its own (Thunderbird too: `Closer::firefox`). Visible ones only, which is what
/// UI Automation lists as the desktop's children too.
pub(super) fn browser_windows() -> Vec<(isize, Kind)> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
        let mut name = [0u16; 32];
        // SAFETY: the buffer is as long as the length says; `window` is one Windows just handed over.
        let (length, visible) = unsafe { (GetClassNameW(window, name.as_mut_ptr(), 32), IsWindowVisible(window) != 0) };
        let class = if length > 0 { String::from_utf16_lossy(&name[..length as usize]) } else { String::new() };
        let kind = match class.as_str() {
            wake::CHROMIUM => Some(Kind::Chromium),
            wake::FIREFOX => Some(Kind::Firefox),
            _ => None,
        };
        if let (true, Some(kind)) = (visible, kind) {
            // SAFETY: `found` is the vector below, which outlives the enumeration.
            unsafe { (*(found as *mut Vec<(isize, Kind)>)).push((window as isize, kind)) };
        }
        1
    }
    let mut found: Vec<(isize, Kind)> = Vec::new();
    // SAFETY: the callback is the function above, and `found` outlives the call.
    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<(isize, Kind)> as LPARAM) };
    found
}

/// Windows that are the desktop and the taskbar, whatever their title.
pub(super) const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// A visible top-level window with a title.
pub(super) struct TopWindow {
    pub(super) handle: isize,
    pub(super) pid: u32,
    pub(super) title: String,
    pub(super) class: String,
    /// Down on the taskbar.
    pub(super) minimized: bool,
    /// A dialog of another window (it has an owner).
    pub(super) owned: bool,
    /// A tool window: no taskbar button of its own.
    pub(super) tool: bool,
}

/// The visible top-level windows that have a title, with the process each belongs to.
pub(super) fn top_level_windows() -> Vec<TopWindow> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
        // SAFETY: `window` is one Windows just handed over, and the buffers are as long as their lengths say.
        unsafe {
            if IsWindowVisible(window) != 0 {
                let length = GetWindowTextLengthW(window);
                if length > 0 {
                    let mut title = vec![0u16; length as usize + 1];
                    let copied = GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32);
                    let mut class = [0u16; 64];
                    let class_length = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(window, &mut pid);
                    if copied > 0 {
                        // SAFETY: `found` is the vector below, which outlives the enumeration.
                        (*(found as *mut Vec<TopWindow>)).push(TopWindow {
                            handle: window as isize,
                            pid,
                            title: String::from_utf16_lossy(&title[..copied as usize]),
                            class: String::from_utf16_lossy(&class[..class_length.max(0) as usize]),
                            minimized: IsIconic(window) != 0,
                            owned: !GetWindow(window, GW_OWNER).is_null(),
                            tool: GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0,
                        });
                    }
                }
            }
        }
        1
    }
    let mut found: Vec<TopWindow> = Vec::new();
    // SAFETY: the callback is the function above, and `found` outlives the call.
    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<TopWindow> as LPARAM) };
    found
}

/// Posts a close request to every top-level window of the process, as the X button does. The
/// program may ask to save first, or not close at all; that is what `KILL_AFTER` is for.
pub(super) fn ask_to_close(pid: u32) {
    unsafe extern "system" fn each(window: HWND, pid: LPARAM) -> i32 {
        let mut owner = 0u32;
        // SAFETY: `window` is one Windows just handed over; `owner` is a valid place to write to.
        unsafe {
            GetWindowThreadProcessId(window, &mut owner);
            if owner == pid as u32 {
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
        }
        1
    }
    // SAFETY: the callback is the function above, and takes no state beyond the process id.
    unsafe { EnumWindows(Some(each), pid as LPARAM) };
}

/// Closes the window, like clicking its X.
pub(super) fn close_window(window: &UIElement) -> uiautomation::Result<()> {
    window.get_pattern::<UIWindowPattern>()?.close()
}

/// Middle-clicks a tab, which closes it in a Chromium browser, with messages posted to the browser's
/// window: the cursor stays where it is and nothing is brought to the front. Where on the tab is
/// well away from its close button.
pub(super) fn middle_click(handle: isize, tab: &UIElement) -> uiautomation::Result<()> {
    let rect = tab.get_bounding_rectangle()?;
    // A tab that is not on screen (scrolled out of its strip) has no place to click.
    if rect.get_width() <= 0 || rect.get_height() <= 0 {
        return Err(uiautomation::Error::new(-1, "the tab is not on screen"));
    }
    let mut point = POINT { x: rect.get_left() + rect.get_width() / 4, y: rect.get_top() + rect.get_height() / 2 };
    // SAFETY: all zeros is a valid rectangle.
    let mut client = unsafe { std::mem::zeroed() };
    // SAFETY: a window handle Windows handed over a moment ago (a stale one only fails), and places to write to.
    let inside = unsafe { ScreenToClient(handle as HWND, &mut point) != 0 && GetClientRect(handle as HWND, &mut client) != 0 }
        && (0..client.right).contains(&point.x)
        && (0..client.bottom).contains(&point.y);
    if !inside {
        return Err(uiautomation::Error::new(-1, "the tab is not in the window"));
    }
    let position = ((point.y as u32 & 0xFFFF) << 16 | (point.x as u32 & 0xFFFF)) as LPARAM;
    // SAFETY: as above; posted, so a window that does not answer holds nothing up.
    unsafe {
        PostMessageW(handle as HWND, WM_MOUSEMOVE, 0, position);
        PostMessageW(handle as HWND, WM_MBUTTONDOWN, 0x10, position);
        PostMessageW(handle as HWND, WM_MBUTTONUP, 0, position);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_browsers_own_programs_are_browsers() {
        for browser in ["chrome.exe", "msedge.exe", "brave.exe", "vivaldi.exe", "opera.exe", "chromium.exe", "browser.exe", "arc.exe"] {
            assert!(is_browser_program(browser), "{browser}");
        }
        // Apps made of Chromium that have an address bar of their own, or none: never a browser.
        for app in ["spotify.exe", "discord.exe", "code.exe", "slack.exe", "obsidian.exe", "steamwebhelper.exe", "firefox.exe", ""] {
            assert!(!is_browser_program(app), "{app}");
        }
        // Names are lower case (`program_file` makes them so), whole, and with their extension.
        assert!(!is_browser_program("chrome") && !is_browser_program("notchrome.exe") && !is_browser_program("chrome.exe.bak"));
    }

    #[test]
    fn a_tab_is_clicked_only_where_it_has_no_button_to_press() {
        for clicked in ["opera.exe", "vivaldi.exe", "zen.exe"] {
            assert!(is_clicked_program(clicked), "{clicked}");
        }
        // The others close a tab by its button, and say so when it is missing.
        for pressed in ["chrome.exe", "msedge.exe", "brave.exe", "firefox.exe", "spotify.exe", ""] {
            assert!(!is_clicked_program(pressed), "{pressed}");
        }
    }

    #[test]
    fn a_window_that_is_not_there_has_no_program() {
        assert_eq!(program_file(0), None);
        assert!(!is_browser(0) && !may_middle_click(0));
    }

    #[test]
    fn the_browser_windows_found_are_of_the_two_families() {
        // Whatever is open on the machine, every window found is of one of the two families.
        for (handle, kind) in browser_windows() {
            assert!(handle != 0 && matches!(kind, Kind::Chromium | Kind::Firefox));
        }
        assert_eq!(wake::CHROMIUM, "Chrome_WidgetWin_1");
        assert_eq!(wake::FIREFOX, "MozillaWindowClass");
    }
}
