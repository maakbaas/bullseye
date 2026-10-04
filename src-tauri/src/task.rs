//! The login task, registered by the installer (`SiteBlocker.exe --install`, see `setup.rs`),
//! that runs the entry point at every logon.
//!
//! It is an ordinary named task in Task Scheduler, listed under Task Scheduler's Task Library
//! and removed by uninstalling the app (Settings > Apps). A forced stop turns it off so that
//! nothing starts by itself again; starting the app turns it back on.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output};

/// The task's name, exactly as it appears in Task Scheduler.
pub const NAME: &str = "SiteBlocker";

/// `CREATE_NO_WINDOW`: schtasks is a console program, and the app has no console of its own, so
/// without this every call would flash a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Whether the login task is registered. Running without it is fine (e.g. in development).
pub fn exists() -> bool {
    exists_as(NAME)
}

/// Turns the login task on or off. A task that was never installed is left as it is: there is
/// nothing that could start the pair again, so there is nothing to turn off.
///
/// The installer registers it for this user without elevation, so the app can always do this;
/// a task registered from an administrator shell by hand would need an administrator, and the
/// caller reports that rather than hides it.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if !exists() {
        return Ok(());
    }
    let (flag, state) = if enabled { ("/ENABLE", "on") } else { ("/DISABLE", "off") };
    check(schtasks(&["/Change", "/TN", NAME, flag]), &format!("the {NAME} task could not be turned {state}"))
}

/// Registers the login task to run `exe --launch` at every logon, replacing any earlier one.
pub fn install(exe: &Path) -> Result<(), String> {
    install_as(NAME, exe)
}

/// Removes the login task. A task that is not there is not an error.
pub fn remove() -> Result<(), String> {
    remove_as(NAME)
}

fn exists_as(name: &str) -> bool {
    schtasks(&["/Query", "/TN", name]).is_ok_and(|output| output.status.success())
}

fn install_as(name: &str, exe: &Path) -> Result<(), String> {
    let user = std::env::var("USERNAME").map_err(|_| "no USERNAME to register the task for")?;
    let user = match std::env::var("USERDOMAIN") {
        Ok(domain) => format!(r"{domain}\{user}"),
        Err(_) => user,
    };
    // Only an XML file carries every setting schtasks needs here (no time limit, which a
    // day-long block needs). UTF-16 with a byte order mark, as its declaration says.
    let file = std::env::temp_dir().join(format!("{name}-task.xml"));
    let utf16: Vec<u8> =
        [0xfeff_u16].into_iter().chain(task_xml(&user, exe).encode_utf16()).flat_map(u16::to_le_bytes).collect();
    std::fs::write(&file, utf16).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    let output = schtasks(&["/Create", "/TN", name, "/XML", &file.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&file);
    check(output, &format!("the {name} task could not be registered"))
}

fn remove_as(name: &str) -> Result<(), String> {
    if !exists_as(name) {
        return Ok(());
    }
    check(schtasks(&["/Delete", "/TN", name, "/F"]), &format!("the {name} task could not be removed"))
}

/// The task as Task Scheduler XML. Registered for this user without elevation, so the app can
/// turn it on and off itself. It runs in the user's interactive session, since closing browser
/// tabs goes through UI Automation, which needs their desktop (so not as SYSTEM).
fn task_xml(user: &str, exe: &Path) -> String {
    let user = escape(user);
    let exe = escape(&exe.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts Site Blocker at logon. Installed and removed with the Site Blocker app (Settings &gt; Apps).</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <!-- Retry if the entry point fails to start at logon. (A launcher that is ended later is
         started again by the app, not by this; see watchdog.rs.) -->
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <!-- No time limit: a block may be a day long. -->
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--launch</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn check(output: std::io::Result<Output>, what: &str) -> Result<(), String> {
    let output = output.map_err(|e| format!("cannot run schtasks: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let because = if reason.is_empty() { format!("exit code {}", output.status) } else { reason };
    Err(format!("{what}: {because}"))
}

fn schtasks(args: &[&str]) -> std::io::Result<Output> {
    Command::new("schtasks.exe").args(args).creation_flags(CREATE_NO_WINDOW).output()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_is_named_plainly() {
        // Deliberately not disguised: this name is what shows up in Task Scheduler.
        assert_eq!(NAME, "SiteBlocker");
        assert!(NAME.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn a_task_that_is_not_registered_does_not_block_a_forced_stop() {
        // Only where the task was never installed: elsewhere this would turn the real task off.
        if exists() {
            return eprintln!("skipping: the {NAME} task is installed");
        }
        assert_eq!(set_enabled(false), Ok(()));
        assert_eq!(set_enabled(true), Ok(()));
    }

    #[test]
    fn the_task_runs_the_entry_point_of_this_exe() {
        let xml = task_xml(r"PC\me", Path::new(r"C:\Users\me\AppData\Local\Site Blocker\SiteBlocker.exe"));
        assert!(xml.contains(r"<Command>C:\Users\me\AppData\Local\Site Blocker\SiteBlocker.exe</Command>"));
        assert!(xml.contains("<Arguments>--launch</Arguments>"));
        assert!(xml.contains(r"<UserId>PC\me</UserId>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert_eq!(escape(r#"a&b<"c">'"#), "a&amp;b&lt;&quot;c&quot;&gt;&apos;");
    }

    /// Registers and removes a real task under a test name: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_task_can_be_registered_and_removed_without_elevation() {
        let name = "SiteBlocker-test";
        let exe = std::env::current_exe().unwrap();
        install_as(name, &exe).unwrap();
        assert!(exists_as(name));
        // Again, over the existing one, as a reinstall does.
        install_as(name, &exe).unwrap();
        remove_as(name).unwrap();
        assert!(!exists_as(name));
        assert_eq!(remove_as(name), Ok(()));
    }
}
