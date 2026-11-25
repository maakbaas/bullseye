//! Blocked programs, recognized by what is written inside their file, so renaming or moving one does not
//! get it past a list.
//!
//! A program is picked (a file, or a running program), never typed: its file name and three fields of its
//! version info (what Properties → Details shows, written in by whoever built it) make the entry. A
//! running program matches an entry
//! - by its **original file name and its company**, when the entry has an original file name (renaming a
//!   file leaves the name inside it as it was);
//! - else by its **description and its company**, when the entry has a description (Claude Code has no
//!   original file name, but "Claude Code" as its description);
//! - and always by its **file name**, which is all an entry rests on when its program has no version info.
//!
//! Comparisons ignore letter case and the spaces around a field; an empty field counts as missing. The
//! product name is not used: some products put their version number in it, which changes with updates.
//! Signatures are not checked either: editing the version info breaks a signature, but the edited file
//! then matches nothing anyway, and that edit is the one way past an entry.
//!
//! An entry is one line of text, the fields joined by `|` (which no file name holds; one in a field
//! read from a file becomes `/`): `steam.exe|steam.exe|Steam|Valve Corporation` is the file name, the
//! original file name, the description and the company. So the lists stay lists of text everywhere
//! (`lists::Entries`): saved, sent, synced and compared as the other lists are.
//!
//! Some programs must never be closed, because closing them breaks Windows or this app (`PROTECTED`):
//! recognized by their version info (an original file name of `PROTECTED` from Microsoft, or Bullseye's
//! own), so a game renamed `svchost.exe` is not spared; a process whose file cannot be read (`System`) by
//! its name. An entry for one of them is refused.

use serde::Serialize;

use crate::strings::t;

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
    "bullseye-app.exe",
    "bullseye.exe",
    "system",
    "registry",
    "memory compression",
    "secure system",
];

/// The company of Windows' own programs.
const MICROSOFT: &str = "microsoft corporation";

/// What Bullseye's own programs say they are (`productName` and `publisher` in `tauri.conf.json`), as
/// description and company: they have no original file name.
const BULLSEYE: &str = "bullseye";

/// Characters a file name cannot contain, which also keeps paths out.
const NOT_IN_A_NAME: [char; 8] = ['\\', '/', ':', '"', '<', '>', '|', '?'];

/// A program as its file says: the file name and the fields of its version info that tell it apart.
/// Fields are trimmed, empty when missing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub name: String,
    pub original: String,
    pub description: String,
    pub company: String,
}

/// What an entry is recognized by, besides its file name (`Program::basis`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Basis {
    Original,
    Description,
    Name,
}

/// An entry as the window and the CLI show it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Described {
    /// The entry itself, as the list keeps it.
    pub entry: String,
    /// What it is: its file name.
    pub label: String,
    /// What it is recognized by, in a sentence.
    pub basis: String,
}

/// A running program, for the window and the CLI to pick from: its entry described, and where its file is.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Running {
    #[serde(flatten)]
    pub described: Described,
    pub path: String,
}

impl Program {
    /// A program known by its file name only (its file has no version info, or cannot be read).
    pub fn named(name: &str) -> Program {
        Program { name: clean(name), ..Program::default() }
    }

    /// The program from its fields as read from a file: each trimmed, a `|` made `/`.
    pub fn read(name: &str, original: &str, description: &str, company: &str) -> Program {
        Program { name: clean(name), original: clean(original), description: clean(description), company: clean(company) }
    }

    /// The entry: the fields joined by `|`.
    pub fn entry(&self) -> String {
        [&self.name, &self.original, &self.description, &self.company].map(|f| f.as_str()).join("|")
    }

    /// An entry read back. The error says in words what is wrong.
    pub fn parse(entry: &str) -> Result<Program, String> {
        let fields: Vec<&str> = entry.split('|').collect();
        let [name, original, description, company] = fields[..] else {
            return Err(t("targets.programs.notAnEntry", &[("entry", entry.trim())]));
        };
        let program = Program::read(name, original, description, company);
        if program.name.is_empty() || program.name.contains(NOT_IN_A_NAME) {
            return Err(t("targets.programs.notAnEntry", &[("entry", entry.trim())]));
        }
        Ok(program)
    }

    /// What the program is recognized by, besides its file name.
    pub fn basis(&self) -> Basis {
        if !self.original.is_empty() {
            Basis::Original
        } else if !self.description.is_empty() {
            Basis::Description
        } else {
            Basis::Name
        }
    }

    /// What tells the program apart, letter case aside: copies of one program under other file names
    /// are one (`running` lists them once).
    pub fn identity(&self) -> String {
        match self.basis() {
            Basis::Original => format!("original|{}|{}", self.original, self.company),
            Basis::Description => format!("description|{}|{}", self.description, self.company),
            Basis::Name => format!("name|{}", self.name),
        }
        .to_lowercase()
    }

    /// Whether `running` is this entry's program: the same file name, or the same original file name
    /// (else description) and company, letter case aside.
    pub fn matches(&self, running: &Program) -> bool {
        let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
        same(&self.name, &running.name)
            || match self.basis() {
                Basis::Original => same(&self.original, &running.original) && same(&self.company, &running.company),
                Basis::Description => same(&self.description, &running.description) && same(&self.company, &running.company),
                Basis::Name => false,
            }
    }

    /// Whether this is one of the programs that are never closed (`PROTECTED`): by its version info when
    /// it has some, else by its name.
    pub fn is_protected(&self) -> bool {
        let company = self.company.to_lowercase();
        if self.basis() == Basis::Name {
            return is_protected(&self.name);
        }
        // Windows' own read their version info from a language file, which names that file.
        let original = self.original.to_lowercase();
        let original = original.strip_suffix(".mui").unwrap_or(&original);
        (company == MICROSOFT && PROTECTED.contains(&original)) || (company == BULLSEYE && self.description.eq_ignore_ascii_case(BULLSEYE))
    }

    /// The entry as the window and the CLI show it.
    pub fn describe(&self) -> Described {
        let values = [("original", self.original.as_str()), ("description", self.description.as_str()), ("company", self.company.as_str())];
        let key = match (self.basis(), self.company.is_empty()) {
            (Basis::Original, false) => "original",
            (Basis::Original, true) => "originalOnly",
            (Basis::Description, false) => "description",
            (Basis::Description, true) => "descriptionOnly",
            (Basis::Name, _) => "name",
        };
        Described { entry: self.entry(), label: self.name.clone(), basis: t(&format!("targets.programs.basis.{key}"), &values) }
    }
}

/// Trimmed, without `|` (which joins an entry's fields) or control characters.
fn clean(field: &str) -> String {
    field.trim().chars().filter(|c| !c.is_control()).map(|c| if c == '|' { '/' } else { c }).collect()
}

/// Each entry described, for the window (`describe_programs`) and the CLI; one that cannot be read is
/// shown as it is.
pub fn describe(entries: &[String]) -> Vec<Described> {
    entries
        .iter()
        .map(|entry| match Program::parse(entry) {
            Ok(program) => program.describe(),
            Err(why) => Described { entry: entry.clone(), label: entry.clone(), basis: why },
        })
        .collect()
}

/// The programs the CLI is given (`--programs`), as entries: an entry as it is (as `target show --json`
/// has it), else the path of a program's file, read by `read` (`None`: no such file), else, when
/// removing from `have`, the entries of that file name. The error names what is none of them.
pub fn entries_given(given: &[String], have: &[String], read: &dyn Fn(&std::path::Path) -> Option<Program>) -> Result<Vec<String>, String> {
    let mut entries = Vec::new();
    for value in given.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        if value.contains('|') {
            entries.push(Program::parse(value)?.entry());
        } else if let Some(program) = read(std::path::Path::new(value)) {
            entries.push(program.entry());
        } else {
            let named: Vec<String> =
                have.iter().filter(|entry| Program::parse(entry).is_ok_and(|p| p.name.eq_ignore_ascii_case(value))).cloned().collect();
            if named.is_empty() {
                return Err(t("targets.programs.notAnEntry", &[("entry", value)]));
            }
            entries.extend(named);
        }
    }
    Ok(entries)
}

#[derive(Default)]
pub struct ProgramList {
    /// The entries, kept so a session can be written to disk and resumed (`state.rs`).
    entries: Vec<String>,
    programs: Vec<Program>,
}

impl ProgramList {
    /// Reads the entries, ignoring empty lines. The error says in words what is wrong.
    pub fn parse<S: AsRef<str>>(lines: &[S]) -> Result<ProgramList, String> {
        let mut programs = Vec::new();
        for line in lines.iter().map(|l| l.as_ref().trim()).filter(|l| !l.is_empty()) {
            programs.push(Program::parse(line)?);
        }
        let protected: Vec<String> =
            programs.iter().filter(|p| p.is_protected() || is_protected(&p.name)).map(|p| p.describe().label).collect();
        if !protected.is_empty() {
            return Err(t("targets.programs.protected", &[("programs", &protected.join(", "))]));
        }
        Ok(ProgramList { entries: programs.iter().map(Program::entry).collect(), programs })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries as written down, which are normalized and so match exactly what they matched.
    pub fn entries(&self) -> Vec<String> {
        self.entries.clone()
    }

    /// Whether a running program is on the list. Never one of `PROTECTED`, whatever the list says.
    pub fn is_blocked(&self, running: &Program) -> bool {
        !running.is_protected() && self.programs.iter().any(|p| p.matches(running))
    }
}

/// The title of Steam's built-in browser window, whatever page it shows.
pub const STEAM_BROWSER_TITLE: &str = "Steam - Browser";

/// Whether a window is Steam's built-in browser.
pub fn is_steam_browser(title: &str, class: &str, program: &str) -> bool {
    title == STEAM_BROWSER_TITLE && class == "SDL_app" && program.eq_ignore_ascii_case("steamwebhelper.exe")
}

/// Whether a program (by file name, any case) is one of those that are never closed.
pub fn is_protected(file_name: &str) -> bool {
    PROTECTED.contains(&file_name.to_lowercase().as_str())
}

#[cfg(windows)]
pub use read::{of_file, of_process, running};

/// Elsewhere (the sync server, the tests on Linux) a file says nothing but its name.
#[cfg(not(windows))]
pub fn of_file(path: &std::path::Path) -> Program {
    Program::named(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Elsewhere no Windows programs run.
#[cfg(not(windows))]
pub fn running() -> Vec<Running> {
    Vec::new()
}

/// Reading a program's file: its version info (`GetFileVersionInfoW`), the file of a process.
#[cfg(windows)]
mod read {
    use std::collections::BTreeMap;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};

    use super::{Program, Running};

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    /// The program in a file: its name, and its version info when it has some.
    pub fn of_file(path: &Path) -> Program {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(info) = version_info(path) else { return Program::named(&name) };
        let languages = translations(&info);
        // The first language that has any of the fields; English as written by most tools, else.
        let fallback = [(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0000, 0x04B0)];
        for (language, code_page) in languages.into_iter().chain(fallback) {
            let field = |key: &str| text(&info, &format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\{key}")).unwrap_or_default();
            let (original, description, company) = (field("OriginalFilename"), field("FileDescription"), field("CompanyName"));
            if !(original.trim().is_empty() && description.trim().is_empty() && company.trim().is_empty()) {
                return Program::read(&name, &original, &description, &company);
            }
        }
        Program::named(&name)
    }

    /// The file of a running process, by its id; `None` if it cannot be opened (it has gone, or it is
    /// one of Windows' own that nobody may open).
    pub fn path_of(pid: u32) -> Option<std::path::PathBuf> {
        // SAFETY: the handle is closed below; the buffer outlives the call, its length given in characters.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buffer = vec![0u16; 32768];
            let mut length = buffer.len() as u32;
            let read = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
            CloseHandle(process);
            (read != 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
        }
    }

    /// The program a process runs, by its id and name (all there is when its file cannot be read).
    pub fn of_process(pid: u32, name: &str) -> Program {
        path_of(pid).map(|path| of_file(&path)).unwrap_or_else(|| Program::named(name))
    }

    /// The programs running now that could be picked (not the protected ones), one per program, by
    /// their label.
    pub fn running() -> Vec<Running> {
        let mut system = sysinfo::System::new();
        let kind = sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always);
        system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, kind);
        let mut found: BTreeMap<String, Running> = BTreeMap::new();
        for process in system.processes().values() {
            let Some(path) = process.exe() else { continue };
            let program = of_file(path);
            // Only what a list can keep: Windows gives some processes a path without a file name.
            if program.is_protected() || super::is_protected(&program.name) || Program::parse(&program.entry()).is_err() {
                continue;
            }
            let described = program.describe();
            found.entry(program.identity()).or_insert(Running { described, path: path.to_string_lossy().into_owned() });
        }
        let mut programs: Vec<Running> = found.into_values().collect();
        programs.sort_by_cached_key(|p| p.described.label.to_lowercase());
        programs
    }

    /// The file's version info, as one block; `None` if it has none.
    fn version_info(path: &Path) -> Option<Vec<u8>> {
        let path = wide(path.as_os_str());
        // SAFETY: the path is null-terminated; the buffer has the size Windows asked for.
        unsafe {
            let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
            if size == 0 {
                return None;
            }
            let mut info = vec![0u8; size as usize];
            (GetFileVersionInfoW(path.as_ptr(), 0, size, info.as_mut_ptr().cast()) != 0).then_some(info)
        }
    }

    /// A value in the block: where it is and its length (in bytes, or characters for text).
    fn value(info: &[u8], key: &str) -> Option<(*const u8, usize)> {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        let mut found: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut length = 0u32;
        // SAFETY: the block is the one Windows filled; what is found points into it, which outlives the use.
        let ok = unsafe { VerQueryValueW(info.as_ptr().cast(), key.as_ptr(), &mut found, &mut length) };
        (ok != 0 && !found.is_null() && length > 0).then_some((found as *const u8, length as usize))
    }

    /// The languages and code pages the block has texts in.
    fn translations(info: &[u8]) -> Vec<(u16, u16)> {
        let Some((at, bytes)) = value(info, "\\VarFileInfo\\Translation") else { return Vec::new() };
        // SAFETY: `bytes` bytes at `at` lie inside the block; read unaligned, as pairs of u16.
        (0..bytes / 4)
            .map(|n| unsafe { (at.add(n * 4).cast::<u16>().read_unaligned(), at.add(n * 4 + 2).cast::<u16>().read_unaligned()) })
            .collect()
    }

    /// A text of the block, without its ending null.
    fn text(info: &[u8], key: &str) -> Option<String> {
        let (at, chars) = value(info, key)?;
        // SAFETY: `chars` UTF-16 characters at `at` lie inside the block; read unaligned.
        let units: Vec<u16> = (0..chars).map(|n| unsafe { at.add(n * 2).cast::<u16>().read_unaligned() }).collect();
        Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(entry: &str) -> Program {
        Program::parse(entry).unwrap()
    }

    const STEAM: &str = "steam.exe|steam.exe|Steam|Valve Corporation";

    #[test]
    fn an_entry_is_its_fields_joined_and_read_back() {
        let steam = program(STEAM);
        assert_eq!((steam.name.as_str(), steam.original.as_str(), steam.description.as_str()), ("steam.exe", "steam.exe", "Steam"));
        assert_eq!(steam.entry(), STEAM);
        // Trimmed, and what would split an entry or cannot be shown taken out.
        let read = Program::read(" a.exe ", "a|b.exe", "Tab\there", " ");
        assert_eq!(read.entry(), "a.exe|a/b.exe|Tabhere|");
        assert_eq!(program(&read.entry()), read);
        // Four fields and a file name, or it is not a program.
        for bad in ["steam.exe", "*", "a|b|c", "a|b|c|d|e", "|steam.exe|Steam|Valve", r"C:\Games\x.exe|||", "a?.exe|||"] {
            assert_eq!(
                Program::parse(bad).unwrap_err(),
                format!("Not a program: {}. Give the path of a program's file", bad.trim()),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_program_is_recognized_by_its_original_name_else_its_description_and_its_company() {
        let steam = program(STEAM);
        assert_eq!(steam.basis(), Basis::Original);
        // Renamed or copied: what is inside the file stays.
        assert!(steam.matches(&Program::read("homework.exe", "steam.exe", "Steam", "Valve Corporation")));
        assert!(steam.matches(&Program::read("STEAM.EXE", "", "", "")), "the file name always, letter case aside");
        assert!(steam.matches(&Program::read("x.exe", "STEAM.EXE", "Other", " valve corporation ")));
        // Another company's file of that name (Chromium's helpers are Chrome's and Brave's) is another program.
        assert!(!steam.matches(&Program::read("x.exe", "steam.exe", "Steam", "Someone else")));
        assert!(!steam.matches(&Program::read("x.exe", "", "Steam", "Valve Corporation")));

        // No original name: the description (Claude Code's case).
        let claude = program("claude.exe||Claude Code|Anthropic PBC");
        assert_eq!(claude.basis(), Basis::Description);
        assert!(claude.matches(&Program::read("renamed.exe", "", "Claude Code", "Anthropic PBC")));
        assert!(!claude.matches(&Program::read("renamed.exe", "", "Claude", "Anthropic PBC")));
        assert!(!claude.matches(&Program::read("renamed.exe", "", "Claude Code", "")));

        // Nothing in the file: the name alone.
        let bare = program("tool.exe|||");
        assert_eq!(bare.basis(), Basis::Name);
        assert!(bare.matches(&Program::named("Tool.exe")));
        assert!(!bare.matches(&Program::read("other.exe", "", "", "")));
    }

    #[test]
    fn what_windows_and_this_app_need_is_known_by_its_file_so_a_renamed_program_is_not_spared() {
        let svchost = Program::read("svchost.exe", "svchost.exe.mui", "Host Process for Windows Services", "Microsoft Corporation");
        assert!(svchost.is_protected());
        // Renamed to anything, still Windows'.
        assert!(Program::read("x.exe", "EXPLORER.EXE.MUI", "Windows Explorer", "Microsoft Corporation").is_protected());
        assert!(Program::read("bullseye-app.exe", "", "Bullseye", "Bullseye").is_protected());
        // A game renamed after one of them is the game.
        assert!(!Program::read("dwm.exe", "game.exe", "Game", "Studio").is_protected());
        assert!(!Program::read("svchost.exe", "", "Game", "Studio").is_protected());
        // Another company's file of a Windows name, or Microsoft's own other programs, can go.
        assert!(!Program::read("x.exe", "svchost.exe", "", "Someone else").is_protected());
        assert!(!Program::read("notepad.exe", "NOTEPAD.EXE.MUI", "Notepad", "Microsoft Corporation").is_protected());
        // A process whose file says nothing (or cannot be read) by its name.
        assert!(Program::named("System").is_protected() && Program::named("csrss.exe").is_protected());
        assert!(!Program::named("game.exe").is_protected());

        // A list never closes one, even if it was somehow given one; an entry for one is refused.
        let list = ProgramList { entries: vec![], programs: vec![Program::named("explorer.exe")] };
        assert!(!list.is_blocked(&Program::named("explorer.exe")));
        let why = ProgramList::parse(&[STEAM, "explorer.exe|EXPLORER.EXE.MUI|Windows Explorer|Microsoft Corporation", "dwm.exe|||"])
            .err()
            .unwrap();
        assert_eq!(why, "Would close programs Windows or this app needs: Windows Explorer (explorer.exe), dwm.exe");
    }

    #[test]
    fn a_list_reads_its_entries_and_blocks_what_they_match() {
        let list = ProgramList::parse(&["", STEAM, "  ", "tool.exe|||"]).unwrap();
        assert_eq!(list.entries(), vec![STEAM, "tool.exe|||"]);
        assert!(!list.is_empty() && ProgramList::default().is_empty());
        assert!(list.is_blocked(&Program::read("homework.exe", "steam.exe", "Steam", "Valve Corporation")));
        assert!(list.is_blocked(&Program::named("tool.exe")));
        assert!(!list.is_blocked(&Program::named("chrome.exe")));
        assert!(ProgramList::parse(&["steam*"]).is_err());
    }

    #[test]
    fn an_entry_says_what_it_is_and_what_it_is_recognized_by() {
        let said = |entry: &str| program(entry).describe();
        assert_eq!(
            said(STEAM),
            Described {
                entry: STEAM.into(),
                label: "steam.exe".into(),
                basis: "Recognized by its original name steam.exe from Valve Corporation".into()
            }
        );
        assert_eq!(said("a.exe|a.exe||").basis, "Recognized by its original name a.exe");
        assert_eq!(said("claude.exe||Claude Code|Anthropic PBC").basis, "Recognized by its description Claude Code from Anthropic PBC");
        assert_eq!(said("b.exe||Tool|").basis, "Recognized by its description Tool");
        let bare = said("tool.exe|||");
        assert_eq!((bare.label.as_str(), bare.basis.as_str()), ("tool.exe", "Recognized by its file name only"));
        // One that cannot be read is shown as it is, with why.
        let old = &describe(&["steam*".to_string()])[0];
        assert_eq!((old.label.as_str(), old.basis.as_str()), ("steam*", "Not a program: steam*. Give the path of a program's file"));
    }

    #[test]
    fn copies_of_one_program_are_one() {
        let a = Program::read("a.exe", "steam.exe", "Steam", "Valve Corporation");
        let b = Program::read("b.exe", "STEAM.EXE", "Other words", "valve corporation");
        assert_eq!(a.identity(), b.identity());
        assert_ne!(program("claude.exe||Claude Code|Anthropic PBC").identity(), program("claude.exe||Claude|Anthropic PBC").identity());
        assert_ne!(Program::named("a.exe").identity(), Program::named("b.exe").identity());
    }

    #[test]
    fn the_cli_gives_programs_by_file_as_kept_or_by_name_to_remove() {
        let have = vec![STEAM.to_string(), "tool.exe|||".to_string()];
        let files = |path: &std::path::Path| (path.to_str() == Some(r"C:\Games\game.exe")).then(|| Program::named("game.exe"));
        let given =
            |values: &[&str], have: &[String]| entries_given(&values.iter().map(|v| v.to_string()).collect::<Vec<_>>(), have, &files);
        assert_eq!(given(&[r"C:\Games\game.exe", " ", STEAM], &[]).unwrap(), vec!["game.exe|||", STEAM]);
        // To remove: also by file name, which the list shows.
        assert_eq!(given(&["STEAM.EXE"], &have).unwrap(), vec![STEAM]);
        assert_eq!(given(&["steam.exe", "tool.exe"], &have).unwrap(), vec![STEAM, "tool.exe|||"]);
        assert_eq!(given(&["Steam"], &have).unwrap_err(), "Not a program: Steam. Give the path of a program's file");
        assert_eq!(given(&["nope.exe"], &have).unwrap_err(), "Not a program: nope.exe. Give the path of a program's file");
        assert_eq!(given(&["a|b"], &[]).unwrap_err(), "Not a program: a|b. Give the path of a program's file");
    }

    #[cfg(windows)]
    #[test]
    fn a_programs_file_says_what_it_is() {
        let ping = std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join("System32").join("ping.exe");
        let read = of_file(&ping);
        assert_eq!(read.name.to_lowercase(), "ping.exe");
        assert!(read.original.to_lowercase().starts_with("ping.exe") && read.company == "Microsoft Corporation", "{read:?}");
        // A file without version info, or none at all: its name.
        let dir = std::env::temp_dir().join(format!("programs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("plain.exe");
        std::fs::write(&plain, b"not a program").unwrap();
        assert_eq!(of_file(&plain), Program::named("plain.exe"));
        assert_eq!(of_file(&dir.join("missing.exe")), Program::named("missing.exe"));
        let _ = std::fs::remove_dir_all(&dir);
        // This process's own file, by its id; one that is not there by its name.
        let me = of_process(std::process::id(), "me.exe");
        assert!(me.name.to_lowercase().ends_with(".exe") && me.name != "me.exe", "{me:?}");
        assert_eq!(of_process(u32::MAX - 3, "gone.exe"), Program::named("gone.exe"));
        // What runs can be picked, Windows' own not: this test is running.
        let running = running();
        assert!(running.iter().any(|r| r.path.to_lowercase().ends_with(&me.name.to_lowercase())), "this test is running");
        assert!(running.iter().all(|r| !program(&r.described.entry).is_protected()));
    }

    #[test]
    fn steams_browser_is_told_by_its_title_its_class_and_its_program() {
        assert!(is_steam_browser(STEAM_BROWSER_TITLE, "SDL_app", "steamwebhelper.exe"));
        assert!(is_steam_browser("Steam - Browser", "SDL_app", "SteamWebHelper.EXE"), "letter case aside in the program");
        // The main window (just "Steam"), another program with that title, or another kind of window: not it.
        assert!(!is_steam_browser("Steam", "SDL_app", "steamwebhelper.exe"));
        assert!(!is_steam_browser("Steam - Browser", "SDL_app", "chrome.exe"));
        assert!(!is_steam_browser("Steam - Browser", "Chrome_WidgetWin_1", "steamwebhelper.exe"));
        assert!(!is_steam_browser("steam - browser", "SDL_app", "steamwebhelper.exe"), "the title is exact");
        assert!(!is_steam_browser("", "", ""));
    }
}
