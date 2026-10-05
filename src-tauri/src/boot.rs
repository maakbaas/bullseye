//! Which start of Windows this is, for the lock that lasts until the computer restarts
//! (`lock::Lock::UntilRestart`).
//!
//! The id is kept in a volatile registry key, which Windows forgets when it restarts: the first
//! process to ask after a restart makes a new one, and a block saved with another id has ended.
//! The service keeps it under HKLM (`SOFTWARE\bullseye-boot`), which a standard user cannot write;
//! the debug build's embedded service, running as the user, falls back to HKCU. A shut down with
//! Fast Startup keeps it: Windows hibernates then rather than restarts.

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE,
    REG_BINARY, REG_CREATED_NEW_KEY, REG_OPTION_VOLATILE,
};

const KEY: &str = r"SOFTWARE\bullseye-boot";
const VALUE: &str = "id";

/// This start of Windows, the same for every call until it restarts; `None` if the registry
/// cannot be reached.
pub fn id() -> Option<String> {
    static ID: OnceLock<Option<String>> = OnceLock::new();
    ID.get_or_init(|| read_or_make(HKEY_LOCAL_MACHINE).or_else(|| read_or_make(HKEY_CURRENT_USER))).clone()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn read_or_make(root: HKEY) -> Option<String> {
    let mut key: HKEY = std::ptr::null_mut();
    let mut disposition = 0;
    // SAFETY: the strings are null-terminated and the out pointers are valid.
    let made = unsafe {
        RegCreateKeyExW(
            root,
            wide(KEY).as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_VOLATILE,
            KEY_READ | KEY_WRITE,
            std::ptr::null(),
            &mut key,
            &mut disposition,
        )
    };
    if made != ERROR_SUCCESS {
        return None;
    }
    let id = if disposition == REG_CREATED_NEW_KEY { make(key) } else { read(key) };
    // SAFETY: `key` was opened above and is closed once.
    unsafe { RegCloseKey(key) };
    id
}

/// A new random id, written to the new key.
fn make(key: HKEY) -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    // SAFETY: the value name is null-terminated and `bytes` is as long as said.
    let written = unsafe { RegSetValueExW(key, wide(VALUE).as_ptr(), 0, REG_BINARY, bytes.as_ptr(), bytes.len() as u32) };
    (written == ERROR_SUCCESS).then(|| hex(&bytes))
}

fn read(key: HKEY) -> Option<String> {
    let mut bytes = [0u8; 16];
    let mut size = bytes.len() as u32;
    let mut kind = 0;
    // SAFETY: the value name is null-terminated and `bytes` holds `size` bytes.
    let found = unsafe { RegQueryValueExW(key, wide(VALUE).as_ptr(), std::ptr::null(), &mut kind, bytes.as_mut_ptr(), &mut size) };
    (found == ERROR_SUCCESS && kind == REG_BINARY && size == 16).then(|| hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_id_stays_the_same_until_windows_restarts() {
        let id = super::id().expect("a boot id");
        assert_eq!(id.len(), 32);
        // Asked again, as the service is after it was restarted, it is the same.
        let again = super::read_or_make(super::HKEY_LOCAL_MACHINE).or_else(|| super::read_or_make(super::HKEY_CURRENT_USER));
        assert_eq!(again, Some(id));
    }
}
