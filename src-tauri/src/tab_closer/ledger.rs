//! What was done to close something, so a later pass can tell whether it worked.
//!
//! A close is a request: a button pressed, a message posted, a click sent. Nothing says it was
//! carried out, and some browsers ignore some of them. So each is written down with the tab it was
//! for (or `window`, `panel`), and when the same thing is still there a moment later it counts as not
//! having worked: that is logged, and the next try waits longer each time (up to `LONGEST_WAIT`), so a
//! close that does not work is neither repeated every pass nor written to the log every pass.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// What a tab is recorded as (`Ledger::check`, `record`): its id, which no other tab has.
pub(super) fn tab_key(id: &[i32]) -> String {
    format!("tab {id:?}")
}

/// How long after a close was asked for it is waited for before it is called not to have worked.
pub(super) const VERIFY: Duration = Duration::from_millis(700);
/// The longest wait between two tries of a close that does not work.
const LONGEST_WAIT: Duration = Duration::from_secs(30);
/// How long a record of a close is kept: long enough to be told it worked, then no more.
const KEEP: Duration = Duration::from_secs(120);

/// What the record says about a close that is about to be tried.
#[derive(Debug, PartialEq)]
pub(super) enum Check {
    /// Nothing was tried for it: go ahead.
    Fresh,
    /// Tried a moment ago: too soon to say whether it worked, look again soon.
    Waiting,
    /// Tried, did not work, and it is not yet time to try again.
    Backoff,
    /// Tried this many times and it is still there: try again.
    Failed(u32),
}

struct Attempt {
    at: Instant,
    count: u32,
}

#[derive(Default)]
pub(super) struct Ledger {
    /// (window, what for: a tab by its id, or a whole window or panel) -> the last try.
    attempts: HashMap<(isize, String), Attempt>,
}

/// How long after a try the next one may be made: twice as long after each, up to `LONGEST_WAIT`.
fn wait_after(count: u32) -> Duration {
    VERIFY.saturating_mul(1 << count.min(8)).min(LONGEST_WAIT)
}

impl Ledger {
    pub(super) fn check(&self, window: isize, key: &str, now: Instant) -> Check {
        let Some(attempt) = self.attempts.get(&(window, key.to_string())) else { return Check::Fresh };
        let since = now.saturating_duration_since(attempt.at);
        if since < VERIFY {
            Check::Waiting
        } else if since < wait_after(attempt.count) {
            Check::Backoff
        } else {
            Check::Failed(attempt.count)
        }
    }

    /// A try was made now.
    pub(super) fn record(&mut self, window: isize, key: &str, now: Instant) {
        let count = self.attempts.get(&(window, key.to_string())).map_or(0, |attempt| attempt.count);
        self.attempts.insert((window, key.to_string()), Attempt { at: now, count: count.saturating_add(1) });
    }

    /// Nothing blocked is in the window any more: what was tried there worked.
    pub(super) fn clear_window(&mut self, window: isize) {
        self.attempts.retain(|(handle, _), _| *handle != window);
    }

    /// Forgets old records, and those of windows that are gone.
    pub(super) fn prune(&mut self, now: Instant, windows: &[isize]) {
        self.attempts.retain(|(handle, _), attempt| windows.contains(handle) && now.saturating_duration_since(attempt.at) < KEEP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn a_close_is_waited_for_then_judged_and_tried_again_after_a_longer_wait() {
        let start = Instant::now();
        let mut ledger = Ledger::default();
        // Nothing tried: go ahead.
        assert_eq!(ledger.check(1, "tab [1]", start), Check::Fresh);
        ledger.record(1, "tab [1]", start);
        // Too soon to say whether it worked.
        assert_eq!(ledger.check(1, "tab [1]", after(start, 100)), Check::Waiting);
        let verify = VERIFY.as_millis() as u64;
        assert_eq!(ledger.check(1, "tab [1]", after(start, verify - 1)), Check::Waiting);
        // Judged not to have worked, but not yet time to try again (one wait, twice as long).
        assert_eq!(ledger.check(1, "tab [1]", after(start, verify)), Check::Backoff);
        assert_eq!(ledger.check(1, "tab [1]", after(start, 2 * verify - 1)), Check::Backoff);
        // Then: it is still there after one try, so try again.
        assert_eq!(ledger.check(1, "tab [1]", after(start, 2 * verify)), Check::Failed(1));
        // The second try waits twice as long again.
        let second = after(start, 2 * verify);
        ledger.record(1, "tab [1]", second);
        assert_eq!(ledger.check(1, "tab [1]", after(second, 100)), Check::Waiting);
        assert_eq!(ledger.check(1, "tab [1]", after(second, 4 * verify - 1)), Check::Backoff);
        assert_eq!(ledger.check(1, "tab [1]", after(second, 4 * verify)), Check::Failed(2));
        // A check made at a time before the try is not an error.
        assert_eq!(ledger.check(1, "tab [1]", start), Check::Waiting);
    }

    #[test]
    fn the_wait_doubles_with_each_try_up_to_the_longest() {
        assert_eq!(wait_after(0), VERIFY);
        assert_eq!(wait_after(1), VERIFY * 2);
        assert_eq!(wait_after(2), VERIFY * 4);
        assert!(wait_after(5) < LONGEST_WAIT && wait_after(6) > wait_after(5));
        for count in [7, 8, 9, 100, u32::MAX] {
            assert_eq!(wait_after(count), LONGEST_WAIT, "{count}");
        }
    }

    #[test]
    fn what_was_tried_is_kept_by_window_and_by_what_for() {
        let start = Instant::now();
        let mut ledger = Ledger::default();
        ledger.record(1, "tab [1]", start);
        // Another tab, another window, another kind of close: each its own.
        assert_eq!(ledger.check(1, "tab [2]", start), Check::Fresh);
        assert_eq!(ledger.check(2, "tab [1]", start), Check::Fresh);
        assert_eq!(ledger.check(1, "window", start), Check::Fresh);
        assert_eq!(ledger.check(1, "tab [1]", start), Check::Waiting);
        assert_eq!(tab_key(&[42, 7]), "tab [42, 7]");
        assert_ne!(tab_key(&[1]), tab_key(&[1, 0]));
    }

    #[test]
    fn a_window_with_nothing_blocked_in_it_has_its_records_cleared() {
        let start = Instant::now();
        let mut ledger = Ledger::default();
        ledger.record(1, "tab [1]", start);
        ledger.record(1, "window", start);
        ledger.record(2, "tab [1]", start);
        ledger.clear_window(1);
        assert_eq!(ledger.check(1, "tab [1]", start), Check::Fresh);
        assert_eq!(ledger.check(1, "window", start), Check::Fresh);
        assert_eq!(ledger.check(2, "tab [1]", start), Check::Waiting, "another window's stays");
    }

    #[test]
    fn old_records_and_those_of_windows_that_are_gone_are_forgotten() {
        let start = Instant::now();
        let mut ledger = Ledger::default();
        ledger.record(1, "tab [1]", start);
        ledger.record(2, "tab [1]", start);
        ledger.record(3, "tab [1]", after(start, 100_000));
        // Window 2 is gone; window 1 is there but its record is old by now.
        ledger.prune(after(start, KEEP.as_millis() as u64 + 1), &[1, 3]);
        assert_eq!(ledger.check(1, "tab [1]", start), Check::Fresh, "too old");
        assert_eq!(ledger.check(2, "tab [1]", start), Check::Fresh, "its window is gone");
        assert_ne!(ledger.check(3, "tab [1]", start), Check::Fresh, "recent, and its window is there");
    }
}
