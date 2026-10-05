//! The app, the service and the CLI share this library. `main.rs` is the tray app's exe (which also
//! runs as the service and for the installer), `bin/bullseye-cli.rs` the console exe of the CLI
//! (`cli.rs`, `CLI.md`).

mod block;
pub mod cli;
mod client;
mod clock;
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
mod watchdog;

use client::Remote;
use definitions::Definition;
use proto::{Overview, Request, Response};
use schedule::Week;
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, RunEvent, State, WindowEvent,
};

// The page's commands. **Each one has a CLI command that does the same** (`cli::GUI_COMMANDS`, which a
// test checks against the page): a new one needs one there too, and a line in `CLI.md`.

/// Every block, and which of them run.
#[tauri::command]
fn block_status(remote: State<Remote>) -> Result<Overview, String> {
    remote.status()
}

/// Adds a block, or changes the one with its id. The service checks it again (`handler.rs`) and
/// refuses it while that block runs.
#[tauri::command]
fn save_block(block: Definition, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Save { block })
}

#[tauri::command]
fn delete_block(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Delete { id })
}

/// Starts a block as it is defined. The service owns it from then on: it goes on even if this app
/// is ended or Windows restarts.
#[tauri::command]
fn start_block(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Start { id })
}

#[tauri::command]
fn stop_block(id: String, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Stop { id })
}

/// A weekly schedule written as text (`schedule.rs`), for the schedule editor; `None` is always
/// active. Needs no service: it only reads the text.
#[tauri::command]
fn parse_schedule(text: String) -> Result<Option<Week>, String> {
    Week::parse(&text)
}

/// A weekly schedule as text, after checking it, for the schedule editor and the block's summary.
#[tauri::command]
fn describe_schedule(week: Week) -> Result<String, String> {
    week.normalized().map(|week| week.describe())
}

/// A typed duration (`2h 30m`) in words, for a timer lock's summary and its confirmation. Needs no
/// service: it only reads the text.
#[tauri::command]
fn describe_duration(text: String) -> Result<String, String> {
    match duration::parse(&text)? {
        Some(length) => Ok(duration::words(length.as_secs())),
        None => Err("No duration given".into()),
    }
}

/// Ends every block for good, if the unlock password is right, and quits the app. The service
/// stays; with no block left it has nothing to do, and starts the app again at the next logon.
#[tauri::command]
fn force_exit(password: String, app: AppHandle, remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Unlock { password })?;
    watchdog::note("app", "stopped by the unlock password");
    app.exit(0);
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
    if std::env::args_os().any(|arg| arg == "--prepare-update") {
        std::process::exit(setup::prepare_update());
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

    tauri::Builder::default()
        .manage(remote)
        .manage(ui::Ui::new())
        .invoke_handler(tauri::generate_handler![
            block_status,
            save_block,
            delete_block,
            start_block,
            stop_block,
            force_exit,
            parse_schedule,
            describe_schedule,
            describe_duration,
            ui::window_ready
        ])
        // The window's X hides it (it is destroyed after a while, `ui.rs`); only the unlock password quits.
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
            // The blocks can also be unlocked from the CLI; the service then tells the app to quit,
            // and it does as after the window's Unlock.
            let handle = app.handle().clone();
            app.state::<Remote>().when_told_to_quit(move || handle.exit(0));
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
