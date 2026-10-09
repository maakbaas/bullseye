//! What the CLI prints: sessions, targets and conditions as the window shows them, and the questions
//! it asks before what cannot be undone, in the window's words.

use super::*;

/// The Store apps an import leaves out, as they are not installed here, in a sentence; empty for none.
/// `checked`: whether what is installed could be told at all (else everything is kept).
pub(super) fn left_out_words(left_out: &[definitions::LeftOut], checked: bool) -> String {
    if !checked {
        return t("settings.importAppsUnknown", &[]);
    }
    if left_out.is_empty() {
        return String::new();
    }
    let apps: Vec<String> =
        left_out.iter().map(|app| t("settings.importAppIn", &[("app", &quoted(&app.name)), ("target", &quoted(&app.target))])).collect();
    t("settings.importAppsLeftOut", &[("apps", &joined(&apps))])
}

/// The other way to unlock, in words.
pub(super) fn describe_unlock_way(status: &UnlockStatus) -> String {
    let when = |open: &str, closed: &str| match (status.way_open, &status.way_left_words) {
        (true, _) => open.to_string(),
        (false, Some(left)) => format!("{closed} (in {left})"),
        (false, None) => closed.to_string(),
    };
    match &status.way {
        None => "Only the delay unlocks.".into(),
        Some(Lock::Text { .. }) => "Unlocking is also possible by typing a text in, in the window.".into(),
        Some(Lock::RandomText { .. }) => "Unlocking is also possible by typing a random text in, in the window.".into(),
        Some(Lock::Password { .. }) => "Unlocking is also possible at once with a password (`unlock --password`).".into(),
        Some(Lock::Condition { .. }) => {
            when("Unlocking is open: its condition is not active.", "Unlocking is locked while its condition is active")
        }
        Some(_) => "Only the delay unlocks.".into(),
    }
}

/// What a session's lock says of its condition besides its name: nothing for a schedule (the times are
/// on the Conditions page, and only there), and the drive for a flash drive, which is what makes
/// A webhook condition as words.
pub(super) fn describe_webhook(hook: &Hook) -> String {
    let matched = match (hook.matcher.on, hook.matcher.status, &hook.matcher.body) {
        (Match::Status, Some(status), _) => format!("HTTP {status}"),
        (Match::Body, _, Some(body)) => format!("body “{body}”"),
        _ => "(not set)".into(),
    };
    let (usually, other) = if hook.fallback { ("active", "not active") } else { ("not active", "active") };
    format!(
        "{} {} every {}: {usually}, {other} on {matched}",
        if hook.method == Method::Post { "POST" } else { "GET" },
        hook.url,
        duration::period(hook.interval)
    )
}

/// A condition as its tile's Active row says it.
pub(super) fn condition_words(overview: &Overview, rule: &Rule) -> String {
    match rule {
        Rule::Schedule { week } => t("conditions.active", &[("times", &week.describe())]),
        Rule::FlashDrive { drive, .. } => t("conditions.activeDrive", &[("drive", &quoted(drive))]),
        Rule::Webhook(hook) => t("conditions.activeWebhook", &[("url", &hook.url)]),
        Rule::Combined { mode, members, .. } => {
            let names: Vec<String> = members
                .iter()
                .map(|id| overview.conditions.iter().find(|c| &c.id == id).map_or_else(|| t("common.gone", &[]), |c| c.name.clone()))
                .collect();
            t(if *mode == Mode::Any { "conditions.activeAny" } else { "conditions.activeAll" }, &[("names", &joined(&names))])
        }
    }
}

/// A target's or condition's Used by row, label and all; or that no session uses it.
pub(super) fn used_by(overview: &Overview, id: &str) -> String {
    let library = Library { sessions: overview.sessions.clone(), ..Library::default() };
    if library.users_of(id).is_empty() {
        users(overview, id)
    } else {
        format!("{} {}", t("tiles.usedBy", &[]), users(overview, id))
    }
}

/// The question before starting a session with a lock that holds, as the window asks it (`locks.js`):
/// its heading and its body; none for no lock, or an end that has passed (the service says why).
pub(super) fn lock_question(overview: &Overview, session: &Definition) -> Option<(String, String)> {
    let name = quoted(&session.name);
    let ask = |method: &str, heading: &str, body: &str, values: &[(&str, &str)]| {
        let values: Vec<(&str, &str)> = [("name", name.as_str())].into_iter().chain(values.iter().copied()).collect();
        Some((t(&format!("locks.{method}.{heading}"), &values), t(&format!("locks.{method}.{body}"), &values)))
    };
    match &session.lock {
        Lock::None => None,
        Lock::Timed { kind: TimedKind::For, .. } => {
            let duration = duration::countdown(session.lock.duration().ok()??.as_secs());
            ask("timed", "forConfirmHeading", "forConfirmBody", &[("duration", &duration)])
        }
        Lock::Timed { kind: TimedKind::Until, value } => {
            let end = localtime::until(value).ok()?;
            ask("timed", "untilConfirmHeading", "untilConfirmBody", &[("when", &end.when), ("left", &end.left)])
        }
        Lock::UntilRestart => ask("until_restart", "confirmHeading", "confirmBody", &[]),
        Lock::Condition { condition } => {
            let condition = overview.conditions.iter().find(|c| &c.id == condition)?;
            let detail = match &condition.rule {
                Rule::FlashDrive { drive, .. } => t("locks.condition.driveDetail", &[("drive", &quoted(drive))]),
                _ => String::new(),
            };
            ask("condition", "confirmHeading", "confirmBody", &[("condition", &quoted(&condition.name)), ("detail", &detail)])
        }
        Lock::Delay { .. } => {
            let delay = duration::countdown(session.lock.delay().ok()??.as_secs());
            ask("delay", "confirmHeading", "confirmBody", &[("delay", &delay)])
        }
        Lock::Text { .. } => ask("text", "confirmHeading", "confirmBody", &[]),
        Lock::RandomText { length, .. } => ask("random_text", "confirmHeading", "confirmBody", &[("n", &length.to_string())]),
        Lock::Password { .. } => ask("password", "confirmHeading", "confirmBody", &[]),
    }
}

/// A session in one line, as its tile shows it: its tag, then what its Condition, Allowance and Lock
/// rows say (`status.rs`), as the service wrote them.
pub(super) fn state(overview: &Overview, session: &Definition) -> String {
    match status_of(overview, session) {
        Some(status) => {
            format!("{} · {} · {} · {}", status.state.words(), status.condition.plain(), status.allowance.plain(), status.lock.plain())
        }
        None => String::new(),
    }
}

pub(super) fn status_of<'a>(overview: &'a Overview, session: &Definition) -> Option<&'a Status> {
    overview.status.iter().find(|status| status.id == session.id)
}

/// Names in rows, padded to the longest.
pub(super) fn table(rows: &[(String, String)]) -> String {
    let width = rows.iter().map(|(name, _)| name.chars().count()).max().unwrap_or(0);
    rows.iter().map(|(name, what)| format!("  {name:width$}  {what}\n")).collect()
}

/// What a target's or condition's tile says in its Used by row: the sessions that use it, or that
/// none does (`Library::users_of`).
pub(super) fn users(overview: &Overview, id: &str) -> String {
    let library = Library { sessions: overview.sessions.clone(), ..Library::default() };
    let names: Vec<String> = library.users_of(id).iter().map(|b| b.name.clone()).collect();
    if names.is_empty() {
        t("tiles.none", &[])
    } else {
        joined(&names)
    }
}

/// The question before deleting, as the window asks it (`tiles.js`): what goes, and what that does to
/// the sessions that use it (`Library::fallout`), or that a running session is stopped.
pub(super) fn delete_question(overview: &Overview, kind: &str, name: &str, id: &str) -> String {
    let values = [("name", quoted(name)), ("noun", t(&format!("{kind}.noun"), &[]))];
    let values: Vec<(&str, &str)> = values.iter().map(|(key, value)| (*key, value.as_str())).collect();
    let mut sentences = vec![t("delete.body", &values)];
    if kind == "sessions" {
        if overview.running.iter().any(|r| r.id == id) {
            sentences.push(t("delete.stopsIt", &[]));
        }
    } else {
        sentences.extend(fallout(overview, id));
    }
    format!("{} {}", t("delete.heading", &values), sentences.join(". "))
}

/// What deleting a target or condition does to the sessions that use it, a sentence each, as the
/// window's question says it; none when no session uses it.
pub(super) fn fallout(overview: &Overview, id: &str) -> Vec<String> {
    let library = Library { sessions: overview.sessions.clone(), conditions: overview.conditions.clone(), ..Library::default() };
    let fallout = library.fallout(id);
    [
        (&fallout.without_it, "losesTarget"),
        (&fallout.always, "becomesAlways"),
        (&fallout.unlocked, "losesLock"),
        (&fallout.shrinks, "shrinksCombination"),
        (&fallout.removed, "removesCombination"),
    ]
    .into_iter()
    .filter(|(names, _)| !names.is_empty())
    .map(|(names, key)| t(&format!("delete.{key}"), &[("names", &joined(&names.iter().map(|n| quoted(n)).collect::<Vec<_>>()))]))
    .collect()
}

/// Everything, as the window's pages show it: the sessions, the targets, the conditions.
pub(super) fn describe_overview(overview: &Overview) -> String {
    let sessions: Vec<(String, String)> = overview.sessions.iter().map(|b| (b.name.clone(), state(overview, b))).collect();
    let lists: Vec<(String, String)> = overview.targets.iter().map(|l| (l.name.clone(), used_by(overview, &l.id))).collect();
    let conditions: Vec<(String, String)> = overview
        .conditions
        .iter()
        .map(|c| (c.name.clone(), format!("{} · {}", condition_words(overview, &c.rule), used_by(overview, &c.id))))
        .collect();
    let mut text = String::new();
    for (title, rows, empty) in [
        (t("sessions.title", &[]), sessions, "  None yet. Make one with: bullseye new <name> --targets <targets>\n"),
        (t("targets.title", &[]), lists, "  None yet. Make one with: bullseye target new <name> --sites <sites>\n"),
        (t("conditions.title", &[]), conditions, "  None (sessions without one are always active).\n"),
    ] {
        text.push_str(&format!("{title}:\n"));
        text.push_str(&if rows.is_empty() { empty.to_string() } else { table(&rows) });
    }
    if !overview.clock_synced {
        text.push_str("(Waiting for internet time: until then, time with the service not running does not count.)\n");
    }
    text
}

pub(super) fn actions_on(actions: Actions) -> Vec<String> {
    match serde_json::to_value(actions) {
        Ok(Value::Object(fields)) => fields.iter().filter(|(_, on)| **on == Value::Bool(true)).map(|(name, _)| name.clone()).collect(),
        _ => Vec::new(),
    }
}

/// `Label:` then its value, the values lined up.
pub(super) fn field(label: &str, value: &str) -> String {
    format!("{:<13}{value}\n", format!("{label}:"))
}

/// One session, as its tile shows it: its rows, each value with its status; and its id.
pub(super) fn describe_session(overview: &Overview, session: &Definition) -> String {
    let status = status_of(overview, session);
    // A row's value, then what its status says.
    let row = |value: String, line: Option<&Line>| match line {
        Some(line) => format!("{value} · {}", line.plain()),
        None => value,
    };
    let gone = || t("common.gone", &[]);
    let targets: Vec<String> =
        session.targets.iter().map(|l| overview.targets.iter().find(|n| &n.id == l).map_or_else(gone, |n| n.name.clone())).collect();
    let condition = match &session.condition {
        None => t("sessions.fields.always", &[]),
        Some(id) => overview.conditions.iter().find(|c| &c.id == id).map_or_else(gone, |c| c.name.clone()),
    };
    let allowance = match &session.allowance {
        None => t("allowance.type.none", &[]),
        Some(terms) => {
            let kind = match terms {
                Terms::Budget { .. } => "budget",
                Terms::Pomodoro { .. } => "pomodoro",
                Terms::Delayed { .. } => "delayed",
            };
            t(&format!("allowance.type.{kind}"), &[])
        }
    };
    let method = match &session.lock {
        Lock::None => "none",
        Lock::Timed { .. } => "timed",
        Lock::UntilRestart => "until_restart",
        Lock::Condition { .. } => "condition",
        Lock::Delay { .. } => "delay",
        Lock::Text { .. } => "text",
        Lock::RandomText { .. } => "random_text",
        Lock::Password { .. } => "password",
    };
    let mut text = field("Name", &session.name) + &field("Id", &session.id);
    if let Some(status) = status {
        text += &field("State", &status.state.words());
    }
    text += &field(&t("sessions.fields.blockCount", &[("n", &targets.len().to_string())]), &joined(&targets));
    text += &field(&t("sessions.fields.when", &[]), &row(condition, status.map(|s| &s.condition)));
    text += &field(&t("sessions.fields.unless", &[]), &row(allowance, status.map(|s| &s.allowance)));
    text += &field(&t("sessions.fields.lock", &[]), &row(t(&format!("locks.{method}.label"), &[]), status.map(|s| &s.lock)));
    text
}

/// The target editor's sections (`forms.js`, `SECTIONS`; a test holds them equal): the lists and actions in each, by key.
pub(super) const SECTIONS: [(&str, &[&str], &[&str]); 5] = [
    ("websites", &["filters", "allowed"], &[]),
    ("programs", &["programs"], &[]),
    ("store", &["store_apps"], &[]),
    ("titles", &["titles", "allowed_titles"], &["window_titles_minimize"]),
    ("other", &[], &["obsidian_web_viewer"]),
];

/// One target, as its editor has it: each section with something in it, its lists by their labels and
/// the actions it has on; then the sessions that use it.
pub(super) fn describe_target(overview: &Overview, list: &Target) -> String {
    let on = actions_on(list.actions);
    let mut entries = list.entries.clone();
    let mut text = field("Name", &list.name) + &field("Id", &list.id);
    let mut empty = true;
    for (section, lists, actions) in SECTIONS {
        let mut lines = String::new();
        for key in lists {
            let items = entries.field(key).cloned().unwrap_or_default();
            if !items.is_empty() {
                lines += &format!("  {}: {}\n", t(&format!("targets.{key}.label"), &[]), items.join(", "));
            }
        }
        // A section's actions act on its lists: shown with them, or alone where it has none.
        if !lines.is_empty() || lists.is_empty() {
            for action in actions.iter().filter(|action| on.iter().any(|o| o == *action)) {
                lines += &format!("  {}\n", t(&format!("targets.actions.{action}.label"), &[]));
            }
        }
        if !lines.is_empty() {
            empty = false;
            text += &format!("{}\n{lines}", t(&format!("targets.sections.{section}.title"), &[]));
        }
    }
    if empty {
        text += &format!("{}\n", t("targets.nothing", &[]));
    }
    text + &field(&t("tiles.usedBy", &[]), &users(overview, &list.id))
}

/// A weekly schedule, each day on a line of its own, named as the week grid names it.
pub(super) fn per_day(week: &Week) -> String {
    const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let mut text = String::new();
    for (key, spans) in DAYS.iter().zip(&week.days) {
        let mut day = Week::default();
        day.days[0] = spans.clone();
        let times = if spans.is_empty() { t("sessions.now.inactive", &[]) } else { day.describe().trim_start_matches("Mon ").to_string() };
        text.push_str(&format!("  {}  {times}\n", t(&format!("conditions.days.{key}"), &[])));
    }
    text
}
