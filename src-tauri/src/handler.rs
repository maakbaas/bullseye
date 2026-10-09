//! What the service does with each request from the app and the installer. Everything in a request
//! is checked here again: it comes from any local user's program, not necessarily ours. The same
//! `handle` answers over the pipe (`service.rs`) and, in a debug build, inside the app
//! (`service::local_handler`), so the rules cannot differ between the two.

use std::sync::{Arc, Mutex};

use crate::allowance;
use crate::definitions::{Condition, Definition, Definitions, Library, Resolved, Target, Via};
use crate::duration;
use crate::lists::Entries;
use crate::localtime::Zone;
use crate::lock::{self, Lock, TimedKind};
use crate::proto::{Check, FreeEntries, Overview, Policy, Request, Response, UnlockStatus};
use crate::session::{Part, Sessions, StartLock};
use crate::sync::LockExt;
use crate::unlock::{self, UnlockDelay};
use crate::watchdog::AppSlot;

/// The installer asked the service to end, and to remove itself and everything it keeps.
pub struct Shutdown;

/// What `handle` works with.
pub struct Context {
    sessions: Sessions,
    definitions: Definitions,
    /// Which process is the app (`watchdog.rs`).
    slot: Arc<AppSlot>,
    /// How long unlocking takes: none to begin with, so unlock is free until a delay is set.
    unlock: UnlockDelay,
    /// Held while the definitions change or a session starts, so a session cannot start while a list or
    /// condition it uses is being changed (the check and the change are one step).
    editing: Mutex<()>,
}

impl Context {
    pub fn new(sessions: Sessions, definitions: Definitions, slot: Arc<AppSlot>) -> Context {
        Context { sessions, definitions, slot, unlock: UnlockDelay::default(), editing: Mutex::default() }
    }

    /// With the unlock delay kept in its own file (the service's, `UnlockDelay::new`).
    pub fn with_unlock(mut self, unlock: UnlockDelay) -> Context {
        self.unlock = unlock;
        self
    }
}

/// Answers one request from the process `caller`, and says whether the service should end
/// afterwards. Everything in the request is checked here again: it comes from any local user's
/// program, not necessarily ours.
pub fn handle(context: &Context, caller: u32, request: Request) -> (Response, Option<Shutdown>) {
    let sessions = &context.sessions;
    let done = |result: Result<(), String>| match result {
        Ok(()) => Response::Done,
        Err(why) => Response::Failed(why),
    };
    match request {
        Request::Status => {
            let status = sessions.status();
            // Password locks without their hashes: a client never sees them.
            let library = context.definitions.library().redacted();
            // What each session's rows say, by the Windows clock and time zone (a date's "today").
            let (zone, now) = (Zone::current(), std::time::SystemTime::now());
            let rows = library
                .sessions
                .iter()
                .map(|session| {
                    crate::status::of(session, status.running.iter().find(|r| r.id == session.id), &library.conditions, &zone, now)
                })
                .collect();
            let overview = Overview {
                targets: library.targets,
                conditions: library.conditions,
                sessions: library.sessions,
                running: status.running,
                unlock: unlock_status(context),
                clock_synced: status.clock_synced,
                status: rows,
            };
            (Response::Status(overview), None)
        }
        // The app asks to be taken in; anyone else is refused, and the app then ends itself.
        Request::Register => match context.slot.register(caller) {
            Ok(()) => (Response::Done, None),
            Err(why) => (Response::Refused(why), None),
        },
        Request::Policy => {
            // This request comes from the thread that closes tabs: hearing it is how the service
            // knows that is going on. It counts only from the process taken in as the app.
            context.slot.heard(caller);
            let policies = sessions
                .active()
                .map(|active| active.sessions)
                .unwrap_or_default()
                .into_iter()
                .map(|session| Policy {
                    entries: session.lists.entries(),
                    actions: session.actions,
                    // What an allowance lets through: the always-allowed sites and titles are the policy's own.
                    free: session
                        .free
                        .iter()
                        .map(|free| FreeEntries {
                            session: free.session.clone(),
                            key: free.key.clone(),
                            entries: Entries { allowed: Vec::new(), allowed_titles: Vec::new(), ..free.lists.entries() },
                        })
                        .collect(),
                })
                .collect();
            // What the app reports from here counts from now.
            sessions.polled();
            (Response::Policy(policies), None)
        }
        // What the app saw in use of what an allowance lets through. Anyone may say so, but it only ever
        // counts against a session, never for it; a key the session does not count is ignored.
        Request::Report { used } => {
            let used: Vec<(String, String)> = used.into_iter().take(1000).map(|used| (used.session, used.key)).collect();
            sessions.charge(&used);
            (Response::Done, None)
        }
        // A running session is a copy of its definition and its parts, and one rule decides what may
        // be done to it and to what it uses: while a locked session is involved, a change may only make
        // it stricter and nothing may be deleted (`edit_session`, `edit_shared`, `delete_session`,
        // `delete_shared`); otherwise the change is made and the running sessions follow.
        Request::SaveSession { session } => (done(edit_session(context, session)), None),
        Request::DeleteSession { id } => (done(delete_session(context, &id)), None),
        Request::SaveTarget { target: list } => (done(edit_shared(context, Proposed::List(list))), None),
        Request::DeleteTarget { id } => (done(delete_shared(context, &id, |d| d.delete_target(&id))), None),
        Request::SaveCondition { condition } => (done(edit_shared(context, Proposed::Condition(condition))), None),
        Request::DeleteCondition { id } => (done(delete_shared(context, &id, |d| d.delete_condition(&id))), None),
        // A copy changes nothing that exists, so also of what a running session holds.
        Request::Duplicate { kind, id, new_id } => {
            let _editing = context.editing.locked();
            (done(context.definitions.duplicate(kind, &id, &new_id)), None)
        }
        Request::Import { library } => (done(import(context, library)), None),
        Request::Start { id } => (done(start(context, &id)), None),
        Request::Lock { id, lock } => (done(lock_running(context, &id, lock)), None),
        Request::Check { check } => (done(check_stricter(context, check)), None),
        // A password lock stops with its own password, a text lock with its text typed in; the
        // others take none.
        Request::Stop { id, password, text } => {
            let given = password.is_some_and(|password| sessions.password_matches(&id, &password))
                || text.is_some_and(|text| sessions.text_matches(&id, &text));
            (done(sessions.stop(&id, given)), None)
        }
        Request::AskToStop { id } => (done(sessions.ask_to_stop(&id)), None),
        Request::AbortStop { id } => (done(sessions.abort_stop(&id)), None),
        Request::TakeBreak { id } => (done(sessions.take_break(&id)), None),
        Request::EndBreak { id } => (done(sessions.end_break(&id)), None),
        Request::Unlock => (done(unlock(context)), None),
        Request::UnlockWith { password, text } => (done(unlock_with(context, password.as_deref(), text.as_deref())), None),
        Request::SetUnlockWay { way } => (done(set_unlock_way(context, way)), None),
        Request::Export => (Response::Library(context.definitions.library()), None),
        Request::AskForUnlock => (done(context.unlock.request(sessions.now())), None),
        Request::AbortUnlock => (done(context.unlock.abort()), None),
        Request::SetUnlockDelay { delay } => (done(set_unlock_delay(context, &delay)), None),
        // Refused while a session cannot be stopped, judged by internet time, like the session itself.
        Request::PrepareUninstall => match sessions.locked() {
            Some(why) => (Response::Refused(why), None),
            None => (Response::Done, Some(Shutdown)),
        },
    }
}

/// Everything replaced (Import): not while a session is locked, as that would be a way out of it.
/// Sessions that run without being locked end with it, as what they were is gone.
fn import(context: &Context, library: Library) -> Result<(), String> {
    let _editing = context.editing.locked();
    if let Some(why) = context.sessions.locked() {
        return Err(format!("{why}; importing replaces everything, so it waits until no session is locked"));
    }
    context.definitions.replace(library)?;
    for id in context.sessions.running_ids() {
        // One that has become locked since stays as it is: it is a copy, and cannot be stopped.
        let _ = context.sessions.stop(&id, false);
    }
    Ok(())
}

/// Deleting a session that runs: it ends with it, unless it is locked (it could not be stopped).
fn delete_session(context: &Context, id: &str) -> Result<(), String> {
    let _editing = context.editing.locked();
    if context.sessions.running_ids().iter().any(|r| r == id) {
        if context.sessions.locked_ids().iter().any(|r| r == id) {
            return Err("A locked session cannot be deleted until it can be stopped".into());
        }
        context.sessions.stop(id, false)?;
    }
    context.definitions.delete_session(id)
}

/// Deleting a target or condition. Unless a locked session uses it (it would loosen that session)
/// it goes, and the running sessions that used it follow what is left: a session that is left without
/// a target goes on doing nothing, one that lost its condition is always active, and
/// one that lost its lock condition has no lock now (`Sessions::refresh`).
fn delete_shared(context: &Context, id: &str, delete: impl FnOnce(&Definitions) -> Result<(), String>) -> Result<(), String> {
    let _editing = context.editing.locked();
    let running = context.sessions.running_ids();
    let library = context.definitions.library();
    let using = library.users_of(id);
    let users: Vec<String> = using.iter().filter(|b| running.contains(&b.id)).map(|b| b.id.clone()).collect();
    let locked = context.sessions.locked_ids();
    if let Some(session) = using.into_iter().find(|b| locked.contains(&b.id)) {
        return Err(format!("“{}” is locked and uses this; it can be deleted once that can be stopped", session.name));
    }
    delete(&context.definitions)?;
    let library = context.definitions.library();
    for user in users {
        if library.session(&user).is_none() || refresh_running(context, std::slice::from_ref(&user)).is_err() {
            let _ = context.sessions.stop(&user, false);
        }
    }
    Ok(())
}

/// Saving a session. One that runs can be changed (its name, targets, condition and allowance)
/// unless it is locked right now (then only its name, more targets, the condition Always, and its
/// allowance only to be stricter: `allowance::tightened`), and the running session follows; its lock stays as it is: a running
/// session that is not locked is given another lock by `lock_running` (`Request::Lock`), never by saving.
fn edit_session(context: &Context, session: Definition) -> Result<(), String> {
    let _editing = context.editing.locked();
    if !context.sessions.running_ids().contains(&session.id) {
        return context.definitions.save_session(session);
    }
    let old = context.definitions.library().session(&session.id).ok_or("No such session")?.clone();
    if context.sessions.locked_ids().contains(&session.id) {
        // Its name can always be changed, locked or not: that loosens nothing.
        // (A password lock comes back from the service without its hash, so two of them are the same.)
        let same_lock =
            session.lock == old.lock || (matches!(session.lock, Lock::Password { .. }) && matches!(old.lock, Lock::Password { .. }));
        // Stricter only: more targets (none taken out), the condition Always, a stricter allowance.
        let kept_targets = old.targets.iter().all(|target| session.targets.contains(target));
        let condition = session.condition == old.condition || session.condition.is_none();
        if !kept_targets || !condition || !same_lock {
            return Err("A locked session cannot be changed until it can be stopped, except for its name, more targets, the condition Always and a stricter allowance".into());
        }
        allowance::tightened(old.allowance.as_ref(), session.allowance.as_ref())?;
        let id = session.id.clone();
        let library = context.definitions.library();
        if let Some(gone) = session.targets.iter().find(|target| library.target(target).is_none()) {
            return Err(format!("No such target: {gone}"));
        }
        let Definition { name, targets, condition, allowance, .. } = session;
        context.definitions.save_session(Definition { name, targets, condition, allowance, ..old })?;
        return refresh_running(context, &[id]);
    }
    if session.lock != old.lock {
        return Err("The lock of a running session is not changed by saving it: lock it now instead, or stop it first".into());
    }
    // Its targets as they would be: a running session keeps something to block.
    let library = context.definitions.library();
    let lists = session
        .targets
        .iter()
        .map(|l| library.target(l).cloned().ok_or("A target of this session is gone"))
        .collect::<Result<Vec<_>, _>>()?;
    something_to_block(&parts_of(&lists)?)?;
    let id = session.id.clone();
    context.definitions.save_session(session)?;
    refresh_running(context, &[id])
}

/// Gives the running sessions `ids` what their definitions are now (`Sessions::refresh`).
fn refresh_running(context: &Context, ids: &[String]) -> Result<(), String> {
    let library = context.definitions.library();
    for id in ids {
        let resolved = library.resolve(id)?;
        let parts = parts_of(&resolved.lists)?;
        context.sessions.refresh(id, &resolved.session.name, parts, resolved.condition, resolved.lock_condition, resolved.allowance);
    }
    Ok(())
}

/// A target or condition as it is to be saved.
enum Proposed {
    List(Target),
    Condition(Condition),
}

/// Saving a target or condition. While running sessions use it, what runs is kept the same as
/// what is defined: if none of them is locked right now (it could be stopped anyway) the change
/// is made and goes to them, and if one is locked the change may only make things stricter (more
/// to block, more times: `Target::tightened_by`, `Rule::tightened_by`), which then goes to
/// all of them. Otherwise a locked session could be loosened by editing what it uses.
fn edit_shared(context: &Context, proposed: Proposed) -> Result<(), String> {
    let _editing = context.editing.locked();
    check_shared(context, &proposed)?;
    let library = context.definitions.library();
    let id = match &proposed {
        Proposed::List(new) => new.id.clone(),
        Proposed::Condition(new) => new.id.clone(),
    };
    let running = context.sessions.running_ids();
    let users: Vec<String> = library.users_of(&id).into_iter().filter(|b| running.contains(&b.id)).map(|b| b.id.clone()).collect();
    match proposed {
        Proposed::List(list) => context.definitions.save_target(list)?,
        Proposed::Condition(condition) => context.definitions.save_condition(condition)?,
    }
    // What the sessions that run use is what is defined now.
    refresh_running(context, &users)
}

/// Whether a target or condition may be saved like this: any way while no locked session uses it, and
/// when one does, only to be stricter (`Target::tightened_by`, `Rule::tightened_by`).
fn check_shared(context: &Context, proposed: &Proposed) -> Result<(), String> {
    let library = context.definitions.library();
    let id = match proposed {
        Proposed::List(new) => &new.id,
        Proposed::Condition(new) => &new.id,
    };
    // Only running sessions are locked.
    let locked = context.sessions.locked_ids();
    let locked: Vec<String> = library.users_of(id).into_iter().filter(|b| locked.contains(&b.id)).map(|b| b.id.clone()).collect();
    if locked.is_empty() {
        return Ok(());
    }
    match proposed {
        Proposed::List(new) => {
            if let Some(old) = library.target(&new.id) {
                old.tightened_by(new)?;
            }
        }
        Proposed::Condition(new) => {
            if let Some(old) = library.condition(&new.id) {
                for (session, via) in library.usages(&new.id).into_iter().filter(|(session, _)| locked.contains(&session.id)) {
                    // In a combination that needs all its conditions, more active time in
                    // this one is not more in the combination: it stays as it is.
                    if via == Via::All {
                        if old.rule != new.rule.clone().checked()? {
                            return Err(format!(
                                "“{}” is locked and has this in a combination that needs all of its conditions, so it cannot change",
                                session.name
                            ));
                        }
                    } else {
                        old.rule.tightened_by(&new.rule)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Whether a change would be accepted, without making it (`Request::Check`): the window asks before it
/// asks the person to confirm making a locked session stricter, so nothing is confirmed that is then
/// refused. The rules are the ones the change itself goes by.
fn check_stricter(context: &Context, check: Check) -> Result<(), String> {
    let _editing = context.editing.locked();
    let locked = |id: &str| context.sessions.locked_ids().iter().any(|locked| locked == id);
    let library = context.definitions.library();
    match check {
        Check::AllowanceKind { id, kind } => {
            let old = library.session(&id).ok_or("No such session")?;
            if locked(&id) {
                allowance::kind_allowed(old.allowance.as_ref(), &kind)?;
            }
            Ok(())
        }
        Check::Allowance { id, allowance } => {
            let old = library.session(&id).ok_or("No such session")?;
            if locked(&id) {
                allowance::tightened(old.allowance.as_ref(), allowance.as_ref())?;
            }
            Ok(())
        }
        Check::Lock { id, lock } => {
            // Whether it can be read at all, for any session or the other way to unlock (no id).
            lock.check()?;
            if !context.sessions.running_ids().iter().any(|running| running == &id) {
                return Ok(());
            }
            let mut library = library;
            library.sessions.iter_mut().find(|session| session.id == id).ok_or("No such session")?.lock = lock;
            let start = library.resolve(&id).and_then(|resolved| start_lock(&resolved))?;
            context.sessions.check_relock(&id, start)
        }
        Check::Target { target } => check_shared(context, &Proposed::List(target)),
        Check::Condition { condition } => check_shared(context, &Proposed::Condition(condition)),
    }
}

/// What a running session enforces of its targets: each with its own lists and actions.
fn parts_of(lists: &[Target]) -> Result<Vec<Part>, String> {
    lists.iter().map(|list| Ok(Part { name: list.name.clone(), lists: list.lists()?, actions: list.actions })).collect()
}

/// Refused when nothing in `parts` can run: a session is not started, nor left running, with nothing
/// to block. An action has nothing to do with an empty list of its own kind, while one that needs no
/// list (Obsidian's) always does.
fn something_to_block(parts: &[Part]) -> Result<(), String> {
    if parts.iter().any(Part::can_run) {
        return Ok(());
    }
    Err("Nothing to block: add a site or program to its targets, or turn on an action that needs no list".into())
}

/// Starts a session as it is defined now, with its targets and condition, checked again where it
/// counts.
fn start(context: &Context, id: &str) -> Result<(), String> {
    let _editing = context.editing.locked();
    let resolved = context.definitions.library().resolve(id)?;
    let lock = start_lock(&resolved)?;
    let parts = parts_of(&resolved.lists)?;
    something_to_block(&parts)?;
    context.sessions.start_with(id, &resolved.session.name, parts, resolved.condition, resolved.allowance, lock)
}

/// Locks a running session that is not locked: the lock is saved as the session's (so it is what
/// the next start has too, a password hashed as always) and the session runs with it from now on,
/// as if it started with it now (`Sessions::relock`). A session that is locked keeps its lock, except
/// that one that ends at a time may end later and a delay may get longer; the rest is refused, and
/// nothing is kept.
fn lock_running(context: &Context, id: &str, lock: Lock) -> Result<(), String> {
    let _editing = context.editing.locked();
    if !context.sessions.running_ids().iter().any(|running| running == id) {
        return Err("This session is not running: choose its lock for the next start instead".into());
    }
    let old = context.definitions.library().session(id).ok_or("No such session")?.clone();
    context.definitions.save_session(Definition { lock, ..old.clone() })?;
    let applied = context
        .definitions
        .library()
        .resolve(id)
        .and_then(|resolved| start_lock(&resolved))
        .and_then(|lock| context.sessions.relock(id, lock));
    if applied.is_err() {
        // Not given to the session, so not kept as its lock either.
        context.definitions.save_session(old)?;
    }
    applied
}

/// What a session's lock, as defined, starts as.
fn start_lock(resolved: &Resolved) -> Result<StartLock, String> {
    let lock = &resolved.session.lock;
    Ok(match lock {
        Lock::None => StartLock::None,
        Lock::Timed { kind: TimedKind::For, .. } => StartLock::For(lock.duration()?.ok_or("No duration for the lock")?),
        Lock::Timed { kind: TimedKind::Until, .. } => StartLock::Until(lock.until()?.ok_or("No end for the lock")?),
        Lock::Condition { .. } => StartLock::Condition(resolved.lock_condition.clone().ok_or("No condition for the lock")?),
        Lock::UntilRestart => StartLock::UntilRestart,
        Lock::Delay { .. } => StartLock::Delay(lock.delay()?.ok_or("No delay for the lock")?),
        Lock::Text { text } => StartLock::Text(text.clone()),
        // Drawn now, so it cannot be learnt before.
        Lock::RandomText { length, .. } => StartLock::Text(lock::random_text(*length)?),
        Lock::Password { .. } => StartLock::Password(lock.password_hash().ok_or("Set a password for the password lock")?.to_string()),
    })
}

/// Where unlocking stands now.
fn unlock_status(context: &Context) -> UnlockStatus {
    let delay = context.unlock.delay();
    let now = context.sessions.now();
    let remaining = context.unlock.remaining(now);
    let way = context.unlock.way();
    let way_open = way.as_ref().is_some_and(|way| way.is_open(now));
    let way_left = way.as_ref().and_then(|way| way.left(now));
    UnlockStatus {
        delay,
        delay_words: duration::countdown(delay),
        remaining,
        remaining_words: remaining.map(duration::left),
        way: way.map(|way| way.lock.redacted()),
        way_open,
        way_left,
        way_left_words: way_left.map(duration::left),
    }
}

/// Sets the other way to unlock (`unlock::Way`), from a lock method as the user chose it: a condition
/// (its rule looked up now), a text, a random text (drawn now) or a password (only its hash kept).
/// Never the delay, which is always there. Setting one needs unlocking to be ready, or no
/// session to be locked; removing it is always free.
fn set_unlock_way(context: &Context, lock: Option<Lock>) -> Result<(), String> {
    let _editing = context.editing.locked();
    let now = context.sessions.now();
    let idle = context.sessions.locked().is_none();
    let Some(lock) = lock.filter(|lock| !matches!(lock, Lock::None)) else {
        return context.unlock.set_way(None, now, idle);
    };
    lock.check()?;
    let (lock, opening) = match lock {
        Lock::Condition { ref condition } => {
            let rule = context.definitions.library().rule_of(condition)?;
            let opening = unlock::Opening::While { rule, zone: Zone::current().name().to_string() };
            (lock, Some(opening))
        }
        Lock::Text { .. } => (lock, None),
        // Its text is drawn now, and again after each unlock (`unlock_with`).
        Lock::RandomText { length, .. } => (Lock::RandomText { length, text: Some(lock::random_text(length)?) }, None),
        // Only its hash is kept; a password lock without a new password keeps the one it has.
        // As a session's: a hash from a client is not taken, only the one kept from before.
        Lock::Password { password, .. } => {
            let hash = match password {
                Some(password) if password.is_empty() => return Err("The password to unlock with cannot be empty".into()),
                Some(password) => unlock::new_hash(&password)?,
                None => match context.unlock.way().map(|way| way.lock) {
                    Some(Lock::Password { hash: Some(kept), .. }) => kept,
                    _ => return Err("Set a password for this way to unlock".into()),
                },
            };
            (Lock::Password { password: None, hash: Some(hash) }, None)
        }
        _ => {
            return Err("The way to unlock can be a condition, a custom text, a random text or a password; the delay is always there".into())
        }
    };
    context.unlock.set_way(Some(unlock::Way { lock, opening }), now, idle)
}

/// Sets how long unlocking takes, from text.
fn set_unlock_delay(context: &Context, text: &str) -> Result<(), String> {
    let seconds = unlock::parse_delay(text)?;
    let now = context.sessions.now();
    // With no session locked there is nothing to get around, and nothing to unlock.
    if context.sessions.locked().is_some() {
        context.unlock.set_delay(seconds, now)
    } else {
        context.unlock.set_delay_idle(seconds, now)
    }
}

/// Ends every session, once unlocking is ready: always with no delay, else when the countdown is
/// over. The app stays: it has nothing to enforce then. **The escape hatch**: nothing but the
/// countdown stands in its way, whatever the sessions are locked with. Unlocking uses up the
/// request: the next time starts again.
fn unlock(context: &Context) -> Result<(), String> {
    unlock_with(context, None, None)
}

/// As `unlock`, and ready also when the other way to unlock lets go (`unlock::Way`: at once, or an
/// answer given), **or** the countdown is over: either is enough, never both.
fn unlock_with(context: &Context, password: Option<&str>, text: Option<&str>) -> Result<(), String> {
    let now = context.sessions.now();
    let by_way = context.unlock.way().is_some_and(|way| way.is_open(now) || way.answered(password, text));
    if !by_way && context.unlock.remaining(now) != Some(0) {
        return Err("Unlocking is not ready yet: ask for it and wait for the countdown, or use the other way to unlock".into());
    }
    context.sessions.reset();
    // A random text is used up: the next unlock has a new one. (Unlocked either way: if none can be
    // drawn or kept, the old one stays.)
    if let Some(unlock::Way { lock: Lock::RandomText { length, .. }, opening }) = context.unlock.way() {
        let redrawn = lock::random_text(length).and_then(|text| {
            context.unlock.set_way(Some(unlock::Way { lock: Lock::RandomText { length, text: Some(text) }, opening }), now, true)
        });
        if let Err(e) = redrawn {
            eprintln!("unlock: the random text stays as it was: {e}");
        }
    }
    context.unlock.abort()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tests::{session, target, weekly};
    use crate::definitions::{Condition, Definition, Kind, Target};
    use crate::state::{Actions, Store};
    use std::fs;
    use std::time::Duration;

    fn context(name: &str) -> Context {
        let dir = std::env::temp_dir().join(format!("service-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let sessions = Sessions::with_store(Store::new(dir.join("state.json")));
        Context::new(sessions, Definitions::at(Some(dir.join("sessions.json"))), Arc::new(AppSlot::new(false)))
    }

    /// What most tests need: a session with one target of its own (both with the session's id) and,
    /// maybe, a condition of its own.
    #[derive(Clone)]
    struct One {
        list: Target,
        session: Definition,
        condition: Option<Condition>,
    }

    impl One {
        fn actions(mut self, actions: Actions) -> One {
            self.list.actions = actions;
            self
        }
        fn allowed(mut self, allowed: &[&str]) -> One {
            self.list.entries.allowed = allowed.iter().map(|a| a.to_string()).collect();
            self
        }
        fn programs(mut self, programs: &[&str]) -> One {
            self.list.entries.programs = programs.iter().map(|p| p.to_string()).collect();
            self
        }
        fn lock(mut self, lock: Lock) -> One {
            self.session.lock = lock;
            self
        }
        fn week(mut self, text: &str) -> One {
            let condition = weekly(&format!("{}-schedule", self.session.id), text);
            self.session.condition = Some(condition.id.clone());
            self.condition = Some(condition);
            self
        }
    }

    /// A session of one list with these sites and every action, always active, not locked.
    fn definition(id: &str, filters: &[&str]) -> One {
        One { list: target(id, filters), session: session(id, &[id]), condition: None }
    }

    fn timed(id: &str, filters: &[&str], duration: &str) -> One {
        definition(id, filters).lock(Lock::timed_for(duration))
    }

    /// Saves the list, the condition and the session; the first answer that is not Done, or Done.
    fn save(context: &Context, one: One) -> Response {
        let mut requests = vec![Request::SaveTarget { target: one.list }];
        requests.extend(one.condition.map(|condition| Request::SaveCondition { condition }));
        requests.push(Request::SaveSession { session: one.session });
        requests.into_iter().map(|r| handle(context, 0, r).0).find(|r| *r != Response::Done).unwrap_or(Response::Done)
    }

    /// Saves it all and starts the session.
    fn run(context: &Context, one: One) -> Response {
        let id = one.session.id.clone();
        assert_eq!(save(context, one), Response::Done);
        handle(context, 0, Request::Start { id }).0
    }

    fn policies(context: &Context) -> Vec<Policy> {
        let Response::Policy(policies) = handle(context, 0, Request::Policy).0 else { panic!("no policy") };
        policies
    }

    fn overview(context: &Context) -> Overview {
        let Response::Status(overview) = handle(context, 0, Request::Status).0 else { panic!("no status") };
        overview
    }

    fn failed(response: Response) -> bool {
        matches!(response, Response::Failed(_))
    }

    #[test]
    fn requests_are_checked_again_by_the_service() {
        let context = context("checked");
        // Empty lists, and no action that goes without one: nothing to do.
        let only_the_lists = Actions { obsidian_web_viewer: false, window_titles_minimize: false };
        assert!(failed(run(&context, definition("a", &[]).actions(only_the_lists))));
        // A bad duration, and an address that cannot match, are not even saved.
        for one in [timed("c", &["a.com"], "soon"), definition("d", &["has space.com"])] {
            assert!(failed(save(&context, one)));
        }
        // A session with no target, and one that is not defined.
        assert_eq!(handle(&context, 0, Request::SaveSession { session: session("e", &[]) }).0, Response::Done);
        assert!(failed(handle(&context, 0, Request::Start { id: "e".into() }).0));
        assert!(failed(handle(&context, 0, Request::Start { id: "nope".into() }).0));
        assert!(policies(&context).is_empty());
        assert_eq!(overview(&context).sessions.len(), 2);
    }

    #[test]
    fn an_empty_list_starts_a_session_when_an_action_without_the_list_is_on() {
        let context = context("empty-list");
        let obsidian_only = Actions { obsidian_web_viewer: true, window_titles_minimize: false };
        assert_eq!(run(&context, definition("a", &[]).actions(obsidian_only)), Response::Done);
        // The session keeps what it was started with: no entries, and Obsidian's action only.
        let policy = &policies(&context)[0];
        assert!(policy.entries.filters.is_empty());
        assert_eq!(policy.actions, obsidian_only);

        // With all actions on (the default), an empty list is fine too: Obsidian's can run.
        assert_eq!(run(&context, definition("b", &[])), Response::Done);
    }

    #[test]
    fn sessions_are_defined_started_and_stopped_on_their_own() {
        let context = context("several");
        assert_eq!(run(&context, timed("work", &["a.com"], "1h")), Response::Done);
        assert_eq!(run(&context, definition("evening", &["b.com"])), Response::Done);
        assert_eq!(save(&context, definition("idle", &["c.com"])), Response::Done);
        let overview = overview(&context);
        assert_eq!(overview.sessions.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["evening", "idle", "work"]);
        assert_eq!(overview.running.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["work", "evening"]);
        assert_eq!(policies(&context).len(), 2);

        // A running session cannot be changed, deleted or started again; an idle one can.
        assert!(failed(handle(&context, 0, Request::SaveSession { session: session("work", &["idle"]) }).0));
        assert!(failed(handle(&context, 0, Request::DeleteSession { id: "work".into() }).0));
        assert!(failed(handle(&context, 0, Request::Start { id: "work".into() }).0));
        assert_eq!(handle(&context, 0, Request::DeleteSession { id: "idle".into() }).0, Response::Done);

        // The unlocked one stops, and can then be changed; the timed one cannot stop.
        assert_eq!(handle(&context, 0, Request::Stop { id: "evening".into(), password: None, text: None }).0, Response::Done);
        assert!(failed(handle(&context, 0, Request::Stop { id: "work".into(), password: None, text: None }).0));
        assert_eq!(save(&context, definition("evening", &["d.com"])), Response::Done);
        assert_eq!(policies(&context).iter().map(|p| p.entries.filters.clone()).collect::<Vec<_>>(), vec![vec!["a.com".to_string()]]);
    }

    #[test]
    fn a_target_or_condition_an_unlocked_running_session_uses_can_change_and_goes_to_it() {
        let context = context("shared");
        // Two sessions share the list "social"; one also has a list of its own and a schedule.
        assert_eq!(handle(&context, 0, Request::SaveTarget { target: target("social", &["youtube.com"]) }).0, Response::Done);
        assert_eq!(save(&context, definition("games", &[]).programs(&["steam"]).week("daily")), Response::Done);
        let mut evening = session("evening", &["social", "games"]);
        evening.condition = Some("games-schedule".into());
        assert_eq!(handle(&context, 0, Request::SaveSession { session: evening }).0, Response::Done);
        assert_eq!(handle(&context, 0, Request::SaveSession { session: session("work", &["social"]) }).0, Response::Done);
        assert_eq!(handle(&context, 0, Request::Start { id: "evening".into() }).0, Response::Done);
        // Both lists end up in what the app enforces.
        assert_eq!(policies(&context).len(), 2);

        // It is not locked, so what it uses can be changed, in either direction, and the running
        // session gets the change.
        assert_eq!(handle(&context, 0, Request::SaveTarget { target: target("social", &["reddit.com"]) }).0, Response::Done);
        let filters: Vec<_> = policies(&context).iter().flat_map(|p| p.entries.filters.clone()).collect();
        assert_eq!(filters, vec!["reddit.com".to_string()]);
        assert_eq!(handle(&context, 0, Request::SaveCondition { condition: weekly("games-schedule", "weekends") }).0, Response::Done);
        assert!(overview(&context).running[0].condition.as_ref().is_some_and(|rule| rule.week().unwrap().days[0].is_empty()));
        // So can deleting it: the running session keeps what is left, and is always active once its
        // condition is gone.
        assert_eq!(handle(&context, 0, Request::DeleteCondition { id: "games-schedule".into() }).0, Response::Done);
        assert!(overview(&context).running[0].condition.is_none());
        assert_eq!(handle(&context, 0, Request::DeleteTarget { id: "games".into() }).0, Response::Done);
        assert_eq!(overview(&context).running[0].targets.len(), 1);
        // Deleting its last target leaves the session running with nothing to do.
        assert_eq!(handle(&context, 0, Request::DeleteTarget { id: "social".into() }).0, Response::Done);
        assert!(overview(&context).running[0].targets.is_empty() && overview(&context).sessions[0].targets.is_empty());
    }

    #[test]
    fn an_allowance_is_set_on_the_session_and_only_gets_stricter_while_it_is_locked() {
        use crate::allowance::{Anchor, Reset, Scope, Terms};
        let context = context("allowance");
        let budget = |allowed: &str| {
            Some(Terms::Budget {
                allowed: allowed.into(),
                scope: Scope::Total,
                reset: Reset::Every,
                every: "1d".into(),
                anchor: Anchor::Clock,
            })
        };
        let with = |id: &str, allowance: Option<Terms>| {
            let mut one = definition(id, &["a.com"]).lock(Lock::timed_for("1h"));
            one.session.allowance = allowance;
            one
        };
        // Terms that cannot be read are refused with the session.
        assert!(failed(save(&context, with("w", budget("soon")))));
        assert_eq!(run(&context, with("w", budget("30m"))), Response::Done);
        // Kept checked, as it was given.
        assert_eq!(overview(&context).sessions[0].allowance, budget("30m"));

        // With time left the site is not enforced but watched, and the status says so.
        let policy = &policies(&context)[0];
        assert!(policy.entries.filters.is_empty());
        assert_eq!(policy.free.len(), 1);
        assert_eq!(policy.free[0].entries.filters, vec!["a.com"]);
        assert!(overview(&context).running[0].allowance.is_some());

        // The session is locked: its allowance can get stricter (and the running session follows), not looser.
        let saved = |one: One| handle(&context, 0, Request::SaveSession { session: one.session }).0;
        assert!(failed(saved(with("w", budget("1h")))));
        let pomodoro = Terms::Pomodoro { blocked: "50m".into(), allowed: "10m".into(), anchor: crate::allowance::Anchor::Start };
        assert!(failed(saved(with("w", Some(pomodoro)))));
        assert_eq!(saved(with("w", budget("10m"))), Response::Done);
        assert!(overview(&context).running[0].allowance.as_ref().is_some_and(|status| status.left.is_some_and(|left| left <= 600)));
        // Taking it away is stricter too; giving it back is not.
        assert_eq!(saved(with("w", None)), Response::Done);
        assert!(overview(&context).running[0].allowance.is_none() && policies(&context)[0].free.is_empty());
        assert!(failed(saved(with("w", budget("10m")))));

        // An unlocked running session takes any allowance, at once.
        assert_eq!(handle(&context, 0, Request::Unlock).0, Response::Done);
        assert_eq!(run(&context, definition("o", &["b.com"])), Response::Done);
        assert!(overview(&context).running[0].allowance.is_none());
        let mut open = definition("o", &["b.com"]);
        open.session.allowance = budget("2h");
        assert_eq!(saved(open), Response::Done);
        assert!(overview(&context).running[0].allowance.is_some());
    }

    #[test]
    fn what_a_locked_session_uses_can_only_get_stricter() {
        let context = context("stricter");
        assert_eq!(handle(&context, 0, Request::SaveTarget { target: target("social", &["youtube.com"]) }).0, Response::Done);
        assert_eq!(handle(&context, 0, Request::SaveCondition { condition: weekly("office", "mon-fri 8-17") }).0, Response::Done);
        let mut work = session("work", &["social"]);
        work.condition = Some("office".into());
        work.lock = Lock::timed_for("1h");
        assert_eq!(handle(&context, 0, Request::SaveSession { session: work }).0, Response::Done);
        assert_eq!(handle(&context, 0, Request::Start { id: "work".into() }).0, Response::Done);

        // Less to block, another site instead, Obsidian's action off, an allowed site added: all refused.
        assert!(failed(handle(&context, 0, Request::SaveTarget { target: target("social", &[]) }).0));
        assert!(failed(handle(&context, 0, Request::SaveTarget { target: target("social", &["reddit.com"]) }).0));
        let mut loose = target("social", &["youtube.com"]);
        loose.actions.obsidian_web_viewer = false;
        assert!(failed(handle(&context, 0, Request::SaveTarget { target: loose }).0));
        let mut allow = target("social", &["youtube.com"]);
        allow.entries.allowed = vec!["google.com".into()];
        assert!(failed(handle(&context, 0, Request::SaveTarget { target: allow }).0));
        let mut allow_title = target("social", &["youtube.com"]);
        allow_title.entries.allowed_titles = vec!["Notes".into()];
        assert_eq!(
            handle(&context, 0, Request::SaveTarget { target: allow_title }).0,
            Response::Failed(
                "A locked session uses this target, so always-allowed window titles can be taken out but not added: Notes".into()
            )
        );
        // Fewer times, a schedule made something else: refused.
        assert!(failed(handle(&context, 0, Request::SaveCondition { condition: weekly("office", "mon-fri 9-17") }).0));

        // More to block, and more times, are fine, and go to the running session.
        assert_eq!(handle(&context, 0, Request::SaveTarget { target: target("social", &["youtube.com", "reddit.com"]) }).0, Response::Done);
        let mut filters = overview(&context).running[0].targets[0].entries.filters.clone();
        filters.sort();
        assert_eq!(filters, vec!["reddit.com".to_string(), "youtube.com".to_string()]);
        assert_eq!(handle(&context, 0, Request::SaveCondition { condition: weekly("office", "mon-sat 8-17") }).0, Response::Done);
        assert!(overview(&context).running[0].condition.as_ref().is_some_and(|rule| !rule.week().unwrap().days[5].is_empty()));
        // And what it uses cannot be deleted.
        assert!(failed(handle(&context, 0, Request::DeleteTarget { id: "social".into() }).0));
    }

    #[test]
    fn a_combination_is_a_condition_and_what_it_holds_follows_the_rules_of_what_uses_it() {
        use crate::condition::{Mode, Rule};
        let context = context("combination");
        let combination = |id: &str, mode: Mode, members: &[&str]| Condition {
            id: id.into(),
            name: format!("Combination {id}"),
            rule: Rule::Combined { mode, members: members.iter().map(|m| m.to_string()).collect(), rules: Vec::new() },
        };
        let save = |condition: Condition| handle(&context, 0, Request::SaveCondition { condition }).0;
        assert_eq!(save(weekly("daily", "daily")), Response::Done);
        assert_eq!(save(weekly("sat", "sat")), Response::Done);
        assert_eq!(save(combination("either", Mode::Any, &["daily", "sat"])), Response::Done);
        assert_eq!(save(combination("both", Mode::All, &["daily", "sat"])), Response::Done);
        // Not of nothing, of one, of a combination, or of itself.
        assert!(failed(save(combination("x", Mode::Any, &["daily", "nope"]))));
        assert!(failed(save(combination("x", Mode::Any, &["daily"]))));
        assert!(failed(save(combination("x", Mode::Any, &["daily", "either"]))));
        assert!(failed(save(combination("x", Mode::Any, &["x", "sat"]))));

        // Two locked sessions, one by each combination, copy what it holds when they start.
        assert_eq!(handle(&context, 0, Request::SaveTarget { target: target("social", &["youtube.com"]) }).0, Response::Done);
        for (id, condition) in [("w", "either"), ("v", "both")] {
            let mut session = session(id, &["social"]);
            session.condition = Some(condition.into());
            session.lock = Lock::timed_for("1h");
            assert_eq!(handle(&context, 0, Request::SaveSession { session }).0, Response::Done);
            assert_eq!(handle(&context, 0, Request::Start { id: id.into() }).0, Response::Done);
        }
        let running = overview(&context).running;
        assert!(running.iter().all(|r| matches!(&r.condition, Some(Rule::Combined { rules, .. }) if rules.len() == 2)));

        // A condition in an "any" can get more times, not fewer; in an "all", it cannot change.
        assert!(failed(save(weekly("sat", "sun"))));
        assert!(failed(save(weekly("sat", "sat; sun"))));
        // The combination itself can only get more active: "either" gets one more, "both" one less.
        assert_eq!(save(weekly("sun", "sun")), Response::Done);
        assert_eq!(save(combination("either", Mode::Any, &["daily", "sat", "sun"])), Response::Done);
        assert!(failed(save(combination("either", Mode::Any, &["daily"]))));
        assert!(failed(save(combination("both", Mode::All, &["daily", "sat", "sun"]))));
        assert_eq!(save(combination("both", Mode::All, &["daily", "sat"])), Response::Done);
        // What goes to the running session is what it holds now.
        let running = overview(&context).running;
        let either = running.iter().find(|r| r.id == "w").unwrap();
        assert!(matches!(&either.condition, Some(Rule::Combined { rules, .. }) if rules.len() == 3));
    }

    #[test]
    fn a_running_session_that_is_not_locked_can_be_changed_but_not_given_a_lock_by_saving() {
        let context = context("edit-running");
        assert_eq!(run(&context, definition("a", &["a.com"])), Response::Done);
        assert_eq!(handle(&context, 0, Request::SaveTarget { target: target("b", &["b.com"]) }).0, Response::Done);
        // Its name and targets can change, and the running session follows.
        let mut changed = session("a", &["b"]);
        changed.name = "Renamed".into();
        assert_eq!(handle(&context, 0, Request::SaveSession { session: changed }).0, Response::Done);
        let running = &overview(&context).running[0];
        assert_eq!(running.name, "Renamed");
        assert_eq!(running.targets[0].entries.filters, vec!["b.com".to_string()]);
        // A lock is not given to a session that runs by saving it (`Request::Lock` does that), and it
        // cannot be left with nothing to block.
        let mut locked = session("a", &["b"]);
        locked.lock = Lock::timed_for("1h");
        assert!(failed(handle(&context, 0, Request::SaveSession { session: locked }).0));
        assert!(failed(handle(&context, 0, Request::SaveSession { session: session("a", &[]) }).0));
        // Deleting it ends it.
        assert_eq!(handle(&context, 0, Request::DeleteSession { id: "a".into() }).0, Response::Done);
        assert!(overview(&context).running.is_empty() && overview(&context).sessions.is_empty());
    }

    #[test]
    fn the_status_has_what_each_sessions_rows_say() {
        let context = context("rows");
        // Stopped: what is set; running: what is happening (`status.rs` tests each wording).
        assert_eq!(save(&context, definition("a", &["a.com"])), Response::Done);
        let row = |context: &Context| overview(context).status.into_iter().find(|s| s.id == "a").expect("its rows");
        assert_eq!((row(&context).condition.text, row(&context).lock.text), ("At all times".to_string(), "Not locked".to_string()));
        assert_eq!(handle(&context, 0, Request::Start { id: "a".into() }).0, Response::Done);
        assert_eq!(row(&context).condition.text, "Active");
    }

    #[test]
    fn a_running_session_that_is_not_locked_can_be_locked_now() {
        let context = context("lock-now");
        let lock = |id: &str, lock: Lock| handle(&context, 0, Request::Lock { id: id.into(), lock }).0;
        let hour = || Lock::timed_for("1h");
        // Only a session that runs: one that does not has its lock chosen for its next start.
        assert_eq!(save(&context, definition("a", &["a.com"])), Response::Done);
        assert_eq!(lock("a", hour()), Response::Failed("This session is not running: choose its lock for the next start instead".into()));
        assert_eq!(handle(&context, 0, Request::Start { id: "a".into() }).0, Response::Done);
        // A lock that does not read is refused, and nothing changes.
        assert!(failed(lock("a", Lock::timed_for("soon"))));
        assert_eq!(overview(&context).sessions[0].lock, Lock::None);
        // A date that has passed: refused, and the session keeps the lock it had, also as defined.
        assert!(failed(lock("a", Lock::timed_until("2000-01-01 00:00"))));
        assert_eq!(overview(&context).sessions[0].lock, Lock::None);
        assert!(!overview(&context).running[0].locked);
        // Locked now, for an hour from now, and that is its lock for the next start too.
        assert_eq!(lock("a", hour()), Response::Done);
        let status = overview(&context);
        assert!(status.running[0].locked && status.running[0].remaining.is_some_and(|left| (3595..=3600).contains(&left)));
        assert_eq!(status.sessions[0].lock, hour());
        assert!(failed(handle(&context, 0, Request::Stop { id: "a".into(), password: None, text: None }).0));
        // Locked, so its lock stays, but it may end later, which is then its lock for the next start.
        assert_eq!(
            lock("a", Lock::None),
            Response::Failed(
                "This session is locked, so its lock cannot be changed until it can be stopped, except to end later or to wait longer"
                    .into()
            )
        );
        assert_eq!(overview(&context).sessions[0].lock, hour());
        let shorter = Lock::timed_for("30m");
        assert_eq!(lock("a", shorter), Response::Failed("This session is locked, so its lock can only end later".into()));
        assert_eq!(overview(&context).sessions[0].lock, hour());
        let longer = Lock::timed_for("2h");
        assert_eq!(lock("a", longer.clone()), Response::Done);
        assert_eq!(overview(&context).sessions[0].lock, longer);
        assert!(overview(&context).running[0].remaining.is_some_and(|left| left > 7000));
    }

    #[test]
    fn a_password_lock_given_to_a_running_session_is_hashed_and_stops_it() {
        let context = context("lock-now-password");
        assert_eq!(run(&context, definition("a", &["a.com"])), Response::Done);
        let password = Lock::Password { password: Some("open sesame".into()), hash: None };
        assert_eq!(handle(&context, 0, Request::Lock { id: "a".into(), lock: password }).0, Response::Done);
        let status = overview(&context);
        assert!(status.running[0].locked && status.running[0].password);
        // Never sent back.
        assert_eq!(status.sessions[0].lock, Lock::Password { password: None, hash: None });
        let stop = |password: &str| handle(&context, 0, Request::Stop { id: "a".into(), password: Some(password.into()), text: None }).0;
        assert!(failed(stop("wrong")));
        assert_eq!(stop("open sesame"), Response::Done);
    }

    #[test]
    fn a_session_whose_lock_condition_is_off_can_take_another_lock() {
        let context = context("lock-now-free");
        run_free(&context);
        // Off, so it could be stopped and started again with any lock: none too.
        assert_eq!(handle(&context, 0, Request::Lock { id: "w".into(), lock: Lock::None }).0, Response::Done);
        assert_eq!(overview(&context).sessions[0].lock, Lock::None);
        assert_eq!(handle(&context, 0, Request::Stop { id: "w".into(), password: None, text: None }).0, Response::Done);
    }

    /// A schedule that is not on right now by the clock the target goes by (UTC): one span of ten
    /// minutes today, well away from now; `later` puts it somewhere else, equally away from now.
    fn quiet_now(context: &Context, id: &str, later: bool) -> Condition {
        use crate::condition::Rule;
        use crate::localtime::Zone;
        use crate::schedule::{Span, Week};
        let now = Zone::fixed(0).local(context.sessions.now()).unwrap();
        let start = (if now.minute < 700 { 800 } else { 100 }) + (if later { 200 } else { 0 });
        let mut week = Week::default();
        week.days[now.weekday] = vec![Span(start, start + 10)];
        Condition { id: id.into(), name: format!("Quiet {id}"), rule: Rule::Schedule { week } }
    }

    /// A session locked by the condition `gate`, which is not on now: it runs, and is free.
    fn run_free(context: &Context) {
        assert_eq!(handle(context, 0, Request::SaveTarget { target: target("social", &["youtube.com"]) }).0, Response::Done);
        assert_eq!(handle(context, 0, Request::SaveCondition { condition: quiet_now(context, "gate", false) }).0, Response::Done);
        let mut work = session("w", &["social"]);
        work.lock = Lock::Condition { condition: "gate".into() };
        assert_eq!(handle(context, 0, Request::SaveSession { session: work }).0, Response::Done);
        assert_eq!(handle(context, 0, Request::Start { id: "w".into() }).0, Response::Done);
        assert!(!overview(context).running[0].locked);
    }

    #[test]
    fn a_lock_condition_that_is_off_can_be_changed_and_one_that_locks_can_only_get_stricter() {
        let context = context("free-lock");
        run_free(&context);
        // Off, so the session could be stopped now: the condition may change in any direction.
        assert_eq!(handle(&context, 0, Request::SaveCondition { condition: quiet_now(&context, "gate", true) }).0, Response::Done);
        assert!(!overview(&context).running[0].locked);
        // Switched on, it locks the session at once, and from then on can only get more times.
        assert_eq!(handle(&context, 0, Request::SaveCondition { condition: weekly("gate", "daily") }).0, Response::Done);
        assert!(overview(&context).running[0].locked);
        assert!(failed(handle(&context, 0, Request::SaveCondition { condition: quiet_now(&context, "gate", false) }).0));
        // And nothing locked is deleted, nor ended by an import.
        assert!(failed(handle(&context, 0, Request::DeleteCondition { id: "gate".into() }).0));
        assert!(failed(handle(&context, 0, Request::DeleteSession { id: "w".into() }).0));
        assert!(failed(handle(&context, 0, Request::DeleteTarget { id: "social".into() }).0));
        let library = overview(&context).library();
        assert!(failed(handle(&context, 0, Request::Import { library }).0));
        assert_eq!(overview(&context).running.len(), 1);
    }

    #[test]
    fn deleting_the_lock_condition_of_a_session_that_is_free_leaves_it_with_no_lock() {
        let context = context("free-lock-deleted");
        run_free(&context);
        assert_eq!(handle(&context, 0, Request::DeleteCondition { id: "gate".into() }).0, Response::Done);
        let running = &overview(&context).running[0];
        assert!(!running.locked && running.lock_condition.is_none());
        assert_eq!(overview(&context).sessions[0].lock, Lock::None);
    }

    #[test]
    fn with_no_session_locked_the_delay_can_be_shortened_freely() {
        let context = context("hatch-shorten-unlocked");
        let ask = |request: Request| handle(&context, 0, request).0;
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1h".into() }), Response::Done);
        // A session that is running but not locked is nothing to get around.
        assert_eq!(run(&context, definition("a", &["a.com"])), Response::Done);
        assert_eq!(ask(Request::SetUnlockDelay { delay: "10m".into() }), Response::Done);
    }

    #[test]
    fn only_the_installed_app_is_taken_in_as_the_app() {
        use std::process::{Command, Stdio};
        let mut context = context("register");
        context.slot = Arc::new(AppSlot::new(true));
        // A process that is not the installed exe, whatever it calls itself.
        let mut stranger = Command::new("ping.exe").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();

        let (response, _) = handle(&context, stranger.id(), Request::Register);
        assert!(matches!(response, Response::Refused(why) if why.contains("not the installed")));
        assert!(matches!(handle(&context, 0, Request::Register).0, Response::Refused(_)));
        // It may ask for the policy all the same; that is just information, and counts for nothing.
        assert_eq!(handle(&context, stranger.id(), Request::Policy).0, Response::Policy(Vec::new()));
        assert!(context.slot.supervise(true, false), "no app should have been taken in");

        // This test process is the installed exe as far as the service can tell. Only if it runs
        // in the console session is it taken in; elsewhere an app just runs unwatched.
        let (response, _) = handle(&context, std::process::id(), Request::Register);
        assert_eq!(response, Response::Done);
        // A second app is refused while the first one is there.
        let (response, _) = handle(&context, stranger.id(), Request::Register);
        assert!(matches!(response, Response::Refused(_)));

        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    #[test]
    fn programs_alone_can_start_a_session_and_what_windows_needs_is_refused() {
        let programs_only = Actions { obsidian_web_viewer: false, window_titles_minimize: false };
        let one = |programs: &[&str]| timed("p", &[], "1h").actions(programs_only).programs(programs);
        let context = context("programs");

        // The programs action alone, with nothing listed, has nothing to do.
        assert!(failed(run(&context, one(&[]))));
        // Entries that would close what Windows or this app needs, or are not names.
        for bad in ["*", "explorer.exe", "s*", r"C:\Games\game.exe"] {
            let Response::Failed(why) = save(&context, one(&[bad])) else { panic!("{bad} was accepted") };
            assert!(why.contains("needs") || why.contains("Not a program name"), "{bad}: {why}");
        }
        assert!(policies(&context).is_empty());

        // A real one starts, and is what the app is told to close.
        assert_eq!(run(&context, one(&["Steam*", "game"])), Response::Done);
        let policy = &policies(&context)[0];
        assert_eq!(policy.entries.programs, vec!["game.exe", "steam*.exe"]);
        assert!(policy.entries.filters.is_empty());
        assert_eq!(policy.actions, programs_only);
    }

    #[test]
    fn allowed_sites_travel_with_the_session_and_must_be_valid() {
        let context = context("allowed");
        let one = |allowed: &[&str]| timed("a", &["*"], "1h").allowed(allowed);
        // An allowed entry that cannot match is refused like a bad entry.
        assert!(failed(save(&context, one(&["has space.com"]))));

        // Everything blocked but one site: the app is told about both.
        assert_eq!(run(&context, one(&["Google.com"])), Response::Done);
        let policy = &policies(&context)[0];
        assert_eq!(policy.entries.filters, vec!["*"]);
        assert_eq!(policy.entries.allowed, vec!["google.com"]);
        assert_eq!(overview(&context).running[0].targets[0].entries.allowed, vec!["google.com"]);
    }

    #[test]
    fn allowed_sites_alone_are_not_something_to_block() {
        let context = context("allowed-alone");
        // No sites to block and only an allowed one: nothing to do.
        let tabs_only = Actions { obsidian_web_viewer: false, window_titles_minimize: false };
        assert!(failed(run(&context, timed("a", &[], "1h").allowed(&["google.com"]).actions(tabs_only))));
        assert!(policies(&context).is_empty());
    }

    #[test]
    fn a_timed_session_cannot_be_shortened_stopped_or_uninstalled() {
        let context = context("timed");
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        // Not shortened through its definition, not stopped.
        assert!(failed(handle(&context, 0, Request::SaveSession { session: timed("a", &["a.com"], "1s").session }).0));
        assert!(failed(handle(&context, 0, Request::Stop { id: "a".into(), password: None, text: None }).0));
        // Uninstalling does not get through, and the service keeps running.
        let (response, shutdown) = handle(&context, 0, Request::PrepareUninstall);
        assert!(matches!(response, Response::Refused(_)) && shutdown.is_none());
        // The policy is what the app enforces.
        assert_eq!(policies(&context)[0].entries.filters, vec!["a.com"]);
    }

    #[test]
    fn a_lock_schedule_locks_while_it_is_active() {
        let context = context("lock-schedule");
        // A lock schedule that does not exist is refused.
        assert!(failed(save(&context, definition("a", &["a.com"]).lock(Lock::Condition { condition: "nope".into() }))));
        // Its own schedule, active all week, so locked now while the session's schedule is Always:
        // no stop, no uninstall.
        assert_eq!(handle(&context, 0, Request::SaveCondition { condition: weekly("daily", "daily") }).0, Response::Done);
        assert_eq!(run(&context, definition("a", &["a.com"]).lock(Lock::Condition { condition: "daily".into() })), Response::Done);
        let Response::Failed(why) = handle(&context, 0, Request::Stop { id: "a".into(), password: None, text: None }).0 else {
            panic!("stopped")
        };
        assert!(why.contains("locked by its lock schedule"), "{why}");
        assert!(matches!(handle(&context, 0, Request::PrepareUninstall).0, Response::Refused(_)));
        let running = &overview(&context).running[0];
        assert!(running.locked && running.lock_condition.is_some() && running.condition.is_none() && running.remaining.is_none());
        // While it runs, its lock schedule cannot lose times or be deleted.
        assert!(failed(handle(&context, 0, Request::SaveCondition { condition: weekly("daily", "weekends") }).0));
        assert!(failed(handle(&context, 0, Request::DeleteCondition { id: "daily".into() }).0));
    }

    #[test]
    fn until_a_time_and_until_a_restart_are_locks_too() {
        let context = context("until");
        // A time that is not one is refused when saved; one that has passed when started.
        assert!(failed(save(&context, definition("a", &["a.com"]).lock(Lock::timed_until("tomorrow")))));
        let Response::Failed(why) = run(&context, definition("a", &["a.com"]).lock(Lock::timed_until("2001-01-01 00:00"))) else {
            panic!("started")
        };
        assert!(why.contains("has passed"), "{why}");
        // A time ahead locks like a duration.
        assert_eq!(run(&context, definition("a", &["a.com"]).lock(Lock::timed_until("2999-01-01 00:00"))), Response::Done);
        assert!(overview(&context).running[0].remaining.is_some());
        assert!(failed(handle(&context, 0, Request::Stop { id: "a".into(), password: None, text: None }).0));
        // Until Windows restarts: locked, no stop, no uninstall.
        assert_eq!(run(&context, definition("b", &["b.com"]).lock(Lock::UntilRestart)), Response::Done);
        let b = overview(&context).running.into_iter().find(|r| r.id == "b").unwrap();
        assert!(b.until_restart && b.locked && b.remaining.is_none());
        assert!(failed(handle(&context, 0, Request::Stop { id: "b".into(), password: None, text: None }).0));
        assert!(matches!(handle(&context, 0, Request::PrepareUninstall).0, Response::Refused(_)));
    }

    #[test]
    fn without_a_session_the_installer_gets_through() {
        let context = context("idle");
        let (response, shutdown) = handle(&context, 0, Request::PrepareUninstall);
        assert_eq!(response, Response::Done);
        assert!(shutdown.is_some());
        // An unlocked session does not stop an uninstall either.
        assert_eq!(run(&context, definition("a", &["a.com"])), Response::Done);
        assert!(handle(&context, 0, Request::PrepareUninstall).1.is_some());
    }

    // ---- The escape hatch (`unlock.rs`): it must never break, whatever the sessions are locked with.

    /// Every way of locking a session that a test can start (a restart lock needs Windows' boot id).
    fn every_lock() -> Vec<(&'static str, One)> {
        vec![
            ("duration", timed("d", &["d.com"], "1h")),
            ("until", definition("u", &["u.com"]).lock(Lock::timed_until("2040-06-01 00:00"))),
            ("condition", definition("c", &["c.com"]).week("daily").lock(Lock::Condition { condition: "c-schedule".into() })),
            ("delay", definition("l", &["l.com"]).lock(Lock::Delay { delay: "1h".into() })),
            ("text", definition("t", &["t.com"]).lock(Lock::Text { text: "Write this out".into() })),
            ("password", definition("p", &["p.com"]).lock(Lock::Password { password: Some("secret".into()), hash: None })),
        ]
    }

    #[test]
    fn with_no_delay_unlocking_ends_every_session_whatever_its_lock() {
        let context = context("hatch-free");
        // A clean install: no delay, nothing asked for, ready.
        let status = overview(&context).unlock;
        assert_eq!((status.delay, status.remaining), (0, Some(0)));
        for (kind, one) in every_lock() {
            assert_eq!(run(&context, one), Response::Done, "{kind}");
        }
        assert_eq!(overview(&context).running.len(), 6);
        assert!(overview(&context).running.iter().all(|running| running.locked));
        // Each of them refuses to be stopped...
        for id in ["d", "u", "c", "l", "t", "p"] {
            assert!(failed(handle(&context, 0, Request::Stop { id: id.into(), password: None, text: None }).0), "{id}");
        }
        // ...and unlocking ends them all at once; what was defined stays.
        assert_eq!(handle(&context, 0, Request::Unlock).0, Response::Done);
        assert!(!context.sessions.is_running() && policies(&context).is_empty());
        assert_eq!(overview(&context).sessions.len(), 6);
    }

    #[test]
    fn another_way_to_unlock_works_instead_of_the_countdown_never_as_well() {
        let context = context("hatch-way");
        let ask = |request: Request| handle(&context, 0, request).0;
        let way = |lock: Lock| Request::SetUnlockWay { way: Some(lock) };
        // The delay is always there: it is not a way of its own.
        assert!(failed(ask(way(Lock::Delay { delay: "1h".into() }))));
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1h".into() }), Response::Done);
        // Nothing is locked yet, so one can be set; a password is kept as its hash, never sent back.
        assert_eq!(ask(way(Lock::Password { password: Some("secret".into()), hash: None })), Response::Done);
        let status = overview(&context).unlock;
        assert_eq!(status.way, Some(Lock::Password { password: None, hash: None }));
        assert!(!status.way_open);
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        // Locked, the countdown not asked for: refused, also with the wrong password.
        assert!(failed(ask(Request::Unlock)) && context.sessions.is_running());
        assert!(failed(ask(Request::UnlockWith { password: Some("wrong".into()), text: None })) && context.sessions.is_running());
        assert!(failed(ask(Request::UnlockWith { password: None, text: Some("secret".into()) })) && context.sessions.is_running());
        // The right one ends every session at once, without the countdown.
        assert_eq!(ask(Request::UnlockWith { password: Some("secret".into()), text: None }), Response::Done);
        assert!(!context.sessions.is_running());
    }

    #[test]
    fn a_way_to_unlock_that_is_easier_needs_unlocking_ready_while_something_is_locked() {
        let context = context("hatch-way-locked");
        let ask = |request: Request| handle(&context, 0, request).0;
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1h".into() }), Response::Done);
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        let text = || Request::SetUnlockWay { way: Some(Lock::Text { text: "let me out".into() }) };
        // Locked and not ready: setting a way would be a way around the delay.
        assert!(failed(ask(text())));
        assert_eq!(overview(&context).unlock.way, None);
        // Ready (the countdown asked for and, with a 1 s delay, over): it can be set, and removed at any time.
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1h".into() }), Response::Done);
        assert_eq!(ask(Request::SetUnlockWay { way: None }), Response::Done);
        // The text unlocks, ignoring line breaks.
        let idle = self::context("hatch-way-idle");
        assert_eq!(handle(&idle, 0, Request::SetUnlockDelay { delay: "1h".into() }).0, Response::Done);
        assert_eq!(handle(&idle, 0, text()).0, Response::Done);
        assert_eq!(run(&idle, timed("a", &["a.com"], "1h")), Response::Done);
        assert!(failed(handle(&idle, 0, Request::UnlockWith { password: None, text: Some("let me".into()) }).0));
        assert_eq!(handle(&idle, 0, Request::UnlockWith { password: None, text: Some("let me out".into()) }).0, Response::Done);
        // Removing is free while a session is locked.
        assert_eq!(run(&idle, timed("b", &["b.com"], "1h")), Response::Done);
        assert_eq!(handle(&idle, 0, Request::SetUnlockWay { way: None }).0, Response::Done);
    }

    #[test]
    fn a_random_text_is_drawn_for_a_session_and_for_the_way_to_unlock() {
        let context = context("random-text");
        let ask = |request: Request| handle(&context, 0, request).0;
        let random = |length| Lock::RandomText { length, text: None };
        // A session: drawn when it starts, then stopped by it as by a custom text.
        assert_eq!(run(&context, definition("a", &["a.com"]).lock(random(12))), Response::Done);
        let drawn = overview(&context).running[0].lock_text.clone().unwrap();
        assert!(drawn.len() == 12 && drawn.chars().all(|c| c.is_ascii_alphanumeric()), "{drawn}");
        assert!(failed(ask(Request::Stop { id: "a".into(), password: None, text: Some("wrong".into()) })));
        assert_eq!(ask(Request::Stop { id: "a".into(), password: None, text: Some(drawn.clone()) }), Response::Done);
        // A length out of range is said before anything is saved, for any session or none.
        for id in ["a", ""] {
            let check = Request::Check { check: Check::Lock { id: id.into(), lock: random(0) } };
            assert_eq!(ask(check), Response::Failed("A random text has 1 to 1000 characters".into()));
        }
        let check = Request::Check { check: Check::Lock { id: String::new(), lock: random(30) } };
        assert_eq!(ask(check), Response::Done);
        // A length out of range is refused; a text given with it is not kept.
        assert!(failed(save(&context, definition("b", &["b.com"]).lock(random(0)))));
        assert_eq!(
            save(&context, definition("c", &["c.com"]).lock(Lock::RandomText { length: 4, text: Some("know".into()) })),
            Response::Done
        );
        assert!(overview(&context).sessions.iter().any(|s| s.lock == random(4)));

        // The way to unlock: drawn when it is set, and used up by an unlock.
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1h".into() }), Response::Done);
        assert_eq!(ask(Request::SetUnlockWay { way: Some(random(8)) }), Response::Done);
        let text = || match overview(&context).unlock.way {
            Some(Lock::RandomText { length: 8, text: Some(text) }) => text,
            other => panic!("{other:?}"),
        };
        let first = text();
        assert_eq!(first.len(), 8);
        assert_eq!(run(&context, timed("c", &["c.com"], "1h")), Response::Done);
        assert!(failed(ask(Request::UnlockWith { password: None, text: Some("x".into()) })));
        assert_eq!(ask(Request::UnlockWith { password: None, text: Some(first.clone()) }), Response::Done);
        assert!(!context.sessions.is_running());
        assert_ne!(text(), first, "a new one for the next unlock");
        // Not a way to unlock: a delay, or a lock of no answer.
        assert!(failed(ask(Request::SetUnlockWay { way: Some(Lock::UntilRestart) })));
    }

    #[test]
    fn breaks_are_taken_through_the_service_and_only_get_stricter_while_locked() {
        use crate::allowance::{Anchor, Reset, Terms};
        let context = context("breaks");
        let ask = |request: Request| handle(&context, 0, request).0;
        let breaks = |count, length: &str| Terms::Breaks {
            count,
            length: length.into(),
            reset: Reset::Never,
            every: String::new(),
            anchor: Anchor::Start,
        };
        let mut one = timed("a", &["a.com"], "1h");
        one.session.allowance = Some(breaks(2, "10m"));
        assert_eq!(run(&context, one), Response::Done);
        assert_eq!(ask(Request::TakeBreak { id: "a".into() }), Response::Done);
        assert!(policies(&context).is_empty(), "nothing enforced during a break");
        assert_eq!(ask(Request::EndBreak { id: "a".into() }), Response::Done);
        assert!(!policies(&context).is_empty());
        assert!(failed(ask(Request::EndBreak { id: "a".into() })));
        // Locked: fewer or shorter breaks, or none; not more.
        let with = |terms: Option<Terms>| {
            let mut session = overview(&context).sessions[0].clone();
            session.allowance = terms;
            ask(Request::SaveSession { session })
        };
        assert!(failed(with(Some(breaks(3, "10m")))));
        assert_eq!(with(Some(breaks(2, "5m"))), Response::Done);
        assert_eq!(overview(&context).running[0].allowance.as_ref().and_then(|status| status.breaks_left), Some(1), "what is taken stays");
        assert_eq!(with(None), Response::Done);
        assert!(failed(ask(Request::TakeBreak { id: "a".into() })));
    }

    #[test]
    fn a_condition_as_the_way_to_unlock_is_open_while_it_is_not_active() {
        let context = context("hatch-way-condition");
        let ask = |request: Request| handle(&context, 0, request).0;
        // Never active, in any time zone: two hours that never come together. And always active.
        let mut never = weekly("never", "mon 00:00-01:00");
        never.rule = crate::condition::Rule::Combined {
            mode: crate::condition::Mode::All,
            members: vec!["mon".into(), "tue".into()],
            rules: vec![],
        };
        for condition in
            [weekly("mon", "mon 00:00-01:00"), weekly("tue", "tue 00:00-01:00"), never, weekly("always", "mon-sun 00:00-24:00")]
        {
            assert_eq!(ask(Request::SaveCondition { condition }), Response::Done);
        }
        let way = |condition: &str| Request::SetUnlockWay { way: Some(Lock::Condition { condition: condition.into() }) };
        assert!(failed(ask(way("gone"))), "a condition that is not there");
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1h".into() }), Response::Done);

        // Always active: locked, so the countdown is the way out.
        assert_eq!(ask(way("always")), Response::Done);
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        let status = overview(&context).unlock;
        assert!(!status.way_open && status.way == Some(Lock::Condition { condition: "always".into() }));
        assert!(failed(ask(Request::Unlock)) && context.sessions.is_running());

        // Never active: open, so unlocking needs no countdown and no answer.
        let open = self::context("hatch-way-open");
        let ask = |request: Request| handle(&open, 0, request).0;
        // The conditions it is of first, then the combination.
        let mut conditions = context.definitions.library().conditions;
        conditions.sort_by_key(|c| !c.rule.members().is_empty());
        for condition in conditions {
            assert_eq!(ask(Request::SaveCondition { condition }), Response::Done);
        }
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1h".into() }), Response::Done);
        assert_eq!(ask(way("never")), Response::Done);
        assert_eq!(run(&open, timed("a", &["a.com"], "1h")), Response::Done);
        assert!(overview(&open).unlock.way_open);
        assert_eq!(ask(Request::Unlock), Response::Done);
        assert!(!open.sessions.is_running());
    }

    #[test]
    fn a_password_way_keeps_its_password_when_none_is_given() {
        let context = context("hatch-way-kept");
        let ask = |request: Request| handle(&context, 0, request).0;
        let password = |password: Option<&str>| Request::SetUnlockWay {
            way: Some(Lock::Password { password: password.map(str::to_string), hash: None }),
        };
        // Without one set before, there is nothing to keep; an empty one, or a hash a client made, is no password.
        assert!(failed(ask(password(None))));
        assert_eq!(ask(password(Some(""))), Response::Failed("The password to unlock with cannot be empty".into()));
        let forged = Request::SetUnlockWay { way: Some(Lock::Password { password: None, hash: Some("00$00".into()) }) };
        assert_eq!(ask(forged), Response::Failed("Set a password for this way to unlock".into()));
        assert_eq!(ask(password(Some("secret"))), Response::Done);
        assert_eq!(ask(password(None)), Response::Done);
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        assert_eq!(ask(Request::UnlockWith { password: Some("secret".into()), text: None }), Response::Done);
        // No way at all, or "none", is no way.
        assert_eq!(ask(Request::SetUnlockWay { way: Some(Lock::None) }), Response::Done);
        assert_eq!(overview(&context).unlock.way, None);
    }

    #[test]
    fn what_the_app_reports_is_counted_against_the_session() {
        use crate::allowance::{Anchor, Reset, Scope, Terms};
        use crate::proto::Used;
        let context = context("report");
        let mut one = definition("a", &["a.com"]);
        one.session.allowance = Some(Terms::Budget {
            allowed: "1s".into(),
            scope: Scope::Total,
            reset: Reset::Never,
            every: String::new(),
            anchor: Anchor::Start,
        });
        assert_eq!(run(&context, one), Response::Done);
        assert_eq!(policies(&context)[0].free.len(), 1, "let through while there is time");
        std::thread::sleep(Duration::from_millis(1100));
        let used = |session: &str| Used { session: session.into(), key: crate::allowance::TOTAL.into() };
        assert_eq!(handle(&context, 0, Request::Report { used: vec![used("a"), used("other")] }).0, Response::Done);
        let policy = &policies(&context)[0];
        assert!(policy.free.is_empty() && policy.entries.filters == vec!["a.com"], "used up: enforced again");
    }

    #[test]
    fn a_delay_lock_is_asked_and_aborted_through_the_service() {
        let context = context("delay-requests");
        let ask = |request: Request| handle(&context, 0, request).0;
        assert_eq!(run(&context, definition("a", &["a.com"]).lock(Lock::Delay { delay: "1h".into() })), Response::Done);
        assert!(failed(ask(Request::Stop { id: "a".into(), password: None, text: None })));
        assert_eq!(ask(Request::AskToStop { id: "a".into() }), Response::Done);
        assert!(overview(&context).running[0].stop_remaining.is_some_and(|left| left > 3500));
        assert_eq!(ask(Request::AbortStop { id: "a".into() }), Response::Done);
        assert_eq!(overview(&context).running[0].stop_remaining, None);
        assert!(failed(ask(Request::AskToStop { id: "gone".into() })));
    }

    #[test]
    fn an_export_has_everything_password_hashes_included() {
        let context = context("export");
        assert_eq!(
            save(&context, definition("a", &["a.com"]).lock(Lock::Password { password: Some("pw".into()), hash: None })),
            Response::Done
        );
        let Response::Library(library) = handle(&context, 0, Request::Export).0 else { panic!("no library") };
        assert_eq!(library.targets.len(), 1);
        assert!(matches!(&library.sessions[0].lock, Lock::Password { hash: Some(hash), password: None } if !hash.is_empty()));
        // The status, unlike the export, never has the hash.
        assert_eq!(overview(&context).sessions[0].lock, Lock::Password { password: None, hash: None });
    }

    #[test]
    fn a_locked_session_can_still_be_renamed_and_nothing_else() {
        let context = context("rename-locked");
        let ask = |request: Request| handle(&context, 0, request).0;
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        let library = overview(&context);
        let mut renamed = library.sessions[0].clone();
        renamed.name = "New name".into();
        assert_eq!(ask(Request::SaveSession { session: renamed.clone() }), Response::Done);
        assert_eq!(overview(&context).running[0].name, "New name");
        assert_eq!(overview(&context).sessions[0].name, "New name");
        // Anything more is refused: its lists, its condition.
        let mut more = renamed;
        more.condition = Some("somewhere".into());
        assert!(failed(ask(Request::SaveSession { session: more })));
    }

    #[test]
    fn a_locked_session_takes_more_targets_and_the_condition_always() {
        let context = context("locked-stricter");
        let ask = |session: &Definition| handle(&context, 0, Request::SaveSession { session: session.clone() }).0;
        for list in [target("social", &["youtube.com"]), target("games", &["steam.com"])] {
            assert_eq!(handle(&context, 0, Request::SaveTarget { target: list }).0, Response::Done);
        }
        for condition in [weekly("office", "mon-fri 8-17"), weekly("weekend", "sat-sun 0-24")] {
            assert_eq!(handle(&context, 0, Request::SaveCondition { condition }).0, Response::Done);
        }
        let mut work = session("work", &["social"]);
        work.condition = Some("office".into());
        work.lock = Lock::timed_for("1h");
        assert_eq!(ask(&work), Response::Done);
        assert_eq!(handle(&context, 0, Request::Start { id: "work".into() }).0, Response::Done);
        let locked = || overview(&context).sessions[0].clone();

        // Another condition, a target taken out, or one that does not exist: refused.
        let mut other = locked();
        other.condition = Some("weekend".into());
        assert!(failed(ask(&other)));
        let mut fewer = locked();
        fewer.targets = vec!["games".into()];
        assert!(failed(ask(&fewer)));
        let mut missing = locked();
        missing.targets.push("nowhere".into());
        assert_eq!(ask(&missing), Response::Failed("No such target: nowhere".into()));

        // A target more, and Always: taken, and the running session follows at once.
        let mut more = locked();
        more.targets.push("games".into());
        more.condition = None;
        assert_eq!(ask(&more), Response::Done);
        assert_eq!(locked().targets, ["social", "games"]);
        assert_eq!(locked().condition, None);
        let running = &overview(&context).running[0];
        assert_eq!((running.targets.len(), running.condition.is_none(), running.locked), (2, true, true));
    }

    #[test]
    fn unlocking_with_nothing_running_is_fine_and_can_be_repeated() {
        let context = context("hatch-idle");
        for _ in 0..3 {
            assert_eq!(handle(&context, 0, Request::Unlock).0, Response::Done);
        }
    }

    #[test]
    fn with_a_delay_unlocking_is_a_countdown_that_can_be_aborted() {
        let context = context("hatch-countdown");
        let ask = |request: Request| handle(&context, 0, request).0;
        let status = || overview(&context).unlock;
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        assert_eq!(ask(Request::SetUnlockDelay { delay: "1s".into() }), Response::Done);
        assert_eq!((status().delay, status().remaining), (1, None));
        // Not asked for: refused, and the session stays.
        assert!(failed(ask(Request::Unlock)) && context.sessions.is_running());
        // Asked for: counting down, and still refused.
        assert_eq!(ask(Request::AskForUnlock), Response::Done);
        assert_eq!(status().remaining, Some(1));
        assert!(status().remaining_words.is_some());
        assert!(failed(ask(Request::Unlock)) && context.sessions.is_running());
        // Aborted: back to locked, as if never asked.
        assert_eq!(ask(Request::AbortUnlock), Response::Done);
        assert_eq!(status().remaining, None);
        assert!(failed(ask(Request::Unlock)));
        // Asked again, and waited for: ready.
        assert_eq!(ask(Request::AskForUnlock), Response::Done);
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(status().remaining, Some(0));
        // Ready, it can still be aborted instead: locked again.
        assert_eq!(ask(Request::AbortUnlock), Response::Done);
        assert_eq!(status().remaining, None);
        assert!(failed(ask(Request::Unlock)) && context.sessions.is_running());
        // Or used: every session ends, and the request is used up: the next time starts again.
        assert_eq!(ask(Request::AskForUnlock), Response::Done);
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(ask(Request::Unlock), Response::Done);
        assert!(!context.sessions.is_running());
        assert_eq!(status().remaining, None);
        assert!(failed(ask(Request::Unlock)));
    }

    #[test]
    fn the_delay_cannot_be_shortened_to_get_around_it() {
        let context = context("hatch-shorten");
        let ask = |request: Request| handle(&context, 0, request).0;
        let delay = |text: &str| Request::SetUnlockDelay { delay: text.into() };
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        assert_eq!(ask(delay("1h")), Response::Done);
        // Shorter, none: refused while unlocking is not ready; the session stays locked.
        for shorter in ["0", "none", "", "30m", "59m 59s"] {
            assert!(failed(ask(delay(shorter))), "{shorter:?}");
        }
        assert_eq!(overview(&context).unlock.delay, 3600);
        // Longer is free; past a day, or not a time, is refused.
        assert_eq!(ask(delay("2h")), Response::Done);
        for wrong in ["25h", "2d", "soon", "-1"] {
            assert!(failed(ask(delay(wrong))), "{wrong:?}");
        }
        assert_eq!(overview(&context).unlock.delay, 7200);
        assert_eq!(ask(delay("24h")), Response::Done);
        assert_eq!(overview(&context).unlock.delay, 86_400);
        // Asking for unlocking does not shorten the wait, and unlocking stays refused.
        assert_eq!(ask(Request::AskForUnlock), Response::Done);
        assert!(failed(ask(delay("1h"))) && failed(ask(Request::Unlock)));
        assert!(context.sessions.is_running());
    }

    #[test]
    fn with_nothing_running_the_delay_can_be_shortened_freely() {
        let context = context("hatch-shorten-idle");
        let ask = |request: Request| handle(&context, 0, request).0;
        let delay = |text: &str| Request::SetUnlockDelay { delay: text.into() };
        assert_eq!(ask(delay("1h")), Response::Done);
        assert_eq!(ask(delay("10m")), Response::Done);
        assert_eq!(ask(delay("0")), Response::Done);
        assert_eq!(overview(&context).unlock.delay, 0);
        // As soon as a session runs, the wait is binding again.
        assert_eq!(ask(delay("1h")), Response::Done);
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        assert!(failed(ask(delay("10m"))));
    }

    #[test]
    fn once_unlocking_is_ready_the_delay_can_be_shortened_down_to_none() {
        let context = context("hatch-shorten-ready");
        let ask = |request: Request| handle(&context, 0, request).0;
        let delay = |text: &str| Request::SetUnlockDelay { delay: text.into() };
        assert_eq!(ask(delay("1s")), Response::Done);
        assert_eq!(ask(Request::AskForUnlock), Response::Done);
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(ask(delay("0")), Response::Done);
        let status = overview(&context).unlock;
        assert_eq!((status.delay, status.remaining), (0, Some(0)));
    }

    #[test]
    fn the_countdown_survives_a_restart_and_a_bad_file_opens_the_hatch() {
        let dir = std::env::temp_dir().join(format!("hatch-{}-restart", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("unlock.json");
        // A service starting up over the same file.
        let service = |name: &str| -> Context {
            let sessions = Sessions::with_store(Store::new(dir.join(format!("{name}.json"))));
            Context::new(sessions, Definitions::at(None), Arc::new(AppSlot::new(false))).with_unlock(UnlockDelay::at(Some(file.clone())))
        };
        let first = service("one");
        assert_eq!(handle(&first, 0, Request::SetUnlockDelay { delay: "1s".into() }).0, Response::Done);
        assert_eq!(handle(&first, 0, Request::AskForUnlock).0, Response::Done);
        // A restart: another service over the same file counts on.
        let second = service("two");
        let status = overview(&second).unlock;
        assert_eq!((status.delay, status.remaining), (1, Some(1)));
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(overview(&second).unlock.remaining, Some(0));
        assert_eq!(handle(&second, 0, Request::Unlock).0, Response::Done);
        // Unlocking used the request up, and that is remembered too.
        assert_eq!(overview(&service("three")).unlock.remaining, None);
        // Whatever is wrong with the file, unlocking is open: no delay, ready.
        for garbage in ["", "{ nope", "null", "{“delay”: “soon”}", "{“delay”: -1}"] {
            fs::write(&file, garbage).unwrap();
            let status = overview(&service("garbage")).unlock;
            assert_eq!((status.delay, status.remaining), (0, Some(0)), "{garbage:?}");
        }
        assert_eq!(handle(&service("garbage"), 0, Request::Unlock).0, Response::Done);
    }

    #[test]
    fn the_status_says_where_unlocking_stands_in_words() {
        let context = context("hatch-words");
        let status = overview(&context).unlock;
        assert_eq!((status.delay, status.delay_words.as_str(), status.remaining), (0, "0 seconds", Some(0)));
        assert_eq!(handle(&context, 0, Request::SetUnlockDelay { delay: "1h 30m".into() }).0, Response::Done);
        let status = overview(&context).unlock;
        assert_eq!((status.delay, status.delay_words.as_str(), status.remaining), (5400, "1 hour and 30 minutes", None));
        assert_eq!(handle(&context, 0, Request::AskForUnlock).0, Response::Done);
        let status = overview(&context).unlock;
        assert_eq!((status.remaining, status.remaining_words.as_deref()), (Some(5400), Some("2 hours")));
    }

    #[test]
    fn a_password_lock_is_set_hashed_and_stops_only_with_its_password() {
        let context = context("password");
        let one = definition("p", &["a.com"]).lock(Lock::Password { password: Some("partner".into()), hash: None });
        assert_eq!(run(&context, one), Response::Done);
        // What a client sees has neither the password nor its hash.
        assert_eq!(overview(&context).sessions[0].lock, Lock::Password { password: None, hash: None });
        assert!(overview(&context).running[0].password);
        // Not stopped without it, nor with a wrong one; not uninstalled; then stopped with it.
        let stop = |password: Option<&str>| Request::Stop { id: "p".into(), password: password.map(str::to_string), text: None };
        assert!(failed(handle(&context, 0, stop(None)).0));
        assert!(failed(handle(&context, 0, stop(Some("wrong"))).0));
        assert!(matches!(handle(&context, 0, Request::PrepareUninstall).0, Response::Refused(_)));
        assert_eq!(handle(&context, 0, stop(Some("partner"))).0, Response::Done);
        // Saved again without a new password (renamed, say), it keeps its password.
        let mut renamed = overview(&context).sessions[0].clone();
        renamed.name = "Renamed".into();
        assert_eq!(handle(&context, 0, Request::SaveSession { session: renamed }).0, Response::Done);
        assert_eq!(handle(&context, 0, Request::Start { id: "p".into() }).0, Response::Done);
        assert_eq!(handle(&context, 0, stop(Some("partner"))).0, Response::Done);
    }

    #[test]
    fn a_weekly_session_runs_by_its_schedule_and_messages_say_minutes_and_hours() {
        let context = context("weekly");
        // Every day all day: active now, whatever the day.
        assert_eq!(run(&context, timed("w", &["a.com"], "2h 5m").week("daily")), Response::Done);
        let running = &overview(&context).running[0];
        assert!(running.enforcing && running.condition.is_some());
        assert_eq!(policies(&context).len(), 1);
        let Response::Failed(why) = handle(&context, 0, Request::Stop { id: "w".into(), password: None, text: None }).0 else {
            panic!("stopped")
        };
        // A countdown is one unit, rounded.
        assert!(why.contains("for the next 2 hours,"), "{why}");
        let Response::Refused(why) = handle(&context, 0, Request::PrepareUninstall).0 else { panic!("not refused") };
        assert!(why.contains("2 hours"), "{why}");
        // A weekly schedule with no time in it is never active: not even saved.
        let never = Condition {
            id: "n".into(),
            name: "N".into(),
            rule: crate::condition::Rule::Schedule { week: crate::schedule::Week::default() },
        };
        assert!(
            matches!(handle(&context, 0, Request::SaveCondition { condition: never }).0, Response::Failed(why) if why.contains("never be active"))
        );
    }

    #[test]
    fn after_an_unlock_the_app_stays_and_enforces_nothing() {
        let context = context("unlocked");
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        assert!(matches!(handle(&context, 0, Request::Policy).0, Response::Policy(p) if p.len() == 1));
        assert_eq!(handle(&context, 0, Request::Unlock).0, Response::Done);
        assert_eq!(handle(&context, 0, Request::Policy).0, Response::Policy(Vec::new()));
    }

    #[test]
    fn importing_waits_until_no_session_is_locked_and_ends_the_others() {
        let context = context("import");
        assert_eq!(run(&context, timed("a", &["a.com"], "1h")), Response::Done);
        let library = overview(&context).library();
        assert!(failed(handle(&context, 0, Request::Import { library: library.clone() }).0));
        // A copy of the running session, or of its list, is not running.
        let copy = Request::Duplicate { kind: Kind::Session, id: "a".into(), new_id: "a2".into() };
        assert_eq!(handle(&context, 0, copy).0, Response::Done);
        let list = library.sessions[0].targets[0].clone();
        assert_eq!(handle(&context, 0, Request::Duplicate { kind: Kind::Target, id: list, new_id: "l2".into() }).0, Response::Done);
        assert_eq!(overview(&context).running.len(), 1);

        // A session that runs without a lock does not stand in the way, and ends with the import.
        let context = self::context("import-unlocked");
        assert_eq!(run(&context, definition("a", &["a.com"])), Response::Done);
        let library = overview(&context).library();
        assert_eq!(handle(&context, 0, Request::Import { library }).0, Response::Done);
        assert!(overview(&context).running.is_empty());
        assert_eq!(overview(&context).sessions.len(), 1);
    }
}
