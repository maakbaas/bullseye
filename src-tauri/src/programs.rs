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
    "bullseye-cli.exe",
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
