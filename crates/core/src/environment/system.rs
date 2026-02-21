/// System-level environment info (OS, CPU, RAM).
#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub os_name: String,
    pub cpu_count: usize,
    pub total_ram_mb: u64,
}

impl SystemInfo {
    /// Gather system info from the current host.
    pub fn gather() -> Self {
        Self {
            os_name: std::env::consts::OS.to_string(),
            cpu_count: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            total_ram_mb: Self::read_total_ram_mb(),
        }
    }

    #[cfg(target_os = "linux")]
    fn read_total_ram_mb() -> u64 {
        // Read from /proc/meminfo; fallback to 0
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .and_then(|l| {
                        l.split_whitespace().nth(1)?.parse::<u64>().ok()
                    })
            })
            .map(|kb| kb / 1024)
            .unwrap_or(0)
    }

    #[cfg(not(target_os = "linux"))]
    fn read_total_ram_mb() -> u64 {
        // Non-Linux: return 0, caller should handle gracefully
        0
    }
}

/// CPU usage sample (0.0 – 1.0).
#[derive(Debug, Clone, Copy)]
pub struct CpuSample {
    pub usage_ratio: f64,
}

/// Stateful CPU sampler — computes usage delta between ticks from /proc/stat.
#[derive(Debug)]
pub struct CpuSampler {
    prev_idle: u64,
    prev_total: u64,
}

impl CpuSampler {
    pub fn new() -> Self {
        let (idle, total) = Self::read_proc_stat();
        Self { prev_idle: idle, prev_total: total }
    }

    /// Sample current CPU usage as a ratio (0.0–1.0).
    /// Computes delta since last call.
    pub fn sample(&mut self) -> CpuSample {
        let (idle, total) = Self::read_proc_stat();
        let d_idle = idle.saturating_sub(self.prev_idle);
        let d_total = total.saturating_sub(self.prev_total);
        self.prev_idle = idle;
        self.prev_total = total;

        let usage_ratio = if d_total == 0 {
            0.0
        } else {
            1.0 - (d_idle as f64 / d_total as f64)
        };
        CpuSample { usage_ratio: usage_ratio.clamp(0.0, 1.0) }
    }

    #[cfg(target_os = "linux")]
    fn read_proc_stat() -> (u64, u64) {
        // First line of /proc/stat: cpu user nice system idle iowait irq softirq ...
        std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| {
                let line = s.lines().next()?;
                let vals: Vec<u64> = line.split_whitespace()
                    .skip(1) // skip "cpu"
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if vals.len() >= 4 {
                    let idle = vals[3]; // idle field
                    let total: u64 = vals.iter().sum();
                    Some((idle, total))
                } else {
                    None
                }
            })
            .unwrap_or((0, 0))
    }

    #[cfg(not(target_os = "linux"))]
    fn read_proc_stat() -> (u64, u64) {
        (0, 0)
    }
}

impl Default for CpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// RAM usage snapshot.
#[derive(Debug, Clone, Copy)]
pub struct RamSnapshot {
    pub used_mb: u64,
    pub total_mb: u64,
}

impl RamSnapshot {
    pub fn usage_ratio(&self) -> f64 {
        if self.total_mb == 0 {
            return 0.0;
        }
        self.used_mb as f64 / self.total_mb as f64
    }

    /// Sample current RAM usage from the system.
    pub fn sample() -> Self {
        let (total, available) = Self::read_meminfo();
        Self {
            total_mb: total,
            used_mb: total.saturating_sub(available),
        }
    }

    #[cfg(target_os = "linux")]
    fn read_meminfo() -> (u64, u64) {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .map(|s| {
                let mut total_kb = 0u64;
                let mut avail_kb = 0u64;
                for line in s.lines() {
                    if line.starts_with("MemTotal:") {
                        total_kb = line.split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                    } else if line.starts_with("MemAvailable:") {
                        avail_kb = line.split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                    }
                }
                (total_kb / 1024, avail_kb / 1024)
            })
            .unwrap_or((0, 0))
    }

    #[cfg(not(target_os = "linux"))]
    fn read_meminfo() -> (u64, u64) {
        (0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_info_gather() {
        let info = SystemInfo::gather();
        assert!(!info.os_name.is_empty());
        assert!(info.cpu_count >= 1);
    }

    #[test]
    fn ram_snapshot_ratio() {
        let snap = RamSnapshot { used_mb: 400, total_mb: 1000 };
        assert!((snap.usage_ratio() - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn ram_snapshot_zero_total() {
        let snap = RamSnapshot { used_mb: 0, total_mb: 0 };
        assert!((snap.usage_ratio()).abs() < f64::EPSILON);
    }
}
