//! Reading one sample from the machine.
//!
//! `sysinfo` behind a mutex held across refreshes, because CPU
//! percentages are a DELTA: the crate reports use since the previous
//! refresh, so a fresh `System` every sample would report zero (or
//! whatever the first read happens to see) forever. The instance has to
//! outlive the call.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::PowerFlow;
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
///
/// CHARGE comes from `pmset`; CAPACITY and the POWER FLOW come from
/// `ioreg` below. Two subprocesses rather than one because they answer
/// different questions -- see [`ioreg_battery`] for the cost
/// measurement that says both fit on this cadence.
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
    let reading = ioreg_battery();
    Some(Battery {
        percent,
        on_ac,
        capacity_percent: reading.capacity_percent,
        cycle_count: reading.cycle_count,
        power: reading.power,
    })
}

/// What one `ioreg -r -c AppleSmartBattery` read yields.
///
/// A struct rather than a tuple because it is now three unrelated
/// things -- capacity, cycles and the live power flow -- and a
/// three-tuple at the call site is a positional puzzle where two of the
/// members are `Option<f64>`.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct IoregBattery {
    capacity_percent: Option<f64>,
    cycle_count: Option<u32>,
    power: Option<PowerFlow>,
}

/// Capacity relative to design, the cycle count behind it, and the
/// power currently flowing in or out of the cell (#773).
///
/// # The field that looks right and is not
///
/// The obvious reading is `MaxCapacity / DesignCapacity`, and on Apple
/// Silicon it is WRONG. Measured on an M2 Max: the top-level
/// `MaxCapacity` is `100` -- a normalised percentage, not mAh -- while
/// `DesignCapacity` is `8694` mAh. Dividing them yields 1%, which would
/// tell every Apple Silicon user their battery is dead.
///
/// `NominalChargeCapacity` is the mAh figure that pairs with
/// `DesignCapacity` (7381/8694 = 85% on the same machine, which matches
/// what System Information reports). So that is the pair used, and a
/// machine that publishes no `NominalChargeCapacity` gets `None` rather
/// than a plausible-looking number from the wrong field.
///
/// # Why a subprocess is allowed here
///
/// `ioreg -r -c AppleSmartBattery` measured at ~30-40 ms across three
/// runs on an M2 Max -- the same order as the two `pmset` calls this
/// sample already spawns and as the `ioreg` in `health::gpu`. That is
/// the #661 arithmetic: it is milliseconds against a 60-second sampler
/// and a 5-second live poll, so it stays on the timer. A read that cost
/// seconds would belong behind an explicit button instead, the way
/// `size_worktrees` does.
///
/// The power flow is FREE on top of that: it is the same invocation and
/// the same output, three more keys off text already in hand. That is
/// the whole reason #773 is cheap enough to sit on this cadence rather
/// than behind a button.
#[cfg(target_os = "macos")]
fn ioreg_battery() -> IoregBattery {
    let Ok(out) = std::process::Command::new("ioreg")
        .args(["-r", "-c", "AppleSmartBattery"])
        .output()
    else {
        return IoregBattery::default();
    };
    parse_ioreg_battery(&String::from_utf8_lossy(&out.stdout))
}

/// The parse, separated from the subprocess so it can be tested.
///
/// Every interesting case here is a machine state this one is not in:
/// a discharging battery, a cell above its design capacity, a platform
/// publishing only some of the keys. None of them can be arranged by
/// running a command, so the parse takes text and the tests hand it
/// text -- the same split `health::gpu` uses for the same reason.
#[cfg(target_os = "macos")]
fn parse_ioreg_battery(text: &str) -> IoregBattery {
    // Only TOP-LEVEL keys: `BatteryData` is a nested dictionary printed
    // on one line that contains its own `DesignCapacity` and
    // `CycleCount`, so a substring search would match inside it. A
    // top-level key is printed alone on its line as `"Key" = 1234`,
    // which is what `trim` plus an exact prefix match requires.
    let raw = |key: &str| -> Option<u64> {
        let want = format!("\"{key}\" = ");
        text.lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix(&want))
            .and_then(|v| v.trim().parse::<u64>().ok())
    };

    let design = raw("DesignCapacity");
    let nominal = raw("NominalChargeCapacity");
    let capacity_percent = match (nominal, design) {
        // A zero design capacity is not a 100%-worn battery, it is a
        // machine that did not answer -- and it would divide by zero.
        (Some(n), Some(d)) if d > 0 => Some((n as f64 / d as f64) * 100.0),
        _ => None,
    };

    IoregBattery {
        capacity_percent,
        cycle_count: raw("CycleCount").map(|c| c as u32),
        power: power_flow(&raw),
    }
}

/// The live power flow, in watts, from the same `ioreg` dump.
///
/// # `Amperage` is SIGNED, and `ioreg` prints it unsigned
///
/// This is the trap #773 exists to name. IOKit stores the current as a
/// signed 32/64-bit integer -- positive charging, negative discharging
/// -- and `ioreg` prints the raw bit pattern as an unsigned decimal. A
/// discharge therefore arrives as a number near 2^64. The proof is in
/// the same dump on a CHARGING machine, where nothing is discharging at
/// all: `"MaximumDischargeCurrent" = 18446744073709540729`, which
/// reinterpreted as `i64` is -10887 mA -- a plausible ~10.9 A ceiling,
/// and obvious nonsense as 18 quintillion.
///
/// So every current is put back through `as i64`, which is a
/// bit-preserving reinterpretation rather than a conversion. Without
/// it, a laptop on battery would report roughly 233 quintillion watts,
/// and -- worse -- it would look completely correct on any machine that
/// happened to be plugged in while the code was written.
///
/// # `Amperage`, not `InstantAmperage`
///
/// Both keys are published and they agreed exactly on the machine this
/// was measured on (both 1008 mA charging, and 560 and 668 mA on later
/// reads -- always equal to each other). They are not required to:
/// `InstantAmperage` is the gauge's spot reading and `Amperage` is the
/// smoothed one Apple's own power management consumes.
///
/// `Amperage` is chosen because this number is rendered live at a
/// five-second poll and charted over 24 hours. A spot reading swings
/// with whatever the CPU did in the last instant, so the figure would
/// jitter by watts between repaints and the chart would be noise rather
/// than shape -- and the question a reader brings to this panel ("am I
/// draining, and how fast") is about a trend, not an instant.
/// `InstantAmperage` is deliberately not surfaced: two nearly-identical
/// wattages on one panel invite a comparison that means nothing.
///
/// # Absent is not zero, here more than anywhere
///
/// A machine that publishes no `Amperage` gets `None`, not 0 W. Zero
/// watts is a REAL and common reading -- a full battery on mains draws
/// nothing -- so a fabricated zero here is indistinguishable from a
/// measurement, which is exactly the confusion `NotMeasured` exists to
/// prevent.
#[cfg(target_os = "macos")]
fn power_flow(raw: &dyn Fn(&str) -> Option<u64>) -> Option<PowerFlow> {
    // The bit-preserving reinterpretation described above. `as i64` on
    // a `u64` is defined in Rust as exactly this -- it is not a
    // saturating conversion -- which is what makes it the right cast
    // and not merely a convenient one.
    let milliamps = raw("Amperage")? as i64;
    let millivolts = raw("Voltage")? as i64;
    // A non-positive voltage is a battery that did not answer, not one
    // sitting at zero volts, and multiplying by it would report 0 W for
    // a cell that is genuinely charging.
    if millivolts <= 0 {
        return None;
    }
    Some(PowerFlow {
        // mA * mV = microwatts, so the divisor is 1e6. Kept as one
        // expression rather than two so the units cannot drift apart.
        watts: (milliamps as f64 * millivolts as f64) / 1_000_000.0,
        milliamps,
        millivolts,
    })
}

/// Battery on Linux, through `/sys/class/power_supply` (#773).
///
/// # Why this exists now and did not before
///
/// Until #773 this function returned `None` on every non-macOS
/// platform, and a Linux laptop was therefore told it had no battery --
/// which is the same class of untruth as a fabricated zero, just
/// wearing the other mask. #773 needed the RATE on Linux, and a rate is
/// unreadable without the charge beside it, so the whole reading
/// arrives together.
///
/// # It is file reads, not a subprocess
///
/// Four to six `read_to_string` calls against sysfs, which is a virtual
/// filesystem: no process is spawned, and the cost is well under the
/// ~30 ms macOS pays for one `ioreg`. The same #661 arithmetic that
/// admits `ioreg` to this cadence admits this several times over.
///
/// # Which files, and what they are guaranteed to be
///
/// The `power_supply` sysfs class is a documented kernel ABI
/// (`Documentation/ABI/testing/sysfs-class-power`), world-readable, and
/// present for any driver that registers a battery. The units are fixed
/// by that document: `*_now` currents are microamps, voltages
/// microvolts, powers microwatts, energies microwatt-hours and charges
/// microamp-hours.
///
/// # Two units for the same fact
///
/// Drivers publish EITHER a charge pair (`charge_*`, in µAh, the ACPI
/// and most-laptops case) OR an energy pair (`energy_*`, in µWh, what
/// several drivers report instead). Both are tried, charge first,
/// because a machine that publishes only one and got `None` would show
/// no capacity figure for no reason.
///
/// # Rate: `current_now` is UNSIGNED here, unlike macOS
///
/// The opposite trap from [`power_flow`], and worth stating so the two
/// are never made to match. Linux publishes MAGNITUDE in `current_now`
/// and DIRECTION in `status` ("Charging" / "Discharging" / "Full" /
/// "Not charging"), so the sign is applied from the status string
/// rather than reinterpreted from the bits. Reading this one as two's
/// complement, or the macOS one as unsigned, would each produce a
/// confidently wrong number.
#[cfg(target_os = "linux")]
fn battery() -> Option<Battery> {
    // The first directory of `type` "Battery". Enumerated rather than
    // assuming `BAT0`: a machine can number its internal cell `BAT1`,
    // and the same directory also holds the AC adapter and every
    // connected peripheral's battery.
    let dir = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            std::fs::read_to_string(p.join("type"))
                .map(|t| t.trim() == "Battery")
                .unwrap_or(false)
        })?;

    let text = |name: &str| -> Option<String> {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .map(|s| s.trim().to_string())
    };
    let num = |name: &str| -> Option<f64> { text(name)?.parse::<f64>().ok() };

    // `capacity` is the charge percentage the kernel has already
    // computed, which is what every desktop environment shows. The
    // ratio is the fallback for a driver that publishes the pair but
    // not the rounded figure.
    let percent = num("capacity").or_else(|| {
        let (now, full) = (num("charge_now")?, num("charge_full")?);
        (full > 0.0).then(|| (now / full) * 100.0)
    })?;

    let status = text("status").unwrap_or_default();
    // AC is the absence of an active discharge rather than the presence
    // of an adapter: "Full", "Charging" and "Not charging" all mean the
    // mains is carrying the machine, and only "Discharging" means it is
    // not. Reading the adapter's own `online` file would need the
    // adapter located too, for the same answer.
    let on_ac = status != "Discharging";

    // Capacity relative to design, from whichever pair the driver
    // publishes. Same rule as macOS: a zero denominator is a driver
    // that did not answer, not a fully-worn cell.
    let capacity_percent = [
        ("charge_full", "charge_full_design"),
        ("energy_full", "energy_full_design"),
    ]
    .into_iter()
    .find_map(|(full, design)| {
        let (f, d) = (num(full)?, num(design)?);
        (d > 0.0).then(|| (f / d) * 100.0)
    });

    Some(Battery {
        percent,
        on_ac,
        capacity_percent,
        // A driver that does not track cycles publishes 0 rather than
        // omitting the file. Zero cycles on a battery that has been
        // charged is not a reading, so it is treated as absent.
        cycle_count: num("cycle_count").and_then(|c| (c > 0.0).then_some(c as u32)),
        power: linux_power(&num, &status),
    })
}

/// The power flow on Linux, in watts (#773).
///
/// `power_now` is the direct answer in microwatts where the driver
/// publishes it. Where it does not, `current_now` (µA) times
/// `voltage_now` (µV) is the same figure -- the identical arithmetic
/// macOS does, one unit prefix along.
///
/// # The sign comes from `status`, never from the bits
///
/// See the note on [`battery`]: these files carry MAGNITUDE, so a
/// discharging laptop and a charging one publish the same positive
/// number and are told apart only by the status string. The convention
/// returned matches macOS -- positive INTO the cell, negative out of it
/// -- so the UI has one rule for both platforms.
///
/// A status that is neither charging nor discharging ("Full", "Not
/// charging", or a driver publishing nothing) yields `None`. The
/// direction is genuinely unknown there, and putting a sign on a
/// number that has no direction is the kind of confident wrongness
/// this module refuses.
#[cfg(target_os = "linux")]
fn linux_power(num: &dyn Fn(&str) -> Option<f64>, status: &str) -> Option<PowerFlow> {
    let sign = match status {
        "Charging" => 1.0,
        "Discharging" => -1.0,
        _ => return None,
    };
    let millivolts = num("voltage_now").map(|v| (v / 1000.0) as i64)?;
    // A non-positive voltage is a driver that did not answer, not a
    // cell sitting at zero volts, and every branch below divides or
    // multiplies by it.
    if millivolts <= 0 {
        return None;
    }
    let (watts, milliamps) = match (num("power_now"), num("current_now")) {
        // Microwatts, straight from the driver. The current is derived
        // back out of it only so the panel shows the same three figures
        // on both platforms.
        (Some(uw), _) if uw > 0.0 => {
            let w = sign * uw / 1_000_000.0;
            (w, (w / millivolts as f64 * 1_000_000.0) as i64)
        }
        (_, Some(ua)) => {
            let ma = sign * ua / 1000.0;
            (ma * millivolts as f64 / 1_000_000.0, ma as i64)
        }
        _ => return None,
    };
    Some(PowerFlow {
        watts,
        milliamps,
        millivolts,
    })
}

/// No battery reading on Windows, and the view says so rather than
/// implying the machine has none (#705's rule).
///
/// Windows publishes charge through `GetSystemPowerStatus`
/// (`BatteryLifePercent`, `ACLineStatus`) and the richer per-battery
/// figures -- including a rate -- through WMI's `Win32_Battery` and the
/// `IOCTL_BATTERY_QUERY_STATUS` interface. Both are real unprivileged
/// routes, so this is a "not built", never a "cannot": the same verdict
/// `health::gpu` records for Windows GPUs, and for the same reason --
/// `sysinfo` is the natural home for it, and hand-rolling a WMI query
/// for a handful of numbers is work that would be thrown away.
///
/// Until then `None` is honest and a 0 would not be.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

    /// Capacity is a capacity, or it is absent -- never a number from
    /// the wrong field.
    ///
    /// The specific failure guarded is the `MaxCapacity` trap in
    /// [`parse_ioreg_battery`]: on Apple Silicon that key is the literal `100`
    /// while `DesignCapacity` is thousands of mAh, so the obvious
    /// division yields about 1% and would tell every user their battery
    /// is dead. A plausible RANGE is therefore the assertion -- a
    /// capacity in single digits on a machine that boots is the bug,
    /// not a reading.
    ///
    /// Not asserting a capacity was FOUND: CI runners and desktops have
    /// no battery, and absent is the honest answer there.
    #[cfg(target_os = "macos")]
    #[test]
    fn capacity_is_a_capacity_or_it_is_absent() {
        let reading = ioreg_battery();
        let (capacity, cycles) = (reading.capacity_percent, reading.cycle_count);
        if let Some(c) = capacity {
            assert!(
                (10.0..=110.0).contains(&c),
                "capacity of {c}% -- a figure this low is the MaxCapacity \
                 trap, not a worn battery"
            );
        }
        if let Some(n) = cycles {
            // A battery with tens of thousands of cycles would mean the
            // parse picked up something that is not a cycle count.
            assert!(n < 10_000, "implausible cycle count {n}");
        }
        // Charge and capacity are different numbers: a battery that
        // reports both must not report them as the same value by
        // accident of the parse. Only checkable when the machine
        // actually has a battery and is not sitting at exactly its
        // capacity figure, so this is an existence check on the pair
        // rather than an inequality.
        if let Some(b) = battery() {
            assert!((0.0..=100.0).contains(&b.percent));
            assert_eq!(
                b.capacity_percent, capacity,
                "the sample must carry the same capacity this function reads"
            );
        }
    }

    /// The battery read is cheap enough for the cadence it is on.
    ///
    /// The #661 guard, in the same executable form as
    /// `reading_the_gpu_is_cheap_enough_for_the_sampler` above and for
    /// the same reason: #720 added a SECOND subprocess (`ioreg`) to
    /// this sample, and a subprocess per sample is exactly the shape
    /// that belongs behind a button rather than on a timer once it
    /// costs real time.
    ///
    /// Measured at ~30-40 ms on an M2 Max across three runs, against a
    /// 60-second sampler and a 5-second live poll. The bound is
    /// deliberately loose -- a slow CI runner must not fail the build
    /// for being slow -- but it still catches a regression that made
    /// this seconds rather than milliseconds.
    #[test]
    fn reading_the_battery_is_cheap_enough_for_the_sampler() {
        let start = std::time::Instant::now();
        let _ = battery();
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(1000),
            "a battery read took {elapsed:?}, which does not belong on a 5s poll"
        );
    }

    /// A synthetic `ioreg -r -c AppleSmartBattery` dump.
    ///
    /// Hand-written rather than captured. The real output of that
    /// command carries the machine's battery SERIAL NUMBER and its
    /// manufacturer's device name, and this repository is public --
    /// `CONTRIBUTING.md` allows synthetic fixtures only. The keys, the
    /// spacing and the nesting below are the real SHAPE; the numbers
    /// are chosen for the case each test is about.
    ///
    /// `amperage` is passed as the raw text `ioreg` would print, which
    /// is the point of the whole fixture: a discharge is printed as an
    /// enormous unsigned decimal, and nothing about it looks unusual
    /// until it is reinterpreted.
    #[cfg(target_os = "macos")]
    fn ioreg_dump(amperage: &str) -> String {
        format!(
            r#"+-o AppleSmartBattery  <class AppleSmartBattery>
    {{
      "Amperage" = {amperage}
      "ExternalConnected" = Yes
      "BatteryData" = {{"MaxCapacity"=100,"DesignCapacity"=8694,"CycleCount"=414,"Voltage"=12651,"MaximumDischargeCurrent"=18446744073709540729}}
      "NominalChargeCapacity" = 7326
      "MaxCapacity" = 100
      "InstantAmperage" = {amperage}
      "IsCharging" = Yes
      "DesignCapacity" = 8694
      "Voltage" = 12654
      "CycleCount" = 414
    }}
"#
        )
    }

    /// A CHARGING machine reports positive watts (#773).
    ///
    /// The easy direction, and the one that would pass with the bug
    /// this file's sign handling exists to prevent -- which is exactly
    /// why the discharge case below is the one that matters. Kept as
    /// the pair, so a change that broke charging to fix discharging
    /// could not land quietly.
    ///
    /// 1008 mA at 12654 mV is 12.76 W, the reading measured on the
    /// machine #773 was written from.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_charging_battery_reports_power_flowing_in() {
        let r = parse_ioreg_battery(&ioreg_dump("1008"));
        let p = r.power.expect("the dump publishes Amperage and Voltage");
        assert!(p.watts > 0.0, "charging is positive, got {} W", p.watts);
        assert!(
            (p.watts - 12.76).abs() < 0.01,
            "1008 mA at 12654 mV is 12.76 W, got {} W",
            p.watts
        );
        assert_eq!(p.milliamps, 1008);
        assert_eq!(p.millivolts, 12654);
    }

    /// A DISCHARGING machine reports negative watts, not 233 quintillion
    /// of them (#773).
    ///
    /// **This is the test the issue is about.** `ioreg` prints
    /// `Amperage` as an unsigned decimal even though IOKit stores it
    /// signed, so a discharge of 1008 mA arrives as
    /// `18446744073709550608` -- which is 2^64 - 1008. Parsed as a
    /// `u64` and multiplied by the voltage it yields roughly 2.3e20
    /// watts, and the failure is invisible on any machine that happens
    /// to be plugged in, which is most machines most of the time and
    /// certainly the one the feature is developed on.
    ///
    /// The evidence that this encoding is real, from a dump taken while
    /// CHARGING (so nothing was discharging at all):
    /// `"MaximumDischargeCurrent" = 18446744073709540729`, which as an
    /// `i64` is -10887 mA -- a plausible ~10.9 A ceiling, and obvious
    /// nonsense as 18 quintillion.
    ///
    /// The assertion is deliberately not just `watts < 0.0`: a bug that
    /// negated the magnitude and lost the scale would satisfy that, so
    /// the MAGNITUDE is pinned too.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_discharging_battery_reports_watts_and_not_a_quintillion_of_them() {
        // 2^64 - 1008: exactly what `ioreg` prints for -1008 mA.
        let r = parse_ioreg_battery(&ioreg_dump("18446744073709550608"));
        let p = r.power.expect("the dump publishes Amperage and Voltage");
        assert_eq!(
            p.milliamps, -1008,
            "an unsigned Amperage must be reinterpreted as i64"
        );
        assert!(p.watts < 0.0, "discharging is negative, got {} W", p.watts);
        assert!(
            (p.watts + 12.76).abs() < 0.01,
            "-1008 mA at 12654 mV is -12.76 W, got {} W -- a figure far \
             from this is the unsigned-Amperage bug",
            p.watts
        );
    }

    /// A battery publishing no current reports no power, not zero watts.
    ///
    /// The absent-is-not-zero rule, and this is the field where it is
    /// hardest to see: zero watts is a REAL and ordinary reading -- a
    /// full battery on mains draws nothing -- so a fabricated 0 W is
    /// indistinguishable from a measurement rather than merely
    /// implausible.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_battery_that_reports_no_current_reports_no_power() {
        let dump = ioreg_dump("1008").replace("\"Amperage\" = 1008\n", "");
        let r = parse_ioreg_battery(&dump);
        assert!(r.power.is_none(), "absent is never zero watts");
        // The rest of the reading survives: one missing key must not
        // cost the panel its capacity figure.
        assert!(r.capacity_percent.is_some());
    }

    /// The capacity parse still reads only TOP-LEVEL keys.
    ///
    /// The fixture's `BatteryData` line carries its own `DesignCapacity`
    /// and `CycleCount`, printed inline on one line -- which is what the
    /// real command does and what a substring search would match
    /// inside. 7326/8694 is 84.3%, so a parse that picked up the nested
    /// values or the normalised `MaxCapacity` of 100 would miss it
    /// wildly.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_capacity_parse_ignores_the_nested_dictionary() {
        let r = parse_ioreg_battery(&ioreg_dump("1008"));
        let c = r.capacity_percent.expect("the dump publishes the pair");
        assert!(
            (c - 84.26).abs() < 0.05,
            "7326/8694 is 84.3%, got {c}% -- the MaxCapacity trap gives 1%"
        );
        assert_eq!(r.cycle_count, Some(414));
    }

    /// A cell measuring ABOVE its nameplate parses as above 100% (#772).
    ///
    /// The number is not clamped anywhere in this file, and must not
    /// be. `DesignCapacity` is a figure the manufacturer guarantees, not
    /// a ceiling, so a new cell routinely measures a few points over it
    /// -- 8955/8694 is 103%. Clamping that to 100 here would be a lie
    /// that looks like tidiness, and it would hide the very reading
    /// #772 is about from the panel that has to word it correctly.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_new_cell_above_its_design_capacity_is_not_clamped() {
        let dump = ioreg_dump("1008").replace(
            "\"NominalChargeCapacity\" = 7326",
            "\"NominalChargeCapacity\" = 8955",
        );
        let c = parse_ioreg_battery(&dump)
            .capacity_percent
            .expect("the dump publishes the pair");
        assert!(
            c > 100.0,
            "8955/8694 is 103%, got {c}% -- a cell above nameplate must \
             not be clamped to 100"
        );
        assert!((c - 103.0).abs() < 0.1, "expected ~103%, got {c}%");
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
