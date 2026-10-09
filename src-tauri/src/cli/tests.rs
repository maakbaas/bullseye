use super::*;
use crate::definitions::tests::{session, target, weekly};
use crate::definitions::Definitions;
use crate::handler::{handle, Context};
use crate::session::{Running, RunningTarget, Sessions};
use crate::state::Store;
use crate::status;
use crate::watchdog::AppSlot;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    /// The service the commands of this test talk to (`serve`).
    pub static REMOTE: RefCell<Option<Remote>> = const { RefCell::new(None) };
    /// What the command being run says (`cli`).
    pub static SAID: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn args(line: &[&str]) -> Vec<String> {
    line.iter().map(|arg| arg.to_string()).collect()
}

/// A service of its own, in this process and with files of its own, which the commands run on
/// this thread talk to from now on.
fn serve(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sessions = Sessions::with_store(Store::new(dir.join("state.json")));
    let context = Arc::new(Context::new(sessions, Definitions::at(Some(dir.join("sessions.json"))), Arc::new(AppSlot::new(false))));
    REMOTE.with(|remote| *remote.borrow_mut() = Some(Remote::local(Arc::new(move |request| handle(&context, 0, request).0))));
    dir
}

/// A command line run as `bullseye` runs it: what it says, or why it did not work.
fn cli(line: &[&str]) -> Result<String, String> {
    SAID.with(|said| *said.borrow_mut() = Some(String::new()));
    let result = parse(&args(line)).map_err(Failure::Usage).and_then(execute);
    let said = SAID.with(|said| said.borrow_mut().take()).unwrap_or_default();
    match result {
        Ok(()) => Ok(said),
        Err(Failure::Usage(why) | Failure::Failed(why)) => Err(why),
    }
}

/// What a command says, which has to work.
fn ok(line: &[&str]) -> String {
    cli(line).unwrap_or_else(|why| panic!("{line:?}: {why}"))
}

/// Why a command does not work, which it has to.
fn refused(line: &[&str]) -> String {
    cli(line).err().unwrap_or_else(|| panic!("{line:?} worked"))
}

#[test]
fn sessions_targets_and_conditions_are_made_shown_changed_and_deleted() {
    serve("lifecycle");
    assert!(ok(&["status"]).contains("Sessions:\n  None yet"));
    assert_eq!(
        ok(&["target", "new", "Social", "--sites", "youtube.com", "--allowed-sites", "music.youtube.com"]),
        "Made the target “Social”. Use it with: bullseye new <session> --targets \"Social\"\n"
    );
    assert!(ok(&["condition", "new", "Work", "--times", "Mon-Fri 08:00-17:00"]).starts_with("Made the condition “Work”."));
    let made = ok(&["new", "Evening", "--targets", "Social", "--condition", "Work", "--allowance", "budget", "--allowed", "30m"]);
    assert!(
        made.starts_with("Made the session “Evening” (id cli-") && made.ends_with("Start it with: bullseye start \"Evening\"\n"),
        "{made}"
    );

    // Shown as the pages show them, and as JSON for scripts.
    let shown = ok(&["show", "evening"]);
    assert!(
        shown.contains(
            "Target:      Social\nCondition:   Work · By schedule\nAllowance:   Time budget · Allowed for 30 minutes\nLock:        No lock · Not locked\n"
        ),
        "{shown}"
    );
    let json: Value = serde_json::from_str(&ok(&["show", "Evening", "--json"])).unwrap();
    assert_eq!(json["session"]["allowance"]["type"], "budget");
    assert!(json["running"].is_null());
    assert_eq!(json["status"]["state"], "not_running");
    let status = ok(&["status"]);
    assert!(
        status.contains("  Evening  Not running · By schedule")
            && status.contains("  Social  Used by Evening")
            && status.contains("  Work  Active Mon-Fri 08:00-17:00 · Used by Evening")
    );
    let json: Value = serde_json::from_str(&ok(&["status", "--json"])).unwrap();
    assert_eq!(json["sessions"].as_array().map(Vec::len), Some(1));
    assert!(ok(&["target", "show", "Social"]).contains("Always allowed: music.youtube.com"));
    // Only the editor's sections with something in them.
    assert!(!ok(&["target", "show", "Social"]).contains("Windows by title"));
    let json: Value = serde_json::from_str(&ok(&["target", "show", "Social", "--json"])).unwrap();
    assert_eq!(json["filters"][0], "youtube.com");
    let condition = ok(&["condition", "show", "Work"]);
    assert!(condition.starts_with("Work\nActive Mon-Fri 08:00-17:00\n  Mon  08:00-17:00\n"), "{condition}");
    assert!(condition.ends_with("Used by: Evening\n"), "{condition}");
    assert!(ok(&["condition", "check", "Mon 8-9"]).starts_with("Mon 08:00-09:00\n"));
    assert_eq!(ok(&["condition", "check", "always"]), "Always active: a session without a condition\n");
    assert!(cli(&["condition", "check", "Mon 25-26"]).is_err());

    // Changed, copied.
    assert_eq!(ok(&["edit", "Evening", "--name", "Night"]), "Saved the session “Night”.\n");
    assert_eq!(ok(&["duplicate", "Night"]), "Made “Night (2)”, a copy of the session “Night”.\n");
    assert_eq!(ok(&["target", "duplicate", "Social"]), "Made “Social (2)”, a copy of the target “Social”.\n");
    assert_eq!(ok(&["condition", "duplicate", "Work"]), "Made “Work (2)”, a copy of the condition “Work”.\n");
    assert_eq!(
        ok(&["target", "edit", "Social (2)", "--sites", "reddit.com", "--clear", "allowed-sites"]),
        "Saved the target “Social (2)”.\n"
    );
    let copy = ok(&["target", "show", "Social (2)"]);
    assert!(copy.contains("reddit.com") && !copy.contains("music.youtube.com"), "{copy}");
    assert_eq!(ok(&["condition", "edit", "Work (2)", "--name", "Weekend", "--times", "Sat-Sun"]), "Saved the condition “Weekend”.\n");
    ok(&["condition", "new", "Either", "--any", "Work", "Weekend"]);
    assert!(ok(&["condition", "show", "Either"]).contains("Active when any of Work and Weekend is true"));
    assert!(refused(&["condition", "new", "Pair", "--any", "Work"]).contains("at least two"));

    // Started and stopped.
    assert_eq!(ok(&["start", "Night"]), "Started “Night”.\n");
    // Running (its tag says Inactive outside its times).
    let status = ok(&["status"]);
    assert!(status.lines().any(|line| line.starts_with("  Night ") && !line.contains("Not running")), "{status}");
    assert_eq!(ok(&["stop", "Night"]), "Stopped “Night”.\n");

    // Deleted.
    assert_eq!(ok(&["delete", "Night (2)", "--force"]), "Deleted the session “Night (2)”.\n");
    assert_eq!(ok(&["target", "delete", "Social (2)", "--force"]), "Deleted the target “Social (2)”.\n");
    assert_eq!(ok(&["condition", "delete", "Either", "--force"]), "Deleted the condition “Either”.\n");

    // What is not there, or not of the kind.
    assert!(refused(&["show", "Nope"]).starts_with("No session called “Nope”"));
    assert!(refused(&["new", "X", "--targets", "Nope"]).starts_with("No target called “Nope”"));
    assert!(refused(&["target", "new", "Games", "--store-apps", "Microsoft.Xbox*"]).contains("exact package family name"));
    assert!(refused(&["new", "X", "--condition", "Nope"]).starts_with("No condition called “Nope”"));
    assert_eq!(refused(&["condition", "test", "Work"]), "“Work” is not a webhook condition");
}

#[test]
fn items_are_added_and_removed() {
    serve("add-remove");
    ok(&["target", "new", "Social", "--sites", "youtube.com", "reddit.com", "--programs", "Steam", "--actions", "obsidian_web_viewer"]);
    ok(&["target", "new", "Games", "--programs", "game.exe"]);
    ok(&["target", "new", "News", "--sites", "nu.nl"]);
    let list = |name: &str| ok(&["target", "show", name]);

    // A target: entries added as typed (the service keeps them in order), and taken out however they are
    // written, as the list reads them (Steam is steam.exe); what is there already stays once.
    assert_eq!(
        ok(&[
            "target",
            "add",
            "Social",
            "--sites",
            "x.com",
            "YouTube.com",
            "--titles",
            "Inbox, 3 unread",
            "--actions",
            "window_titles_minimize"
        ]),
        "Saved the target “Social”.\n"
    );
    let shown = list("Social");
    assert!(shown.contains("  Sites to block: reddit.com, x.com, youtube.com\n"), "{shown}");
    assert!(
        shown.contains("Windows by title\n  Window titles: Inbox, 3 unread\n")
            && shown.contains("  Minimize these windows\n")
            && shown.contains("  Close Obsidian when web viewer is active\n"),
        "{shown}"
    );
    ok(&["target", "remove", "Social", "--sites", "Reddit.com", "--programs", "steam.exe", "--actions", "obsidian_web_viewer"]);
    let shown = list("Social");
    assert!(shown.contains("  Sites to block: x.com, youtube.com\n") && !shown.contains("Programs to close"), "{shown}");
    assert!(!shown.contains("Obsidian") && shown.contains("Minimize these windows"), "{shown}");
    // What is not there is a typo, and nothing is saved.
    assert_eq!(refused(&["target", "remove", "Social", "--sites", "x.com", "tiktok.com"]), "“tiktok.com” is not in the sites of “Social”");
    assert!(list("Social").contains("x.com, youtube.com"));
    assert_eq!(refused(&["target", "remove", "Social", "--allowed-titles", "Notes"]), "“Notes” is not in the allowed titles of “Social”");
    assert!(refused(&["target", "add", "Social", "--actions", "lasers"]).starts_with("Unknown action “lasers”"));
    assert_eq!(refused(&["target", "add", "Social"]), "Give what to add: --sites, --programs, --titles, … or --actions");
    // All the actions, on and off.
    ok(&["target", "add", "Social", "--actions", "all"]);
    let shown = list("Social");
    assert!(
        shown.contains("Other\n  Close Obsidian when web viewer is active\n") && shown.contains("  Minimize these windows\n"),
        "{shown}"
    );
    ok(&["target", "remove", "Social", "--actions", "all"]);
    let shown = list("Social");
    assert!(!shown.contains("Close ") && !shown.contains("Other"), "{shown}");

    // A session's targets.
    ok(&["new", "Work", "--targets", "Social"]);
    ok(&["add", "work", "--targets", "Games", "News", "Social"]);
    assert!(ok(&["show", "Work"]).contains("Targets:     Social, Games and News"), "{}", ok(&["show", "Work"]));
    ok(&["remove", "Work", "--targets", "social"]);
    assert!(ok(&["show", "Work"]).contains("Targets:     Games and News"));
    assert_eq!(refused(&["remove", "Work", "--targets", "Social"]), "“Social” is not a target of “Work”");
    assert!(refused(&["add", "Work", "--targets", "Nope"]).starts_with("No target called “Nope”"));
    assert_eq!(refused(&["add", "Work"]), "Give the targets to add: --targets <targets>");

    // The conditions of a combination, and nothing else.
    for name in ["Mornings", "Evenings", "Weekend"] {
        ok(&["condition", "new", name, "--times", "sat"]);
    }
    ok(&["condition", "new", "Either", "--any", "Mornings", "Evenings"]);
    ok(&["condition", "add", "Either", "--conditions", "Weekend"]);
    assert!(
        ok(&["condition", "show", "Either"]).contains("Active when any of Mornings, Evenings and Weekend is true"),
        "{}",
        ok(&["condition", "show", "Either"])
    );
    ok(&["condition", "remove", "Either", "--conditions", "Mornings"]);
    assert!(ok(&["condition", "show", "Either"]).contains("Active when any of Evenings and Weekend is true"));
    assert_eq!(refused(&["condition", "remove", "Either", "--conditions", "Mornings"]), "“Mornings” is not in “Either”");
    assert_eq!(
        refused(&["condition", "add", "Weekend", "--conditions", "Mornings"]),
        "“Weekend” is not a combination; add and remove change the conditions of one"
    );
    assert_eq!(refused(&["condition", "remove", "Either"]), "Give the conditions to remove: --conditions <conditions>");
    // The service still holds to its rules: a combination of fewer than two is refused.
    assert!(!refused(&["condition", "remove", "Either", "--conditions", "Weekend"]).is_empty());
    assert_eq!(refused(&["target"]), "`target` needs show, new, edit, add, remove, delete, duplicate");
}

#[test]
fn a_webhook_condition_is_shown_and_tested() {
    serve("webhook");
    let url = "http://127.0.0.1:9/state";
    ok(&["condition", "new", "Home", "--webhook", url, "--match-body", "home", "--fallback", "inactive"]);
    let shown = ok(&["condition", "show", "Home"]);
    assert!(shown.contains("GET http://127.0.0.1:9/state every minute: not active, active on body “home”"), "{shown}");
    ok(&["condition", "new", "Door", "--webhook", url, "--match-status", "423", "--every", "5m"]);
    let shown = ok(&["condition", "show", "Door"]);
    assert!(shown.contains("GET http://127.0.0.1:9/state every 5 minutes: active, not active on HTTP 423"), "{shown}");
    // Nothing answers there.
    assert!(cli(&["condition", "test", "Home"]).is_err());
}

#[test]
fn locks_are_set_shown_and_held_by_the_service() {
    serve("locks");
    ok(&["target", "new", "L", "--sites", "a.com"]);
    ok(&["condition", "new", "Always", "--times", "Mon-Sun 00:00-24:00"]);
    let new = |name: &str, lock: &[&str]| ok(&[&["new", name, "--targets", "L"][..], lock].concat());
    new("Timed", &["--lock-duration", "1h"]);
    new("Slow", &["--lock-delay", "1h"]);
    new("Typed", &["--lock-text", "let me out"]);
    new("Kept", &["--lock-condition", "always"]);
    new("Restart", &["--lock-until-restart"]);
    new("Later", &["--lock-until", "2099-01-01 00:00"]);
    let lock_of = |name: &str| ok(&["show", name]).lines().find(|l| l.starts_with("Lock:")).unwrap_or_default().to_string();
    // The method as the Lock row's dropdown names it, and its status: the tile's row.
    assert_eq!(lock_of("Timed"), "Lock:        Timed · For 60 minutes");
    assert_eq!(lock_of("Slow"), "Lock:        Delay · Delay 60 minutes");
    assert_eq!(lock_of("Typed"), "Lock:        Custom text · Type text to stop");
    assert_eq!(lock_of("Kept"), "Lock:        Condition · During “Always”");
    assert_eq!(lock_of("Restart"), "Lock:        Until restart · Until restart");
    assert_eq!(lock_of("Later"), "Lock:        Timed · Until January 1");

    for name in ["Timed", "Slow", "Typed", "Kept", "Restart"] {
        assert_eq!(ok(&["start", name, "--force"]), format!("Started “{name}”.\n"));
    }
    let status = ok(&["status"]);
    for line in [
        "Timed    Running · locked · Active · Never paused · ",
        "Typed    Running · locked · Active · Never paused · Type text to stop",
        "Restart  Running · locked · Active · Never paused · Until restart",
        "Kept     Running · locked · Active · Never paused · Locked",
    ] {
        assert!(status.contains(line), "{line:?} in {status}");
    }
    assert!(refused(&["stop", "Timed"]).contains("is locked for the next"));
    assert!(refused(&["stop", "Kept"]).contains("is locked by its lock schedule"));
    assert!(refused(&["stop", "Restart"]).contains("until Windows restarts"));
    assert_eq!(refused(&["stop", "Typed"]), "“Typed” is locked until its text is typed in: stop it in the window");
    // A delay: the first stop asks, the next says how long, and --abort ends the wait.
    assert!(ok(&["stop", "Slow"]).starts_with("Stopping “Slow” was asked for: ready in 1 hour."));
    assert!(ok(&["stop", "Slow"]).starts_with("“Slow” can be stopped in "));
    assert_eq!(ok(&["stop", "Slow", "--abort"]), "Stopping “Slow” aborted: back to locked.\n");
    // A password: set with start, as the tile sets it, and stopped with it.
    ok(&["new", "Pw", "--targets", "L"]);
    assert_eq!(ok(&["start", "Pw", "--lock-password", "secret", "--force"]), "Started “Pw”.\n");
    assert!(ok(&["status"]).contains("Pw       Running · locked · Active · Never paused · Password to stop"));
    assert!(cli(&["stop", "Pw", "--password", "wrong"]).is_err());
    assert_eq!(ok(&["stop", "Pw", "--password", "secret"]), "Stopped “Pw”.\n");
    // A running session that is not locked is locked now; one that is keeps its lock.
    assert_eq!(ok(&["start", "Pw", "--no-lock"]), "Started “Pw”.\n");
    assert_eq!(ok(&["lock", "Pw", "--no-lock"]), "“Pw” runs on without a lock.\n");
    assert!(refused(&["lock", "Pw", "--lock-duration", "1h"]).contains("No terminal to ask in; add --force"));
    assert_eq!(ok(&["lock", "Pw", "--lock-duration", "1h", "--force"]), "Locked “Pw” now.\n");
    assert!(ok(&["status"]).contains("Pw       Running · locked · Active · Never paused · 60 minutes left"));
    assert_eq!(
        refused(&["lock", "Pw", "--no-lock"]),
        "This session is locked, so its lock cannot be changed until it can be stopped, except to end later or to wait longer"
    );
    assert!(refused(&["lock", "Pw"]).starts_with("Give the lock"));
    // Without a delay, unlocking is free and ends them all.
    assert_eq!(ok(&["unlock"]), "Unlocked: every session has ended.\n");
    assert!(!ok(&["status"]).contains("Running"));
}

#[test]
fn unlocking_counts_down_and_has_another_way() {
    serve("unlock");
    ok(&["target", "new", "L", "--sites", "a.com"]);
    ok(&["new", "Timed", "--targets", "L", "--lock-duration", "1h"]);
    assert_eq!(ok(&["unlock-delay"]), "Unlocking takes no time: it is free.\n");
    assert_eq!(ok(&["unlock-delay", "1h"]), "Unlocking now takes 1 hour.\n");
    assert_eq!(ok(&["unlock-delay"]), "Unlocking takes 1 hour.\n");
    ok(&["start", "Timed", "--force"]);
    assert!(ok(&["unlock"]).starts_with("Unlocking was asked for: ready in 1 hour."));
    assert!(ok(&["unlock"]).starts_with("Unlocking is not ready yet: "));
    assert_eq!(ok(&["unlock", "--abort"]), "Unlocking aborted: back to locked.\n");
    assert_eq!(ok(&["unlock-way"]), "Only the delay unlocks.\n");
    // Locked and not ready: neither a shorter delay nor another way.
    assert!(cli(&["unlock-delay", "0"]).is_err());
    assert!(cli(&["unlock-way", "--text", "let me out"]).is_err());

    serve("unlock-way");
    ok(&["target", "new", "L", "--sites", "a.com"]);
    ok(&["condition", "new", "Always", "--times", "Mon-Sun 00:00-24:00"]);
    ok(&["new", "Timed", "--targets", "L", "--lock-duration", "1h"]);
    assert_eq!(ok(&["unlock-way", "--text", "let me out"]), "Unlocking is also possible by typing a text in, in the window.\n");
    assert_eq!(ok(&["unlock-way", "--condition", "Always"]), "Unlocking is locked while its condition is active\n");
    assert_eq!(ok(&["unlock-way", "--password", "secret"]), "Unlocking is also possible at once with a password (`unlock --password`).\n");
    ok(&["unlock-delay", "1h"]);
    ok(&["start", "Timed", "--force"]);
    assert!(cli(&["unlock", "--password", "wrong"]).is_err());
    assert_eq!(ok(&["unlock", "--password", "secret"]), "Unlocked: every session has ended.\n");
    assert_eq!(ok(&["unlock-way", "--none"]), "Only the delay unlocks.\n");
    assert_eq!(ok(&["unlock-delay", "0"]), "Unlocking is free again.\n");
}

#[test]
fn everything_moves_through_a_settings_file() {
    let dir = serve("export");
    let file = dir.join("bullseye.json");
    let path = file.to_string_lossy().to_string();
    ok(&["target", "new", "L", "--sites", "a.com"]);
    ok(&["new", "B", "--targets", "L", "--lock-password", "secret", "--allowance", "pomodoro", "--blocked", "50m", "--allowed", "10m"]);
    assert_eq!(ok(&["export", &path]), format!("Wrote every target, condition and session to {path}.\n"));
    serve("import");
    // (The file has no Store apps, so none are left out and nothing is said of them.)
    assert_eq!(ok(&["import", &path, "--force"]), format!("Imported {path}.\n"));
    let shown = ok(&["show", "B"]);
    assert!(
        shown.contains("Lock:        Password · ") && shown.contains("Allowance:   Pomodoro cycle · 50 minutes / 10 minutes\n"),
        "{shown}"
    );
    assert!(refused(&["import", &dir.join("missing.json").to_string_lossy()]).starts_with("Cannot read"));
}

#[test]
fn a_webhook_is_read_from_its_options() {
    let line = [
        "condition",
        "new",
        "H",
        "--webhook",
        "https://x.io/a?b=1",
        "--method",
        "POST",
        "--every",
        "10m",
        "--match-status",
        "503",
        "--fallback",
        "active",
    ];
    let Ok(Command::ConditionNew { rule: RuleChoice::Webhook(hook), .. }) = parse(&args(&line)) else { panic!("not a webhook") };
    assert_eq!((hook.method, hook.interval, hook.fallback), (Method::Post, 600, true));
    assert_eq!(hook.matcher, Matcher { on: Match::Status, status: Some(503), body: None });
    let line = ["condition", "new", "H", "--webhook", "https://x.io/a", "--match-body", "on", "--fallback", "inactive"];
    let Ok(Command::ConditionNew { rule: RuleChoice::Webhook(hook), .. }) = parse(&args(&line)) else { panic!("not a webhook") };
    assert_eq!((hook.method, hook.interval, hook.fallback), (Method::Get, 60, false));
    assert_eq!(hook.matcher, Matcher { on: Match::Body, status: None, body: Some("on".into()) });
    let base = ["condition", "new", "H", "--webhook", "https://x.io/a", "--match-status", "404"];
    for (extra, why) in [
        (&["--method", "put"][..], "--method is get or post"),
        (&["--fallback", "maybe"], "--fallback is active or inactive"),
        (&["--every", "soon"], ""),
        (&["--every", "30s"], "at most once a minute"),
    ] {
        let error = parse(&args(&[&base[..], extra].concat())).err().unwrap_or_else(|| panic!("{extra:?} worked"));
        assert!(error.contains(why), "{extra:?}: {error}");
    }
    // One match: neither, or both, is refused, saying what the match is for.
    let hook = ["condition", "new", "H", "--webhook", "https://x.io/a"];
    assert_eq!(
        parse(&args(&hook)).unwrap_err(),
        "Give one of --match-status and --match-body: the answer that makes the condition not active"
    );
    assert!(parse(&args(&[&hook[..], &["--fallback", "inactive", "--match-status", "200", "--match-body", "on"]].concat()))
        .unwrap_err()
        .ends_with("the answer that makes the condition active"));
    assert!(parse(&args(&[&hook[..], &["--match-status", "OK"]].concat())).unwrap_err().contains("is a number"));
    // An option given twice is a mistake, not a choice.
    assert!(parse(&args(&["new", "X", "--condition", "A", "--condition", "B"])).unwrap_err().contains("more than once"));
}

#[test]
fn a_combination_of_all_and_a_list_renamed() {
    serve("all");
    ok(&["target", "new", "L", "--sites", "a.com"]);
    assert_eq!(ok(&["target", "edit", "L", "--name", "Media"]), "Saved the target “Media”.\n");
    ok(&["condition", "new", "Work", "--times", "Mon-Fri 08:00-17:00"]);
    ok(&["condition", "new", "Late", "--times", "Mon-Sun 16:00-18:00"]);
    ok(&["condition", "new", "Both", "--all", "Work", "Late"]);
    assert!(ok(&["condition", "show", "Both"]).contains("Active when all of Work and Late are true"));
    // A lock until a date and time asks first, with the time it ends.
    ok(&["new", "Later", "--targets", "Media", "--lock-until", "2099-01-01 00:00"]);
    assert_eq!(ok(&["start", "Later", "--force"]), "Started “Later”.\n");
    assert!(ok(&["status"]).contains("Later  Running · locked · Active · Never paused · "));
}

#[test]
fn a_file_with_a_password_lock_but_no_hash_imports_without_a_lock() {
    let dir = serve("import-unlocked");
    let file = dir.join("old.json");
    let mut locked = session("b", &["l"]);
    locked.lock = Lock::Password { password: None, hash: None };
    let library = Library { targets: vec![target("l", &["a.com"])], sessions: vec![locked], ..Library::default() };
    std::fs::create_dir_all(&dir).unwrap();
    definitions::write_settings(&file, library).unwrap();
    ok(&["import", &file.to_string_lossy(), "--force"]);
    assert!(ok(&["show", "Session b"]).contains("Lock:        No lock · Not locked"));
}

#[test]
fn the_way_to_unlock_in_words() {
    let with = |way: Lock, open: bool, left: Option<&str>| {
        describe_unlock_way(&UnlockStatus {
            way: Some(way),
            way_open: open,
            way_left_words: left.map(str::to_string),
            ..UnlockStatus::default()
        })
    };
    let condition = || Lock::Condition { condition: "c".into() };
    assert_eq!(with(condition(), true, None), "Unlocking is open: its condition is not active.");
    assert_eq!(with(condition(), false, Some("2 hours")), "Unlocking is locked while its condition is active (in 2 hours)");
    assert_eq!(with(Lock::timed_for("1h"), false, None), "Only the delay unlocks.");
    assert!(with(Lock::RandomText { length: 8, text: Some("a1b2c3d4".into()) }, false, None).contains("random text"));
}

/// A running session as the status gives it, always active and not locked.
fn a_running(id: &str) -> Running {
    Running {
        id: id.into(),
        name: id.into(),
        targets: vec![],
        remaining: None,
        lock_condition: None,
        until_restart: false,
        lock_text: None,
        stop_delay: None,
        stop_delay_words: None,
        stop_remaining: None,
        stop_remaining_words: None,
        password: false,
        locked: false,
        condition: None,
        enforcing: true,
        condition_change: None,
        remaining_words: None,
        condition_change_words: None,
        lock_change: None,
        lock_change_words: None,
        allowance: None,
    }
}

#[test]
fn a_running_session_reads_like_its_tile() {
    // Its tag, then its rows' words, as the service wrote them (`status.rs` tests each wording).
    let mut overview = library();
    let session = overview.sessions[0].clone();
    assert_eq!(state(&with_status(overview.clone()), &session), "Not running · By schedule · Never paused · During “Work hours”");
    let schedule = weekly("s", "mon 8-9").rule;
    overview.running.push(Running {
        locked: true,
        condition: Some(schedule.clone()),
        condition_change_words: Some("4 hours".into()),
        lock_condition: Some(schedule),
        lock_change_words: Some("1 hour".into()),
        ..a_running("b1")
    });
    assert_eq!(state(&with_status(overview.clone()), &session), "Running · locked · Active for 4 hours · Never paused · Locked for 1 hour");
    overview.running[0].enforcing = false;
    assert_eq!(state(&with_status(overview.clone()), &session), "Inactive · Inactive for 4 hours · Never paused · Locked for 1 hour");
    // Without the service's words (an overview from no service), nothing to say.
    assert_eq!(state(&overview, &session), "");
}

#[test]
fn a_session_with_parts_gone_or_unreadable_is_still_shown() {
    let mut overview = library();
    let mut b = session("b2", &["gone"]);
    b.condition = Some("gone".into());
    let lock_of = |overview: &Overview, lock: Lock| {
        let shown = describe_session(overview, &Definition { lock, ..b.clone() });
        shown.lines().find(|l| l.starts_with("Lock:")).unwrap_or_default().to_string()
    };
    // Without the service's words: the method alone.
    assert_eq!(lock_of(&overview, Lock::timed_for("90m")), "Lock:        Timed");
    assert_eq!(lock_of(&overview, Lock::Delay { delay: "5m".into() }), "Lock:        Delay");
    assert_eq!(lock_of(&overview, Lock::Condition { condition: "gone".into() }), "Lock:        Condition");
    assert_eq!(lock_of(&overview, Lock::Password { password: None, hash: None }), "Lock:        Password");
    assert_eq!(lock_of(&overview, Lock::Text { text: "x".into() }), "Lock:        Custom text");
    assert_eq!(lock_of(&overview, Lock::RandomText { length: 5, text: None }), "Lock:        Random text");
    assert_eq!(lock_of(&overview, Lock::UntilRestart), "Lock:        Until restart");
    let shown = describe_session(&overview, &b);
    assert!(shown.contains("Target:      (deleted)\nCondition:   (deleted)\nAllowance:   No allowance\nLock:        No lock\n"), "{shown}");
    assert!(!shown.contains("State:"), "{shown}");
    // With them: its state, and each row's status beside its value.
    overview.sessions.push(b.clone());
    let shown = describe_session(&with_status(overview), &b);
    assert!(shown.contains("State:       Not running\n"), "{shown}");
    assert!(shown.contains("Condition:   (deleted) · At all times\n"), "{shown}");
    assert!(shown.contains("Lock:        No lock · Not locked\n"), "{shown}");
}

#[test]
fn an_import_says_which_store_apps_it_leaves_out() {
    let claude =
        definitions::LeftOut { target: "Claude".into(), family: "AnthropicPBC.Claude_fnn82j28hfe8t".into(), name: "Claude".into() };
    assert_eq!(left_out_words(std::slice::from_ref(&claude), true), "Left out, not installed on this PC: “Claude” in “Claude”");
    assert_eq!(left_out_words(&[], true), "");
    assert_eq!(left_out_words(&[claude], false), "Could not tell which Store apps are installed here, so all are kept");
}

#[test]
fn help_and_version_need_no_service() {
    let help = cli(&["help"]).unwrap();
    assert!(help.starts_with("Bullseye command line") && help.contains("obsidian_web_viewer") && !help.contains("{actions}"));
    assert_eq!(cli(&["version"]).unwrap(), format!("bullseye {}\n", env!("CARGO_PKG_VERSION")));
}

/// What the service adds to an overview: each session's rows (`status.rs`), with these as running.
fn with_status(mut overview: Overview) -> Overview {
    let zone = localtime::Zone::fixed(0);
    overview.status = overview
        .sessions
        .iter()
        .map(|b| status::of(b, overview.running.iter().find(|r| r.id == b.id), &overview.conditions, &zone, SystemTime::now()))
        .collect();
    overview
}

fn library() -> Overview {
    let mut social = target("l1", &["youtube.com"]);
    social.name = "Social".into();
    let mut work = weekly("s1", "mon-fri 8-17");
    work.name = "Work hours".into();
    let mut b = session("b1", &["l1"]);
    b.name = "Work".into();
    b.condition = Some("s1".into());
    b.lock = Lock::Condition { condition: "s1".into() };
    Overview {
        targets: vec![social],
        conditions: vec![work],
        sessions: vec![b],
        running: vec![],
        unlock: UnlockStatus::default(),
        clock_synced: true,
        status: vec![],
    }
}

/// Every command the window can invoke has CLI commands that do the same, in the help and in
/// `CLI.md`. When this fails, the window has learned something the CLI cannot do yet.
#[test]
fn the_cli_can_do_whatever_the_window_can() {
    let lib = include_str!("../lib.rs");
    let start = lib.find("generate_handler![").expect("no generate_handler in lib.rs") + "generate_handler![".len();
    let list = &lib[start..start + lib[start..].find(']').unwrap()];
    let commands: Vec<&str> = list.split(',').map(|c| c.trim().rsplit("::").next().unwrap()).filter(|c| !c.is_empty()).collect();
    assert!(commands.len() >= 14, "{commands:?}");
    let doc = include_str!("../../../CLI.md");
    for command in commands {
        let Some((_, cli)) = GUI_COMMANDS.iter().find(|(gui, _)| *gui == command) else {
            panic!("the window can invoke `{command}`, which is not in cli::GUI_COMMANDS: give the CLI a command for it");
        };
        for cli in cli.iter() {
            assert!(HELP.contains(&format!("\n  {cli} ")), "the help does not list `{cli}`");
            assert!(doc.contains(&format!("bullseye {cli}")), "CLI.md does not explain `{cli}`");
        }
    }
}

/// The page's action checkboxes are the CLI's action names.
#[test]
fn the_actions_are_the_same_as_in_the_window() {
    let names = action_names();
    assert!(!names.is_empty());
    let page = include_str!("../../../src/js/forms.js");
    let strings = include_str!("../../../src/strings.js");
    for name in names {
        assert!(page.contains(&format!("action(\"{name}\")")), "{name} is not on the page");
        assert!(strings.contains(&format!("{name}: {{")), "{name} has no text in strings.js");
    }
}

#[test]
fn windows_by_title_are_closed_unless_minimized() {
    let parse_actions = |names: &str| parse_actions(&names.split(' ').map(str::to_string).collect::<Vec<_>>());
    let all = parse_actions("all").unwrap();
    assert_eq!(all, Actions::default(), "all is every action as a new target has them: closing windows by title");
    assert!(!all.window_titles_minimize);
    let down = parse_actions("window_titles_minimize").unwrap();
    assert!(down.window_titles_minimize && !down.obsidian_web_viewer);
    assert!(parse_actions("all window_titles_minimize").unwrap().window_titles_minimize);
    // The lists with one way to act on them have no switch: their entries are it.
    for gone in ["exe_names", "store_apps", "window_titles", "window_titles_close", "chromium_tabs", "firefox_split_view"] {
        assert!(parse_actions(gone).unwrap_err().starts_with("Unknown action"), "{gone}");
    }
}

#[test]
fn a_new_target_with_everything() {
    let line = args(&[
        "target",
        "new",
        "Social",
        "--sites",
        "youtube.com",
        "reddit.com",
        "--allowed-sites",
        "youtube.com/watch*",
        "--programs",
        "steam",
        "--store-apps",
        "Microsoft.XboxApp_8wekyb3d8bbwe",
        "--titles",
        "*YouTube*",
        "--allowed-titles",
        "*Visual Studio Code",
        "--actions",
        "window_titles_minimize",
    ]);
    let Ok(Command::TargetNew { name, changes }) = parse(&line) else { panic!("not a new list") };
    assert_eq!(name, "Social");
    let mut list = target("x", &[]);
    changes.apply(&mut list);
    assert_eq!(list.entries.filters, vec!["youtube.com", "reddit.com"]);
    assert_eq!(list.entries.allowed, vec!["youtube.com/watch*"]);
    assert_eq!(list.entries.programs, vec!["steam"]);
    assert_eq!(list.entries.store_apps, vec!["Microsoft.XboxApp_8wekyb3d8bbwe"]);
    assert_eq!(list.entries.titles, vec!["*YouTube*"]);
    assert_eq!(list.entries.allowed_titles, vec!["*Visual Studio Code"]);
    assert_eq!(list.actions, Actions { obsidian_web_viewer: false, window_titles_minimize: true });
}

#[test]
fn a_new_session_finds_its_targets_and_condition_by_name() {
    let line = args(&["new", "Evening", "--targets", "social", "l1", "--condition", "WORK HOURS", "--lock-duration", "2h 30m"]);
    let Ok(Command::New { name, changes }) = parse(&line) else { panic!("not new") };
    assert_eq!(name, "Evening");
    let mut b = session("x", &[]);
    assert!(apply_session_changes(&library(), &mut b, changes).is_ok());
    assert_eq!(
        (b.targets, b.condition, b.lock),
        (vec!["l1".to_string(), "l1".to_string()], Some("s1".to_string()), Lock::timed_for("2h 30m"))
    );

    // `always`, and a list that does not exist.
    let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--condition", "always", "--no-lock"])) else {
        panic!("not edit")
    };
    let mut b = library().sessions[0].clone();
    assert!(apply_session_changes(&library(), &mut b, changes).is_ok());
    assert_eq!((b.condition, b.lock), (None, Lock::None));
    let Ok(Command::Edit { changes, .. }) = parse(&args(&["edit", "Work", "--targets", "Games"])) else { panic!("not edit") };
    assert!(apply_session_changes(&library(), &mut library().sessions[0].clone(), changes).is_err());
}

#[test]
fn start_can_set_the_lock_as_the_tile_does() {
    assert_eq!(
        parse(&args(&["start", "Work", "--lock-condition", "Work hours", "--force"])),
        Ok(Command::Start { session: "Work".into(), lock: Some(Lock::Condition { condition: "Work hours".into() }), force: true })
    );
    assert_eq!(parse(&args(&["start", "Work"])), Ok(Command::Start { session: "Work".into(), lock: None, force: false }));
    assert_eq!(
        parse(&args(&["lock", "Work", "--lock-duration", "2h", "--force"])),
        Ok(Command::Lock { session: "Work".into(), lock: Lock::timed_for("2h"), force: true })
    );
    assert!(parse(&args(&["lock", "Work"])).is_err());
    assert_eq!(
        parse(&args(&["start", "Work", "--lock-until", "2026-10-06 18:00"])),
        Ok(Command::Start { session: "Work".into(), lock: Some(Lock::timed_until("2026-10-06 18:00")), force: false })
    );
    assert_eq!(
        parse(&args(&["start", "Work", "--lock-until-restart"])),
        Ok(Command::Start { session: "Work".into(), lock: Some(Lock::UntilRestart), force: false })
    );
    assert!(parse(&args(&["start", "Work", "--lock-until", "tomorrow"])).is_err());
    assert!(parse(&args(&["start", "Work", "--lock-until-restart", "--no-lock"])).is_err());
    // What starting asks, per lock: the window's question, heading and body.
    let mut overview = library();
    let ask = |overview: &Overview, lock: Lock| lock_question(overview, &Definition { lock, ..overview.sessions[0].clone() });
    let (heading, body) = ask(&overview, overview.sessions[0].lock.clone()).unwrap();
    assert_eq!(heading, "Start “Work” locked during “Work hours”?");
    assert!(body.starts_with("Locked while “Work hours” is active. "), "{body}");
    let (heading, body) = ask(&overview, Lock::timed_for("2h5m")).unwrap();
    assert_eq!(
        (heading.as_str(), body.as_str()),
        ("Start “Work” for 2 hours and 5 minutes?", "Locked for 2 hours and 5 minutes. Only unlocking in Settings ends it early")
    );
    assert!(ask(&overview, Lock::timed_until("2099-01-01 00:00")).unwrap().1.starts_with("Locked until Thursday 1 January 2099, 00:00, "));
    assert_eq!(ask(&overview, Lock::timed_until("2001-01-01 00:00")), None);
    assert!(ask(&overview, Lock::UntilRestart).unwrap().0.contains("until Windows restarts"));
    assert!(ask(&overview, Lock::Delay { delay: "5m".into() }).unwrap().1.starts_with("Stopping takes 5 minutes"));
    assert!(ask(&overview, Lock::Text { text: "x".into() }).unwrap().0.contains("custom text"));
    let random = |length| ask(&overview, Lock::RandomText { length, text: None }).unwrap();
    assert!(random(12).0.contains("random text") && random(12).1.contains("12 random characters"));
    assert!(random(1).1.contains("1 random character,"));
    assert!(ask(&overview, Lock::Password { password: None, hash: None }).unwrap().0.contains("password"));
    assert_eq!(ask(&overview, Lock::None), None);
    assert_eq!(ask(&overview, Lock::Condition { condition: "gone".into() }), None);
    // A flash drive says which one is the key.
    overview.conditions.push(Condition {
        id: "d".into(),
        name: "Key".into(),
        rule: Rule::FlashDrive { serial: "x".into(), drive: "Kingston".into() },
    });
    let (_, body) = ask(&overview, Lock::Condition { condition: "d".into() }).unwrap();
    assert!(body.starts_with("Locked while “Key” is active (while “Kingston” is unplugged)."), "{body}");
}

#[test]
fn conditions_are_made_from_times_or_a_drive() {
    assert_eq!(
        parse(&args(&["condition", "new", "Work hours", "--times", "weekdays 9-17"])),
        Ok(Command::ConditionNew { name: "Work hours".into(), rule: RuleChoice::Times("weekdays 9-17".into()) })
    );
    assert_eq!(
        parse(&args(&["condition", "edit", "Work hours", "--times", "mon-thu 9-17"])),
        Ok(Command::ConditionEdit { condition: "Work hours".into(), name: None, rule: Some(RuleChoice::Times("mon-thu 9-17".into())) })
    );
    assert!(times_of("always").is_err() && times_of("someday").is_err());
    assert_eq!(times_of("sat").ok().map(|w| w.describe()), Some("Sat 00:00-24:00".into()));
}

#[test]
fn several_items_are_the_words_after_their_option() {
    let items = |line: &[&str], name: &str| Options::new(&args(line)).items(name);
    // Each word up to the next option is an item, also the first with =; a word with a space
    // in it (quoted in the shell) is one item, and a comma is part of an item.
    let both = Ok(Some(vec!["a.com".to_string(), "b.com".into()]));
    assert_eq!(items(&["--sites", "a.com", "b.com", "--force"], "--sites"), both);
    assert_eq!(items(&["--sites=a.com", "b.com"], "--sites"), both);
    assert_eq!(
        items(&["--titles", "Inbox, 3 unread", " *Chat* "], "--titles"),
        Ok(Some(vec!["Inbox, 3 unread".to_string(), "*Chat*".into()]))
    );
    assert_eq!(items(&["--force"], "--sites"), Ok(None));
    // A single value is one word, and needs one.
    assert_eq!(Options::new(&args(&["--name"])).value("--name"), Err("--name needs a value".into()));
    // Once, and with something in it.
    assert!(items(&["--sites", "a.com", "--sites", "b.com"], "--sites").unwrap_err().contains("more than once"));
    assert_eq!(items(&["--sites", "--force"], "--sites"), Err("--sites needs one or more items, separated by spaces".into()));
    assert_eq!(items(&["--sites=", " "], "--sites"), Err("--sites needs one or more items, separated by spaces".into()));
    // What is left after the items is for the other options.
    let mut options = Options::new(&args(&["--any", "Work", "Weekend", "--name", "Either"]));
    assert_eq!(options.items("--any"), Ok(Some(vec!["Work".to_string(), "Weekend".into()])));
    assert_eq!(options.value("--name"), Ok(Some("Either".into())));
    assert_eq!(options.finish(), Ok(()));
    // The commands take them so.
    let Ok(Command::TargetNew { changes, .. }) = parse(&args(&["target", "new", "A", "--titles", "Inbox, 3 unread", "*Chat*"])) else {
        panic!("not new")
    };
    assert_eq!(changes.lists.get("titles"), Some(&vec!["Inbox, 3 unread".to_string(), "*Chat*".into()]));
    let Ok(Command::TargetEdit { changes, .. }) = parse(&args(&["target", "edit", "A", "--clear", "sites", "allowed-titles"])) else {
        panic!("not edit")
    };
    assert_eq!(changes.lists.keys().copied().collect::<Vec<_>>(), vec!["allowed_titles", "filters"]);
    assert_eq!(changes.lists.get("filters"), Some(&Vec::new()));
    let Ok(Command::New { changes, .. }) = parse(&args(&["new", "B", "--targets", "Social", "Games"])) else { panic!("not new") };
    assert_eq!(changes.targets, Some(vec!["Social".to_string(), "Games".into()]));
}

#[test]
fn lists_come_from_files_too() {
    let path = std::env::temp_dir().join(format!("cli-{}-sites.txt", std::process::id()));
    std::fs::write(&path, "a.com\r\n\r\nb.com\n").unwrap();
    let line = args(&["target", "new", "A", "--sites-file", path.to_str().unwrap(), "--sites", "c.com"]);
    let Ok(Command::TargetNew { changes, .. }) = parse(&line) else { panic!("not new") };
    assert_eq!(changes.lists.get("filters"), Some(&vec!["c.com".to_string(), "a.com".into(), "b.com".into()]));
    let _ = std::fs::remove_file(path);
}

#[test]
fn what_is_not_understood() {
    for bad in [
        &["frobnicate"][..],
        &["new"],
        &["new", "--targets", "a"],
        &["new", "A", "--lock-duration", "soon"],
        &["new", "A", "--lock-duration", ""],
        &["new", "A", "--lock-duration", "1h", "--no-lock"],
        &["new", "A", "--name", "B"],
        &["new", "A", "--sites", "a.com"],
        &["target"],
        &["target", "new"],
        &["target", "new", "A", "--sites"],
        &["target", "new", "A", "--actions", "obsidian_web_viewer,lasers"],
        &["target", "new", "A", "--clear", "sites"],
        &["target", "edit", "A", "--clear", "everything"],
        &["target", "frob", "A"],
        &["condition", "new", "A"],
        &["condition", "new", "A", "--times", "mon 9-17", "--drive", "Kingston"],
        &["condition", "check"],
        &["edit"],
        &["start"],
        &["start", "A", "B"],
        &["stop", "A", "now"],
        &["target", "new", "A", "--sites-file", "Z:/no/such/file.txt"],
    ] {
        assert!(parse(&args(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn the_other_commands() {
    assert_eq!(parse(&[]), Ok(Command::Help));
    assert_eq!(parse(&args(&["--version"])), Ok(Command::Version));
    assert_eq!(parse(&args(&["status", "--json"])), Ok(Command::Status { json: true }));
    assert_eq!(parse(&args(&["show", "Work", "--json"])), Ok(Command::Show { session: "Work".into(), json: true }));
    assert_eq!(parse(&args(&["delete", "Work", "--force"])), Ok(Command::Delete { session: "Work".into(), force: true }));
    assert_eq!(parse(&args(&["export", "b.json"])), Ok(Command::Export { file: "b.json".into() }));
    assert_eq!(parse(&args(&["import", "b.json", "--force"])), Ok(Command::Import { file: "b.json".into(), force: true }));
    assert!(parse(&args(&["import"])).is_err());
    assert_eq!(parse(&args(&["duplicate", "Work"])), Ok(Command::Duplicate { kind: Kind::Session, item: "Work".into() }));
    assert_eq!(parse(&args(&["target", "duplicate", "Social"])), Ok(Command::Duplicate { kind: Kind::Target, item: "Social".into() }));
    assert_eq!(
        parse(&args(&["condition", "duplicate", "Evenings"])),
        Ok(Command::Duplicate { kind: Kind::Condition, item: "Evenings".into() })
    );
    assert_eq!(parse(&args(&["stop", "Work"])), Ok(Command::Stop { session: "Work".into(), password: None, abort: false }));
    assert_eq!(
        parse(&args(&["stop", "Work", "--password", "pw"])),
        Ok(Command::Stop { session: "Work".into(), password: Some("pw".into()), abort: false })
    );
    assert_eq!(
        parse(&args(&["start", "Work", "--lock-password", "partner"])),
        Ok(Command::Start {
            session: "Work".into(),
            lock: Some(Lock::Password { password: Some("partner".into()), hash: None }),
            force: false
        })
    );
    assert!(parse(&args(&["start", "Work", "--lock-password", "pw", "--no-lock"])).is_err());
    assert_eq!(
        parse(&args(&["start", "Work", "--lock-text", "I will not scroll", "--force"])),
        Ok(Command::Start { session: "Work".into(), lock: Some(Lock::Text { text: "I will not scroll".into() }), force: true })
    );
    assert!(parse(&args(&["start", "Work", "--lock-text", "x", "--lock-password", "pw"])).is_err());
    assert_eq!(
        parse(&args(&["start", "Work", "--lock-random", "12"])),
        Ok(Command::Start { session: "Work".into(), lock: Some(Lock::RandomText { length: 12, text: None }), force: false })
    );
    assert_eq!(
        parse(&args(&["start", "Work", "--lock-random", "twelve"])),
        Err("The number of characters is a whole number, not “twelve”".into())
    );
    assert_eq!(parse(&args(&["start", "Work", "--lock-random", "0"])), Err("A random text has 1 to 1000 characters".into()));
    // A flash drive is named here and looked up among the connected ones when the command runs.
    assert_eq!(
        parse(&args(&["condition", "new", "Key", "--drive", "Kingston"])),
        Ok(Command::ConditionNew { name: "Key".into(), rule: RuleChoice::Drive("Kingston".into()) })
    );
    assert_eq!(parse(&args(&["drives", "--json"])), Ok(Command::Drives { json: true }));
    assert_eq!(parse(&args(&["target", "show", "Social"])), Ok(Command::TargetShow { target: "Social".into(), json: false }));
    assert_eq!(
        parse(&args(&["target", "delete", "Social", "--force"])),
        Ok(Command::TargetDelete { target: "Social".into(), force: true })
    );
    assert_eq!(parse(&args(&["condition", "show", "Work hours"])), Ok(Command::ConditionShow { condition: "Work hours".into() }));
    assert_eq!(
        parse(&args(&["condition", "delete", "Work hours"])),
        Ok(Command::ConditionDelete { condition: "Work hours".into(), force: false })
    );
    assert_eq!(parse(&args(&["condition", "check", "weekdays 9-5pm"])), Ok(Command::ConditionCheck { times: "weekdays 9-5pm".into() }));
    assert_eq!(parse(&args(&["unlock"])), Ok(Command::Unlock { abort: false, password: None }));
    assert_eq!(parse(&args(&["unlock", "--password", "pw"])), Ok(Command::Unlock { abort: false, password: Some("pw".into()) }));
    assert_eq!(parse(&args(&["unlock-way"])), Ok(Command::UnlockWay { way: None }));
    assert_eq!(parse(&args(&["unlock-way", "--none"])), Ok(Command::UnlockWay { way: Some(None) }));
    assert_eq!(
        parse(&args(&["unlock-way", "--text", "let me out"])),
        Ok(Command::UnlockWay { way: Some(Some(Lock::Text { text: "let me out".into() })) })
    );
    assert!(parse(&args(&["unlock-way", "--text", "x", "--password", "y"])).is_err());
    assert_eq!(
        parse(&args(&["unlock-way", "--random", "8"])),
        Ok(Command::UnlockWay { way: Some(Some(Lock::RandomText { length: 8, text: None })) })
    );
    assert!(parse(&args(&["unlock-way", "--random", "x"])).is_err());
    assert!(parse(&args(&["unlock-way", "--duration", "1h"])).is_err());
    assert_eq!(parse(&args(&["unlock", "--abort"])), Ok(Command::Unlock { abort: true, password: None }));
    assert_eq!(parse(&args(&["unlock-delay"])), Ok(Command::UnlockDelay { delay: None }));
    assert_eq!(parse(&args(&["unlock-delay", "2h 30m"])), Ok(Command::UnlockDelay { delay: Some("2h 30m".into()) }));
    assert_eq!(parse(&args(&["unlock-delay", "0"])), Ok(Command::UnlockDelay { delay: Some("0".into()) }));
}

#[test]
fn deleting_says_what_it_does_to_the_sessions() {
    // In the window's words, a sentence each, then all of it as the window's question.
    let mut overview = library();
    assert_eq!(fallout(&overview, "l1"), ["Goes on without it: “Work”"]);
    assert_eq!(fallout(&overview, "s1"), ["Always active from now: “Work”", "No lock from now: “Work”"]);
    assert_eq!(fallout(&overview, "other"), Vec::<String>::new());
    assert_eq!(
        delete_question(&overview, "targets", "Social", "l1"),
        "Delete “Social”? Deletes the target for good. Goes on without it: “Work”"
    );
    assert_eq!(delete_question(&overview, "sessions", "Work", "b1"), "Delete “Work”? Deletes the session for good");
    overview.running.push(a_running("b1"));
    assert_eq!(
        delete_question(&overview, "sessions", "Work", "b1"),
        "Delete “Work”? Deletes the session for good. It is running and will be stopped"
    );
}

#[test]
fn things_are_found_by_id_or_name() {
    let lists =
        vec![target("a1", &[]), Target { name: "Games".into(), ..target("b2", &[]) }, Target { name: "games".into(), ..target("c3", &[]) }];
    fn key(l: &Target) -> (&String, &String) {
        (&l.id, &l.name)
    }
    assert_eq!(find(&lists, "a1", "target", key).ok().map(|l| l.id.as_str()), Some("a1"));
    assert_eq!(find(&lists, "list A1", "target", key).ok().map(|l| l.id.as_str()), Some("a1"));
    assert!(matches!(find(&lists, "Games", "target", key), Err(Failure::Failed(why)) if why.contains("b2, c3")));
    assert!(find(&lists, "Nope", "target", key).is_err());
    assert!(new_id().chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && new_id().len() <= 64);
}

#[test]
fn the_status_reads_like_the_pages() {
    let empty = Overview {
        targets: vec![],
        conditions: vec![],
        sessions: vec![],
        running: vec![],
        unlock: UnlockStatus::default(),
        clock_synced: true,
        status: vec![],
    };
    assert!(describe_overview(&empty).contains("Sessions:\n  None yet"));
    let mut overview = library();
    overview.running.push(Running {
        id: "b1".into(),
        name: "Work".into(),
        targets: vec![RunningTarget {
            name: "Social".into(),
            entries: Entries { filters: vec!["youtube.com".into()], ..Entries::default() },
            actions: Actions::default(),
        }],
        remaining: None,
        lock_condition: Week::parse("mon-fri 8-17").unwrap().map(|week| Rule::Schedule { week }),
        until_restart: false,
        lock_text: None,
        stop_delay: None,
        stop_delay_words: None,
        stop_remaining: None,
        stop_remaining_words: None,
        password: false,
        locked: false,
        condition: Week::parse("mon-fri 8-17").unwrap().map(|week| Rule::Schedule { week }),
        enforcing: false,
        condition_change: Some(3600),
        remaining_words: None,
        condition_change_words: Some(duration::countdown(3600)),
        lock_change: Some(3600),
        lock_change_words: Some(duration::countdown(3600)),
        allowance: None,
    });
    overview.clock_synced = false;
    let overview = with_status(overview);
    let text = describe_overview(&overview);
    assert!(text.contains("  Work  Inactive · Inactive for 1 hour · Never paused · Unlocked for 1 hour"), "{text}");
    assert!(text.contains("Targets:\n  Social  Used by Work"), "{text}");
    assert!(text.contains("Conditions:\n  Work hours  Active Mon-Fri 08:00-17:00 · Used by Work"), "{text}");
    assert!(text.contains("Waiting for internet time"), "{text}");

    let session = describe_session(&overview, &overview.sessions[0]);
    assert!(session.contains("State:       Inactive\n"), "{session}");
    assert!(session.contains("Target:      Social\n") && session.contains("Condition:   Work hours · Inactive for 1 hour\n"), "{session}");
    assert!(session.contains("Lock:        Condition · Unlocked for 1 hour\n"), "{session}");
    assert_eq!(
        resolve_lock(&overview, Lock::Condition { condition: "work HOURS".into() }).ok(),
        Some(Lock::Condition { condition: "s1".into() })
    );
    // A target as its editor has it: its sections with something in them, by the window's labels.
    let none = switched(Actions::default(), &["all".to_string()], false).unwrap();
    let empty = Target { entries: Entries::default(), actions: none, ..overview.targets[0].clone() };
    assert!(describe_target(&overview, &empty).contains("Nothing to block\n"));
    let list = describe_target(&overview, &overview.targets[0]);
    assert!(list.contains("Websites\n  Sites to block: youtube.com\n") && list.ends_with("Used by:     Work\n"), "{list}");
}

/// What `new` and `edit` say of the allowance, as terms over the ones the session has.
fn allowance_of(line: &[&str], old: Option<Terms>) -> Result<Option<Terms>, String> {
    let changes = match parse(&args(line))? {
        Command::New { changes, .. } | Command::Edit { changes, .. } => changes,
        other => panic!("not new or edit: {other:?}"),
    };
    let mut b = library().sessions[0].clone();
    b.allowance = old;
    match apply_session_changes(&library(), &mut b, changes) {
        Ok(()) => Ok(b.allowance),
        Err(Failure::Failed(why) | Failure::Usage(why)) => Err(why),
    }
}

#[test]
fn a_session_has_its_allowance_set_by_its_options() {
    let budget = |allowed: &str, scope, reset| {
        Some(Terms::Budget { allowed: allowed.into(), scope, reset, every: String::new(), anchor: Anchor::Start })
    };
    let every = |allowed: &str, scope, period: &str, anchor| {
        Some(Terms::Budget { allowed: allowed.into(), scope, reset: Reset::Every, every: period.into(), anchor })
    };
    let cycle = |blocked: &str, allowed: &str, anchor| Some(Terms::Pomodoro { blocked: blocked.into(), allowed: allowed.into(), anchor });
    assert_eq!(
        allowance_of(
            &["new", "Social", "--allowance", "budget", "--allowed", "30m", "--scope", "each", "--reset=1d", "--anchor", "clock"],
            None
        ),
        Ok(every("30m", Scope::Each, "1d", Anchor::Clock))
    );
    // Every so long, from the start by default; never and by condition.
    assert_eq!(allowance_of(&["new", "A", "--allowed", "1h", "--reset", "36h"], None), Ok(every("1h", Scope::Total, "36h", Anchor::Start)));
    assert_eq!(
        allowance_of(&["new", "A", "--allowed", "1h", "--reset", "Condition"], None),
        Ok(budget("1h", Scope::Total, Reset::Condition))
    );
    // --anchor alone keeps a budget a budget, and a reset every so long keeps its period.
    let daily = every("30m", Scope::Total, "1d", Anchor::Start);
    assert_eq!(allowance_of(&["edit", "Work", "--anchor", "clock"], daily.clone()), Ok(every("30m", Scope::Total, "1d", Anchor::Clock)));
    assert_eq!(allowance_of(&["edit", "Work", "--reset", "never"], daily), Ok(budget("30m", Scope::Total, Reset::Never)));
    assert!(allowance_of(&["new", "A", "--allowed", "1h", "--reset", "7h", "--anchor", "clock"], None)
        .unwrap_err()
        .contains("fits a day evenly"));
    // The defaults: the whole session, never reset; the kind can be left out.
    assert_eq!(allowance_of(&["new", "A", "--allowed", "1h"], None), Ok(budget("1h", Scope::Total, Reset::Never)));
    // A cycle is told by its own options.
    assert_eq!(
        allowance_of(&["new", "P", "--blocked", "50m", "--allowed", "10m", "--anchor", "clock"], None),
        Ok(cycle("50m", "10m", Anchor::Clock))
    );
    // Without its options the allowance stays as it is; editing replaces what is named and keeps the rest.
    let old = cycle("50m", "10m", Anchor::Start);
    assert_eq!(allowance_of(&["edit", "Work", "--name", "Pauses"], old.clone()), Ok(old.clone()));
    assert_eq!(allowance_of(&["edit", "Work", "--allowed", "5m"], old.clone()), Ok(cycle("50m", "5m", Anchor::Start)));
    // Another kind starts afresh, and none takes it away.
    assert_eq!(
        allowance_of(&["edit", "Work", "--allowance", "budget", "--allowed", "20m"], old.clone()),
        Ok(budget("20m", Scope::Total, Reset::Never))
    );
    assert_eq!(allowance_of(&["edit", "Work", "--allowance", "NONE"], old.clone()), Ok(None));
    // What cannot be: no time, a cycle that does not fit the day on the clock, and words that are not options.
    assert!(allowance_of(&["new", "A", "--allowance", "budget"], None).is_err());
    assert!(allowance_of(&["new", "P", "--blocked", "25m", "--allowed", "10m", "--anchor", "clock"], None).is_err());
    for bad in [
        &["new", "A", "--allowance", "forever"][..],
        &["new", "A", "--scope", "some"],
        &["new", "A", "--reset", "monthly"],
        &["new", "A", "--anchor", "noon"],
        &["new", "A", "--allowance", "none", "--allowed", "5m"],
        &["allowance", "new", "A"],
    ] {
        assert!(parse(&args(bad)).is_err(), "{bad:?}");
    }

    // It is shown on the session, and what it is doing with a running one.
    let overview = {
        let mut overview = library();
        overview.sessions[0].allowance = cycle("50m", "10m", Anchor::Clock);
        with_status(overview)
    };
    let shown = describe_session(&overview, &overview.sessions[0]);
    assert!(shown.contains("Allowance:   Pomodoro cycle · 50 minutes / 10 minutes\n"), "{shown}");
}

#[test]
fn breaks_are_set_by_their_options_and_taken_with_break() {
    let breaks = |count, length: &str, reset, every: &str, anchor| {
        Some(Terms::Breaks { count, length: length.into(), reset, every: every.into(), anchor })
    };
    assert_eq!(
        allowance_of(
            &["new", "A", "--allowance", "breaks", "--breaks", "3", "--length", "10m", "--reset", "1d", "--anchor", "clock"],
            None
        ),
        Ok(breaks(3, "10m", Reset::Every, "1d", Anchor::Clock))
    );
    // Without --allowance, --breaks or --length say it; edit keeps what is not named.
    let old = breaks(3, "10m", Reset::Never, "", Anchor::Start);
    assert_eq!(allowance_of(&["new", "A", "--breaks", "2", "--length", "5m"], None), Ok(breaks(2, "5m", Reset::Never, "", Anchor::Start)));
    assert_eq!(allowance_of(&["edit", "Work", "--breaks", "1"], old.clone()), Ok(breaks(1, "10m", Reset::Never, "", Anchor::Start)));
    assert_eq!(
        allowance_of(&["edit", "Work", "--reset", "condition"], old.clone()),
        Ok(breaks(3, "10m", Reset::Condition, "", Anchor::Start))
    );
    assert_eq!(
        allowance_of(&["new", "A", "--allowance", "breaks", "--length", "5m"], None),
        Err("Give --breaks, how many breaks, e.g. 3".into())
    );
    assert_eq!(
        allowance_of(&["new", "A", "--allowance", "breaks", "--breaks", "2"], None),
        Err("Give --length, how long a break is, e.g. 10m".into())
    );
    assert_eq!(
        allowance_of(&["new", "A", "--breaks", "many", "--length", "5m"], None),
        Err("--breaks takes a number of breaks, not “many”".into())
    );
    assert!(allowance_of(&["new", "A", "--breaks", "0", "--length", "5m"], None).unwrap_err().contains("how many breaks"));
    assert_eq!(
        allowance_of(&["new", "A", "--breaks", "2", "--allowed", "5m"], None),
        Err("Breaks take only --breaks, --length, --reset and --anchor".into())
    );
    assert_eq!(
        allowance_of(&["new", "A", "--allowance", "budget", "--breaks", "2"], None),
        Err("--breaks and --length go with --allowance breaks".into())
    );
    assert_eq!(parse(&args(&["break", "Work"])), Ok(Command::Break { session: "Work".into(), end: false }));
    assert_eq!(parse(&args(&["break", "Work", "--end"])), Ok(Command::Break { session: "Work".into(), end: true }));

    // Through the service: taken, refused while one goes on, ended early, and none left.
    serve("breaks");
    ok(&["target", "new", "Social", "--sites", "youtube.com"]);
    ok(&["new", "Pause", "--targets", "Social", "--allowance", "breaks", "--breaks", "1", "--length", "10m"]);
    assert!(refused(&["break", "Pause"]).contains("not running"));
    ok(&["start", "Pause"]);
    assert!(ok(&["show", "Pause"]).contains("Allowance:   Breaks · 1 break left"), "{}", ok(&["show", "Pause"]));
    assert_eq!(ok(&["break", "Pause"]), "“Pause” is on a break for 10 minutes.\n");
    assert!(ok(&["show", "Pause"]).contains("Allowance:   Breaks · 10 minutes left"));
    assert_eq!(refused(&["break", "Pause"]), "A break is going on already");
    assert_eq!(ok(&["break", "Pause", "--end"]), "Ended the break of “Pause”.\n");
    assert!(ok(&["show", "Pause"]).contains("Allowance:   Breaks · None left"));
    assert_eq!(refused(&["break", "Pause"]), "No breaks left");
}

#[test]
fn a_delayed_start_is_set_by_after_or_from() {
    let delayed = |kind, value: &str| Some(Terms::Delayed { kind, value: value.into() });
    assert_eq!(allowance_of(&["new", "A", "--after", "30m"], None), Ok(delayed(DelayKind::After, "30m")));
    assert_eq!(allowance_of(&["new", "A", "--allowance", "delayed", "--after", "1h"], None), Ok(delayed(DelayKind::After, "1h")));
    assert_eq!(allowance_of(&["new", "A", "--from", "2026-10-06 18:00"], None), Ok(delayed(DelayKind::From, "2026-10-06 18:00")));
    // Over any allowance, a delayed start replaces it; over an old delayed start, the one named wins.
    let cycle = Some(Terms::Pomodoro { blocked: "50m".into(), allowed: "10m".into(), anchor: Anchor::Start });
    assert_eq!(allowance_of(&["edit", "Work", "--after", "5m"], cycle.clone()), Ok(delayed(DelayKind::After, "5m")));
    let old = delayed(DelayKind::After, "30m");
    assert_eq!(
        allowance_of(&["edit", "Work", "--from", "2026-10-06 18:00"], old.clone()),
        Ok(delayed(DelayKind::From, "2026-10-06 18:00"))
    );
    assert_eq!(allowance_of(&["edit", "Work", "--after", "10m"], old.clone()), Ok(delayed(DelayKind::After, "10m")));
    // Without --after or --from the old delayed start stays; without one there is nothing to keep.
    assert_eq!(allowance_of(&["edit", "Work", "--allowance", "delayed"], old.clone()), Ok(old.clone()));
    assert!(allowance_of(&["edit", "Work", "--allowance", "delayed"], cycle).unwrap_err().contains("Give --after"));
    assert!(allowance_of(&["new", "A", "--allowance", "delayed"], None).unwrap_err().contains("Give --after"));
    // The start is checked before anything is sent.
    assert!(allowance_of(&["new", "A", "--after", "soon"], None).is_err());
    assert!(allowance_of(&["new", "A", "--after", "0m"], None).is_err());
    assert!(allowance_of(&["new", "A", "--from", "tomorrow"], None).is_err());
    // None takes it away.
    assert_eq!(allowance_of(&["edit", "Work", "--allowance", "none"], old), Ok(None));
    // What does not go together.
    for (line, why) in [
        (&["new", "A", "--after", "5m", "--from", "2026-10-06 18:00"][..], "not both"),
        (&["new", "A", "--after", "5m", "--allowed", "5m"], "takes only --after or --from"),
        (&["new", "A", "--from", "2026-10-06 18:00", "--blocked", "5m"], "takes only --after or --from"),
        (&["new", "A", "--allowance", "delayed", "--scope", "each"], "takes only --after or --from"),
        (&["new", "A", "--allowance", "delayed", "--reset", "never"], "takes only --after or --from"),
        (&["new", "A", "--allowance", "delayed", "--anchor", "clock"], "takes only --after or --from"),
        (&["new", "A", "--allowance", "none", "--after", "5m"], "takes no other allowance options"),
        (&["new", "A", "--allowance", "none", "--from", "2026-10-06 18:00"], "takes no other allowance options"),
        (&["new", "A", "--allowance", "budget", "--after", "5m"], "go with --allowance delayed"),
        (&["new", "A", "--allowance", "pomodoro", "--from", "2026-10-06 18:00"], "go with --allowance delayed"),
    ] {
        let error = parse(&args(line)).unwrap_err();
        assert!(error.contains(why), "{line:?}: {error}");
    }
    assert!(parse(&args(&["new", "A", "--allowance", "wait"])).is_err());
    // It is shown on the session, and what it is doing with a running one.
    let mut overview = library();
    overview.sessions[0].allowance = delayed(DelayKind::After, "30m");
    let shown = describe_session(&with_status(overview.clone()), &overview.sessions[0]);
    assert!(shown.contains("Allowance:   Delayed start · After 30 minutes\n"), "{shown}");
    overview.sessions[0].allowance = delayed(DelayKind::From, "2000-10-06 18:00");
    let shown = describe_session(&with_status(overview.clone()), &overview.sessions[0]);
    assert!(shown.contains("Allowance:   Delayed start · From October 6\n"), "{shown}");
}

#[test]
fn every_list_and_action_has_one_section() {
    let lists: Vec<&str> = SECTIONS.iter().flat_map(|(_, lists, _)| lists.iter().copied()).collect();
    assert_eq!(lists, LIST_OPTIONS.map(|(_, field)| field));
    let mut actions: Vec<&str> = SECTIONS.iter().flat_map(|(_, _, actions)| actions.iter().copied()).collect();
    actions.sort_unstable();
    let all = serde_json::to_value(Actions::default()).unwrap();
    let mut known: Vec<&str> = all.as_object().unwrap().keys().map(String::as_str).collect();
    known.sort_unstable();
    assert_eq!(actions, known);
}

/// The sections are the target editor's (`SECTIONS` in `forms.js`): the same lists and actions, in
/// the same order.
#[test]
fn the_sections_are_the_windows() {
    let page = include_str!("../../../src/js/forms.js");
    let table = &page[page.find("const SECTIONS = [").unwrap()..];
    let table = &table[..table.find("].map(").unwrap()];
    // The quoted words of a line: all of them, or those in its `name: [ … ]`.
    let quoted = |text: &str| text.split('"').skip(1).step_by(2).map(String::from).collect::<Vec<_>>();
    let within = |line: &str, name: &str| {
        let list = &line[line.find(&format!("{name}: [")).unwrap()..];
        quoted(&list[..list.find(']').unwrap()])
    };
    let page: Vec<_> = table
        .lines()
        .filter(|line| line.trim_start().starts_with("{ key:"))
        .map(|line| (quoted(line)[0].clone(), within(line, "lists"), within(line, "actions")))
        .collect();
    let strings = |words: &[&str]| words.iter().map(|word| word.to_string()).collect::<Vec<_>>();
    let ours: Vec<_> = SECTIONS.iter().map(|(key, lists, actions)| (key.to_string(), strings(lists), strings(actions))).collect();
    assert_eq!(page, ours);
}

#[test]
fn a_schedule_is_shown_per_day() {
    let text = per_day(&Week::parse("mon-fri 8-12, 13-17; sat 10-12").unwrap().unwrap());
    assert!(text.starts_with("  Mon  08:00-12:00, 13:00-17:00\n"), "{text}");
    assert!(text.contains("  Tue  08:00-12:00, 13:00-17:00\n"), "{text}");
    assert!(text.ends_with("  Sun  Inactive\n"), "{text}");
}
