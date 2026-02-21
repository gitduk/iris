use tokio::sync::watch;

use crate::types::PressureLevel;

/// Resource budget allocation (in MB).
#[derive(Debug, Clone, Copy)]
pub struct ResourceBudget {
    pub external_response_mb: u64,
    pub internal_growth_mb: u64,
    pub maintenance_mb: u64,
}

/// Fast path hard floor (MB).
const FAST_PATH_FLOOR_MB: u64 = 64;
/// Budget recompute interval.
pub const BUDGET_RECOMPUTE_INTERVAL_SECS: u64 = 60;
/// LLM token budget per 60s sliding window.
pub const LLM_TOKEN_BUDGET_60S: u32 = 10_000;
/// Max LLM calls per tick.
pub const MAX_LLM_CALLS_PER_TICK: u32 = 4;

impl ResourceBudget {
    /// Compute budget from total available memory and pressure level.
    pub fn compute(total_available_mb: u64, pressure: PressureLevel) -> Self {
        let (ext_pct, growth_pct, maint_pct) = match pressure {
            PressureLevel::Normal => (60, 20, 20),
            PressureLevel::High => (70, 10, 20),
            PressureLevel::Critical => (80, 0, 20),
        };

        let external = (total_available_mb * ext_pct / 100).max(FAST_PATH_FLOOR_MB);
        let growth = total_available_mb * growth_pct / 100;
        let maintenance = total_available_mb * maint_pct / 100;

        Self {
            external_response_mb: external,
            internal_growth_mb: growth,
            maintenance_mb: maintenance,
        }
    }

    pub fn total(&self) -> u64 {
        self.external_response_mb + self.internal_growth_mb + self.maintenance_mb
    }
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self::compute(512, PressureLevel::Normal)
    }
}

/// Budget watch channel types.
pub type BudgetSender = watch::Sender<ResourceBudget>;
pub type BudgetReceiver = watch::Receiver<ResourceBudget>;

/// Create a budget watch channel with default budget.
pub fn watch_channel() -> (BudgetSender, BudgetReceiver) {
    watch::channel(ResourceBudget::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_normal_allocation() {
        let b = ResourceBudget::compute(1000, PressureLevel::Normal);
        assert_eq!(b.external_response_mb, 600);
        assert_eq!(b.internal_growth_mb, 200);
        assert_eq!(b.maintenance_mb, 200);
    }

    #[test]
    fn budget_critical_no_growth() {
        let b = ResourceBudget::compute(1000, PressureLevel::Critical);
        assert_eq!(b.internal_growth_mb, 0);
        assert_eq!(b.external_response_mb, 800);
    }

    #[test]
    fn budget_fast_path_floor() {
        let b = ResourceBudget::compute(50, PressureLevel::Normal);
        assert_eq!(b.external_response_mb, FAST_PATH_FLOOR_MB);
    }
}
