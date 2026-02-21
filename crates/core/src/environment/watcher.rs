use std::collections::VecDeque;

use crate::environment::hardware::{
    CPU_HIGH_CONSECUTIVE, CPU_HIGH_THRESHOLD, DegradationSignal,
    HardwareSnapshot, BATTERY_LOW_THRESHOLD,
};
use crate::environment::system::CpuSample;

/// Watches environment samples and emits degradation signals.
#[derive(Debug)]
pub struct EnvironmentWatcher {
    /// Rolling window of recent CPU samples.
    cpu_history: VecDeque<f64>,
    /// Maximum history length.
    max_history: usize,
    /// Last known hardware state.
    last_hardware: HardwareSnapshot,
}

impl EnvironmentWatcher {
    pub fn new() -> Self {
        Self {
            cpu_history: VecDeque::with_capacity(CPU_HIGH_CONSECUTIVE + 1),
            max_history: CPU_HIGH_CONSECUTIVE + 1,
            last_hardware: HardwareSnapshot::default(),
        }
    }

    /// Feed a CPU sample and hardware snapshot; returns any degradation signals.
    pub fn update(
        &mut self,
        cpu: CpuSample,
        hw: HardwareSnapshot,
    ) -> Vec<DegradationSignal> {
        self.last_hardware = hw;

        // Track CPU history
        if self.cpu_history.len() >= self.max_history {
            self.cpu_history.pop_front();
        }
        self.cpu_history.push_back(cpu.usage_ratio);

        let mut signals = Vec::new();

        // Battery low check
        if hw.battery.is_low(BATTERY_LOW_THRESHOLD) {
            signals.push(DegradationSignal::BatteryLow);
        }

        // CPU sustained high check
        if self.cpu_history.len() >= CPU_HIGH_CONSECUTIVE
            && self
                .cpu_history
                .iter()
                .rev()
                .take(CPU_HIGH_CONSECUTIVE)
                .all(|&r| r > CPU_HIGH_THRESHOLD)
        {
            signals.push(DegradationSignal::CpuSustainedHigh);
        }

        signals
    }

    /// Current hardware snapshot.
    pub fn hardware(&self) -> &HardwareSnapshot {
        &self.last_hardware
    }
}

impl Default for EnvironmentWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::hardware::{BatteryState, NetworkState};

    fn hw(battery: BatteryState) -> HardwareSnapshot {
        HardwareSnapshot { battery, network: NetworkState::Online }
    }

    #[test]
    fn no_signals_normal() {
        let mut w = EnvironmentWatcher::new();
        let signals = w.update(
            CpuSample { usage_ratio: 0.5 },
            hw(BatteryState::OnBattery(80)),
        );
        assert!(signals.is_empty());
    }

    #[test]
    fn battery_low_signal() {
        let mut w = EnvironmentWatcher::new();
        let signals = w.update(
            CpuSample { usage_ratio: 0.3 },
            hw(BatteryState::OnBattery(15)),
        );
        assert!(signals.contains(&DegradationSignal::BatteryLow));
    }

    #[test]
    fn cpu_sustained_high() {
        let mut w = EnvironmentWatcher::new();
        let normal_hw = hw(BatteryState::Charging(100));
        // Feed 3 high CPU samples
        w.update(CpuSample { usage_ratio: 0.90 }, normal_hw);
        w.update(CpuSample { usage_ratio: 0.88 }, normal_hw);
        let signals = w.update(CpuSample { usage_ratio: 0.92 }, normal_hw);
        assert!(signals.contains(&DegradationSignal::CpuSustainedHigh));
    }

    #[test]
    fn cpu_high_resets_on_normal() {
        let mut w = EnvironmentWatcher::new();
        let normal_hw = hw(BatteryState::Charging(100));
        w.update(CpuSample { usage_ratio: 0.90 }, normal_hw);
        w.update(CpuSample { usage_ratio: 0.90 }, normal_hw);
        // One normal sample breaks the streak
        w.update(CpuSample { usage_ratio: 0.50 }, normal_hw);
        let signals = w.update(CpuSample { usage_ratio: 0.90 }, normal_hw);
        assert!(!signals.contains(&DegradationSignal::CpuSustainedHigh));
    }
}
