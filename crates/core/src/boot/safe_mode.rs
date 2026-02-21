use std::time::{Duration, Instant};

/// Safe mode state machine.
///
/// Entry: 3 consecutive core boot failures.
/// Exit: N consecutive healthy ticks AND cooldown elapsed.
#[derive(Debug)]
pub struct SafeMode {
    active: bool,
    entered_at: Option<Instant>,
    consecutive_healthy: u32,
    recovery_ticks: u32,
    cooldown: Duration,
}

impl SafeMode {
    pub fn new() -> Self {
        Self::with_params(5, 300)
    }

    pub fn with_params(recovery_ticks: u32, cooldown_secs: u64) -> Self {
        Self {
            active: false,
            entered_at: None,
            consecutive_healthy: 0,
            recovery_ticks,
            cooldown: Duration::from_secs(cooldown_secs),
        }
    }

    /// Enter safe mode (core-only operation).
    pub fn enter(&mut self) {
        self.active = true;
        self.entered_at = Some(Instant::now());
        self.consecutive_healthy = 0;
    }

    /// Record a healthy tick. Returns true if safe mode was exited.
    pub fn record_healthy_tick(&mut self) -> bool {
        if !self.active {
            return false;
        }
        self.consecutive_healthy += 1;
        if self.can_exit() {
            self.active = false;
            self.entered_at = None;
            self.consecutive_healthy = 0;
            true
        } else {
            false
        }
    }

    /// Record an unhealthy tick — resets the healthy counter.
    pub fn record_unhealthy_tick(&mut self) {
        self.consecutive_healthy = 0;
    }

    /// Check if exit conditions are met.
    fn can_exit(&self) -> bool {
        if self.consecutive_healthy < self.recovery_ticks {
            return false;
        }
        match self.entered_at {
            Some(t) => t.elapsed() >= self.cooldown,
            None => false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn consecutive_healthy(&self) -> u32 {
        self.consecutive_healthy
    }

    /// For testing: enter with a custom timestamp.
    #[cfg(test)]
    fn enter_at(&mut self, at: Instant) {
        self.active = true;
        self.entered_at = Some(at);
        self.consecutive_healthy = 0;
    }
}

impl Default for SafeMode {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_active_by_default() {
        let sm = SafeMode::new();
        assert!(!sm.is_active());
    }

    #[test]
    fn enter_activates() {
        let mut sm = SafeMode::new();
        sm.enter();
        assert!(sm.is_active());
    }

    #[test]
    fn healthy_ticks_alone_not_enough() {
        let mut sm = SafeMode::new();
        sm.enter(); // entered just now, cooldown not elapsed
        for _ in 0..10 {
            sm.record_healthy_tick();
        }
        // Still active because cooldown hasn't elapsed
        assert!(sm.is_active());
    }

    #[test]
    fn unhealthy_resets_counter() {
        let mut sm = SafeMode::new();
        sm.enter();
        sm.record_healthy_tick();
        sm.record_healthy_tick();
        sm.record_unhealthy_tick();
        assert_eq!(sm.consecutive_healthy(), 0);
    }

    #[test]
    fn exits_after_cooldown_and_healthy_ticks() {
        let mut sm = SafeMode::new();
        // Enter 6 minutes ago (past the 5-min cooldown)
        sm.enter_at(Instant::now() - Duration::from_secs(360));
        for _ in 0..4 {
            assert!(!sm.record_healthy_tick());
        }
        // 5th healthy tick should trigger exit
        assert!(sm.record_healthy_tick());
        assert!(!sm.is_active());
    }
}
