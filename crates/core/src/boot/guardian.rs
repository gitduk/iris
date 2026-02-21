use std::fmt;

/// Boot phases from PLAN.md §3.12:
/// CoreInit → CapabilityLoad → EnvironmentSense → Ready
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootPhase {
    CoreInit,
    CapabilityLoad,
    EnvironmentSense,
    Ready,
}

impl fmt::Display for BootPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootPhase::CoreInit => write!(f, "CoreInit"),
            BootPhase::CapabilityLoad => write!(f, "CapabilityLoad"),
            BootPhase::EnvironmentSense => write!(f, "EnvironmentSense"),
            BootPhase::Ready => write!(f, "Ready"),
        }
    }
}

/// Consecutive failure threshold before entering safe mode.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Tracks boot attempts and decides whether to enter safe mode.
#[derive(Debug)]
pub struct BootGuardian {
    consecutive_failures: u32,
    current_phase: BootPhase,
    total_boots: u64,
}

impl BootGuardian {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            current_phase: BootPhase::CoreInit,
            total_boots: 0,
        }
    }

    /// Advance to the next boot phase. Returns the new phase.
    pub fn advance(&mut self) -> BootPhase {
        self.current_phase = match self.current_phase {
            BootPhase::CoreInit => BootPhase::CapabilityLoad,
            BootPhase::CapabilityLoad => BootPhase::EnvironmentSense,
            BootPhase::EnvironmentSense => BootPhase::Ready,
            BootPhase::Ready => BootPhase::Ready,
        };
        self.current_phase
    }

    /// Record a successful boot (reached Ready phase).
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.total_boots += 1;
    }

    /// Record a boot failure at the current phase.
    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        // Reset phase for next attempt
        self.current_phase = BootPhase::CoreInit;
    }

    /// Whether safe mode should be entered (3 consecutive failures).
    pub fn should_enter_safe_mode(&self) -> bool {
        self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES
    }

    pub fn current_phase(&self) -> BootPhase {
        self.current_phase
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn total_boots(&self) -> u64 {
        self.total_boots
    }
}

impl Default for BootGuardian {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_phase_sequence() {
        let mut g = BootGuardian::new();
        assert_eq!(g.current_phase(), BootPhase::CoreInit);
        assert_eq!(g.advance(), BootPhase::CapabilityLoad);
        assert_eq!(g.advance(), BootPhase::EnvironmentSense);
        assert_eq!(g.advance(), BootPhase::Ready);
        // Stays at Ready
        assert_eq!(g.advance(), BootPhase::Ready);
    }

    #[test]
    fn safe_mode_after_three_failures() {
        let mut g = BootGuardian::new();
        assert!(!g.should_enter_safe_mode());
        g.record_failure();
        g.record_failure();
        assert!(!g.should_enter_safe_mode());
        g.record_failure();
        assert!(g.should_enter_safe_mode());
    }

    #[test]
    fn success_resets_failure_count() {
        let mut g = BootGuardian::new();
        g.record_failure();
        g.record_failure();
        g.record_success();
        assert_eq!(g.consecutive_failures(), 0);
        assert!(!g.should_enter_safe_mode());
    }

    #[test]
    fn failure_resets_phase() {
        let mut g = BootGuardian::new();
        g.advance(); // CapabilityLoad
        g.record_failure();
        assert_eq!(g.current_phase(), BootPhase::CoreInit);
    }
}
