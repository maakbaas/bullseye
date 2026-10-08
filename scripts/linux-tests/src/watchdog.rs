//! Stand-in for `watchdog.rs`: the log line, and an app slot that looks after nothing.
pub fn note(who: &str, message: &str) {
    eprintln!("{who}: {message}");
}

pub struct AppSlot;

impl AppSlot {
    pub fn new(_supervised: bool) -> AppSlot {
        AppSlot
    }
    pub fn register(&self, _pid: u32) -> Result<(), String> {
        Ok(())
    }
    pub fn heard(&self, _pid: u32) {}
    pub fn supervise(&self, _block_active: bool, _resumed: bool) -> bool {
        true
    }
}
