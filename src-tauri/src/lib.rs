//! The app, the service and the CLI share this library. `main.rs` is the tray app's exe (which also
//! runs as the service and for the installer), `bin/bullseye.rs` the console exe of the CLI
//! (`cli.rs`, `CLI.md`).

mod allowance;
mod boot;
pub mod cli;
mod client;
mod clock;
mod condition;
mod datetime;
mod definitions;
mod drives;
mod duration;
mod filters;
mod handler;
mod lists;
mod localtime;
mod lock;
mod patterns;
mod pipe;
mod programs;
mod proto;
mod registration;
mod schedule;
mod service;
mod session;
mod setup;
mod state;
mod store_apps;
mod sync;
mod system_path;
mod tab_closer;
mod ui;
mod unlock;
mod wake;
mod watchdog;
mod webhook;

use client::Remote;
use definitions::{Condition, Contents, Definition, Fallout, Kind, Library, Target};
use lock::Lock;
use proto::{Overview, Request, Response};
use schedule::Week;
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    RunEvent, State, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;

// The page's commands. **Each one has a CLI command that does the same** (`cli::GUI_COMMANDS`, which a
// test checks against the page): a new one needs one there too, and a line in `CLI.md`.

/// Every target, condition and session, and which sessions run.
#[tauri::command]
fn status(remote: State<Remote>) -> Result<Overview, String> {
    remote.status()
}

/// Adds a session, or changes the one with its id. The service checks it again (`handler.rs`) and
/// refuses it while that session runs.
#[tauri::command]
fn save_session(session: Definition, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SaveSession { session })
}

#[tauri::command]
fn delete_session(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::DeleteSession { id })
}

/// Adds a target, or changes the one with its id; while a locked session uses it, only to be stricter.
#[tauri::command]
fn save_target(target: Target, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SaveTarget { target })
}

/// Takes it out of the sessions that use it too; refused while a running session uses it.
#[tauri::command]
fn delete_target(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::DeleteTarget { id })
}

/// Adds a condition, or changes the one with its id; while a locked session uses it, only to be stricter.
#[tauri::command]
fn save_condition(condition: Condition, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SaveCondition { condition })
}

/// Takes it out of the sessions that use it too; refused while a running session uses it.
#[tauri::command]
fn delete_condition(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::DeleteCondition { id })
}

/// A copy of a session, target or condition, as `new_id`, named after it with a number.
#[tauri::command]
fn duplicate(kind: Kind, id: String, new_id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Duplicate { kind, id, new_id })
}

/// Starts a session as it is defined. The service owns it from then on: it goes on even if this app
/// is ended or Windows restarts.
#[tauri::command]
fn start_session(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Start { id })
}

/// Locks a running session that is not locked, from now on, with this lock (also its lock for the
/// next start).
#[tauri::command]
fn lock_session(id: String, lock: Lock, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Lock { id, lock })
}

#[tauri::command]
fn stop_session(id: String, password: Option<String>, text: Option<String>, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Stop { id, password, text })
}

/// Starts the wait before a session locked with a delay can be stopped (`session::RunLock::Delay`).
#[tauri::command]
fn request_stop(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::AskToStop { id })
}

/// Ends that wait: the session is locked again.
#[tauri::command]
fn abort_stop(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::AbortStop { id })
}

/// Asks a webhook condition once, as the editor Test button does: what the web address answered and what
/// the match makes of it. The request is made off the main thread.
#[tauri::command]
async fn test_webhook(hook: webhook::Hook) -> Result<webhook::Probe, String> {
    tauri::async_runtime::spawn_blocking(move || webhook::probe(&hook)).await.map_err(|e| e.to_string())?
}

/// A weekly schedule as text, after checking it, for the condition editor and the session's summary.
#[tauri::command]
fn describe_schedule(week: Week) -> Result<String, String> {
    week.normalized().map(|week| week.describe())
}

/// A session's allowance in words, after checking them: in full (`30 minutes in total, reset every day`)
/// for the session editor, and as the tile's short label (`Allowed 30 minutes a day`). Needs no service.
#[tauri::command]
fn describe_allowance(terms: allowance::Terms) -> Result<allowance::Described, String> {
    terms.described()
}

/// A typed duration (`2h 30m`) in words, for a duration lock's summary and its confirmation. Needs no
/// service: it only reads the text.
#[tauri::command]
fn describe_duration(text: String) -> Result<String, String> {
    match duration::parse(&text)? {
        Some(length) => Ok(duration::countdown(length.as_secs())),
        None => Err("No duration given".into()),
    }
}

/// An until lock's end as typed (`2026-10-06 18:00`), written out and how long from now, for its
/// form, its summary and its confirmation; refused when it has passed. Needs no service.
#[tauri::command]
fn describe_until(text: String) -> Result<localtime::Until, String> {
    localtime::until(&text)
}

/// The Store apps installed, for the target editor to offer to tick (`store_apps.rs`), and the
/// ones it has (`chosen`) that are not installed here, by the name in their family name.
/// PowerShell takes a second or two to list them, so not on the thread that draws the window.
#[tauri::command]
async fn installed_apps(chosen: Vec<String>) -> Result<Vec<store_apps::StoreApp>, String> {
    let installed = tauri::async_runtime::spawn_blocking(store_apps::installed).await.map_err(|e| e.to_string())??;
    Ok(store_apps::offered(installed, &chosen))
}

/// The flash drives connected now, for the Flash drive lock's settings to offer (`drives.rs`).
/// Asking Windows takes a moment, so not on the thread that draws the window.
#[tauri::command]
async fn connected_drives() -> Result<Vec<drives::Drive>, String> {
    tauri::async_runtime::spawn_blocking(drives::connected).await.map_err(|e| e.to_string())?
}

/// What deleting the target or condition `id` does to `sessions` and `conditions` (the window's copy of them),
/// for the question before it: the service's own rule, so what is asked is what happens.
#[tauri::command]
fn deletion_fallout(sessions: Vec<Definition>, conditions: Vec<Condition>, id: String) -> Fallout {
    Library { sessions, conditions, ..Library::default() }.fallout(&id)
}

/// What the Save and Open dialogs of Export and Import show.
const SETTINGS_FILTER: (&str, &[&str]) = ("Bullseye settings", &["json"]);

/// Export: everything defined, to a settings file chosen in a Save dialog. The file's name, or
/// `None` when the dialog was cancelled. Async, so the dialog does not wait on the main thread.
#[tauri::command]
async fn export_settings(app: tauri::AppHandle, remote: State<'_, Remote>) -> Result<Option<String>, String> {
    let library = remote.library()?;
    let chosen = app.dialog().file().add_filter(SETTINGS_FILTER.0, SETTINGS_FILTER.1).set_file_name("bullseye.json").blocking_save_file();
    let Some(path) = chosen.map(|path| path.into_path().map_err(|e| e.to_string())).transpose()? else { return Ok(None) };
    definitions::write_settings(&path, library)?;
    Ok(path.file_name().map(|name| name.to_string_lossy().into_owned()))
}

/// A settings file chosen in an Open dialog, and what is in it, for the question before importing
/// it; `None` when the dialog was cancelled. Needs no service.
#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> Result<Option<Opened>, String> {
    let chosen = app.dialog().file().add_filter(SETTINGS_FILTER.0, SETTINGS_FILTER.1).blocking_pick_file();
    let Some(path) = chosen.map(|path| path.into_path().map_err(|e| e.to_string())).transpose()? else { return Ok(None) };
    let library = definitions::read_settings(&path)?;
    let contents = library.contents();
    let installed = if library.has_store_apps() { installed_families().await } else { Some(Vec::new()) };
    let apps_checked = installed.is_some();
    let (_, left_out) = library.without_missing_apps(installed.as_deref());
    Ok(Some(Opened { path: path.to_string_lossy().into_owned(), contents, left_out, apps_checked }))
}

/// The installed Store apps' family names, asked of PowerShell off the main thread; `None` if it
/// cannot say.
async fn installed_families() -> Option<Vec<String>> {
    tauri::async_runtime::spawn_blocking(store_apps::installed_families).await.ok().flatten()
}

/// A settings file and what is in it (`open_settings`): what it holds, and the Store apps in it
/// that are not installed here, which Import leaves out (`apps_checked`: whether that could be told).
#[derive(serde::Serialize)]
struct Opened {
    path: String,
    contents: Contents,
    left_out: Vec<definitions::LeftOut>,
    apps_checked: bool,
}

/// Import: replaces every target, condition and session with those in the settings file at
/// `path`, without the Store apps not installed here (`Library::without_missing_apps`). Refused
/// while a session is locked; sessions that run unlocked end with it.
#[tauri::command]
async fn import_settings(path: String, remote: State<'_, Remote>) -> Result<(), String> {
    let library = definitions::read_settings(std::path::Path::new(&path))?;
    let installed = if library.has_store_apps() { installed_families().await } else { None };
    let (library, _) = library.without_missing_apps(installed.as_deref());
    remote.command(Request::Import { library })
}

/// As `unlock_all`, with the answer of the other way to unlock: its text or password (`unlock::Way`).
#[tauri::command]
fn unlock_with(password: Option<String>, text: Option<String>, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::UnlockWith { password, text })?;
    watchdog::note("app", "every session ended by unlocking");
    Ok(())
}

/// Sets the other way to unlock besides the delay, as a lock method, or removes it (`None`).
#[tauri::command]
fn set_unlock_way(way: Option<Lock>, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SetUnlockWay { way })
}

/// Starts the countdown to unlocking (`unlock.rs`); it can be aborted.
#[tauri::command]
fn request_unlock(remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::AskForUnlock)
}

/// Stops the countdown, or ends the readiness: back to locked.
#[tauri::command]
fn abort_unlock(remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::AbortUnlock)
}

/// Sets how long unlocking takes, as typed (`2h 30m`, `0`; at most 24 hours).
#[tauri::command]
fn set_unlock_delay(delay: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SetUnlockDelay { delay })
}

/// Ends every running session, locked or not, once unlocking is ready (`unlock.rs`: at once with no
/// delay, else after the countdown). The app stays open, with nothing to enforce; the definitions
/// stay too.
#[tauri::command]
fn unlock_all(remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Unlock)?;
    watchdog::note("app", "every session ended by unlocking");
    Ok(())
}

/// Who the app talks to. Normally the service, over its pipe. In a debug build (`npm run dev`) the
/// service's logic runs inside the app instead, so development needs no installed service; set
/// `BULLSEYE_USE_SERVICE` to talk to a real, installed one anyway. A release build has no
/// such code.
fn remote_for_this_run() -> Remote {
    #[cfg(debug_assertions)]
    if std::env::var_os("BULLSEYE_USE_SERVICE").is_none() {
        return Remote::local(service::embedded());
    }
    Remote::new()
}

/// Everything the tray app's exe does: the service, the installer's steps, or the app.
pub fn run() {
    // The service itself, started by Windows (`service.rs`).
    if std::env::args_os().any(|arg| arg == "--service") {
        std::process::exit(service::run_dispatcher());
    }
    // Run by the Windows installer and uninstaller (`setup.rs`); the exit code is their answer.
    if std::env::args_os().any(|arg| arg == "--install") {
        std::process::exit(setup::install());
    }
    if std::env::args_os().any(|arg| arg == "--uninstall") {
        std::process::exit(setup::uninstall());
    }

    let remote = remote_for_this_run();

    // Ask the service to take this process in as the app. It refuses if another app is running
    // (a second copy leaves the running one to do the work) or if this is not the installed exe.
    // If the service cannot be reached, go on: it takes the app in on its first policy request.
    // (A local service has nobody to register with.)
    if !remote.is_local() {
        if let Ok(Response::Refused(why)) = client::call(&Request::Register) {
            watchdog::note("app", &format!("not starting: {why}"));
            return;
        }
    }

    tab_closer::spawn(remote.clone());
    wake::spawn_hook();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(remote)
        .manage(ui::Ui::new())
        .invoke_handler(tauri::generate_handler![
            status,
            save_session,
            delete_session,
            save_target,
            delete_target,
            save_condition,
            delete_condition,
            duplicate,
            start_session,
            lock_session,
            stop_session,
            request_stop,
            abort_stop,
            unlock_all,
            unlock_with,
            set_unlock_way,
            request_unlock,
            abort_unlock,
            set_unlock_delay,
            test_webhook,
            describe_schedule,
            describe_allowance,
            describe_duration,
            describe_until,
            deletion_fallout,
            installed_apps,
            connected_drives,
            export_settings,
            open_settings,
            import_settings,
            ui::window_ready
        ])
        // The window's X hides it (it is destroyed after a while, `ui.rs`); the app keeps running.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                ui::dismiss(window);
            }
        })
        .setup(|app| {
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button_state: MouseButtonState::Up, .. } = event {
                        ui::show_in_background(tray.app_handle().clone());
                    }
                })
                .build(app)?;
            ui::spawn_reaper(app.handle().clone());
            #[cfg(debug_assertions)]
            ui::debug_aids(app.handle());

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // With no window the app has nothing to show but must not end: an exit that nobody asked
            // for (the last window going away) is refused, while unlocking's is allowed.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
