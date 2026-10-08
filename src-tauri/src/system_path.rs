//! The installation folder on the system `PATH`, so `bullseye` (the CLI) runs from any terminal:
//! added by `--install`, taken out by `--uninstall` (`setup.rs`), both run as an administrator.
//! What changes in the list is decided by `added` and `removed`, which are platform-free; the rest
//! reads and writes the machine's `Path` in the registry and tells running programs it changed.

/// `path` (a `PATH` value, `;` between folders) with `dir` at its end, or `None` when it has it
/// already (`same_dir`).
pub fn added(path: &str, dir: &str) -> Option<String> {
    if path.split(';').any(|entry| same_dir(entry, dir)) {
        return None;
    }
    let kept = path.trim_end_matches(';');
    Some(if kept.is_empty() { dir.to_string() } else { format!("{kept};{dir}") })
}

/// `path` without `dir`, wherever and however often it is in it, or `None` when it is not. Every
/// other entry stays as it was, in its place.
pub fn removed(path: &str, dir: &str) -> Option<String> {
    let entries: Vec<&str> = path.split(';').collect();
    if !entries.iter().any(|entry| same_dir(entry, dir)) {
        return None;
    }
    Some(entries.into_iter().filter(|entry| !same_dir(entry, dir)).collect::<Vec<_>>().join(";"))
}

/// Whether a `PATH` entry names this folder, as Windows reads it: letter case, quotes, spaces
/// around it and a closing backslash aside.
fn same_dir(entry: &str, dir: &str) -> bool {
    let plain = |text: &str| text.trim().trim_matches('"').trim_end_matches(['\\', '/']).to_lowercase();
    let entry = plain(entry);
    !entry.is_empty() && entry == plain(dir)
}

#[cfg(windows)]
pub use windows::{add, remove};

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};

    const KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    const VALUE: &str = "Path";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// Puts `dir` on the machine's `PATH`, if it is not on it.
    pub fn add(dir: &str) -> Result<(), String> {
        change(|path| super::added(path, dir))
    }

    /// Takes `dir` off the machine's `PATH`, if it is on it.
    pub fn remove(dir: &str) -> Result<(), String> {
        change(|path| super::removed(path, dir))
    }

    /// Reads the machine's `Path`, writes back what `edit` makes of it (nothing if `None`), as the
    /// kind of value it was, and tells running programs (a new terminal sees it then).
    fn change(edit: impl Fn(&str) -> Option<String>) -> Result<(), String> {
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: the key name is null-terminated and `key` is a valid out pointer.
        let opened = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wide(KEY).as_ptr(), 0, KEY_READ | KEY_WRITE, &mut key) };
        if opened != ERROR_SUCCESS {
            return Err(format!("cannot open the system's environment (error {opened})"));
        }
        let result = read(key).and_then(|(path, kind)| match edit(&path) {
            Some(new) => write(key, &new, kind),
            None => Ok(false),
        });
        // SAFETY: `key` was opened above and is closed once.
        unsafe { RegCloseKey(key) };
        if result? {
            let mut answer = 0;
            // SAFETY: "Environment" is null-terminated and outlives the call; the answer is a valid out pointer.
            unsafe {
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    0,
                    wide("Environment").as_ptr() as isize,
                    SMTO_ABORTIFHUNG,
                    5000,
                    &mut answer,
                )
            };
        }
        Ok(())
    }

    /// The value and its kind; an empty one when there is none.
    fn read(key: HKEY) -> Result<(String, u32), String> {
        let name = wide(VALUE);
        let (mut kind, mut size) = (0u32, 0u32);
        // SAFETY: asks for the size only: no buffer.
        let found = unsafe { RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut size) };
        if found != ERROR_SUCCESS {
            return Ok((String::new(), REG_EXPAND_SZ));
        }
        if kind != REG_EXPAND_SZ && kind != REG_SZ {
            return Err(format!("the system's Path is not text (kind {kind})"));
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: `buffer` holds `size` bytes.
        let read = unsafe { RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), &mut kind, buffer.as_mut_ptr().cast(), &mut size) };
        if read != ERROR_SUCCESS {
            return Err(format!("cannot read the system's Path (error {read})"));
        }
        let text = &buffer[..(size as usize / 2).min(buffer.len())];
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        Ok((String::from_utf16_lossy(&text[..end]), kind))
    }

    fn write(key: HKEY, path: &str, kind: u32) -> Result<bool, String> {
        let value = wide(path);
        // SAFETY: the name and value are null-terminated, and the size is the value's, its null included.
        let written = unsafe { RegSetValueExW(key, wide(VALUE).as_ptr(), 0, kind, value.as_ptr().cast(), (value.len() * 2) as u32) };
        if written != ERROR_SUCCESS {
            return Err(format!("cannot write the system's Path (error {written})"));
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = r"C:\Program Files\Bullseye";

    #[test]
    fn the_folder_goes_at_the_end_once() {
        assert_eq!(added(r"C:\Windows;C:\Tools", DIR).as_deref(), Some(r"C:\Windows;C:\Tools;C:\Program Files\Bullseye"));
        // A closing ; is not an empty entry before it, and an empty Path gets just the folder.
        assert_eq!(added(r"C:\Windows;", DIR).as_deref(), Some(r"C:\Windows;C:\Program Files\Bullseye"));
        assert_eq!(added("", DIR).as_deref(), Some(DIR));
        // Already there, however it is written: nothing changes.
        for there in [DIR, r"c:\program files\bullseye\", r#""C:\Program Files\Bullseye""#, r" C:\Program Files\Bullseye ;C:\Windows"] {
            assert_eq!(added(there, DIR), None, "{there}");
        }
        // Not a folder inside it, or one that starts the same.
        assert!(added(r"C:\Program Files\Bullseye\bin;C:\Program Files\Bullseye2", DIR).is_some());
    }

    #[test]
    fn the_folder_goes_wherever_it_is_and_nothing_else_does() {
        assert_eq!(removed(r"C:\Windows;C:\Program Files\Bullseye;C:\Tools", DIR).as_deref(), Some(r"C:\Windows;C:\Tools"));
        assert_eq!(removed(r"c:\program files\bullseye\;C:\Windows;C:\Program Files\Bullseye", DIR).as_deref(), Some(r"C:\Windows"));
        assert_eq!(removed(DIR, DIR).as_deref(), Some(""));
        // Other entries stay as they were, empty ones and %variables% too.
        assert_eq!(removed(r"%SystemRoot%;;C:\Program Files\Bullseye;", DIR).as_deref(), Some("%SystemRoot%;;"));
        // Not there: nothing to write.
        assert_eq!(removed(r"C:\Windows;C:\Program Files\Bullseye2", DIR), None);
        assert_eq!(removed("", DIR), None);
        // Added and taken off again is what it was.
        let path = r"C:\Windows;C:\Tools";
        assert_eq!(removed(&added(path, DIR).unwrap(), DIR).as_deref(), Some(path));
    }
}
