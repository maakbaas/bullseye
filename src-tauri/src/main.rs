#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The tray app's exe; everything is in the library (`lib.rs`).

fn main() {
    bullseye_core::run()
}
