#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod block;
mod client;
mod clock;
mod filters;
mod pipe;
mod programs;
mod proto;
mod service;
mod setup;
mod state;
mod tab_closer;
mod unlock;
mod watchdog;

use block::Status;
use client::Remote;
use proto::{Request, Response};
use state::Enforcers;
use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State, WindowEvent,
};

/// Starts blocking with the given filter lines, allowed-site lines, program lines and enforcers, for `duration` (`5s`, `10m`, `3h`,
/// `1d`) or, when empty, until stopped.
///
/// The service owns the block and checks all of this again (`service::start`); the answer here
/// is only its verdict. It goes on even if this app is ended or Windows restarts.
#[tauri::command]
fn start_block(
    filters: Vec<String>,
    allowed: Vec<String>,
    programs: Vec<String>,
    duration: String,
    enforcers: Enforcers,
    remote: State<Remote>,
) -> Result<(), String> {
    remote.command(Request::Start { filters, allowed, programs, duration, enforcers })
}

#[tauri::command]
fn stop_block(remote: State<Remote>) -> Result<(), String> {
    remote.command(Request::Stop)
}

#[tauri::command]
fn block_status(remote: State<Remote>) -> Result<Status, String> {
    remote.status()
}

/// Ends the block for good, if the unlock password is right, and quits the app. The service
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
/// `SITEBLOCKER_USE_SERVICE` to talk to a real one (or a `--service-console`) anyway. A release
/// build has no such code.
fn remote_for_this_run() -> Remote {
    #[cfg(debug_assertions)]
    if std::env::var_os("SITEBLOCKER_USE_SERVICE").is_none() {
        return Remote::local(service::embedded());
    }
    Remote::new()
}

fn main() {
    // The service itself, started by Windows (`service.rs`).
    if std::env::args_os().any(|arg| arg == "--service") {
        std::process::exit(service::run_dispatcher());
    }
    // The same in a terminal, for development.
    if std::env::args_os().any(|arg| arg == "--service-console") {
        service::run_console();
        return;
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
