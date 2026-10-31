//! Stand-in for `boot.rs`: the server's view has no Windows that starts again, so it is always the
//! same start (a session locked until restart ends there only when a computer says it ended).
pub fn id() -> Option<String> {
    Some("server".into())
}
