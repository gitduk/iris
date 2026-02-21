/// Battery status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    /// On AC power or battery level unknown.
    Unknown,
    /// Running on battery with approximate percentage.
    OnBattery(u8),
    /// Plugged in / charging.
    Charging(u8),
}

impl BatteryState {
    /// Returns true if battery is below the given threshold.
    pub fn is_low(&self, threshold_pct: u8) -> bool {
        match self {
            BatteryState::OnBattery(pct) => *pct < threshold_pct,
            _ => false,
        }
    }
}

/// Network reachability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    Online,
    Offline,
    Unknown,
}

/// Combined hardware snapshot.
#[derive(Debug, Clone, Copy)]
pub struct HardwareSnapshot {
    pub battery: BatteryState,
    pub network: NetworkState,
}

impl Default for HardwareSnapshot {
    fn default() -> Self {
        Self {
            battery: BatteryState::Unknown,
            network: NetworkState::Unknown,
        }
    }
}

/// Degradation thresholds from PLAN.md §3.12.
pub const BATTERY_LOW_THRESHOLD: u8 = 20;
/// CPU high threshold — 3 consecutive samples above this → pause intrinsic tasks.
pub const CPU_HIGH_THRESHOLD: f64 = 0.85;
/// Number of consecutive high-CPU samples before degradation.
pub const CPU_HIGH_CONSECUTIVE: usize = 3;

/// Degradation signal emitted by the watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradationSignal {
    /// Battery low — tick interval should increase.
    BatteryLow,
    /// CPU sustained high — pause intrinsic tasks.
    CpuSustainedHigh,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_low_detection() {
        assert!(BatteryState::OnBattery(15).is_low(20));
        assert!(!BatteryState::OnBattery(50).is_low(20));
        assert!(!BatteryState::Charging(10).is_low(20));
        assert!(!BatteryState::Unknown.is_low(20));
    }

    #[test]
    fn hardware_snapshot_default() {
        let snap = HardwareSnapshot::default();
        assert_eq!(snap.battery, BatteryState::Unknown);
        assert_eq!(snap.network, NetworkState::Unknown);
    }
}
