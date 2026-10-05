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

const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager, String> {
    ServiceManager::local_computer(None::<&str>, access).map_err(|e| format!("cannot reach the service manager: {e}"))
}

fn is_os_error(error: &windows_service::Error, code: i32) -> bool {
    matches!(error, windows_service::Error::Winapi(e) if e.raw_os_error() == Some(code))
}

/// Registers the service for `exe`, locks it down, and starts it. Needs an administrator, and a
/// machine without the service (Bullseye is only installed on a clean machine).
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
    let service = manager.create_service(&info, access).map_err(|e| format!("cannot create the service: {e}"))?;
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
    service.start::<OsString>(&[]).map_err(|e| format!("cannot start the service: {e}"))
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
