#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod block;
mod clock;
mod filters;
mod setup;
mod state;
mod tab_closer;
mod task;
mod unlock;
mod watchdog;

use block::{parse_duration, Blocklist, Status};
use filters::FilterList;
use state::Enforcers;
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State, WindowEvent,
};

/// Starts blocking with the given filter lines and enforcers, for `duration` (`5s`, `10m`, `3h`,
/// `1d`) or, when empty, until stopped.
///
/// The block is written to disk and a launcher is started for it, so it goes on even if this
/// app is ended or Windows restarts (see `state.rs` and `watchdog.rs`).
#[tauri::command]
fn start_block(filters: Vec<String>, duration: String, enforcers: Enforcers, blocklist: State<Blocklist>) -> Result<(), String> {
    let duration = parse_duration(&duration)?;
    let filters = FilterList::parse(&filters).map_err(|_| "Invalid block list syntax".to_string())?;
    if !enforcers.any() {
        return Err("Choose at least one enforcer".into());
    }
    // Only the enforcers that go by the list need one; Obsidian's does not.
    if filters.is_empty() && enforcers.use_block_list() {
        return Err("Enter at least one site to block".into());
    }
    blocklist.start(filters, duration, enforcers)?;
    // Have a launcher watching before the block can be ended by ending the app.
    watchdog::spawn_launcher();
    Ok(())
}

#[tauri::command]
fn stop_block(blocklist: State<Blocklist>) -> Result<(), String> {
    blocklist.stop().map_err(|_| "A timed block cannot be stopped early".into())
}

#[tauri::command]
fn block_status(blocklist: State<Blocklist>) -> Status {
    blocklist.status()
}

/// Ends the block for good: the app, the launcher and the login task that starts them, so
/// nothing comes back by itself. Needs the unlock password.
#[tauri::command]
fn force_exit(password: String, app: AppHandle, blocklist: State<Blocklist>) -> Result<(), String> {
    if !unlock::check(&password) {
        return Err("Wrong password".into());
    }
    // The login task first: if it cannot be turned off, nothing has been changed yet and the
    // user is told why rather than left with a task that restarts everything.
    task::set_enabled(false).inspect_err(|e| watchdog::note("stop", e))?;
    blocklist.clear_saved();
    watchdog::kill_all();
    watchdog::note("stop", "stopped by the unlock password");
    app.exit(0);
    Ok(())
}

fn main() {
    // Run by the Windows installer and uninstaller (`setup.rs`); the exit code is their answer.
    if std::env::args_os().any(|arg| arg == "--install") {
        std::process::exit(setup::install());
    }
    if std::env::args_os().any(|arg| arg == "--uninstall") {
        std::process::exit(setup::uninstall());
    }

    // The entry point: one command that starts the app and the launcher that watches it, and
    // that is also what the login task runs at every logon (`watchdog.rs`).
    if std::env::args_os().any(|arg| arg == "--launch") {
        // Returns only when another launcher is already running; this process must then not go
        // on to become the app.
        watchdog::launch_main();
        return;
    }

    // One app instance only. A second copy (started by the launcher, or by launching the
    // executable again) leaves the running one to do the work.
    if !watchdog::claim_app() {
        return;
    }
    // Starting the app is how things are started again after a forced stop.
    watchdog::restore_task();

    // Before the block is restored, so the first second of blocking already asks the internet.
    clock::spawn_sync();
    let blocklist = Blocklist::new();
    // Pick up a block that was running when the app or Windows last stopped, so a restart
    // does not end it.
    blocklist.restore();
    tab_closer::spawn(blocklist.clone());
    // Keeps a launcher running while a timed block is saved, so ending the launcher does not
    // leave the app unwatched.
    watchdog::guard_launcher();

    tauri::Builder::default()
        .manage(blocklist)
        .invoke_handler(tauri::generate_handler![start_block, stop_block, block_status, force_exit])
        // The window's X hides to the tray; only the unlock password quits.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button_state: MouseButtonState::Up, .. } = event {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
