//! Stand-in for `watchdog.rs`: the log line, and an app slot that looks after nothing.
pub fn note(who: &str, message: &str) {
    eprintln!("{who}: {message}");
}

pub struct AppSlot;

pub struct Missing {
    pub app: bool,
    pub worker: bool,
}

impl AppSlot {
    pub fn new(_supervised: bool) -> AppSlot {
        AppSlot
    }
    pub fn register(&self, _pid: u32) -> Result<(), String> {
        Ok(())
    }
    pub fn register_worker(&self, _pid: u32) -> Result<(), String> {
        Ok(())
    }
    pub fn heard(&self, _pid: u32) -> Result<(), String> {
        Ok(())
    }
    pub fn supervise(&self, _block_active: bool, _resumed: bool) -> Missing {
        Missing {
            app: true,
            worker: true,
        }
    }
}
