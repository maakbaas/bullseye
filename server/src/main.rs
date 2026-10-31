//! The Bullseye sync server: `hub.rs` (the log, and the server's own view of what it comes to) behind
//! HTTP (`http.rs`), configured by environment variables:
//!
//! | Variable | Default | What |
//! |---|---|---|
//! | `BULLSEYE_TOKEN` | (required) | The token every request but `/api/health` carries |
//! | `BULLSEYE_DATA` | `/data` | The folder for the log, the computers and the view |
//! | `BULLSEYE_LISTEN` | `0.0.0.0:8080` | Where to listen; HTTPS is the reverse proxy's |
//!
//! The modules below are the app's own, built from src-tauri/src; the few that need Windows are the
//! stand-ins of scripts/linux-tests (and `boot.rs` here).
// The app's modules are its own crate's there; public here, clippy would ask for more of them.
#![allow(dead_code, unused_imports, clippy::new_without_default)]

#[path = "../../src-tauri/src/allowance.rs"]
pub mod allowance;
#[path = "../../src-tauri/src/clock.rs"]
pub mod clock;
#[path = "../../src-tauri/src/condition.rs"]
pub mod condition;
#[path = "../../src-tauri/src/datetime.rs"]
pub mod datetime;
#[path = "../../src-tauri/src/definitions.rs"]
pub mod definitions;
#[path = "../../src-tauri/src/duration.rs"]
pub mod duration;
#[path = "../../src-tauri/src/filters.rs"]
pub mod filters;
#[path = "../../src-tauri/src/handler.rs"]
pub mod handler;
#[path = "../../src-tauri/src/hub.rs"]
pub mod hub;
#[path = "../../src-tauri/src/lists.rs"]
pub mod lists;
#[path = "../../src-tauri/src/localtime.rs"]
pub mod localtime;
#[path = "../../src-tauri/src/lock.rs"]
pub mod lock;
#[path = "../../src-tauri/src/patterns.rs"]
pub mod patterns;
#[path = "../../src-tauri/src/programs.rs"]
pub mod programs;
#[path = "../../src-tauri/src/proto.rs"]
pub mod proto;
#[path = "../../src-tauri/src/replica.rs"]
pub mod replica;
#[path = "../../src-tauri/src/schedule.rs"]
pub mod schedule;
#[path = "../../src-tauri/src/session.rs"]
pub mod session;
#[path = "../../src-tauri/src/state.rs"]
pub mod state;
#[path = "../../src-tauri/src/status.rs"]
pub mod status;
#[path = "../../src-tauri/src/store_apps.rs"]
pub mod store_apps;
#[path = "../../src-tauri/src/strings.rs"]
pub mod strings;
#[path = "../../src-tauri/src/sync.rs"]
pub mod sync;
#[path = "../../src-tauri/src/sync_api.rs"]
pub mod sync_api;
#[path = "../../src-tauri/src/syncing.rs"]
pub mod syncing;
#[path = "../../src-tauri/src/unlock.rs"]
pub mod unlock;
#[path = "../../src-tauri/src/web.rs"]
pub mod web;
#[path = "../../src-tauri/src/webhook.rs"]
pub mod webhook;

pub mod boot;
#[path = "../../scripts/linux-tests/src/drives.rs"]
pub mod drives;
#[path = "../../scripts/linux-tests/src/watchdog.rs"]
pub mod watchdog;

mod http;

use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let token = std::env::var("BULLSEYE_TOKEN").unwrap_or_default();
    if token.trim().is_empty() {
        eprintln!("bullseye-sync: set BULLSEYE_TOKEN to a token");
        std::process::exit(2);
    }
    let data = PathBuf::from(std::env::var("BULLSEYE_DATA").unwrap_or_else(|_| "/data".into()));
    let listen = std::env::var("BULLSEYE_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into());
    if let Err(e) = std::fs::create_dir_all(data.join("view")) {
        eprintln!("bullseye-sync: cannot use {}: {e}", data.display());
        std::process::exit(1);
    }
    // Internet time, as a computer's service has it, so the view's times are the computers'.
    clock::spawn_sync();
    let hub = Arc::new(hub::Hub::new(hub::view(&data.join("view")), Some(data.clone())));
    // The window's page, for managing every computer from a browser (the image has it in /app/web).
    let page = PathBuf::from(std::env::var("BULLSEYE_PAGE").unwrap_or_else(|_| "/app/web".into()));
    let api = http::Api::new(hub, token.trim());
    let api = Arc::new(if page.join("index.html").is_file() { api.with_page(page) } else { api });
    let server = match http::listen(&listen) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("bullseye-sync: cannot listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("bullseye-sync {}: listening on {listen}, data in {}", env!("CARGO_PKG_VERSION"), data.display());
    http::serve(api, server);
}
