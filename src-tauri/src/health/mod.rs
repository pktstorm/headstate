//! What the machine is doing, and what Headstate is costing it.
//!
//! One sample a minute while the app runs, kept for 24 hours
//! (`store::health`). The view reads the current sample live and the
//! series for its charts.
//!
//! # What is NOT here
//!
//! **Temperature in degrees.** On macOS the SMC is only readable with
//! elevated privileges -- `powermetrics --samplers smc` requires sudo,
//! and there is no unprivileged path -- so a degrees reading would work
//! only for a user who ran the app as root. What IS available is the
//! platform's thermal PRESSURE, a coarse label, and that is what
//! [`Sample::thermal`] carries. The UI says so rather than letting a
//! label that looks like a temperature imply one.
//!
//! **The GPU, on most platforms.** macOS answers through IOKit and
//! Linux answers for AMD cards through sysfs. Intel on Linux genuinely
//! cannot be read without `CAP_PERFMON`; Windows and NVIDIA-on-Linux
//! could be, but each needs machinery (a PDH loop, a hand-declared NVML
//! ABI) that `sysinfo` is about to provide for free. [`Sample::gpus`]
//! is empty on all of them and the view draws no panel, which is the
//! same rule as everything else here. [`gpu`] carries the evidence
//! behind each verdict.
//!
//! # Absent is not zero
//!
//! Every optional field is `None` when the platform does not expose it,
//! never `0.0`. A zero that means "not measured" reads as a real
//! measurement, which is the failure `packages::run::missing_tool`
//! exists to avoid on the other side of the app: "no updates" and "the
//! check did not run" are opposite answers.
//!
//! # The two halves of "what Headstate costs"
//!
//! [`Sample`] is the machine. [`Footprint`] (`footprint`) is this app's
//! own share of it -- our process, the `git`/`gh`/Docker subprocesses we
//! spawn, and the Docker daemon we keep resident. That is the LIVE half
//! only: the disk half of the same panel is `size_worktrees`,
//! `size_artifacts`, `size_venvs` and `docker_disk_usage`, which already
//! exist and are slow enough (~13s, #661) that they must stay behind an
//! explicit "Measure" and out of the sampler.

use serde::{Deserialize, Serialize};

pub mod collect;
pub mod footprint;
pub mod gpu;

pub use footprint::Footprint;
pub use gpu::Gpu;

/// One moment, as the UI consumes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    /// RFC 3339, matching every other timestamp this app stores.
    pub sampled_at: String,
    /// 1, 5 and 15 minute load averages. `None` on platforms without
    /// them (Windows has no equivalent).
    pub load: Option<[f64; 3]>,
    /// Whole-machine CPU use, 0-100.
    pub cpu_percent: Option<f64>,
    /// Per-core, 0-100, in the platform's own core order.
    pub cpu_per_core: Vec<f64>,
    pub memory: Memory,
    /// Every GPU the platform will describe, which on several is none.
    ///
    /// Empty is "nothing discoverable", and the view draws no panel for
    /// it rather than a panel of zeroes -- see `health::gpu` for which
    /// platforms can be read unprivileged and which cannot.
    ///
    /// A `Vec` rather than an `Option<Gpu>` because a machine can have
    /// two (an Intel Mac with integrated and discrete graphics), and
    /// collapsing them would have to pick one and hide the other.
    pub gpus: Vec<Gpu>,
    pub disks: Vec<Volume>,
    pub battery: Option<Battery>,
    /// `nominal`, `fair`, `serious`, `critical` -- NOT degrees. See the
    /// module docs.
    pub thermal: Option<String>,
    pub networks: Vec<Interface>,
    /// Seconds since boot.
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    /// What the OS believes is reclaimable, which is not `total - used`
    /// on any modern platform: cache counts as used and is available.
    pub available: u64,
    /// Swap, where the platform reports it.
    pub swap_total: u64,
    pub swap_used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Volume {
    pub mount: String,
    pub total: u64,
    pub available: u64,
    /// True for the volume the app itself lives on, which is the one a
    /// user filling their disk cares about first.
    pub is_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Battery {
    pub percent: f64,
    pub on_ac: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Interface {
    pub name: String,
    /// Cumulative since boot, not since the last sample: a consumer that
    /// wants a rate subtracts two samples, and one that wants a total
    /// does not have to add them all up.
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}
