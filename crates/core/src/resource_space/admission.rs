use crate::resource_space::budget::BudgetReceiver;

/// Estimated resource cost for a task spawn.
#[derive(Debug, Clone, Copy)]
pub struct ResourceEstimate {
    pub memory_mb: u64,
    pub is_external: bool,
}

/// Admission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionResult {
    Admitted,
    Rejected,
}

/// Check whether a task with the given estimate can be admitted
/// under the current budget.
pub fn check(rx: &BudgetReceiver, estimate: ResourceEstimate) -> AdmissionResult {
    let budget = *rx.borrow();

    let available = if estimate.is_external {
        budget.external_response_mb
    } else {
        budget.internal_growth_mb
    };

    if estimate.memory_mb <= available {
        AdmissionResult::Admitted
    } else {
        AdmissionResult::Rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_space::budget;
    use crate::types::PressureLevel;

    #[test]
    fn admits_within_budget() {
        let (_tx, rx) = budget::watch_channel();
        // Default budget: 512 MB Normal → external = 307 MB
        let est = ResourceEstimate { memory_mb: 100, is_external: true };
        assert_eq!(check(&rx, est), AdmissionResult::Admitted);
    }

    #[test]
    fn rejects_over_budget() {
        let (tx, rx) = budget::watch_channel();
        // Set a critical budget with 0 growth
        let critical = crate::resource_space::budget::ResourceBudget::compute(
            200,
            PressureLevel::Critical,
        );
        tx.send(critical).unwrap();
        let est = ResourceEstimate { memory_mb: 10, is_external: false };
        // Critical → internal_growth_mb = 0, so 10 > 0 → rejected
        assert_eq!(check(&rx, est), AdmissionResult::Rejected);
    }

    #[test]
    fn admits_external_under_critical() {
        let (tx, rx) = budget::watch_channel();
        let critical = crate::resource_space::budget::ResourceBudget::compute(
            200,
            PressureLevel::Critical,
        );
        tx.send(critical).unwrap();
        // Critical 200 MB → external = 160 MB (80%)
        let est = ResourceEstimate { memory_mb: 100, is_external: true };
        assert_eq!(check(&rx, est), AdmissionResult::Admitted);
    }
}
