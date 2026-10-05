//! The app, the service and the CLI share this library. `main.rs` is the tray app's exe (which also
//! runs as the service and for the installer), `bin/bullseye-cli.rs` the console exe of the CLI
//! (`cli.rs`, `CLI.md`).

mod block;
mod boot;
pub mod cli;
mod client;
mod clock;
mod datetime;
mod definitions;
mod duration;
mod filters;
mod handler;
mod localtime;
mod lock;
mod pipe;
mod programs;
mod proto;
mod registration;
mod schedule;
mod service;
mod setup;
mod state;
mod sync;
mod tab_closer;
mod ui;
mod unlock;
mod wake;
mod watchdog;

use client::Remote;
use definitions::{BlockList, Contents, Definition, Fallout, Kind, Library, NamedSchedule};
use proto::{Overview, Request, Response};
use schedule::Week;
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    RunEvent, State, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;

// The page's commands. **Each one has a CLI command that does the same** (`cli::GUI_COMMANDS`, which a
// test checks against the page): a new one needs one there too, and a line in `CLI.md`.

/// Every block list, schedule and block, and which blocks run.
#[tauri::command]
fn block_status(remote: State<Remote>) -> Result<Overview, String> {
    remote.status()
}

/// Adds a block, or changes the one with its id. The service checks it again (`handler.rs`) and
/// refuses it while that block runs.
#[tauri::command]
fn save_block(block: Definition, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SaveBlock { block })
}

#[tauri::command]
fn delete_block(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::DeleteBlock { id })
}

/// Adds a block list, or changes the one with its id; refused while a running block uses it.
#[tauri::command]
fn save_list(list: BlockList, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SaveList { list })
}

/// Takes it out of the blocks that use it too; refused while a running block uses it.
#[tauri::command]
fn delete_list(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::DeleteList { id })
}

/// Adds a schedule, or changes the one with its id; refused while a running block uses it.
#[tauri::command]
fn save_schedule(schedule: NamedSchedule, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::SaveSchedule { schedule })
}

/// Takes it out of the blocks that use it too; refused while a running block uses it.
#[tauri::command]
fn delete_schedule(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::DeleteSchedule { id })
}

/// A copy of a block, block list or schedule, as `new_id`, named after it with a number.
#[tauri::command]
fn duplicate(kind: Kind, id: String, new_id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Duplicate { kind, id, new_id })
}

/// Starts a block as it is defined. The service owns it from then on: it goes on even if this app
/// is ended or Windows restarts.
#[tauri::command]
fn start_block(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Start { id })
}

#[tauri::command]
fn stop_block(id: String, password: Option<String>, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Stop { id, password })
}

/// A weekly schedule as text, after checking it, for the schedule editor and the block's summary.
#[tauri::command]
fn describe_schedule(week: Week) -> Result<String, String> {
    week.normalized().map(|week| week.describe())
}

/// A typed duration (`2h 30m`) in words, for a duration lock's summary and its confirmation. Needs no
/// service: it only reads the text.
#[tauri::command]
fn describe_duration(text: String) -> Result<String, String> {
    match duration::parse(&text)? {
        Some(length) => Ok(duration::words(length.as_secs())),
        None => Err("No duration given".into()),
    }
}

/// An until lock's end as typed (`2026-10-06 18:00`), written out and how long from now, for its
/// form, its summary and its confirmation; refused when it has passed. Needs no service.
#[tauri::command]
fn describe_until(text: String) -> Result<localtime::Until, String> {
    localtime::until(&text)
}

/// What deleting the block list or schedule `id` does to `blocks` (the window's copy of them),
/// for the question before it: the service's own rule, so what is asked is what happens.
#[tauri::command]
fn deletion_fallout(blocks: Vec<Definition>, id: String) -> Fallout {
    Library { blocks, ..Library::default() }.fallout(&id)
}

/// What the Save and Open dialogs of Export and Import show.
const SETTINGS_FILTER: (&str, &[&str]) = ("Bullseye settings", &["json"]);

/// Export: everything defined, to a settings file chosen in a Save dialog. The file's name, or
/// `None` when the dialog was cancelled. Async, so the dialog does not wait on the main thread.
#[tauri::command]
async fn export_settings(app: tauri::AppHandle, remote: State<'_, Remote>) -> Result<Option<String>, String> {
    let library = remote.status()?.library();
    let chosen =
        app.dialog().file().add_filter(SETTINGS_FILTER.0, SETTINGS_FILTER.1).set_file_name("Bullseye settings.json").blocking_save_file();
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
    let contents = definitions::read_settings(&path)?.contents();
    Ok(Some(Opened { path: path.to_string_lossy().into_owned(), contents }))
}

/// A settings file and what is in it (`open_settings`).
#[derive(serde::Serialize)]
struct Opened {
    path: String,
    contents: Contents,
}

/// Import: replaces every block list, schedule and block with those in the settings file at
/// `path`. Refused while a block runs.
#[tauri::command]
fn import_settings(path: String, remote: State<Remote>) -> Result<(), String> {
    let library = definitions::read_settings(std::path::Path::new(&path))?;
    remote.command(Request::Import { library })
}

/// Ends every running block, locked or not, if the unlock password is right. The app stays open,
/// with nothing to enforce; the definitions stay too.
#[tauri::command]
fn unlock_all(password: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Unlock { password })?;
    watchdog::note("app", "every block ended with the unlock password");
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
            block_status,
            save_block,
            delete_block,
            save_list,
            delete_list,
            save_schedule,
            delete_schedule,
            duplicate,
            start_block,
            stop_block,
            unlock_all,
            describe_schedule,
            describe_duration,
            describe_until,
            deletion_fallout,
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
            // for (the last window going away) is refused, while the unlock password's is allowed.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
