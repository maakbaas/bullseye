//! Stand-in for `boot.rs`: always the same start of Windows.
pub fn id() -> Option<String> {
    Some("test-boot".into())
}
