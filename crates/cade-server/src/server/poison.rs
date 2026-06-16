//! Mutex poison recovery with structured tracing.
//!
//! `std::sync::Mutex` poisons when a holder panics.  The historical
//! pattern across the server crate was:
//!
//! ```ignore
//! let g = m.lock().unwrap_or_else(|e| e.into_inner());
//! ```
//!
//! This silently recovers, hiding the prior panic from operators.  The
//! helpers in this module record an `error!` event on the
//! `cade::poison` target so a poisoned mutex is at least observable in
//! the logs.  Behaviour for the happy path is unchanged: `lock()`
//! succeeds and the guard is returned.
//!
//! The helpers are intentionally `pub(crate)` and minimal; they do not
//! own a recovery policy beyond "log and continue" because the pre-
//! existing call sites already chose that policy.

#[allow(unused_imports)]
use std::sync::{Mutex, MutexGuard};

/// Format the structured log message emitted on poison recovery.
/// Pure helper, kept side-effect free so it can be unit-tested without
/// installing a tracing subscriber.
#[allow(dead_code)]
pub(crate) fn fmt_poison_recovery(label: &str) -> String {
    format!("mutex poisoned at '{label}': recovering inner data; a previous holder panicked")
}

/// Acquire `m`, recovering from poison after emitting an
/// `error!(target = "cade::poison")` event.  `label` should identify
/// the lock's purpose (e.g. `"context_cache"`); it is included verbatim
/// in the log line.
#[allow(dead_code)]
pub(crate) fn lock_or_recover<'a, T>(m: &'a Mutex<T>, label: &'static str) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::error!(target: "cade::poison", "{}", fmt_poison_recovery(label));
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn fmt_poison_recovery_includes_label() {
        let s = fmt_poison_recovery("context_cache");
        assert!(s.contains("context_cache"), "label missing: {s}");
        assert!(
            s.contains("recovering"),
            "must mention recovery action: {s}"
        );
    }

    #[test]
    fn fmt_poison_recovery_mentions_prior_panic() {
        // Operators reading the log must understand the mutex did not
        // poison itself — a previous holder panicked.
        let s = fmt_poison_recovery("buckets");
        assert!(
            s.to_lowercase().contains("panic"),
            "must reference the prior panic: {s}"
        );
    }

    #[test]
    fn lock_or_recover_happy_path_returns_guard() {
        let m = Mutex::new(42_u32);
        let g = lock_or_recover(&m, "happy");
        assert_eq!(*g, 42);
    }

    #[test]
    fn lock_or_recover_recovers_after_poison() {
        let m = Arc::new(Mutex::new(7_u32));
        let m2 = Arc::clone(&m);

        // Poison the mutex by panicking inside the critical section.
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().expect("first lock");
            panic!("intentional poison for test");
        })
        .join();

        assert!(m.is_poisoned(), "precondition: mutex is poisoned");

        // Helper must not panic and must yield the inner data.
        let g = lock_or_recover(&m, "recover");
        assert_eq!(*g, 7);
    }
}
