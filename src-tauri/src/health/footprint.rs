//! What Headstate itself is costing, live (#665).
//!
//! The rest of the System Health view is diagnostic: it says the machine
//! is busy. This says whether *we* are why. So the question here is
//! narrower than "what is running" -- it is "which of the running
//! processes are ours, or are ours by proxy".
//!
//! Since #687 this file also carries the machine's own top processes,
//! for the CPU and Memory detail pages. Those are a SEPARATE pair of
//! fields, not a widening of the three groups below: "are we why" and
//! "what is why" are different questions, and one list answering both
//! would answer neither. See "The machine's own top processes" below.
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
//! # The machine's own top processes (#687)
//!
//! The three groups above are Headstate's cost. [`Footprint::top_cpu`]
//! and [`Footprint::top_memory`] are the OTHER question -- "what is
//! using this machine" -- which the CPU and Memory detail pages ask and
//! no panel could previously answer.
//!
//! That is an addition to this file rather than a new module or a new
//! command because **the data was already here and being thrown away**.
//! The refresh below is `ProcessesToUpdate::All`: it walks every process
//! on the machine and then keeps three of them. So the only thing
//! missing was returning more of what had already been read.
//!
//! Measured before it was written, on a 1436-process machine, warm:
//!
//! ```text
//! run 0: 1435 processes refreshed in 33.5ms   (cold)
//! run 1: 1436 processes refreshed in 24.1ms
//! run 2: 1436 processes refreshed in 20.4ms
//! run 3: 1436 processes refreshed in 19.2ms
//! ```
//!
//! 19-24ms warm. That is the same order as the `ioreg` read that sits on
//! the five-second timer, and three orders below the ~13s
//! `size_worktrees` that #661 forced behind a button -- so this needs
//! neither its own cadence nor a "Measure" affordance. Selecting the top
//! [`TOP_N`] adds two bounded partial passes over a list already in
//! memory; it is not another read of anything.
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
    /// The [`TOP_N`] biggest CPU consumers on the WHOLE machine (#687).
    ///
    /// Not ours, and deliberately so: this is the half that answers
    /// "what is using my CPU" rather than "are we why". The three
    /// fields above are Headstate's own cost and stay exactly as they
    /// were -- this is an addition beside them, not a widening of them,
    /// because the CPU detail page needs the machine's answer and the
    /// footprint panel needs ours, and one list cannot be both.
    pub top_cpu: Vec<Process>,
    /// The [`TOP_N`] biggest resident sets on the whole machine.
    ///
    /// A separate list rather than the same processes re-sorted,
    /// because the two questions have different answers: the process
    /// pinning a core is rarely the one holding 8 GB, and a caller that
    /// re-sorted one list by the other metric would show the top of a
    /// set that was chosen by the wrong measure -- the eighth-hungriest
    /// process would be missing simply because it was not also busy.
    pub top_memory: Vec<Process>,
    /// How many processes were running in total when the two lists
    /// above were taken.
    ///
    /// Reported so the UI can say what it is NOT showing. Eight rows
    /// out of 1436 is a defensible answer to "what is using my CPU";
    /// eight rows presented as if they were everything is not, and the
    /// difference costs one number. A reader who needs the other 1428
    /// then knows to reach for Activity Monitor rather than assuming
    /// this list is exhaustive.
    pub process_count: usize,
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

/// How many machine-wide processes each detail page names.
///
/// Eight, and the number is a judgement about the QUESTION rather than
/// about the payload. "What is using my CPU" is answered by the few
/// processes that dominate it; a list of all 1436 is not an answer at
/// all, it is the filtering problem handed back to the person who asked.
///
/// Eight rather than three or twenty, for three reasons:
///
/// - **It outlasts one app.** A browser is five or six processes on a
///   modern machine, so a top-three is routinely a single application
///   listed three times, which reads as "Chrome" and hides everything
///   else. Eight has room for the noisy app AND whatever is behind it.
/// - **It fits without scrolling.** Eight rows sit in one panel on a
///   phone as well as a desktop, so the answer is on screen at the
///   moment it is read. A list that must be scrolled to be compared is
///   one where the reader loses the top row while looking at the last.
/// - **The tail is not information.** Below the top few, every process
///   on an ordinary machine is at 0-1% and a few MB. Rows that all say
///   the same thing dilute the ones that do not.
///
/// Deliberately not user-configurable. A setting here would be a knob
/// on a question that has one good answer, and the honest reporting of
/// what is NOT shown (the count of the rest, in the UI) costs nothing
/// and cannot be got wrong.
pub const TOP_N: usize = 8;

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
        // Every process, for the top-N selection below. Built in the
        // same pass that classifies ours rather than a second traversal
        // of the map: the classification already visits all of them.
        let mut all: Vec<Process> = Vec::with_capacity(sys.processes().len());

        for (pid, proc) in sys.processes() {
            let name = proc.name().to_string_lossy();
            let out = || Process {
                pid: pid.as_u32(),
                name: name.to_string(),
                cpu_percent: f64::from(proc.cpu_usage()),
                memory: proc.memory(),
            };

            // Every process is a candidate for the machine-wide lists,
            // including our own and the ones classified below. A CPU
            // page that hid Headstate from its own top-eight would be
            // the one view in the app that lies about the app -- and if
            // we ARE the reason a core is pinned, that is exactly the
            // row the user came to find.
            all.push(out());

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

        // Sorted here rather than in the UI, for the same reason
        // `children` is: the order is what makes the list an answer, and
        // a client that sorted its own copy could disagree with the
        // count of what was left out -- which only this side knows.
        //
        // `select_nth_unstable_by` rather than a full sort: the answer
        // is eight rows out of 1436, so ordering the other 1428 relative
        // to each other is work nobody reads. Ties broken by PID after
        // the partition so the eight are stable between five-second
        // polls; without it two processes at the same 0.0% would swap
        // places on every tick and the table would flicker for no
        // reason the user did anything to cause.
        let process_count = all.len();
        let top_cpu = top_by(&all, |p| {
            // `Reverse` so the biggest sorts first. The `f64` needs a
            // total order it does not have natively -- see `ordered`,
            // which also keeps a NaN from panicking the sampler.
            std::cmp::Reverse(ordered(p.cpu_percent))
        });
        let top_memory = top_by(&all, |p| std::cmp::Reverse(p.memory));

        Footprint {
            sampled_at: now.to_string(),
            app,
            children,
            docker_daemon,
            top_cpu,
            top_memory,
            process_count,
        }
    }
}

/// An `f64` as something with a total order, NaN LOWEST.
///
/// `f64` is only `PartialOrd`, and the sort below needs `Ord`. A
/// `partial_cmp().unwrap()` would panic on a NaN, inside a five-second
/// timer, on whichever platform's accounting produced it.
///
/// NaN is forced to the bottom rather than left where `total_cmp` puts
/// it. `total_cmp` orders a positive NaN ABOVE every finite value, so
/// under the descending sort a single nonsense reading would take first
/// place on the CPU page -- the most prominent row in the view, given to
/// the one process whose measurement failed. A reading that means
/// nothing must rank below every reading that means something. The test
/// for this caught the bug in exactly that form.
#[derive(PartialEq)]
struct Ordered(f64);

fn ordered(v: f64) -> Ordered {
    // NEGATIVE_INFINITY, not 0.0: zero is a real and common CPU figure,
    // so mapping a failed reading onto it would make the two
    // indistinguishable -- the "absent is not zero" rule, in the
    // comparator.
    Ordered(if v.is_nan() { f64::NEG_INFINITY } else { v })
}

impl Eq for Ordered {}

impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ordered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// The [`TOP_N`] processes by `key`, biggest first, ties by PID.
///
/// # Two things this avoids, both on the five-second poll
///
/// It selects rather than sorts. A full sort of 1436 rows to read eight
/// of them orders 1428 rows nobody reads.
///
/// And it borrows rather than takes ownership, cloning only the eight
/// that survive. Called twice per sample, so taking `Vec<Process>` by
/// value meant the caller cloning the whole list for the first call --
/// ~1436 heap-allocated names per poll, to keep sixteen. Indices are
/// partitioned instead, and `Process` is cloned once it is known to be
/// in the answer.
fn top_by<K: Ord>(all: &[Process], key: impl Fn(&Process) -> K) -> Vec<Process> {
    let mut idx: Vec<usize> = (0..all.len()).collect();
    // One comparator for both the partition and the sort, and it breaks
    // ties by PID. That matters twice over: without it the partition
    // could put either of two equal processes on the cut line, so WHICH
    // eight came back would shift between five-second polls with
    // nothing on the machine having changed -- and the surviving eight
    // would then reorder among themselves as well. Both show up as a
    // table that flickers for no reason the user did anything to cause.
    let cmp = |&a: &usize, &b: &usize| {
        key(&all[a])
            .cmp(&key(&all[b]))
            .then_with(|| all[a].pid.cmp(&all[b].pid))
    };
    if idx.len() > TOP_N {
        // Everything at or before index TOP_N-1 is <= everything after
        // it under `cmp` (which the callers reverse, so: biggest
        // first). The order WITHIN the first TOP_N is unspecified,
        // which is why the sort below is not redundant.
        idx.select_nth_unstable_by(TOP_N - 1, cmp);
        idx.truncate(TOP_N);
    }
    idx.sort_by(cmp);
    idx.into_iter().map(|i| all[i].clone()).collect()
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

    /// The machine-wide lists are bounded, ordered, and honest about
    /// what they leave out (#687).
    ///
    /// Shape rather than values, like every other test here: what is
    /// hottest on a CI runner is whatever it is. What must hold anywhere
    /// is that the answer is the top few rather than everything, that it
    /// is sorted so the first row is the answer, and that the count of
    /// the rest is reported rather than implied.
    #[test]
    fn the_machine_wide_lists_are_a_bounded_top_n() {
        let f = Footprints::new();
        let fp = f.sample("2026-01-01T00:00:00Z");

        assert!(
            fp.process_count > 0,
            "a machine running this test has processes"
        );
        assert!(fp.top_cpu.len() <= TOP_N, "{} rows", fp.top_cpu.len());
        assert!(fp.top_memory.len() <= TOP_N, "{} rows", fp.top_memory.len());
        // Only when the machine has fewer processes than TOP_N may the
        // lists be shorter -- otherwise a short list means the selection
        // dropped rows it should have kept.
        assert_eq!(fp.top_cpu.len(), fp.process_count.min(TOP_N));
        assert_eq!(fp.top_memory.len(), fp.process_count.min(TOP_N));

        for pair in fp.top_cpu.windows(2) {
            assert!(
                pair[0].cpu_percent > pair[1].cpu_percent
                    || (pair[0].cpu_percent == pair[1].cpu_percent && pair[0].pid < pair[1].pid),
                "cpu order: {:?} before {:?}",
                pair[0],
                pair[1]
            );
        }
        for pair in fp.top_memory.windows(2) {
            assert!(
                pair[0].memory > pair[1].memory
                    || (pair[0].memory == pair[1].memory && pair[0].pid < pair[1].pid),
                "memory order: {:?} before {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// The selection really is the TOP, not merely eight rows.
    ///
    /// The failure this guards is quiet: `select_nth_unstable_by` with
    /// the comparator the wrong way round still returns eight sorted
    /// processes, and a page showing the eight IDLEST processes on the
    /// machine looks entirely plausible until someone checks it against
    /// Activity Monitor. So the property is asserted directly -- nothing
    /// outside the list may beat anything in it.
    #[test]
    fn nothing_left_out_beats_anything_included() {
        // Synthetic rather than a real sample: the point is the
        // selection, and a fabricated set is the only way to know what
        // the right answer was. Names are placeholders -- this repo is
        // public, and a fixture must never carry a real process list.
        let all: Vec<Process> = (0..50)
            .map(|i| Process {
                pid: 1000 + i,
                name: format!("proc-{i}"),
                // Deliberately NOT monotonic in pid: an ordering bug
                // that happened to return the first eight would pass
                // against a sorted input.
                cpu_percent: f64::from((i * 37) % 100),
                memory: u64::from((i * 61) % 100) * 1_000_000,
            })
            .collect();

        let top_cpu = top_by(&all, |p| std::cmp::Reverse(ordered(p.cpu_percent)));
        let top_mem = top_by(&all, |p| std::cmp::Reverse(p.memory));

        assert_eq!(top_cpu.len(), TOP_N);
        let kept: std::collections::HashSet<u32> = top_cpu.iter().map(|p| p.pid).collect();
        let floor = top_cpu.last().expect("TOP_N is not zero").cpu_percent;
        for p in &all {
            if !kept.contains(&p.pid) {
                assert!(
                    p.cpu_percent <= floor,
                    "{} at {}% was left out below a kept {floor}%",
                    p.name,
                    p.cpu_percent
                );
            }
        }

        let kept_mem: std::collections::HashSet<u32> = top_mem.iter().map(|p| p.pid).collect();
        let mem_floor = top_mem.last().expect("TOP_N is not zero").memory;
        for p in &all {
            if !kept_mem.contains(&p.pid) {
                assert!(
                    p.memory <= mem_floor,
                    "{} was left out above the floor",
                    p.name
                );
            }
        }

        // The two lists answer different questions and must be allowed
        // to disagree: a set chosen by CPU is not the set chosen by
        // resident size, which is exactly why they are two fields.
        assert_ne!(kept, kept_mem, "the synthetic set was built to differ");
    }

    /// Equal processes are ordered by PID, so the table does not
    /// flicker between polls.
    ///
    /// The realistic case, not an exotic one: on an idle machine most
    /// processes report exactly 0.0%, so far more than TOP_N are tied
    /// at the cut line. Without the PID tie-break, `select_nth_unstable`
    /// is free to return a different eight each time -- and the eight
    /// it keeps are free to reorder -- which the user sees as a table
    /// reshuffling every five seconds for no reason they caused.
    ///
    /// Asserted by running the selection twice over inputs that differ
    /// only in their ORDER, which is exactly what the process map's
    /// hash iteration varies between samples.
    #[test]
    fn ties_are_broken_by_pid_so_the_order_is_stable_between_polls() {
        let idle: Vec<Process> = (0..40)
            .map(|i| Process {
                pid: 3000 + i,
                name: format!("proc-{i}"),
                // All identical: the idle-machine case.
                cpu_percent: 0.0,
                memory: 1024,
            })
            .collect();
        let mut shuffled = idle.clone();
        shuffled.reverse();

        let a = top_by(&idle, |p| std::cmp::Reverse(ordered(p.cpu_percent)));
        let b = top_by(&shuffled, |p| std::cmp::Reverse(ordered(p.cpu_percent)));

        let pids = |v: &[Process]| v.iter().map(|p| p.pid).collect::<Vec<_>>();
        assert_eq!(
            pids(&a),
            pids(&b),
            "the same tied processes in a different order must select the same eight, in the same order"
        );
        // And that order is by PID, ascending -- a defined answer
        // rather than merely a repeatable one.
        assert_eq!(pids(&a), (3000..3000 + TOP_N as u32).collect::<Vec<_>>());
    }

    /// A NaN CPU reading sorts last instead of panicking.
    ///
    /// `partial_cmp().unwrap()` in the comparator would take down the
    /// sampler on any platform whose accounting produced a NaN, and it
    /// would do it inside a five-second timer -- so the total order is
    /// asserted rather than assumed.
    #[test]
    fn a_nonsense_cpu_reading_sorts_last_rather_than_panicking() {
        let all = vec![
            Process {
                pid: 1,
                name: "nan".into(),
                cpu_percent: f64::NAN,
                memory: 1,
            },
            Process {
                pid: 2,
                name: "real".into(),
                cpu_percent: 5.0,
                memory: 2,
            },
        ];
        let top = top_by(&all, |p| std::cmp::Reverse(ordered(p.cpu_percent)));
        assert_eq!(top[0].name, "real", "a real reading outranks a NaN");
        assert_eq!(top[1].name, "nan");
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
