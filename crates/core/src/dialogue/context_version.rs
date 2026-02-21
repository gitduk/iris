//! Context version counter — monotonically increasing version that increments
//! on each new external input. Used to detect and cancel stale reasoning tasks.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Shared context version counter.
/// Clone-cheap (Arc-backed). Readers snapshot the version before spawning
/// slow-path work and compare after completion to detect staleness.
#[derive(Clone, Debug)]
pub struct ContextVersion {
    inner: Arc<AtomicU64>,
}

impl ContextVersion {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Increment the version (called when new external input arrives).
    /// Returns the new version number.
    pub fn bump(&self) -> u64 {
        self.inner.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Read the current version.
    pub fn current(&self) -> u64 {
        self.inner.load(Ordering::SeqCst)
    }

    /// Check if a previously captured version is still current (not stale).
    pub fn is_current(&self, captured: u64) -> bool {
        self.current() == captured
    }
}

impl Default for ContextVersion {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        let cv = ContextVersion::new();
        assert_eq!(cv.current(), 0);
    }

    #[test]
    fn bump_increments() {
        let cv = ContextVersion::new();
        assert_eq!(cv.bump(), 1);
        assert_eq!(cv.bump(), 2);
        assert_eq!(cv.current(), 2);
    }

    #[test]
    fn is_current_detects_staleness() {
        let cv = ContextVersion::new();
        let snapshot = cv.current();
        assert!(cv.is_current(snapshot));

        cv.bump();
        assert!(!cv.is_current(snapshot));
    }

    #[test]
    fn clone_shares_state() {
        let cv = ContextVersion::new();
        let cv2 = cv.clone();
        cv.bump();
        assert_eq!(cv2.current(), 1);
    }
}
