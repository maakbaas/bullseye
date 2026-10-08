//! The platform-free modules of the app, built from src-tauri/src as they are, with stand-ins for
//! the ones that need Windows (`boot`, `drives`, `watchdog`, `client`). See Cargo.toml.
#![allow(dead_code, unused_imports)]

#[path = "../../../src-tauri/src/allowance.rs"]
pub mod allowance;
#[path = "../../../src-tauri/src/session.rs"]
pub mod session;
#[path = "../../../src-tauri/src/cli.rs"]
pub mod cli;
#[path = "../../../src-tauri/src/clock.rs"]
pub mod clock;
#[path = "../../../src-tauri/src/condition.rs"]
pub mod condition;
#[path = "../../../src-tauri/src/datetime.rs"]
pub mod datetime;
#[path = "../../../src-tauri/src/definitions.rs"]
pub mod definitions;
#[path = "../../../src-tauri/src/duration.rs"]
pub mod duration;
#[path = "../../../src-tauri/src/filters.rs"]
pub mod filters;
#[path = "../../../src-tauri/src/handler.rs"]
pub mod handler;
#[path = "../../../src-tauri/src/lists.rs"]
pub mod lists;
#[path = "../../../src-tauri/src/localtime.rs"]
pub mod localtime;
#[path = "../../../src-tauri/src/lock.rs"]
pub mod lock;
#[path = "../../../src-tauri/src/patterns.rs"]
pub mod patterns;
#[path = "../../../src-tauri/src/programs.rs"]
pub mod programs;
#[path = "../../../src-tauri/src/proto.rs"]
pub mod proto;
#[path = "../../../src-tauri/src/schedule.rs"]
pub mod schedule;
#[path = "../../../src-tauri/src/state.rs"]
pub mod state;
#[path = "../../../src-tauri/src/store_apps.rs"]
pub mod store_apps;
#[path = "../../../src-tauri/src/sync.rs"]
pub mod sync;
#[path = "../../../src-tauri/src/system_path.rs"]
pub mod system_path;
#[path = "../../../src-tauri/src/unlock.rs"]
pub mod unlock;
#[path = "../../../src-tauri/src/webhook.rs"]
pub mod webhook;

pub mod boot;
pub mod client;
pub mod drives;
pub mod watchdog;
