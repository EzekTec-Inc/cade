//! Declarative state signals for the Ratatui renderer.
//!
//! [`Signal`] wraps a `tokio::sync::watch` channel so that components can
//! subscribe to state changes declaratively, eliminating CPU-busy polling
//! in the idle render loop.
//!
//! When a signal is written, it automatically sets a global dirty flag.
//! The TUI tick loop checks this flag and skips rendering entirely when
//! no signal has fired, reducing idle render CPU to near-zero.

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::watch;

/// Global dirty flag. Set to `true` whenever any [`Signal::write`] is called.
/// The TUI tick loop resets it to `false` after each draw.
pub(crate) static GLOBAL_DIRTY: AtomicBool = AtomicBool::new(false);

/// Returns `true` if any signal has been written since the last check.
/// Atomically resets the flag to `false`.
pub fn take_global_dirty() -> bool {
    GLOBAL_DIRTY.swap(false, Ordering::AcqRel)
}

/// A declarative state signal backed by `tokio::sync::watch`.
///
/// - `.read()` returns an immutable snapshot of the current value.
/// - `.write(val)` updates the value and marks the renderer as dirty.
/// - `.subscribe()` returns a receiver for async polling.
#[derive(Debug)]
pub struct Signal<T> {
    tx: watch::Sender<T>,
    rx: watch::Receiver<T>,
}

impl<T: Clone + Send + 'static> Signal<T> {
    /// Create a new signal with the given initial value.
    pub fn new(initial: T) -> Self {
        let (tx, rx) = watch::channel(initial);
        Self { tx, rx }
    }

    /// Acquire an immutable snapshot of the current value.
    pub fn read(&self) -> T {
        self.rx.borrow().clone()
    }

    /// Update the signal value and mark the renderer as dirty.
    ///
    /// If the new value equals the current value, the write is a no-op
    /// (the dirty flag is NOT set, avoiding unnecessary redraws).
    pub fn write(&self, val: T)
    where
        T: PartialEq,
    {
        if self.rx.borrow().eq(&val) {
            return;
        }
        self.tx.send(val).ok();
        GLOBAL_DIRTY.store(true, Ordering::Release);
    }

    /// Force-update the signal value even if unchanged (always sets dirty).
    pub fn write_force(&self, val: T) {
        self.tx.send(val).ok();
        GLOBAL_DIRTY.store(true, Ordering::Release);
    }

    /// Subscribe to value changes via a `watch::Receiver`.
    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.rx.clone()
    }

    /// The underlying sender, for use with `select!` / async patterns.
    pub fn sender(&self) -> watch::Sender<T> {
        self.tx.clone()
    }
}

impl<T: Clone + Send + 'static> Default for Signal<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// A collection of named signals that components can subscribe to.
///
/// Provides ergonomic access to common TUI state signals without
/// passing individual `Signal<T>` instances through every function.
#[derive(Debug)]
pub struct SignalRegistry {
    /// Signalled when the agent begins/finishes thinking.
    pub thinking: Signal<bool>,
    /// Signalled when streaming text is being received.
    pub streaming: Signal<bool>,
    /// Signalled when the active plan changes.
    pub plan_changed: Signal<bool>,
    /// Signalled when new render lines are pushed.
    pub content_changed: Signal<bool>,
    /// Signalled on mode/permission changes.
    pub mode_changed: Signal<bool>,
}

impl SignalRegistry {
    pub fn new() -> Self {
        Self {
            thinking: Signal::new(false),
            streaming: Signal::new(false),
            plan_changed: Signal::new(false),
            content_changed: Signal::new(false),
            mode_changed: Signal::new(false),
        }
    }

    /// Returns `true` if any signal has been written since the last poll.
    /// Used by the tick loop to decide whether to render.
    pub fn any_dirty(&self) -> bool {
        take_global_dirty()
    }
}

impl Default for SignalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_read_write() {
        let s = Signal::new(0u32);
        assert_eq!(s.read(), 0);

        s.write(42);
        assert_eq!(s.read(), 42);
    }

    #[test]
    fn signal_noop_does_not_set_dirty() {
        GLOBAL_DIRTY.store(false, Ordering::SeqCst);
        let s = Signal::new(10u32);
        // Write the same value — should be a no-op.
        s.write(10);
        assert!(!take_global_dirty(), "noop write should not set dirty");
    }

    #[test]
    fn signal_write_sets_dirty() {
        GLOBAL_DIRTY.store(false, Ordering::SeqCst);
        let s = Signal::new(0u32);
        s.write(1);
        assert!(take_global_dirty(), "write should set dirty flag");
    }

    #[test]
    fn signal_write_force_always_sets_dirty() {
        GLOBAL_DIRTY.store(false, Ordering::SeqCst);
        let s = Signal::new(5u32);
        s.write_force(5);
        assert!(
            take_global_dirty(),
            "write_force should set dirty even for same value"
        );
    }

    #[tokio::test]
    async fn signal_subscribe_receives_updates() {
        let s = Signal::new("hello".to_string());
        let mut rx = s.subscribe();

        s.write("world".to_string());
        assert!(rx.changed().await.is_ok());
        assert_eq!(*rx.borrow_and_update(), "world");
    }

    #[test]
    fn signal_registry_any_dirty() {
        GLOBAL_DIRTY.store(false, Ordering::SeqCst);
        let reg = SignalRegistry::new();

        assert!(!reg.any_dirty(), "fresh registry should not be dirty");

        reg.content_changed.write(true);
        assert!(reg.any_dirty(), "registry should detect signal write");
    }

    #[test]
    fn take_global_dirty_resets_flag() {
        GLOBAL_DIRTY.store(true, Ordering::SeqCst);
        assert!(take_global_dirty());
        assert!(!take_global_dirty(), "second call should return false");
    }
}
