//! Machine detection and the engine thread/hash policy.
//!
//! The policy is backed by measurements on an Apple M4 (4 performance + 6 efficiency
//! cores, 16 GB); see `docs/ENGINE.md`. In short: nodes/s keep growing up to 8 threads
//! but 6 already give ~87 % of the peak while leaving the rest of the machine to the
//! browser, screen capture and overlay, and the hash table stops paying off beyond a
//! few hundred MB for interactive analysis (2 GB and more even slows the search down).

use crate::uci::EngineConfig;

/// What we know about the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hardware {
    /// Logical processors available to this process.
    pub logical_cores: u32,
    /// Performance cores on hybrid CPUs (`None` if unknown or not hybrid-aware).
    pub perf_cores: Option<u32>,
    /// Installed physical memory in MiB.
    pub ram_mb: u64,
}

/// Upper bounds: beyond these Stockfish gains little for interactive analysis.
const MAX_THREADS: u32 = 16;
const MIN_HASH_MB: u64 = 16;
const MAX_HASH_MB: u64 = 1024;

impl Hardware {
    pub fn detect() -> Hardware {
        let logical = std::thread::available_parallelism()
            .map(|n| u32::try_from(n.get()).unwrap_or(u32::MAX))
            .unwrap_or(2)
            .max(1);
        Hardware {
            logical_cores: logical,
            perf_cores: detect_perf_cores(logical),
            ram_mb: detect_ram_mb().unwrap_or(4096),
        }
    }
}

#[cfg(target_os = "macos")]
fn sysctl(name: &str) -> Option<String> {
    let out = std::process::Command::new("sysctl")
        .args(["-n", name])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(target_os = "macos")]
fn detect_perf_cores(logical: u32) -> Option<u32> {
    // Present on Apple silicon; missing on Intel Macs.
    let p: u32 = sysctl("hw.perflevel0.physicalcpu")?.parse().ok()?;
    (p > 0 && p <= logical).then_some(p)
}

#[cfg(not(target_os = "macos"))]
fn detect_perf_cores(_logical: u32) -> Option<u32> {
    // Windows/Linux hybrid detection needs per-core efficiency classes; treated as unknown.
    None
}

#[cfg(target_os = "macos")]
fn detect_ram_mb() -> Option<u64> {
    let bytes: u64 = sysctl("hw.memsize")?.parse().ok()?;
    Some(bytes / (1024 * 1024))
}

#[cfg(target_os = "linux")]
fn detect_ram_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kb / 1024)
}

#[cfg(windows)]
fn detect_ram_mb() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; dwLength is set as the API requires.
    unsafe {
        let mut s: MEMORYSTATUSEX = std::mem::zeroed();
        s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        (GlobalMemoryStatusEx(&mut s) != 0).then_some(s.ullTotalPhys / (1024 * 1024))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn detect_ram_mb() -> Option<u64> {
    None
}

/// Largest power of two `<= v` (`v >= 1`).
fn floor_pow2(v: u64) -> u64 {
    1u64 << (63 - v.max(1).leading_zeros())
}

/// Search threads for `hw`, leaving headroom for everything else on the machine.
fn recommended_threads(hw: &Hardware) -> u32 {
    let logical = hw.logical_cores.max(1);
    let mut t = match hw.perf_cores {
        // Hybrid CPU: all performance cores plus a third of the efficiency cores
        // (M4: 4P + 6E -> 6 threads, ~87 % of the 8-thread peak nodes/s).
        Some(p) if p < logical => p + (logical - p) / 3,
        // Uniform CPU: keep max(2, a third) of the cores free.
        _ => logical.saturating_sub(2.max(logical.div_ceil(3))),
    };
    t = t.clamp(1, MAX_THREADS);
    // Tiny machines: never more than one search thread per 2 GB.
    let by_ram = u32::try_from((hw.ram_mb / 2048).max(1)).unwrap_or(u32::MAX);
    t.min(by_ram)
}

/// Hash size (MiB, power of two): 1/32 of RAM, within [16, 1024].
fn recommended_hash_mb(hw: &Hardware) -> u32 {
    let h = floor_pow2((hw.ram_mb / 32).max(1)).clamp(MIN_HASH_MB, MAX_HASH_MB);
    u32::try_from(h).unwrap_or(1024)
}

/// The measured-best configuration for `hw`: full strength, no depth limit, MultiPV 3.
pub fn recommended_config(hw: &Hardware) -> EngineConfig {
    EngineConfig {
        threads: recommended_threads(hw),
        hash_mb: recommended_hash_mb(hw),
        multi_pv: 3,
        depth_limit: None,
        move_time_ms: None,
        skill_level: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(logical: u32, perf: Option<u32>, ram_mb: u64) -> (u32, u32) {
        let c = recommended_config(&Hardware {
            logical_cores: logical,
            perf_cores: perf,
            ram_mb,
        });
        assert_eq!(c.multi_pv, 3);
        assert_eq!(c.depth_limit, None);
        assert_eq!(c.skill_level, None);
        assert!(c.hash_mb.is_power_of_two());
        (c.threads, c.hash_mb)
    }

    #[test]
    fn two_core_laptop_4gb() {
        assert_eq!(cfg(2, None, 4096), (1, 128));
    }

    #[test]
    fn eight_core_16gb() {
        assert_eq!(cfg(8, None, 16 * 1024), (5, 512));
    }

    #[test]
    fn m4_4p_6e_16gb() {
        assert_eq!(cfg(10, Some(4), 16 * 1024), (6, 512));
    }

    #[test]
    fn big_workstation_is_capped() {
        assert_eq!(cfg(32, None, 64 * 1024), (16, 1024));
    }

    #[test]
    fn degenerate_inputs_stay_sane() {
        assert_eq!(cfg(1, None, 512), (1, 16));
        assert_eq!(cfg(4, None, 8 * 1024), (2, 256));
        // perf cores equal to logical cores behaves like a uniform CPU.
        assert_eq!(cfg(8, Some(8), 16 * 1024), (5, 512));
    }

    #[test]
    fn detect_returns_something_usable() {
        let hw = Hardware::detect();
        assert!(hw.logical_cores >= 1);
        assert!(hw.ram_mb >= 256);
        let c = recommended_config(&hw);
        assert!(c.threads >= 1 && c.hash_mb >= 16);
    }
}
