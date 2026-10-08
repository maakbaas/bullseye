//! Apps installed from the Microsoft Store (packaged apps), blocked by their **package family
//! name**: `Microsoft.WindowsCalculator_8wekyb3d8bbwe`, exactly, letter case aside: the window ticks
//! them from the installed ones, and a `*` is refused (`lists::store_apps`).
//!
//! A packaged app runs from its own folder under `WindowsApps`, named
//! `<name>_<version>_<architecture>__<publisher id>`, and the family name is the name and the
//! publisher id of it. So the app a process belongs to is read from where its exe is, with nothing
//! to look up. What the window offers to tick (`installed`) is what the Start menu lists of them,
//! by name, from PowerShell's `Get-StartApps`, whose ids are `<family name>!<app>`.
//!
//! Apps Windows itself is made of (the shell, Settings, the lock screen, runtimes) are never
//! closed, whatever the list says (`PROTECTED`), and are not offered.

// Windows only, so that the tests of the platform-free code build elsewhere too (`scripts/linux-tests`).
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Family names (lower case) that begin like these are Windows' own and never closed.
const PROTECTED: [&str; 12] = [
    "microsoft.windows.",
    "microsoftwindows.",
    "windows.",
    "microsoft.aad.",
    "microsoft.accountscontrol",
    "microsoft.lockapp",
    "microsoft.ui.xaml",
    "microsoft.vclibs",
    "microsoft.net.",
    "microsoft.microsoftedge",
    "microsoft.win32webviewhost",
    "microsoft.xboxgamecallableui",
];

/// `CREATE_NO_WINDOW`: PowerShell is a console program, which would flash a window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The Start menu's packaged apps: a name and the family name in each id (`<family>!<app>`; the
/// ids of other programs have no `!` after an underscore-and-publisher family name).
const SCRIPT: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
    Get-StartApps | Where-Object { $_.AppID -like '*_*!*' } | \
    ForEach-Object { [pscustomobject]@{ name = $_.Name; family = ($_.AppID -split '!')[0] } } | \
    ConvertTo-Json -Compress";

/// An app that can be blocked, for the window to offer: an installed one, or one a block list has
/// that is not installed here (it came from another computer, by Import).
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct StoreApp {
    /// What the Start menu calls it; for one not installed, what its family name says (`readable`).
    pub name: String,
    /// What the list holds.
    pub family: String,
    /// Whether it is installed here.
    #[serde(default)]
    pub installed: bool,
}

/// An app's name as its family name says it, for one that is not installed here, where the Start
/// menu cannot be asked: the part after the publisher's prefix and before the publisher id
/// (`AnthropicPBC.Claude_fnn82j28hfe8t` is "Claude"), else the family name itself.
pub fn readable(family: &str) -> String {
    let name = family.split('_').next().unwrap_or_default();
    let last = name.rsplit('.').next().unwrap_or_default().trim();
    if last.is_empty() {
        family.to_string()
    } else {
        last.to_string()
    }
}

/// What the window offers: the installed apps, and after them the `chosen` ones (a block list's
/// entries) that are not installed here, named by `readable`.
pub fn offered(mut apps: Vec<StoreApp>, chosen: &[String]) -> Vec<StoreApp> {
    for family in chosen.iter().map(|f| f.trim()).filter(|f| !f.is_empty()) {
        if !apps.iter().any(|app| app.family.eq_ignore_ascii_case(family)) {
            apps.push(StoreApp { name: readable(family), family: family.to_string(), installed: false });
        }
    }
    apps
}

/// Whether the app is part of Windows, which is never closed.
pub fn is_protected(family: &str) -> bool {
    let family = family.to_lowercase();
    PROTECTED.iter().any(|start| family.starts_with(start))
}

/// The package family name of the app a program belongs to, from where its exe is
/// (`...\WindowsApps\<name>_<version>_<architecture>__<publisher id>\app.exe`), or None for a
/// program that is not a packaged app.
pub fn family_of(exe: &Path) -> Option<String> {
    let mut parts = exe.components().map(|part| part.as_os_str().to_string_lossy());
    parts.find(|part| part.eq_ignore_ascii_case("WindowsApps"))?;
    let folder = parts.next()?;
    let (name, _) = folder.split_once('_')?;
    let (_, publisher) = folder.rsplit_once('_')?;
    (!name.is_empty() && !publisher.is_empty()).then(|| format!("{name}_{publisher}"))
}

/// The installed apps that can be blocked, by name. Asks PowerShell, which takes a second or two.
pub fn installed() -> Result<Vec<StoreApp>, String> {
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().map_err(|e| format!("Cannot ask PowerShell for the installed apps: {e}"))?;
    if !output.status.success() {
        return Err(format!("PowerShell could not list the installed apps: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// What the script printed: nothing, one app (an object) or several (an array).
fn parse(output: &str) -> Result<Vec<StoreApp>, String> {
    let output = output.trim().trim_start_matches('\u{feff}');
    if output.is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value = serde_json::from_str(output).map_err(|e| format!("Unreadable list of apps: {e}"))?;
    let found: Vec<StoreApp> = match value {
        serde_json::Value::Array(items) => items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect(),
        item => serde_json::from_value(item).into_iter().collect(),
    };
    let mut apps: Vec<StoreApp> = Vec::new();
    for app in found {
        // One entry per app, though a package can have several in the Start menu.
        if !app.name.trim().is_empty() && !is_protected(&app.family) && !apps.iter().any(|a| a.family.eq_ignore_ascii_case(&app.family)) {
            apps.push(StoreApp { installed: true, ..app });
        }
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_of_a_program_is_read_from_where_it_is() {
        let calculator =
            Path::new(r"C:\Program Files\WindowsApps\Microsoft.WindowsCalculator_11.2502.2.0_x64__8wekyb3d8bbwe\CalculatorApp.exe");
        assert_eq!(family_of(calculator).as_deref(), Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe"));
        // A resource id between the architecture and the publisher changes nothing.
        let with_resources = Path::new(r"C:\Program Files\WindowsApps\A.B_1.0.0.0_neutral_split.language-nl_8wekyb3d8bbwe\x.exe");
        assert_eq!(family_of(with_resources).as_deref(), Some("A.B_8wekyb3d8bbwe"));
        assert_eq!(family_of(Path::new(r"C:\Program Files\Steam\steam.exe")), None);
        assert_eq!(family_of(Path::new(r"C:\Program Files\WindowsApps")), None);
    }

    #[test]
    fn windows_own_apps_are_protected() {
        assert!(is_protected("Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy"));
        assert!(is_protected("windows.immersivecontrolpanel_cw5n1h2txyewy"));
        assert!(is_protected("MicrosoftWindows.Client.CBS_cw5n1h2txyewy"));
        assert!(!is_protected("Microsoft.WindowsCalculator_8wekyb3d8bbwe"));
        assert!(!is_protected("SpotifyAB.SpotifyMusic_zpdnekdrzrea0"));
    }

    #[test]
    fn the_scripts_output_is_one_entry_per_app_by_name() {
        let two = r#"[{"name":"Spotify","family":"SpotifyAB.SpotifyMusic_zpdnekdrzrea0"},{"name":"Calculator","family":"Microsoft.WindowsCalculator_8wekyb3d8bbwe"},{"name":"Calculator (2)","family":"microsoft.windowscalculator_8wekyb3d8bbwe"},{"name":"Settings","family":"windows.immersivecontrolpanel_cw5n1h2txyewy"}]"#;
        let apps = parse(two).unwrap();
        assert_eq!(apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Calculator", "Spotify"]);
        // PowerShell prints a lone object, not an array, for one app; and nothing for none.
        assert_eq!(parse(r#"{"name":"Spotify","family":"SpotifyAB.SpotifyMusic_zpdnekdrzrea0"}"#).unwrap().len(), 1);
        assert!(parse("").unwrap().is_empty());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn an_app_that_is_not_installed_here_is_offered_by_the_name_in_its_family() {
        assert_eq!(readable("AnthropicPBC.Claude_fnn82j28hfe8t"), "Claude");
        assert_eq!(readable("Microsoft.WindowsCalculator_8wekyb3d8bbwe"), "WindowsCalculator");
        assert_eq!(readable("SpotifyMusic_zpdnekdrzrea0"), "SpotifyMusic");
        assert_eq!(readable("odd."), "odd.");
        let installed = parse(r#"{"name":"Spotify","family":"SpotifyAB.SpotifyMusic_zpdnekdrzrea0"}"#).unwrap();
        assert!(installed[0].installed);
        let chosen = ["spotifyab.spotifymusic_zpdnekdrzrea0".to_string(), "AnthropicPBC.Claude_fnn82j28hfe8t".to_string(), " ".to_string()];
        let apps = offered(installed, &chosen);
        assert_eq!(
            apps.iter().map(|a| (a.name.as_str(), a.installed)).collect::<Vec<_>>(),
            vec![("Spotify", true), ("Claude", false)],
            "an installed one is not offered twice, whatever its letter case"
        );
        assert_eq!(apps[1].family, "AnthropicPBC.Claude_fnn82j28hfe8t");
    }
}
