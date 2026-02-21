use crate::types::CapabilityState;

/// Error when an invalid state transition is attempted.
#[derive(Debug, Clone)]
pub struct InvalidTransition {
    pub from: CapabilityState,
    pub to: CapabilityState,
}

impl std::fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid transition: {:?} → {:?}", self.from, self.to)
    }
}

impl std::error::Error for InvalidTransition {}

/// Validate a capability state transition per the lifecycle state machine.
///
/// Valid transitions:
///   staged → active_candidate (self-test passed)
///   staged → quarantined (self-test failed)
///   active_candidate → confirmed (10 min stable run)
///   active_candidate → quarantined (crash after restart failure)
///   confirmed → quarantined (regression failure)
///   confirmed → retired (user-confirmed retirement)
///   quarantined → staged (new version fix)
///   quarantined → retired (quarantine_count >= 3, user-confirmed)
pub fn validate_transition(
    from: CapabilityState,
    to: CapabilityState,
) -> Result<(), InvalidTransition> {
    let valid = matches!(
        (from, to),
        (CapabilityState::Staged, CapabilityState::ActiveCandidate)
            | (CapabilityState::Staged, CapabilityState::Quarantined)
            | (CapabilityState::ActiveCandidate, CapabilityState::Confirmed)
            | (CapabilityState::ActiveCandidate, CapabilityState::Quarantined)
            | (CapabilityState::Confirmed, CapabilityState::Quarantined)
            | (CapabilityState::Confirmed, CapabilityState::Retired)
            | (CapabilityState::Quarantined, CapabilityState::Staged)
            | (CapabilityState::Quarantined, CapabilityState::Retired)
    );
    if valid {
        Ok(())
    } else {
        Err(InvalidTransition { from, to })
    }
}

/// Maximum quarantine count before a capability should be retired.
pub const MAX_QUARANTINE_COUNT: i32 = 3;

/// Check if a quarantined capability should be retired.
pub fn should_retire(quarantine_count: i32) -> bool {
    quarantine_count >= MAX_QUARANTINE_COUNT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions() {
        let valid_pairs = [
            (CapabilityState::Staged, CapabilityState::ActiveCandidate),
            (CapabilityState::Staged, CapabilityState::Quarantined),
            (CapabilityState::ActiveCandidate, CapabilityState::Confirmed),
            (CapabilityState::ActiveCandidate, CapabilityState::Quarantined),
            (CapabilityState::Confirmed, CapabilityState::Quarantined),
            (CapabilityState::Confirmed, CapabilityState::Retired),
            (CapabilityState::Quarantined, CapabilityState::Staged),
            (CapabilityState::Quarantined, CapabilityState::Retired),
        ];
        for (from, to) in &valid_pairs {
            assert!(
                validate_transition(*from, *to).is_ok(),
                "expected {:?} → {:?} to be valid",
                from,
                to
            );
        }
    }

    #[test]
    fn invalid_transitions() {
        let invalid_pairs = [
            (CapabilityState::Staged, CapabilityState::Confirmed),
            (CapabilityState::Staged, CapabilityState::Retired),
            (CapabilityState::ActiveCandidate, CapabilityState::Staged),
            (CapabilityState::ActiveCandidate, CapabilityState::Retired),
            (CapabilityState::Confirmed, CapabilityState::Staged),
            (CapabilityState::Confirmed, CapabilityState::ActiveCandidate),
            (CapabilityState::Retired, CapabilityState::Staged),
            (CapabilityState::Quarantined, CapabilityState::Confirmed),
        ];
        for (from, to) in &invalid_pairs {
            assert!(
                validate_transition(*from, *to).is_err(),
                "expected {:?} → {:?} to be invalid",
                from,
                to
            );
        }
    }

    #[test]
    fn should_retire_at_threshold() {
        assert!(!should_retire(0));
        assert!(!should_retire(2));
        assert!(should_retire(3));
        assert!(should_retire(5));
    }
}
