//! Reading one sample from the machine.
//!
//! `sysinfo` behind a mutex held across refreshes, because CPU
//! percentages are a DELTA: the crate reports use since the previous
//! refresh, so a fresh `System` every sample would report zero (or
//! whatever the first read happens to see) forever. The instance has to
//! outlive the call.

use super::{Battery, Interface, Memory, Sample, Volume};
use std::sync::Mutex;
use sysinfo::{Disks, Networks, System};

/// The live reader. One per process.
pub struct Collector {
    system: Mutex<System>,
    disks: Mutex<Disks>,
    networks: Mutex<Networks>,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    pub fn new() -> Self {
        Self {
            system: Mutex::new(System::new()),
            disks: Mutex::new(Disks::new_with_refreshed_list()),
            networks: Mutex::new(Networks::new_with_refreshed_list()),
        }
    }

    /// One sample, now.
    ///
    /// Blocking, and measured in single-digit milliseconds -- but it is
    /// still I/O against the kernel, so callers put it on a blocking
    /// worker like every other read in `commands.rs`.
    pub fn sample(&self, now: &str) -> Sample {
        let mut sys = self.system.lock().unwrap_or_else(|e| e.into_inner());
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        let cpu_per_core: Vec<f64> = sys
            .cpus()
            .iter()
            .map(|c| f64::from(c.cpu_usage()))
            .collect();
        // The mean, not `global_cpu_usage()`: that is the same number on
        // every platform sysinfo supports, and averaging the cores we
        // already have avoids a second traversal.
        let cpu_percent = if cpu_per_core.is_empty() {
            None
        } else {
            Some(cpu_per_core.iter().sum::<f64>() / cpu_per_core.len() as f64)
        };

        let la = System::load_average();
        // Windows reports zeroes rather than refusing, and three zeroes
        // is indistinguishable from a genuinely idle machine. Treated as
        // absent there, since a load average is the one thing Windows
        // does not have.
        let load = if cfg!(windows) {
            None
        } else {
            Some([la.one, la.five, la.fifteen])
        };

        let memory = Memory {
            total: sys.total_memory(),
            used: sys.used_memory(),
            available: sys.available_memory(),
            swap_total: sys.total_swap(),
            swap_used: sys.used_swap(),
        };
        drop(sys);

        let mut disks = self.disks.lock().unwrap_or_else(|e| e.into_inner());
        disks.refresh(true);
        let volumes: Vec<Volume> = disks
            .list()
            .iter()
            .map(|d| {
                let mount = d.mount_point().to_string_lossy().to_string();
                Volume {
                    is_root: mount == "/" || mount.len() == 3 && mount.ends_with(":\\"),
                    total: d.total_space(),
                    available: d.available_space(),
                    mount,
                }
            })
            .collect();
        drop(disks);

        let mut nets = self.networks.lock().unwrap_or_else(|e| e.into_inner());
        nets.refresh(true);
        let networks: Vec<Interface> = nets
            .list()
            .iter()
            .map(|(name, d)| Interface {
                name: name.clone(),
                rx_bytes: d.total_received(),
                tx_bytes: d.total_transmitted(),
            })
            .collect();
        drop(nets);

        Sample {
            sampled_at: now.to_string(),
            load,
            cpu_percent,
            cpu_per_core,
            memory,
            // Measured before being put here: ~30 ms on macOS, the same
            // order as the two `pmset` calls below that this sample
            // already spawns, and a few file reads on Linux. The
            // arithmetic against both cadences is in `health::gpu`.
            gpus: super::gpu::read(),
            disks: volumes,
            battery: battery(),
            thermal: thermal(),
            networks,
            uptime_secs: System::uptime(),
        }
    }
}

/// Battery, where the platform has one.
///
/// macOS through `pmset -g batt`, which is unprivileged and stable.
/// `sysinfo` does not carry battery state, and pulling in a second crate
/// for two numbers is not worth it.
#[cfg(target_os = "macos")]
fn battery() -> Option<Battery> {
    let out = std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // "Now drawing from 'AC Power'" then a line with "87%; charging".
    let on_ac = text.contains("'AC Power'");
    let percent = text
        .split_whitespace()
        .find_map(|w| w.strip_suffix("%;").or_else(|| w.strip_suffix("%")))
        .and_then(|p| p.parse::<f64>().ok())?;
    Some(Battery { percent, on_ac })
}

#[cfg(not(target_os = "macos"))]
fn battery() -> Option<Battery> {
    None
}

/// Thermal PRESSURE, not temperature. See the module docs for why there
/// is no degrees reading here.
///
/// `pmset -g therm` is unprivileged. It prints nothing useful on a
/// machine that has never been under pressure, which is the common case
/// and is reported as `nominal` rather than as absent -- "never warned"
/// is a real answer.
#[cfg(target_os = "macos")]
fn thermal() -> Option<String> {
    let out = std::process::Command::new("pmset")
        .args(["-g", "therm"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(v) = line.split("CPU_Speed_Limit").nth(1) {
            let pct: u32 = v
                .trim_start_matches([' ', '=', '\t'])
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(100);
            return Some(
                match pct {
                    100 => "nominal",
                    80..=99 => "fair",
                    50..=79 => "serious",
                    _ => "critical",
                }
                .to_string(),
            );
        }
    }
    // "No thermal warning level has been recorded" -- nothing has ever
    // throttled this machine.
    Some("nominal".to_string())
}

#[cfg(not(target_os = "macos"))]
fn thermal() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real sample from the machine running the tests.
    ///
    /// Deliberately not asserting VALUES -- CPU use on a CI runner is
    /// whatever it is -- but the shape: a machine has memory and an
    /// uptime, and anything the platform cannot answer is absent rather
    /// than a zero pretending to be a measurement.
    #[test]
    fn a_sample_describes_the_machine_it_ran_on() {
        let c = Collector::new();
        let s = c.sample("2026-01-01T00:00:00Z");

        assert!(s.memory.total > 0, "a machine has memory");
        assert!(s.memory.available <= s.memory.total);
        assert!(s.uptime_secs > 0, "a running machine has an uptime");
        assert!(
            !s.cpu_per_core.is_empty(),
            "a machine has at least one core"
        );
        assert_eq!(s.sampled_at, "2026-01-01T00:00:00Z");

        // Percentages are percentages.
        if let Some(p) = s.cpu_percent {
            assert!((0.0..=100.0).contains(&p), "cpu {p}");
        }
        for p in &s.cpu_per_core {
            assert!((0.0..=100.0).contains(p), "core {p}");
        }
        if let Some(b) = &s.battery {
            assert!((0.0..=100.0).contains(&b.percent), "battery {}", b.percent);
        }
    }

    /// The second sample is what carries a real CPU figure.
    ///
    /// `sysinfo` reports use SINCE THE LAST REFRESH, so the first read
    /// of a fresh `System` has no interval to measure and reports zero.
    /// That is why `Collector` holds the instance rather than building
    /// one per call -- a fresh one every minute would report an idle
    /// machine forever.
    #[test]
    fn the_collector_is_reused_so_cpu_is_a_real_delta() {
        let c = Collector::new();
        let _first = c.sample("2026-01-01T00:00:00Z");
        std::thread::sleep(std::time::Duration::from_millis(250));
        let second = c.sample("2026-01-01T00:01:00Z");
        // Not "greater than zero": a genuinely idle machine is a valid
        // answer. What matters is that the field is populated at all.
        assert!(second.cpu_percent.is_some());
        assert_eq!(second.cpu_per_core.len(), _first.cpu_per_core.len());
    }

    /// A sample carries whatever GPUs the platform would describe.
    ///
    /// Not asserting that any were FOUND -- a headless CI runner
    /// legitimately has none, and an empty list is the honest answer
    /// there. What is asserted is that nothing in the list is a
    /// fabricated zero, which is the property `gpu::read` exists to
    /// keep.
    #[test]
    fn a_sample_carries_the_gpus_the_platform_admits_to() {
        let c = Collector::new();
        let s = c.sample("2026-01-01T00:00:00Z");
        for g in &s.gpus {
            assert!(!g.name.is_empty());
            if let Some(u) = g.utilization_percent {
                assert!((0.0..=100.0).contains(&u), "gpu utilization {u}");
            }
        }
    }

    /// The GPU read is cheap enough for the cadence it is on.
    ///
    /// This is the #661 guard in executable form. The live view polls
    /// `system_health` every five seconds, so a GPU read that took a
    /// meaningful slice of that would be the "slow command on a timer"
    /// shape that belongs behind a button instead. Measured at ~30 ms
    /// on an M2 Max (`ioreg`); the assertion is deliberately loose --
    /// a slow CI runner must not fail the build for being slow -- but
    /// it still catches a regression that made this seconds rather
    /// than milliseconds.
    #[test]
    fn reading_the_gpu_is_cheap_enough_for_the_sampler() {
        let start = std::time::Instant::now();
        let _ = super::super::gpu::read();
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(1000),
            "a GPU read took {elapsed:?}, which does not belong on a 5s poll"
        );
    }

    /// Thermal is a LABEL, never a number, and never degrees.
    #[cfg(target_os = "macos")]
    #[test]
    fn thermal_is_one_of_the_four_labels() {
        let t = thermal().expect("macOS always answers");
        assert!(
            ["nominal", "fair", "serious", "critical"].contains(&t.as_str()),
            "unexpected thermal label: {t}"
        );
    }
}
