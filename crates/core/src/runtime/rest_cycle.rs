use std::time::Instant;

/// RestMode state machine.
/// Tracks entry/exit conditions and rest duration for the runtime.
///
/// Entry: energy < 0.2 && no external events (handled by loop_control::next_mode)
/// Exit: energy >= 0.8 || external input arrives || max rest duration exceeded
#[derive(Debug)]
pub struct RestCycle {
    active: bool,
    entered_at: Option<Instant>,
    rest_ticks: u64,
    /// Maximum rest ticks before forced wake (prevents infinite sleep).
    max_rest_ticks: u64,
    /// Energy threshold to exit rest mode.
    wake_energy: f32,
}

impl RestCycle {
    pub fn new() -> Self {
        Self {
            active: false,
            entered_at: None,
            rest_ticks: 0,
            max_rest_ticks: 300, // ~10 min at 2000ms tick
            wake_energy: 0.8,
        }
    }

    /// Enter rest mode.
    pub fn enter(&mut self) {
        if !self.active {
            self.active = true;
            self.entered_at = Some(Instant::now());
            self.rest_ticks = 0;
            tracing::info!("entering rest mode");
        }
    }

    /// Record a rest tick. Returns true if rest mode should continue.
    pub fn tick(&mut self) -> bool {
        if !self.active {
            return false;
        }
        self.rest_ticks += 1;
        if self.rest_ticks >= self.max_rest_ticks {
            tracing::info!(rest_ticks = self.rest_ticks, "max rest duration reached, waking");
            self.exit();
            return false;
        }
        true
    }

    /// Check if rest mode should exit based on energy or external input.
    /// Returns true if we should wake up.
    pub fn should_wake(&self, energy: f32, has_external_events: bool) -> bool {
        if !self.active {
            return false;
        }
        energy >= self.wake_energy || has_external_events
    }

    /// Exit rest mode.
    pub fn exit(&mut self) {
        if self.active {
            let duration = self.entered_at.map(|t| t.elapsed());
            tracing::info!(?duration, rest_ticks = self.rest_ticks, "exiting rest mode");
            self.active = false;
            self.entered_at = None;
            self.rest_ticks = 0;
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn rest_ticks(&self) -> u64 {
        self.rest_ticks
    }
}

impl Default for RestCycle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_cycle_enter_exit() {
        let mut rc = RestCycle::new();
        assert!(!rc.is_active());

        rc.enter();
        assert!(rc.is_active());
        assert_eq!(rc.rest_ticks(), 0);

        rc.exit();
        assert!(!rc.is_active());
    }

    #[test]
    fn rest_cycle_tick_counts() {
        let mut rc = RestCycle::new();
        rc.enter();
        assert!(rc.tick());
        assert!(rc.tick());
        assert_eq!(rc.rest_ticks(), 2);
    }

    #[test]
    fn rest_cycle_max_ticks_forces_wake() {
        let mut rc = RestCycle::new();
        rc.max_rest_ticks = 3;
        rc.enter();
        assert!(rc.tick());
        assert!(rc.tick());
        assert!(!rc.tick()); // 3rd tick → forced wake
        assert!(!rc.is_active());
    }

    #[test]
    fn rest_cycle_should_wake_on_energy() {
        let mut rc = RestCycle::new();
        rc.enter();
        assert!(!rc.should_wake(0.5, false));
        assert!(rc.should_wake(0.85, false));
    }

    #[test]
    fn rest_cycle_should_wake_on_external_input() {
        let mut rc = RestCycle::new();
        rc.enter();
        assert!(!rc.should_wake(0.1, false));
        assert!(rc.should_wake(0.1, true));
    }

    #[test]
    fn rest_cycle_not_active_no_wake() {
        let rc = RestCycle::new();
        assert!(!rc.should_wake(1.0, true));
    }
}
