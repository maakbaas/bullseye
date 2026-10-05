//! Small helpers for state shared between threads, and for threads that must not die quietly.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::watchdog::note;

/// Locking that carries on after another thread crashed while holding the lock.
///
/// Rust marks such a lock "poisoned", and the usual `.lock().unwrap()` then crashes every thread
/// that touches it next: in the service that would be the ticker, the supervisor and every request,
/// while the process itself stays up, so Windows would not restart it. What this app keeps behind
/// its locks is plain data (a list, an end time, a few flags) that is still valid after a crash,
/// so the right answer is to take the lock anyway.
pub trait LockExt<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Starts a thread that is part of what keeps the app or the service working: the ticker, the
/// supervisor, the pipe, the tab closer. If it crashes, the whole process ends. Windows then
/// starts the service again (and the service starts the app again), and the block is read back
/// from disk, which beats a process that looks healthy while one of its jobs is no longer done.
pub fn spawn_critical(name: &'static str, work: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if catch_unwind(AssertUnwindSafe(work)).is_err() {
                note("thread", &format!("the {name} thread crashed; ending the process so it is started again"));
                std::process::exit(1);
            }
        })
        .expect("cannot start a thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_lock_is_still_usable_after_a_thread_crashed_holding_it() {
        let shared = Arc::new(Mutex::new(vec![1, 2, 3]));
        let crashing = shared.clone();
        let result = thread::spawn(move || {
            let mut list = crashing.lock().unwrap();
            list.push(4);
            panic!("crashed while holding the lock");
        })
        .join();
        assert!(result.is_err());
        // The plain way now fails for everyone...
        assert!(shared.lock().is_err());
        // ...this way carries on, with the data as the crashed thread left it.
        assert_eq!(*shared.locked(), vec![1, 2, 3, 4]);
        shared.locked().push(5);
        assert_eq!(shared.locked().len(), 5);
    }
}
