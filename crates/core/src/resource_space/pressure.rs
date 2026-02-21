use crate::types::PressureLevel;

/// RAM threshold for High pressure.
const RAM_HIGH_THRESHOLD: f64 = 0.70;
/// RAM threshold for Critical pressure.
const RAM_CRITICAL_THRESHOLD: f64 = 0.85;
/// Storage threshold for High pressure.
const STORAGE_HIGH_THRESHOLD: f64 = 0.80;
/// Storage threshold for Critical pressure.
const STORAGE_CRITICAL_THRESHOLD: f64 = 0.90;

/// System resource snapshot.
#[derive(Debug, Clone, Copy)]
pub struct ResourceSnapshot {
    pub ram_usage_ratio: f64,
    pub storage_usage_ratio: f64,
}

/// Evaluate pressure level from resource snapshot.
pub fn evaluate(snapshot: &ResourceSnapshot) -> PressureLevel {
    if snapshot.ram_usage_ratio >= RAM_CRITICAL_THRESHOLD
        || snapshot.storage_usage_ratio >= STORAGE_CRITICAL_THRESHOLD
    {
        PressureLevel::Critical
    } else if snapshot.ram_usage_ratio >= RAM_HIGH_THRESHOLD
        || snapshot.storage_usage_ratio >= STORAGE_HIGH_THRESHOLD
    {
        PressureLevel::High
    } else {
        PressureLevel::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_normal() {
        let snap = ResourceSnapshot { ram_usage_ratio: 0.50, storage_usage_ratio: 0.60 };
        assert_eq!(evaluate(&snap), PressureLevel::Normal);
    }

    #[test]
    fn pressure_high_ram() {
        let snap = ResourceSnapshot { ram_usage_ratio: 0.75, storage_usage_ratio: 0.50 };
        assert_eq!(evaluate(&snap), PressureLevel::High);
    }

    #[test]
    fn pressure_critical_storage() {
        let snap = ResourceSnapshot { ram_usage_ratio: 0.50, storage_usage_ratio: 0.92 };
        assert_eq!(evaluate(&snap), PressureLevel::Critical);
    }

    #[test]
    fn pressure_critical_ram() {
        let snap = ResourceSnapshot { ram_usage_ratio: 0.90, storage_usage_ratio: 0.50 };
        assert_eq!(evaluate(&snap), PressureLevel::Critical);
    }
}
