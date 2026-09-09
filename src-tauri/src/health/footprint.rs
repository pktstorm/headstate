//! What Headstate itself is costing, live (#665).
//!
//! The rest of the System Health view is diagnostic: it says the machine
//! is busy. This says whether *we* are why. So the question here is
//! narrower than "what is running" -- it is "which of the running
//! processes are ours, or are ours by proxy", and everything else on the
//! machine is deliberately not reported.
//!
//! # Three groups, and why those three
//!
//! 1. **This process.** The Tauri host: the webview, the poller, the
//!    SQLite writes. Usually the smallest of the three.
//! 2. **Its children.** `git`, `gh`, and the Docker CLI. This is where
//!    the cost actually is -- `worktrees::scan` fans out 52 distinct
//!    `git` call sites and a refresh runs many of them at once, so a
//!    machine that feels slow *because of Headstate* is nearly always
//!    slow in this group rather than in group 1.
//! 3. **The Docker daemon, if running.** Not our process and not our
//!    child, but Headstate is the reason a user opened the Docker view
//!    and started it, and a daemon holding 4 GB is a cost the user will
//!    reasonably attribute to this app. Reported separately from the two
//!    groups we own so the attribution stays honest.
//!
//! # This is the LIVE half only
//!
//! The disk half of the same panel -- worktrees, artifacts, virtualenvs,
//! Docker's own reclaimable space -- comes from `size_worktrees`,
//! `size_artifacts`, `size_venvs` and `docker_disk_usage`, which already
//! exist, are already tested, and are already what the Worktrees,
//! Artifacts and Docker views show. There is deliberately no sizing code
//! here and no command that combines the two halves: those four are slow
//! (`size_worktrees` is ~13s on a real 147-worktree machine, the #661
//! timeout), they belong behind a "Measure" affordance, and folding them
//! into anything that shares a call site with this would put a 13-second
//! walk one mistake away from the once-a-minute sampler.
//!
//! Everything in this file is a kernel read of an already-open process
//! table. No subprocess, no filesystem walk, no `du`. That is what makes
//! it safe on the sampler's cadence.
//!
//! # Absent is not zero
//!
//! A tool that is not running is `None`, never a zero. `git` at 0% and
//! `git` not running at all are opposite facts, and a panel that renders
//! the second as the first tells the user their fan-out is idle when it
//! never started -- the same failure `packages::run::missing_tool`
//! exists to avoid on the other side of the app. Group 2 therefore
//! reports only the tools it actually found, and an empty list means
//! "none of them were running at this instant", which is the normal
//! state between refreshes.

use serde::{Deserialize, Serialize};

/// The live cost of Headstate at one instant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Footprint {
    /// RFC 3339, matching [`super::Sample::sampled_at`] and every other
    /// timestamp this app stores.
    pub sampled_at: String,
    /// The Tauri host process.
    ///
    /// `None` only if the platform will not tell us our own PID, which
    /// should not happen anywhere this ships -- but a fabricated zero
    /// for "we could not find ourselves" would read as an idle app.
    pub app: Option<Process>,
    /// The tool subprocesses we spawn, one entry per LIVE process.
    ///
    /// Empty when none of them are running, which is the ordinary state
    /// between refreshes -- NOT a row of zeroes. Several entries may
    /// share a `name`: a worktree scan runs many `git` at once, and
    /// collapsing them would hide exactly the fan-out this panel exists
    /// to show.
    pub children: Vec<Process>,
    /// The Docker daemon, or `None` when Docker is not running.
    ///
    /// `None` here is a real and common answer -- most users do not have
    /// Docker up -- which is precisely why it must not be a zero.
    pub docker_daemon: Option<Process>,
}

/// One process, as the panel renders it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Process {
    pub pid: u32,
    /// The executable's own name, as the OS reports it. On Linux this is
    /// the kernel's `comm`, capped at 15 characters -- none of the names
    /// we match are near that, but a caller must not assume it is a full
    /// path.
    pub name: String,
    /// CPU use, as a percentage of ONE core. Above 100 on a process
    /// using more than one core, which `git` legitimately does, so the
    /// UI must not clamp this to 100 the way it can for
    /// [`super::Sample::cpu_percent`].
    ///
    /// Like every CPU figure from `sysinfo`, this is a delta since the
    /// previous refresh of the same `System`, which is why
    /// [`Footprints`] holds its instance rather than building one per
    /// call.
    pub cpu_percent: f64,
    /// Resident set size in bytes: physical RAM the process is using
    /// now. Not virtual size, which on any process linking a webview is
    /// a large number that means nothing to a user.
    pub memory: u64,
}

/// The subprocesses whose cost we attribute to ourselves.
///
/// Matched by NAME rather than by walking the process tree from our own
/// PID. Cheaper -- the tree walk needs every process's parent and a
/// transitive closure, this needs one string compare -- and it is also
/// more correct for the question being asked: a `git` that Headstate
/// started but that has since been reparented (its spawning thread gone,
/// or a `gh` that forked) is still Headstate's cost, and a parent walk
/// would lose it.
///
/// The trade is that a `git` the user is running in their own terminal
/// is counted here too. That is the right way to be wrong for a panel
/// whose job is "is this app why the machine is busy": over-attributing
/// a process the user can see for themselves is a smaller error than
/// under-reporting the fan-out we caused.
///
/// `du` is on the list because the issue lists it, not because we spawn
/// it: `worktrees::scan::dir_size` walks in-process precisely to avoid
/// one subprocess per worktree across 296 of them. If that ever changes
/// back, this already covers it; until then it is simply never present,
/// which is a `None` and not a zero.
///
/// Written WITHOUT the executable suffix; [`matches`] adds it. See there
/// for why that is not cosmetic.
const WATCHED: &[&str] = &["git", "gh", "du", "docker"];

/// Process names for the Docker daemon.
///
/// Docker Desktop (macOS and Windows) runs the engine behind
/// `com.docker.backend`; a native Linux install runs `dockerd`. The
/// user-facing Electron app ("Docker Desktop") is deliberately NOT here:
/// it is a GUI the user chose to open, not the daemon whose memory
/// Headstate's `docker` calls are keeping resident.
const DAEMONS: &[&str] = &["com.docker.backend", "dockerd"];

/// Whether an OS-reported process name is one of `list`.
///
/// Exists because a plain equality check is silently wrong on Windows,
/// which is one of the three platforms this ships to. `sysinfo` reports
/// the name the OS gives it, and on Windows that carries the extension:
/// the process is `git.exe`, not `git`. An exact match against `"git"`
/// would therefore find nothing on Windows and report an empty
/// `children` -- an absence that looks exactly like "no tools running"
/// and would never be investigated, because an empty list is this
/// panel's normal state between refreshes.
///
/// So the suffix is stripped rather than baked into the lists, using
/// `EXE_SUFFIX` for the same reason `docker::cli` does: it is empty on
/// Unix, so the comparison is unchanged there.
///
/// Case-insensitive on the stem as well, because Windows paths are, and
/// a `GIT.EXE` is the same cost as a `git.exe`.
fn matches(name: &str, list: &[&str]) -> bool {
    let stem = name
        .strip_suffix(std::env::consts::EXE_SUFFIX)
        .filter(|_| !std::env::consts::EXE_SUFFIX.is_empty())
        .unwrap_or(name);
    list.iter().any(|w| stem.eq_ignore_ascii_case(w))
}

/// The live reader for [`Footprint`].
///
/// Holds its own `System` for the same reason [`super::collect::Collector`]
/// does: `sysinfo` reports CPU use since the previous refresh, so a
/// fresh instance every call would report every process at 0% forever.
/// Separate from `Collector` rather than folded into it because the two
/// refresh different things at different times -- the machine sample
/// touches CPU, memory, disks and networks, this touches the process
/// table -- and sharing one mutex would make each wait on the other's
/// kernel read for no benefit.
pub struct Footprints {
    system: std::sync::Mutex<sysinfo::System>,
}

impl Default for Footprints {
    fn default() -> Self {
        Self::new()
    }
}

impl Footprints {
    pub fn new() -> Self {
        Self {
            system: std::sync::Mutex::new(sysinfo::System::new()),
        }
    }

    /// One reading, now.
    ///
    /// Blocking -- it reads the kernel's process table -- so callers put
    /// it on a blocking worker like every other read in `commands.rs`.
    ///
    /// Cheap enough for the once-a-minute sampler: the refresh asks for
    /// CPU and memory only, and explicitly not for tasks (threads),
    /// command lines, environments or user IDs. On Linux `with_tasks` in
    /// particular means a `readdir` of `/proc/<pid>/task` for every
    /// process on the machine, which is the one part of a process
    /// refresh that is genuinely expensive.
    pub fn sample(&self, now: &str) -> Footprint {
        let mut sys = self.system.lock().unwrap_or_else(|e| e.into_inner());

        // ProcessesToUpdate::All rather than a PID list: we cannot know
        // which PIDs the `git` fan-out is using without first listing
        // them, so a targeted refresh would need this same pass anyway.
        //
        // `remove_dead_processes: true` matters more than it looks. The
        // subprocesses here are short-lived by design, so without it the
        // map would accumulate every `git` the app has ever run and this
        // would report hundreds of dead processes at 0% -- zeroes that
        // read as "running and idle", which is the exact failure the
        // module docs are about.
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory(),
        );

        // `Err` means the platform would not tell us our own PID. Absent
        // rather than defaulted: reporting some other process as "us"
        // would be worse than reporting nothing.
        let own = sysinfo::get_current_pid().ok();

        let mut app = None;
        let mut children = Vec::new();
        let mut docker_daemon = None;

        for (pid, proc) in sys.processes() {
            let name = proc.name().to_string_lossy();
            let out = || Process {
                pid: pid.as_u32(),
                name: name.to_string(),
                cpu_percent: f64::from(proc.cpu_usage()),
                memory: proc.memory(),
            };

            if Some(*pid) == own {
                app = Some(out());
            } else if matches(&name, WATCHED) {
                children.push(out());
            } else if matches(&name, DAEMONS) {
                // The LARGEST match wins, not the first. Docker Desktop
                // runs several `com.docker.backend` processes (three on
                // the machine this was written on) and they are not
                // equal shares of one daemon -- one holds the engine and
                // the rest are small. Taking whichever the process map
                // happened to yield first would report a number that
                // changed between samples for no reason the user did.
                //
                // A single figure rather than a sum, because the sum is
                // the wrong answer to a different question: the panel
                // asks "what is the daemon costing", and adding three
                // helpers to the engine inflates it.
                let candidate = out();
                if docker_daemon
                    .as_ref()
                    .is_none_or(|d: &Process| d.memory < candidate.memory)
                {
                    docker_daemon = Some(candidate);
                }
            }
        }
        drop(sys);

        // Biggest first: the panel's job is to name the expensive one,
        // and on a busy refresh this list can be dozens of `git` long.
        // Ties broken by PID so the order is stable between samples
        // rather than shuffling with the process map's hash order.
        children.sort_by(|a, b| b.memory.cmp(&a.memory).then_with(|| a.pid.cmp(&b.pid)));

        Footprint {
            sampled_at: now.to_string(),
            app,
            children,
            docker_daemon,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real reading from the machine running the tests.
    ///
    /// Asserting SHAPE, not values, for the same reason
    /// `collect::tests` does: what `git` is doing on a CI runner is
    /// whatever it is. What must hold anywhere is that a running process
    /// finds itself and has real memory behind it.
    #[test]
    fn a_footprint_finds_the_process_it_ran_in() {
        let f = Footprints::new();
        let fp = f.sample("2026-01-01T00:00:00Z");

        assert_eq!(fp.sampled_at, "2026-01-01T00:00:00Z");
        let app = fp
            .app
            .expect("the test binary is a process and can see itself");
        assert!(app.memory > 0, "a running process has a resident set");
        assert!(app.cpu_percent >= 0.0, "cpu {}", app.cpu_percent);
        assert_eq!(
            app.pid,
            std::process::id(),
            "the app entry must be THIS process, not some other one"
        );
    }

    /// Absent is not zero.
    ///
    /// The whole point of #665's convention: a tool that is not running
    /// must be missing from `children`, never present at 0%. Checked
    /// with a name nothing can be running under, because asserting the
    /// same thing about `git` would be flaky -- a real `git` may well be
    /// running while the suite does.
    #[test]
    fn a_process_that_is_not_running_is_absent_rather_than_zero() {
        let f = Footprints::new();
        let fp = f.sample("2026-01-01T00:00:00Z");

        assert!(
            !fp.children
                .iter()
                .any(|c| c.name == "headstate-not-a-real-tool"),
            "a tool nobody is running must not appear at all"
        );
        // And nothing we DID report is a placeholder: every child is a
        // process that exists, so it has a PID and a resident set.
        for c in &fp.children {
            assert!(c.pid > 0, "{} has no pid", c.name);
            assert!(
                matches(&c.name, WATCHED),
                "{} is not a tool we spawn",
                c.name
            );
        }
    }

    /// Docker's daemon is `None` when it is not running, and a real
    /// process when it is. Both are valid on a developer machine, so the
    /// assertion is on the shape of whichever answer came back.
    #[test]
    fn the_docker_daemon_is_absent_or_real_but_never_a_zero() {
        let f = Footprints::new();
        let fp = f.sample("2026-01-01T00:00:00Z");

        if let Some(d) = fp.docker_daemon {
            assert!(
                matches(&d.name, DAEMONS),
                "reported {} as the daemon",
                d.name
            );
            assert!(d.memory > 0, "a running daemon has a resident set");
        }
    }

    /// The second reading is what carries a real CPU figure.
    ///
    /// Same delta problem as `collect::Collector`, and the same reason
    /// [`Footprints`] holds its `System`: a fresh one per call has no
    /// interval to measure and would report every process idle forever.
    #[test]
    fn the_reader_is_reused_so_cpu_is_a_real_delta() {
        let f = Footprints::new();
        let _first = f.sample("2026-01-01T00:00:00Z");
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        // Burn a little CPU so there is something for the second read to
        // find, rather than asserting against an idle test thread.
        let mut spin = 0u64;
        let until = std::time::Instant::now() + std::time::Duration::from_millis(120);
        while std::time::Instant::now() < until {
            spin = spin.wrapping_add(1);
        }
        assert!(spin > 0);

        let second = f.sample("2026-01-01T00:01:00Z");
        let app = second.app.expect("still running");
        // Not "greater than zero" as a hard floor on the value: a
        // scheduler can give a busy loop no measurable slice on a loaded
        // runner. What matters is that the field is populated and sane.
        assert!(
            app.cpu_percent >= 0.0 && app.cpu_percent.is_finite(),
            "cpu {}",
            app.cpu_percent
        );
    }

    /// Children come back biggest-first, so the panel can name the
    /// expensive one without sorting client-side -- and the order is
    /// stable rather than following the process map's hash order.
    #[test]
    fn children_are_ordered_by_cost() {
        let f = Footprints::new();
        let fp = f.sample("2026-01-01T00:00:00Z");
        for pair in fp.children.windows(2) {
            assert!(
                pair[0].memory > pair[1].memory
                    || (pair[0].memory == pair[1].memory && pair[0].pid < pair[1].pid),
                "{:?} before {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// The name match must survive Windows' `.exe`.
    ///
    /// The failure this guards is silent, which is why it is worth a
    /// test of its own: an exact match against `"git"` finds nothing on
    /// Windows, where the process is `git.exe`, and the result is an
    /// empty `children` -- indistinguishable from the ordinary "no tools
    /// running right now", so nobody would ever look.
    ///
    /// Written to run on every platform rather than only Windows: the
    /// suffixed spellings are asserted only where the suffix exists,
    /// but the unsuffixed ones and the non-matches must hold everywhere.
    #[test]
    fn the_name_match_survives_the_platform_executable_suffix() {
        assert!(matches("git", WATCHED));
        assert!(matches("gh", WATCHED));
        assert!(matches("dockerd", DAEMONS));

        // Near misses are misses. `github-desktop` is not our `gh`, and
        // partial matching here would attribute a whole other app's
        // memory to Headstate.
        assert!(!matches("github-desktop", WATCHED));
        assert!(!matches("digit", WATCHED));
        assert!(!matches("", WATCHED));
        assert!(!matches("git", DAEMONS));

        if !std::env::consts::EXE_SUFFIX.is_empty() {
            let exe = |n: &str| format!("{n}{}", std::env::consts::EXE_SUFFIX);
            assert!(matches(&exe("git"), WATCHED));
            assert!(matches(&exe("docker"), WATCHED));
            assert!(matches(&exe("GIT"), WATCHED), "windows paths fold case");
            assert!(matches(&exe("dockerd"), DAEMONS));
            assert!(!matches(&exe("github-desktop"), WATCHED));
        } else {
            // On Unix the suffix is empty, so a literal ".exe" is just
            // part of the name and must NOT be stripped into a match.
            assert!(!matches("git.exe", WATCHED));
        }
    }

    /// The sample must stay cheap enough for the once-a-minute sampler.
    ///
    /// A generous bound, because this runs on CI runners of unknown
    /// speed and alongside seven other test threads: the point is to
    /// catch someone adding a `du`, a directory walk, or a subprocess to
    /// this path, all of which are seconds rather than milliseconds. It
    /// is not a benchmark.
    #[test]
    fn a_sample_is_cheap_enough_for_a_timer() {
        let f = Footprints::new();
        let _warm = f.sample("2026-01-01T00:00:00Z");
        let started = std::time::Instant::now();
        let _ = f.sample("2026-01-01T00:01:00Z");
        let took = started.elapsed();
        assert!(
            took < std::time::Duration::from_secs(2),
            "a footprint sample took {took:?}; something expensive was added to this path"
        );
    }
}
