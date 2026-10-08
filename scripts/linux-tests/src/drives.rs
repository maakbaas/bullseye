//! Stand-in for `drives.rs`: no drive is ever connected, which is what a made-up serial number
//! finds on Windows too.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Drive {
    pub serial: String,
    pub name: String,
}

pub fn connected() -> Result<Vec<Drive>, String> {
    Ok(Vec::new())
}

pub fn is_present(_serial: &str) -> bool {
    false
}
