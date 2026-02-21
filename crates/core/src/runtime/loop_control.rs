use std::time::Duration;
use crate::config::IrisCfg;

/// Tick frequency mode based on system state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickMode {
    Normal,
    Idle,
    Rest,
}

impl TickMode {
    pub fn interval(self, cfg: &IrisCfg) -> Duration {
        match self {
            Self::Normal => Duration::from_millis(cfg.tick_ms_normal),
            Self::Idle => Duration::from_millis(cfg.tick_ms_idle),
            Self::Rest => Duration::from_millis(cfg.tick_ms_rest),
        }
    }
}

/// Determines the next tick mode based on runtime conditions.
pub fn next_mode(has_external_events: bool, has_pending_tasks: bool, energy: f32) -> TickMode {
    if energy < 0.2 && !has_external_events {
        TickMode::Rest
    } else if has_external_events || has_pending_tasks {
        TickMode::Normal
    } else {
        TickMode::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_transitions() {
        assert_eq!(next_mode(true, false, 1.0), TickMode::Normal);
        assert_eq!(next_mode(false, true, 1.0), TickMode::Normal);
        assert_eq!(next_mode(false, false, 1.0), TickMode::Idle);
        assert_eq!(next_mode(false, false, 0.1), TickMode::Rest);
        // external events override rest
        assert_eq!(next_mode(true, false, 0.1), TickMode::Normal);
    }
}
