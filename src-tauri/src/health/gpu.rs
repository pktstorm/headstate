//! The GPU, where the platform will say without being asked nicely.
//!
//! # Why this is a subprocess, and why that is affordable here
//!
//! On macOS the reading comes from `ioreg -r -d 1 -c IOAccelerator`,
//! which spawns a process. That is normally the shape #661 exists to
//! refuse -- a slow command on a timer -- so it was MEASURED before
//! being put on one:
//!
//! | command             | wall time |
//! |---------------------|-----------|
//! | `ioreg -r -d 1 -c IOAccelerator` | ~30 ms |
//! | `pmset -g batt` (already on this timer)  | ~10-40 ms |
//! | `pmset -g therm` (already on this timer) | ~10-20 ms |
//!
//! Five runs each on an M2 Max. `ioreg` is the same order as the two
//! `pmset` calls `collect` already spawns every sample, so this makes
//! the sample about a third more expensive rather than a different kind
//! of expensive. Against the tightest cadence that exists -- the live
//! view's five-second poll -- 30 ms is 0.6% of one core, and the
//! once-a-minute sampler is 0.05%. That is the whole argument for it
//! being here rather than behind a button; `size_worktrees` is behind a
//! button because it takes THIRTEEN SECONDS, which is not this.
//!
//! # Unified memory
//!
//! On Apple Silicon the GPU has no memory of its own -- it shares the
//! same pool `Sample::memory` reports. So `used`/`total` here is a
//! SHARE of that pool, not a second pool beside it, and
//! [`Gpu::unified_memory`] says which so the view can too. Without that
//! flag the GPU panel and the Memory panel look like they disagree
//! about how much memory the machine has.
//!
//! # Absent is not zero
//!
//! A machine with no discoverable GPU returns `None` from [`read`], and
//! a GPU that reports some fields but not others leaves the rest
//! `None`. Nothing here ever substitutes a `0` for "we could not look":
//! 0% utilization is a claim about an idle GPU, which is the opposite
//! of not having read one. See the `health` module docs.
//!
//! # What the other platforms can and cannot do
//!
//! Investigated for #686, and the answers differ enough per vendor that
//! recording them here is worth more than the code they justify. Two of
//! the three are a genuine "cannot", and one is a deliberate "not yet".
//!
//! **Linux/AMD -- READ, here.** `amdgpu` publishes `gpu_busy_percent`
//! and `mem_info_vram_{used,total}` under `/sys/class/drm/card*/device/`.
//! World-readable is not an assumption: the kernel declares them
//! `S_IRUGO` (0444) -- `mem_info_vram_*` via `DEVICE_ATTR` in
//! `amdgpu_vram_mgr.c`, `gpu_busy_percent` via `AMDGPU_DEVICE_ATTR_RO`,
//! which expands to `__ATTR(_name, S_IRUGO, ...)`. Three small file
//! reads, cheaper than any subprocess.
//!
//! **Linux/Intel -- CANNOT, unprivileged.** There is no
//! `busy_percent`-style sysfs file for `i915` or `xe`; utilization
//! lives in their perf PMUs (`i915_pmu.c`, `xe_pmu.c`), both registered
//! as system-wide (`perf_invalid_context`). That is exactly what
//! `perf_event_paranoid` gates, and its default of 2 disallows CPU
//! event access without `CAP_PERFMON`. DRM fdinfo is no way around it:
//! it is per-open-file, so it cannot describe the whole device.
//!
//! **Linux/NVIDIA -- possible, but not built.** Nothing in sysfs or
//! procfs carries utilization (`/proc/driver/nvidia/gpus/*/information`
//! is static identity only). NVML would work and does NOT need root for
//! queries, but it means `dlopen`ing `libnvidia-ml.so.1` and declaring
//! its ABI by hand. Left for when someone with the hardware to test it
//! wants it, rather than shipped unverifiable.
//!
//! **Windows -- possible, but not built.** The counters Task Manager
//! uses (`\GPU Engine(*)\Utilization Percentage`) are reachable
//! unprivileged through PDH. Three things make it more than an
//! afternoon: the instances are per-engine and per-process and have to
//! be ENUMERATED rather than constructed; PDH rate counters need two
//! collections a second apart before the first number exists, so it
//! needs a handle held open across ticks the way `Collector` holds
//! `System`; and summing engines over-reports against Task Manager,
//! which takes the busiest one. (`D3DKMTQueryStatistics` is not the
//! shortcut it looks like -- Microsoft documents it as "Reserved for
//! system use.") Half-built, it would report a confident number that
//! disagreed with the tool beside it, so it reports nothing.
//!
//! **The likely future for all three.** `sysinfo` -- already in this
//! tree -- has GPU support on `master` behind a `gpu` feature, with the
//! same per-vendor matrix reached above and the same `Option`-per-field
//! shape. It is in no release yet (0.39.6 carries no GPU API at all),
//! so it could not be used here. When it ships, the Windows and NVIDIA
//! gaps close for the cost of a version bump rather than a hand-rolled
//! PDH loop, which is the other half of why neither is written now.

use serde::{Deserialize, Serialize};

/// What one GPU is doing, as the UI consumes it.
///
/// Every field but `name` is optional, because the platforms disagree
/// about which of them they will answer. A `None` is "this platform did
/// not report it", never zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Gpu {
    /// The adapter as the platform names it -- "Apple M2 Max", or the
    /// DRM card for an AMD device. Never a path.
    pub name: String,
    /// Whole-device utilization, 0-100.
    pub utilization_percent: Option<f64>,
    /// Bytes the GPU is using right now.
    pub memory_used: Option<u64>,
    /// Bytes the GPU could use. On a unified-memory machine this is the
    /// share currently allocated to the GPU, NOT a dedicated pool --
    /// see `unified_memory`.
    pub memory_total: Option<u64>,
    /// True when the GPU shares the system's memory rather than having
    /// its own (all Apple Silicon). The view MUST say so: otherwise
    /// this panel and the Memory panel look like they disagree about
    /// the size of the machine.
    pub unified_memory: bool,
}

/// Every GPU the platform will describe, or an empty vector.
///
/// Empty means "nothing discoverable", which the view renders as no
/// panel at all rather than as a panel full of zeroes.
pub fn read() -> Vec<Gpu> {
    platform()
}

/// macOS: IOKit's accelerator nodes, through `ioreg`.
///
/// `ioreg` rather than linking IOKit directly: the data is a handful of
/// integers read once a minute, and a text parse of a stable
/// command's output costs one 30 ms subprocess against pulling
/// `core-foundation` and an `unsafe` IOKit traversal into the tree for
/// the same five numbers. If this ever needed to be read at video
/// rates, that trade would flip.
#[cfg(target_os = "macos")]
fn platform() -> Vec<Gpu> {
    let out = std::process::Command::new("ioreg")
        .args(["-r", "-d", "1", "-c", "IOAccelerator"])
        .output();
    let Ok(out) = out else {
        // `ioreg` missing is not a machine we can describe, and an
        // empty list is the honest answer for it.
        return Vec::new();
    };
    parse_ioreg(&String::from_utf8_lossy(&out.stdout))
}

/// Pull the GPUs out of `ioreg`'s plain-text dump.
///
/// One node per `+-o` line; the keys we want are on their own lines
/// under it, except `PerformanceStatistics`, which is a whole dictionary
/// printed inline on ONE line. Split out for the tests, which is the
/// only way to exercise the multi-GPU and missing-key shapes on a
/// machine that has exactly one Apple GPU.
#[cfg(target_os = "macos")]
fn parse_ioreg(text: &str) -> Vec<Gpu> {
    let mut gpus = Vec::new();
    // Fields accumulate until the next node starts, because `model`
    // and `PerformanceStatistics` are on different lines of the same
    // node and either may come first.
    let mut name: Option<String> = None;
    let mut stats: Option<String> = None;

    // A node is finished either by the next node's header or by the end
    // of the text, so the push lives in one function called from both
    // rather than being duplicated.
    //
    // Both fields are `take`n on EVERY path out of here, including the
    // two early returns. That is what stops one node's `model` being
    // attached to the next node's statistics -- a mix-up that would
    // report real numbers under the wrong GPU's name, which is worse
    // than reporting nothing.
    fn finish(name: &mut Option<String>, stats: &mut Option<String>, out: &mut Vec<Gpu>) {
        let stats = stats.take();
        // A node with neither a name nor any statistics is not a GPU
        // worth reporting -- IOAccelerator matches some nodes that
        // carry no performance dictionary at all.
        let Some(stats) = stats else {
            name.take();
            return;
        };
        let name = name.take().unwrap_or_else(|| "GPU".to_string());
        let used = ioreg_number(&stats, "In use system memory");
        let total = ioreg_number(&stats, "Alloc system memory");
        // "Device Utilization %" is the whole device. Renderer and
        // Tiler are the two halves of Apple's pipeline and are reported
        // separately; the device figure is the one that answers "is the
        // GPU busy", so the other two are not surfaced.
        let util = ioreg_number(&stats, "Device Utilization %").map(|n| n as f64);
        if util.is_none() && used.is_none() && total.is_none() {
            return;
        }
        out.push(Gpu {
            name,
            // Clamped: the key is a percentage by construction, but a
            // bar wider than its track for a driver quirk is a layout
            // bug for no gain.
            utilization_percent: util.map(|u| u.clamp(0.0, 100.0)),
            memory_used: used,
            memory_total: total,
            // Every macOS machine this ships to is Apple Silicon, and
            // the `ioreg` keys themselves say "system memory" -- this
            // IS the unified pool.
            unified_memory: true,
        });
    }

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("+-o") {
            finish(&mut name, &mut stats, &mut gpus);
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("\"model\" = ") {
            name = Some(v.trim().trim_matches('"').to_string());
        } else if trimmed.starts_with("\"PerformanceStatistics\" = ") {
            stats = Some(trimmed.to_string());
        }
    }
    finish(&mut name, &mut stats, &mut gpus);
    gpus
}

/// One `"key"=<number>` out of an `ioreg` dictionary line.
///
/// The dictionary is printed as `{"a"=1,"b"=2}`, so the value runs to
/// the next `,` or `}`. Returns `None` for a key that is absent or not
/// a number -- which is the point: a missing key must not become a
/// zero.
#[cfg(target_os = "macos")]
fn ioreg_number(line: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"=");
    let rest = &line[line.find(&needle)? + needle.len()..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

/// Linux: AMD only, and only through sysfs.
///
/// `gpu_busy_percent` and the `mem_info_vram_*` pair are `amdgpu`'s own
/// files, world-readable and a plain integer each -- three small file
/// reads, cheaper than any of the macOS subprocesses.
///
/// Intel and NVIDIA are deliberately not attempted here; see the module
/// docs for why neither has an unprivileged path that can be assumed to
/// exist.
#[cfg(target_os = "linux")]
fn platform() -> Vec<Gpu> {
    let mut gpus = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return gpus;
    };
    // Sorted, because `read_dir` order is the filesystem's and two runs
    // that listed card0 and card1 in different orders would make the
    // panel's rows swap places between polls.
    let mut cards: Vec<_> = entries
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| {
            let n = n.to_string_lossy();
            // "card0", not "card0-DP-1": the connector nodes are
            // outputs, not devices, and carry none of these files.
            n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit())
        })
        .collect();
    cards.sort();

    for card in cards {
        let dir = std::path::Path::new("/sys/class/drm")
            .join(&card)
            .join("device");
        let util = sysfs_number(&dir.join("gpu_busy_percent")).map(|n| n as f64);
        let used = sysfs_number(&dir.join("mem_info_vram_used"));
        let total = sysfs_number(&dir.join("mem_info_vram_total"));
        // A card that answers none of the three is not an AMD device
        // we can read -- an Intel or NVIDIA card has a `device`
        // directory too, just without these files. Skipped rather than
        // listed as a row of "not measured", because a row like that
        // claims we found a GPU and could not read it, when what
        // actually happened is that we cannot read this VENDOR.
        if util.is_none() && used.is_none() && total.is_none() {
            continue;
        }
        gpus.push(Gpu {
            name: card.to_string_lossy().to_string(),
            utilization_percent: util.map(|u: f64| u.clamp(0.0, 100.0)),
            memory_used: used,
            memory_total: total,
            // Discrete VRAM: `mem_info_vram_total` is the card's own
            // pool, separate from system memory.
            unified_memory: false,
        });
    }
    gpus
}

/// One integer out of a sysfs file, or `None`.
///
/// `None` covers all three ways this fails -- the file is absent, it is
/// unreadable, or it holds something that is not a number -- because
/// the caller treats them identically: the platform did not answer.
#[cfg(target_os = "linux")]
fn sysfs_number(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Windows and everything else: not read.
///
/// Not "no GPU" -- a Windows machine plainly has one, and the empty
/// list here means "Headstate did not look", which is why the view
/// draws no panel rather than a panel of zeroes. The module docs
/// explain what the PDH route would cost and why it waits on
/// `sysinfo`'s released GPU support instead.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform() -> Vec<Gpu> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever this machine is, the reading is well-formed.
    ///
    /// Deliberately not asserting that a GPU was FOUND: CI runners are
    /// frequently headless, and a test that demanded a GPU would fail
    /// for a true answer. What is asserted is that anything reported is
    /// a real measurement -- percentages in range, no zero standing in
    /// for an absent field.
    #[test]
    fn whatever_is_reported_is_well_formed() {
        for g in read() {
            assert!(!g.name.is_empty(), "a reported GPU has a name");
            if let Some(u) = g.utilization_percent {
                assert!((0.0..=100.0).contains(&u), "utilization {u}");
            }
            if let (Some(used), Some(total)) = (g.memory_used, g.memory_total) {
                assert!(used <= total, "{used} in use of {total} allocated");
            }
        }
    }

    /// The machine the tests run on has an Apple GPU, and it answers.
    ///
    /// This is the one place the macOS path is asserted end to end
    /// against the real `ioreg`, so a change to the command's output
    /// that broke the parse would fail here rather than silently
    /// emptying the panel.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_finds_a_gpu_and_calls_its_memory_unified() {
        let gpus = read();
        assert!(!gpus.is_empty(), "every Mac has a GPU");
        for g in &gpus {
            assert!(g.unified_memory, "Apple Silicon shares one pool");
        }
    }

    /// The real shape, captured from `ioreg` on an M2 Max.
    ///
    /// A fixture rather than the live command so the parse is pinned:
    /// the live test above proves the command still answers, and this
    /// proves we read it correctly, which a live test cannot because
    /// the numbers change between runs.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_ioreg_dictionary_is_parsed_into_the_fields_we_show() {
        let text = r#"+-o AGXAcceleratorG14X  <class AGXAcceleratorG14X, id 0x1000005da, registered>
    {
      "IOMatchedAtBoot" = Yes
      "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=10952982528,"Tiler Utilization %"=7,"Renderer Utilization %"=7,"Device Utilization %"=7,"In use system memory"=1202913280}
      "model" = "Apple M2 Max"
      "gpu-core-count" = 38
    }
"#;
        let gpus = parse_ioreg(text);
        assert_eq!(gpus.len(), 1);
        let g = &gpus[0];
        assert_eq!(g.name, "Apple M2 Max");
        assert_eq!(g.utilization_percent, Some(7.0));
        assert_eq!(g.memory_used, Some(1_202_913_280));
        assert_eq!(g.memory_total, Some(10_952_982_528));
        assert!(g.unified_memory);
    }

    /// "In use system memory" and "In use system memory (driver)" share
    /// a prefix, the driver figure is 0, and it comes FIRST in the
    /// dictionary.
    ///
    /// This is why the needle carries the closing quote: a prefix match
    /// on `In use system memory` would hit the driver entry first and
    /// report its zero as the GPU's memory use -- a real zero standing
    /// in for a completely different number, which is the most
    /// convincing kind of wrong.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_driver_memory_key_is_not_mistaken_for_the_gpu_one() {
        let line = r#""PerformanceStatistics" = {"In use system memory (driver)"=0,"In use system memory"=1202913280}"#;
        assert_eq!(
            ioreg_number(line, "In use system memory"),
            Some(1_202_913_280),
            "must skip the (driver) entry that precedes it"
        );
        assert_eq!(ioreg_number(line, "In use system memory (driver)"), Some(0));
    }

    /// A node with no `PerformanceStatistics` is not reported at all.
    ///
    /// `IOAccelerator` matches nodes that carry no statistics
    /// dictionary. Reporting one as a GPU with every field absent would
    /// claim we found a device and could not measure it, when there was
    /// no device.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_node_without_statistics_is_not_a_gpu() {
        let text = r#"+-o SomeAcceleratorStub  <class Stub, id 0x100000001, registered>
    {
      "model" = "Not really a GPU"
    }
"#;
        assert!(parse_ioreg(text).is_empty());
    }

    /// Two accelerator nodes come back as two GPUs, each with its own
    /// numbers -- an Intel Mac with discrete and integrated graphics.
    #[cfg(target_os = "macos")]
    #[test]
    fn two_accelerator_nodes_are_two_gpus() {
        let text = r#"+-o IntelAccelerator  <class IntelAccelerator, id 0x1, registered>
    {
      "PerformanceStatistics" = {"Device Utilization %"=3,"In use system memory"=100,"Alloc system memory"=200}
      "model" = "Intel Iris"
    }
+-o AMDAccelerator  <class AMDAccelerator, id 0x2, registered>
    {
      "PerformanceStatistics" = {"Device Utilization %"=61,"In use system memory"=300,"Alloc system memory"=400}
      "model" = "Radeon Pro"
    }
"#;
        let gpus = parse_ioreg(text);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "Intel Iris");
        assert_eq!(gpus[0].utilization_percent, Some(3.0));
        assert_eq!(gpus[1].name, "Radeon Pro");
        assert_eq!(gpus[1].utilization_percent, Some(61.0));
    }

    /// A skipped node does not lend its name to the next one.
    ///
    /// The nodes are parsed as a stream, so a `model` read from a node
    /// that carried no statistics has to be discarded when that node
    /// ends. Left in place it would label the NEXT GPU's real numbers
    /// with the previous node's name -- plausible-looking output
    /// attributed to the wrong device, which is harder to notice than
    /// a missing panel.
    ///
    /// The second node deliberately has NO `model` of its own. With
    /// one, its `model` line would overwrite the stale name before the
    /// node was pushed and the bug would be invisible -- so a fixture
    /// where both nodes are named passes whether or not the discard
    /// happens, and guards nothing. Verified by reintroducing the leak
    /// and watching this fail.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_statless_node_does_not_lend_its_name_to_the_next() {
        let text = r#"+-o Stub  <class Stub, id 0x1, registered>
    {
      "model" = "Not a GPU"
    }
+-o Real  <class Real, id 0x2, registered>
    {
      "PerformanceStatistics" = {"Device Utilization %"=12,"In use system memory"=5,"Alloc system memory"=9}
    }
"#;
        let gpus = parse_ioreg(text);
        assert_eq!(gpus.len(), 1, "the stub is not a GPU");
        assert_eq!(
            gpus[0].name, "GPU",
            "an unnamed node falls back, and must not inherit 'Not a GPU'"
        );
        assert_eq!(gpus[0].utilization_percent, Some(12.0));
    }

    /// A key the dictionary does not carry is absent, NOT zero.
    ///
    /// The single most important assertion in this file: it is the
    /// difference between "this GPU is idle" and "we could not read
    /// this GPU", which the whole page exists to keep apart.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_missing_key_is_absent_rather_than_zero() {
        let line = r#""PerformanceStatistics" = {"Device Utilization %"=7}"#;
        assert_eq!(ioreg_number(line, "Alloc system memory"), None);

        let text = format!(
            "+-o A  <class A, id 0x1, registered>\n    {{\n      {line}\n      \"model\" = \"Partial\"\n    }}\n"
        );
        let gpus = parse_ioreg(&text);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].utilization_percent, Some(7.0));
        assert_eq!(gpus[0].memory_used, None, "absent is not zero");
        assert_eq!(gpus[0].memory_total, None, "absent is not zero");
    }
}
