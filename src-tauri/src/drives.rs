//! Flash drives and other external disks, told apart by the **serial number** their hardware
//! reports, not by what is on them or the letter they get (both change; formatting a drive changes
//! its volume serial, too). The serial comes from the disk itself, as Windows' Storage
//! Management lists it (`MSFT_PhysicalDisk`, what PowerShell's `Get-PhysicalDisk` shows), for the
//! disks on the USB bus, which also covers external disks that Windows drives over UASP.
//!
//! Only as unique as the maker made it: a drive that reports no serial cannot be told apart, so it
//! is not offered, and cheap drives that share one serial across a batch look like the same drive
//! (one is offered, and any of them unlocks).
//!
//! A flash drive condition (`condition::Rule::FlashDrive`) is active while the drive is *not*
//! connected, so the service asks `is_present` whenever it decides whether a session is active or
//! can be stopped.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::sync::LockExt;

/// `MSFT_PhysicalDisk.BusType` of a disk on the USB bus.
const BUS_USB: u16 = 7;

/// How long a look at the connected drives is good for, so asking whether a session can be stopped
/// does not ask Windows every time.
const FRESH: Duration = Duration::from_secs(2);

/// A drive that is connected, as the window offers it.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Drive {
    /// What tells it apart: the serial number its hardware reports.
    pub serial: String,
    /// What Windows calls it (`Kingston DataTraveler 3.0`).
    pub name: String,
}

/// What Windows says of a disk.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PhysicalDisk {
    friendly_name: Option<String>,
    serial_number: Option<String>,
}

/// The USB drives connected right now that have a serial number, by name; asked of Windows afresh.
pub fn connected() -> Result<Vec<Drive>, String> {
    let storage = wmi::WMIConnection::with_namespace_path(r"ROOT\Microsoft\Windows\Storage")
        .map_err(|e| format!("Cannot ask Windows for the connected drives: {e}"))?;
    let disks: Vec<PhysicalDisk> = storage
        .raw_query(format!("SELECT FriendlyName, SerialNumber FROM MSFT_PhysicalDisk WHERE BusType = {BUS_USB}"))
        .map_err(|e| format!("Cannot ask Windows for the connected drives: {e}"))?;
    let mut drives: Vec<Drive> = Vec::new();
    for disk in disks {
        let serial = tidy(&disk.serial_number.unwrap_or_default());
        if serial.is_empty() || drives.iter().any(|drive| same(&drive.serial, &serial)) {
            continue;
        }
        let name = disk.friendly_name.unwrap_or_default().trim().to_string();
        drives.push(Drive { serial, name: if name.is_empty() { "USB drive".into() } else { name } });
    }
    drives.sort_by_key(|drive| drive.name.to_lowercase());
    Ok(drives)
}

/// Whether the drive with this serial number is connected, as of the last couple of seconds. If
/// Windows cannot be asked, it is not: a lock that waits for the drive stays locked then.
pub fn is_present(serial: &str) -> bool {
    static LAST: Mutex<Option<(Instant, Vec<Drive>)>> = Mutex::new(None);
    let mut last = LAST.locked();
    if last.as_ref().is_none_or(|(looked, _)| looked.elapsed() >= FRESH) {
        *last = Some((Instant::now(), connected().unwrap_or_default()));
    }
    last.as_ref().is_some_and(|(_, drives)| drives.iter().any(|drive| same(&drive.serial, serial)))
}

/// Serial numbers come padded with spaces, and sometimes end in a dot.
fn tidy(serial: &str) -> String {
    serial.trim().trim_end_matches('.').trim().to_string()
}

fn same(a: &str, b: &str) -> bool {
    tidy(a).eq_ignore_ascii_case(&tidy(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_numbers_are_compared_without_padding_or_case() {
        assert_eq!(tidy("  4E0985D05EC9 "), "4E0985D05EC9");
        assert_eq!(tidy("0025_3886_11B6_3DE0."), "0025_3886_11B6_3DE0");
        assert!(same("4e0985d05ec9", " 4E0985D05EC9"));
        assert!(!same("4E0985D05EC9", "4E0985D05ECA"));
    }

    /// With a USB drive connected: `cargo test -- --ignored a_connected`.
    #[test]
    #[ignore = "needs a USB drive connected"]
    fn a_connected_drive_is_listed_and_found() {
        let drives = connected().unwrap();
        println!("{drives:#?}");
        assert!(!drives.is_empty(), "connect a USB drive with a serial number first");
        assert!(is_present(&drives[0].serial));
        assert!(!is_present("no-such-serial-number"));
    }
}
