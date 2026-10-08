//! The CLI's exe: a console program, unlike the tray app's, so it can print and return an exit
//! code. Everything is in `cli.rs`; `CLI.md` explains its use.

fn main() {
    std::process::exit(bullseye_core::cli::main())
}
