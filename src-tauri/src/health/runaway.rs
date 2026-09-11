//! Deciding when busy CPU is a FAULT rather than work (#791).
//!
//! Ten orphaned `yes` processes burned a core each for two days on the
//! reporting machine and nothing noticed. This module is the part of
//! #791 that ships: the aggregate rule, plus a shadow log of what the
//! two per-process rules WOULD have said.
//!
//! # Why the obvious rule is not here
//!
//! "100% CPU for five minutes" would have caught those ten and would
//! also fire on every `yarn build`, Vite dev server, `terraform plan`,
//! alembic migration, ollama run and Rust compile on the same machine.
//! That is alert fatigue inside a day, and an ignored alert is worse
//! than no alert. The CPU LEVEL was never the distinguishing signal:
//! duration, orphanhood and doing no work were.
//!
//! # What ships, and what is only logged
//!
//! | tier | rule | this release |
//! |---|---|---|
//! | 3 | total CPU >= [`AGGREGATE_PERCENT`] for >= [`AGGREGATE_MINUTES`] min, no single process explains it | **alerts** |
//! | 1 | a process >= [`TIER1_PERCENT`] for >= [`TIER1_MINUTES`] min, PPID 1, not allowlisted | shadow log only |
//! | 2 | a process >= [`TIER2_PERCENT`] for >= [`TIER2_MINUTES`] min | shadow log only |
//!
//! Tier 3 ships because it is satisfiable against the schema that
//! already exists: `cpu_percent` is stored per sample
//! (`store::schema`), and `store::health::history` is already
//! gap-preserving and downsampled, so the rule is arithmetic over a
//! series the charts already draw. It is also the rule that matters
//! most for the incident that prompted the issue -- ten processes at
//! 93% each looked unremarkable in a list sorted by CPU, which is
//! exactly why they hid for two days. Ten processes at 93% is a
//! different condition from one at 93%, and **only an aggregate rule
//! sees it**.
//!
//! Tiers 1 and 2 are logged and notify for nothing, deliberately.
//! Their thresholds -- 30 minutes, 2 hours -- were reasoned from one
//! machine's workload, not measured against a baseline. The issue asks
//! for a week of would-be firings before any of them interrupts
//! anyone, and that log is the only thing that turns those numbers
//! from plausible into justified. [`Shadow`] is that log and nothing
//! more: it has no [`Alert`] variant, so there is no path from it to a
//! notification even by accident.
//!
//! Deferred with the notifications, and deliberately NOT built here:
//! per-PID duration tracking as persisted storage keyed on
//! `(pid, start_time)`, the maintained full-path daemon allowlist,
//! sibling-cluster detection, and the idle-work/IO check. #791 keeps
//! them; this release adds no migration.
//!
//! # Everything that decides anything is pure
//!
//! [`evaluate`] and [`shadow`] take data and return verdicts. Neither
//! touches a clock, a database, the process table or a notification
//! API -- the same rule `alerts::evaluate` follows, and for the same
//! reason: the cases this module exists for ("a two-day-old condition
//! must not read as fifteen minutes of burn") are testable as
//! arithmetic instead of by arranging hardware to misbehave for a
//! quarter of an hour.
//!
//! # Gap discipline is the whole of the duration rule
//!
//! The sampler runs at 60s cadence and ONLY while the app is open
//! (`lib.rs`), so a fifteen-minute rule is about fifteen samples and
//! cannot span an app restart or a closed lid. Without gap discipline
//! the first sample after a two-day absence sits next to the last one
//! before it, and a condition nobody watched would be "detected" as
//! fifteen minutes of sustained burn the moment the app reopens --
//! reporting a window the app was not running for.
//!
//! So every duration here is measured over CONSECUTIVE samples no more
//! than [`alerts::GAP_MS`] apart, and a gap ends the run rather than
//! being skipped over. That is the same constant, and the same
//! refusal, as the battery rate rules; `a_gap_is_not_sustained_burn`
//! below is the mutation test for it.
//!
//! # Why Tier 3 needs "no single process explains it"
//!
//! Total CPU at 60% is completely ordinary during a build. What is not
//! ordinary is 60% that no single process accounts for: that is either
//! a fan-out nobody asked for (a spawn loop, a stuck parallel job) or
//! a pile of orphans each individually too small to notice. The
//! largest single share is therefore part of the rule, not decoration
//! -- see [`Aggregate::largest_share`] and
//! [`SINGLE_PROCESS_EXPLAINS`].
//!
//! That share is a LIVE reading and the history is not: per-process
//! data lives in `health::footprint`, is sampled on demand and is
//! never persisted. So the duration half of the rule comes from the
//! stored series and the "no single process" half from the current
//! process table, which is sound because the question the second half
//! answers is about now: if one process explains the load at this
//! instant, the machine is busy rather than broken, and the whole
//! point of the rule is that nothing in the list looks wrong.

use super::alerts::GAP_MS;
use super::Sample;

/// Machine-wide CPU, in percent, that the aggregate rule cares about.
///
/// #791's figure. Low on purpose: the rule's discriminating power is
/// not the level but the combination of a long duration with no single
/// process to blame. A higher bar would have missed the incident --
/// ten `yes` processes on a many-core machine never pushed the
/// machine-wide figure near 100%, which is a second reason they hid.
pub const AGGREGATE_PERCENT: f64 = 60.0;

/// How long the aggregate load must hold, in minutes.
///
/// Fifteen is #791's figure, and at the sampler's 60s cadence it is
/// about fifteen samples -- long enough to outlast a test run or a
/// dependency install, short enough that a genuine runaway is named
/// within the quarter hour rather than in two days.
///
/// It is also deliberately the SHORTEST duration in this module, which
/// is only defensible because the "no single process" clause carries
/// the discrimination. A fifteen-minute rule on level alone would fire
/// on every build on the machine the issue came from.
pub const AGGREGATE_MINUTES: f64 = 15.0;

/// The share of machine CPU above which one process is the explanation.
///
/// Expressed as a fraction of the machine-wide figure rather than an
/// absolute percentage, because that is the actual question: "is this
/// one process, or is it a crowd". Half is the boundary that makes the
/// sentence true -- above it, one process is the majority of the load
/// and the CPU page's sorted list already names it, so an aggregate
/// alert would be a second notification about something the user can
/// already see.
///
/// `Process::cpu_percent` is a percentage of ONE core and the
/// machine-wide figure is a percentage of ALL of them, so the share is
/// computed after normalising by core count -- see [`largest_share`].
///
/// [`largest_share`]: Aggregate::largest_share
pub const SINGLE_PROCESS_EXPLAINS: f64 = 0.5;

/// Tier 1's CPU floor, as a percentage of one core.
pub const TIER1_PERCENT: f64 = 80.0;

/// Tier 1's duration, in minutes. Thirty rather than five, because
/// orphanhood is already doing the discriminating work.
pub const TIER1_MINUTES: f64 = 30.0;

/// Tier 2's CPU floor, as a percentage of one core.
pub const TIER2_PERCENT: f64 = 90.0;

/// Tier 2's duration, in minutes: two hours, past almost any
/// legitimate build on the reporting machine.
pub const TIER2_MINUTES: f64 = 120.0;

/// Something about the machine's CPU worth interrupting someone for.
///
/// One variant, because one tier ships. A second variant would be the
/// change that takes a shadow-logged tier live, and it should be made
/// deliberately and with the distribution in hand -- see the module
/// docs.
#[derive(Debug, Clone, PartialEq)]
pub enum Alert {
    /// Machine-wide CPU has been high for a long time and no single
    /// process accounts for it.
    ///
    /// The one the incident in #791 needed. The numbers are the wording
    /// and not the identity; see [`Alert::key`].
    DiffuseCpu {
        /// The mean machine-wide CPU across the sustained run, 0-100.
        percent: f64,
        /// How long it has held, in minutes, measured over ungapped
        /// samples only.
        minutes: f64,
        /// How many processes were running when the share was read.
        /// Part of the body: "nothing in particular, out of 1436" is
        /// the sentence that makes the alert actionable.
        process_count: usize,
    },
}

impl Alert {
    /// The identity the caller remembers, so a standing condition is
    /// announced ONCE rather than every sixty seconds.
    ///
    /// Deliberately excludes every number, exactly as
    /// `alerts::Alert::key` does: keying on the percentage would make
    /// 61% and 62% different alerts, and a machine grinding at a
    /// wandering 60-something would notify on every tick of the way.
    /// The condition is the identity; the numbers are only the wording.
    pub fn key(&self) -> &'static str {
        match self {
            Alert::DiffuseCpu { .. } => "diffuse_cpu",
        }
    }

    /// The notification title.
    ///
    /// Names the SHAPE of the fault rather than the level, because the
    /// level is not the news -- "CPU at 62%" is a thing a user can see
    /// and would not thank anyone for a notification about.
    pub fn title(&self) -> String {
        match self {
            Alert::DiffuseCpu { .. } => "CPU is busy with nothing in particular".to_string(),
        }
    }

    /// The notification body: what was measured, and why it is being
    /// said.
    ///
    /// Says the duration and says that no one process explains it,
    /// because that second clause is the entire reason the alert is
    /// worth reading. Without it the sentence is "your machine is
    /// busy", which the user knows.
    pub fn body(&self) -> String {
        match self {
            Alert::DiffuseCpu {
                percent,
                minutes,
                process_count,
            } => format!(
                "About {percent:.0}% of the CPU has been in use for {minutes:.0} minutes, and no single process among the {process_count} running accounts for it. That pattern is usually several runaway or orphaned processes rather than one busy program."
            ),
        }
    }
}

/// What a deferred tier WOULD have fired, had it been live.
///
/// Not an [`Alert`] and deliberately not convertible into one: the
/// point of shadow mode is that these produce log lines and nothing
/// else, and a `From` impl would be the one line that quietly turned a
/// week of measurement into a week of notifications.
#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    /// `"tier1"` or `"tier2"`, the tier that would have fired.
    pub tier: &'static str,
    /// Whether it would have been an alert (tier 1) or a warn (tier 2).
    /// Recorded so the eventual tuning can tell the two distributions
    /// apart without re-deriving them from the thresholds.
    pub severity: &'static str,
    /// The process name, as the OS reported it.
    ///
    /// A name, never a path and never a command line. The privacy rule
    /// (CONTRIBUTING, `check-privacy.sh`) is about owners and tokens,
    /// but a full `exe()` path on a developer machine carries directory
    /// names that are nobody's business in a log file -- and the name
    /// is what the eventual allowlist will be tuned against anyway.
    pub name: String,
    /// CPU as a percentage of ONE core, so a multi-core process
    /// legitimately reads above 100.
    pub cpu_percent: f64,
    /// How long the condition has held, in minutes, over ungapped
    /// observations only.
    pub minutes: f64,
    /// Whether the process's parent is PID 1.
    ///
    /// Part of tier 1's rule and recorded for tier 2 as well, because
    /// the distribution of "parented but runaway" is exactly what
    /// decides whether tier 2 is worth having.
    pub orphaned: bool,
    /// Whether the name matched [`SEED_DAEMONS`].
    pub allowlisted: bool,
}

impl Shadow {
    /// The log line. Deliberately one line per would-be firing, with
    /// every field the later tuning needs, so a week of logs greps into
    /// a distribution without anyone having to reconstruct what the
    /// thresholds were at the time.
    pub fn line(&self) -> String {
        format!(
            "runaway shadow: {} ({}) would have fired for {} at {:.0}% of one core for {:.0} min (orphaned={}, allowlisted={})",
            self.tier, self.severity, self.name, self.cpu_percent, self.minutes, self.orphaned, self.allowlisted
        )
    }
}

/// The seed daemon names for the shadow log's tier-1 clause.
///
/// **Not the allowlist #791 asks for, and must not be mistaken for
/// it.** The issue is explicit that the real list matches on FULL PATH,
/// because `node` is far too generic to allowlist by name -- a bare
/// name match here would exonerate any runaway that happened to be
/// called `node`, which is most of them on a JavaScript machine.
///
/// It is a bare-name match anyway, for one release, because this is the
/// SHADOW log: a name that is wrongly exonerated is a log line that
/// does not appear, and the only cost is a slightly thinner
/// distribution to tune against. Getting it wrong in a live rule would
/// be a missed alert. The maintained full-path list is deferred with
/// the tiers that depend on it, and [`Shadow::allowlisted`] records the
/// verdict per firing rather than filtering it out, so a week of logs
/// still shows what the allowlist cost.
///
/// # Why the allowlist is load-bearing rather than a nicety
///
/// `footprint.rs` already documents it: on macOS a great many
/// LEGITIMATE user processes reparent to `launchd` (PID 1) when their
/// spawner exits -- a dev server started from a terminal that was then
/// closed is an orphan by this test and entirely healthy. So PPID 1 is
/// much weaker evidence on macOS than on Linux, where an orphan is
/// genuinely unusual. On the platform this app primarily ships to,
/// tier 1 is "high CPU for half an hour AND not on a list", and the
/// list is carrying most of the weight. That is the main thing the
/// shadow week is meant to measure.
pub const SEED_DAEMONS: &[&str] = &[
    "ollama",
    "terraform",
    "docker",
    "com.docker.backend",
    "claude",
    "mds",
    "mds_stores",
    "mdworker_shared",
    "backupd",
    "kernel_task",
    "launchd",
    "WindowServer",
    "photoanalysisd",
    "Spotlight",
];

/// One process as the rules read it.
///
/// A local type rather than `footprint::Process`, for two reasons that
/// both matter. First, the rules need `parent` and `start_time`, which
/// that type does not carry and should not grow for a shadow log.
/// Second, `footprint::Footprint` is being reshaped by #795/#796 and a
/// dependency here would couple a health rule to that churn; this
/// struct is built straight from `sysinfo` by [`Watcher`] instead.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessObservation {
    pub pid: u32,
    /// Seconds since the epoch, as the platform reports it.
    ///
    /// Paired with `pid` as the process identity. PIDs are RECYCLED, so
    /// a pid alone would let a fresh process inherit the elapsed burn
    /// of a dead one that happened to have the same number -- which is
    /// the failure mode that makes a long-duration rule fire on
    /// something three seconds old.
    pub start_time: u64,
    pub name: String,
    /// CPU as a percentage of ONE core, like `footprint::Process`.
    pub cpu_percent: f64,
    /// The parent PID, or `None` where the platform will not say.
    ///
    /// `None` is NOT treated as PID 1: "we could not read the parent"
    /// and "this process has no living parent" are opposite answers,
    /// and conflating them would make every unreadable process an
    /// orphan. Absent is not zero, the same rule as the rest of
    /// `health`.
    pub parent: Option<u32>,
}

impl ProcessObservation {
    /// Whether this process's parent is PID 1.
    ///
    /// See [`SEED_DAEMONS`] on why this is much weaker evidence on
    /// macOS than the phrase "orphaned" suggests.
    pub fn orphaned(&self) -> bool {
        self.parent == Some(1)
    }

    /// Whether the name is one of [`SEED_DAEMONS`].
    pub fn allowlisted(&self) -> bool {
        let name = self.name.to_ascii_lowercase();
        SEED_DAEMONS
            .iter()
            .any(|d| name == d.to_ascii_lowercase() || name.starts_with(&d.to_ascii_lowercase()))
    }
}

/// The live half of the aggregate rule: what the process table says
/// right now.
///
/// Separate from the stored series because the two halves come from
/// different places and refresh on different schedules -- see the
/// module docs on why that is sound rather than a compromise.
#[derive(Debug, Clone, PartialEq)]
pub struct Aggregate {
    /// The largest single process's CPU, as a percentage of one core.
    pub top_cpu_percent: f64,
    /// How many logical cores the machine has, so the figure above can
    /// be compared with a machine-wide percentage.
    ///
    /// Never zero: [`Aggregate::largest_share`] treats a zero as
    /// "unknown" and returns `None`, because dividing by it would
    /// produce an infinity that clears every threshold at once.
    pub cores: usize,
    /// How many processes were running. Carried into the alert body.
    pub process_count: usize,
}

impl Aggregate {
    /// The biggest process's share of the WHOLE machine, 0.0-1.0, or
    /// `None` when the core count is unknown.
    ///
    /// `top_cpu_percent` is per-core and a machine-wide percentage is
    /// per-machine, so 93% of one core on a ten-core machine is 9.3% of
    /// the machine. Comparing the two numbers directly -- which is the
    /// obvious mistake, and the one that would have declared a single
    /// `yes` process "the explanation" for the whole incident -- is
    /// what this division exists to prevent.
    pub fn largest_share(&self) -> Option<f64> {
        if self.cores == 0 {
            return None;
        }
        Some(self.top_cpu_percent / (self.cores as f64 * 100.0))
    }

    /// Whether one process accounts for the machine's load.
    ///
    /// `None` (unknown core count) is NOT "no single process explains
    /// it": an unknown cannot be evidence for the clause the alert
    /// depends on, so it reads as "cannot say", and [`evaluate`]
    /// stays silent. Silence on missing data is the same choice
    /// `alerts::evaluate` makes for a missing battery reading.
    pub fn one_process_explains_it(&self) -> Option<bool> {
        self.largest_share()
            .map(|share| share >= SINGLE_PROCESS_EXPLAINS)
    }
}

/// A run of consecutive, ungapped samples at the end of the series
/// whose `cpu_percent` is all at or above `floor`.
///
/// Returns `(minutes, mean_percent)`, or `None` when the run does not
/// reach `floor` at the newest sample, when a reading is missing, or
/// when there is not enough measured time.
///
/// Walks BACKWARDS from the newest sample and stops at the first thing
/// that is not comparable -- a gap wider than [`GAP_MS`], a missing
/// `cpu_percent`, an out-of-order timestamp, or a sample below the
/// floor. Stopping rather than skipping is the point: a run of high
/// samples from before a six-hour hole says nothing about the present,
/// and stitching the two sides together is arithmetic over a period
/// nobody measured.
///
/// `cpu_percent` is `Option` on [`Sample`] and a `None` breaks the run
/// for the same reason a missing battery reading breaks the battery
/// trend: "not measured" is an unknown, not a low reading, and treating
/// it as either would be a claim about a minute nobody sampled.
fn sustained_above(samples: &[Sample], floor: f64) -> Option<(f64, f64)> {
    let mut total_minutes = 0.0;
    let mut sum = 0.0;
    let mut count = 0usize;
    let mut newer: Option<(i64, f64)> = None;

    for s in samples.iter().rev() {
        let Some(cpu) = s.cpu_percent else { break };
        if cpu < floor {
            break;
        }
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&s.sampled_at) else {
            break;
        };
        let ms = at.timestamp_millis();
        if let Some((newer_ms, _)) = newer {
            let spacing = newer_ms - ms;
            // Non-positive spacing means the rows are out of order or
            // share an instant; a wider one than GAP_MS means the two
            // samples are not comparable at all. Either way the run
            // ends HERE, with the samples already counted kept.
            if spacing <= 0 || spacing > GAP_MS {
                break;
            }
            total_minutes += spacing as f64 / 60_000.0;
        }
        sum += cpu;
        count += 1;
        newer = Some((ms, cpu));
    }

    // One sample is a reading, not a duration: it spans no measured
    // time at all, so there is nothing to compare against a
    // minutes-long threshold.
    if count < 2 {
        return None;
    }
    Some((total_minutes, sum / count as f64))
}

/// Everything the aggregate rule currently finds true of this series.
///
/// `samples` is oldest-first, as `store::health::history` returns it.
/// `live` is the current process table's answer, or `None` when it
/// could not be read.
///
/// Returns what IS true, not what is NEW: deduplication is
/// `alerts::Fired`'s job, and keeping the two apart is what lets this
/// function stay a pure statement about the data with the "have we said
/// this already" state in exactly one place.
pub fn evaluate(samples: &[Sample], live: Option<&Aggregate>) -> Vec<Alert> {
    let mut out = Vec::new();

    let Some((minutes, percent)) = sustained_above(samples, AGGREGATE_PERCENT) else {
        return out;
    };
    if minutes < AGGREGATE_MINUTES {
        return out;
    }

    // No live reading is silence, not a firing. The "no single process
    // explains it" clause is the whole content of this alert -- without
    // it the sentence is "your machine is busy", which the user can see
    // -- so an unreadable process table means the alert cannot be
    // justified rather than that it can be assumed.
    let Some(live) = live else { return out };
    match live.one_process_explains_it() {
        // One process is the majority of the load: the machine is busy,
        // and the CPU page's sorted list already names the culprit. An
        // alert here would be a notification about something visible.
        Some(true) => return out,
        // Unknown core count: cannot say, so say nothing.
        None => return out,
        Some(false) => {}
    }

    out.push(Alert::DiffuseCpu {
        percent,
        minutes,
        process_count: live.process_count,
    });
    out
}

/// What tiers 1 and 2 WOULD have fired, given one pass of observations
/// and how long each has been above its floor.
///
/// `durations` maps `(pid, start_time)` to minutes observed above the
/// tier floor, which is what [`Watcher`] accumulates. Keyed on the pair
/// rather than the pid alone because PIDs are recycled -- see
/// [`ProcessObservation::start_time`].
///
/// Pure, like [`evaluate`], and for the same reason: "thirty minutes of
/// burn" is testable as arithmetic over a map rather than by keeping a
/// process pinned for half an hour.
///
/// Tier 1 is checked before tier 2 and a process that matches both is
/// reported ONCE, as tier 1. Two log lines about one process would
/// double-count it in the distribution the shadow week exists to
/// produce, which is the one thing this log must get right.
pub fn shadow(
    observations: &[ProcessObservation],
    durations: &std::collections::HashMap<(u32, u64), f64>,
) -> Vec<Shadow> {
    let mut out = Vec::new();
    for p in observations {
        let minutes = durations
            .get(&(p.pid, p.start_time))
            .copied()
            .unwrap_or(0.0);
        let orphaned = p.orphaned();
        let allowlisted = p.allowlisted();

        let tier = if p.cpu_percent >= TIER1_PERCENT
            && minutes >= TIER1_MINUTES
            && orphaned
            && !allowlisted
        {
            Some(("tier1", "alert"))
        } else if p.cpu_percent >= TIER2_PERCENT && minutes >= TIER2_MINUTES {
            Some(("tier2", "warn"))
        } else {
            None
        };

        if let Some((tier, severity)) = tier {
            out.push(Shadow {
                tier,
                severity,
                name: p.name.clone(),
                cpu_percent: p.cpu_percent,
                minutes,
                orphaned,
                allowlisted,
            });
        }
    }
    out
}

/// The in-memory per-process duration approximation behind the shadow
/// log.
///
/// # Why in memory, and why an approximation
///
/// #791 asks for per-PID duration tracking keyed on
/// `(pid, start_time)`, persisted so it survives a restart. That is
/// deferred: it needs a schema migration, and it is only worth its
/// migration once a tier actually notifies. A shadow log does not need
/// to survive anything -- a process whose accumulated burn is forgotten
/// on relaunch is a log line that does not appear, and the
/// distribution is thinner by one entry. So this is a `HashMap` in the
/// sampler's own thread and nothing else. **No storage, no migration.**
///
/// The approximation is therefore deliberately conservative in one
/// direction: it can only ever UNDER-report. A process that has burned
/// for two days shows up here with however many minutes the app has
/// been open for, never more. That is the right direction for the
/// failure it is avoiding -- see the gap discussion in the module docs
/// -- because over-reporting is what would invent sustained burn the
/// app never watched.
///
/// # Gap discipline, again
///
/// [`Watcher::observe`] takes the elapsed milliseconds since the
/// previous pass and REFUSES to credit a span wider than [`GAP_MS`].
/// The sampler's own clock is what produces that number, and across a
/// closed lid or an app restart it is hours: crediting it would hand
/// every process on the machine half a day of "sustained" burn the
/// moment the app reopens. An over-wide span resets the accumulation
/// instead, which is the same decision `sustained_above` makes for the
/// stored series.
#[derive(Debug, Default)]
pub struct Watcher {
    /// `(pid, start_time)` to minutes observed at or above the floor.
    minutes: std::collections::HashMap<(u32, u64), f64>,
}

impl Watcher {
    /// Credit `elapsed_ms` to every observation still above the floor,
    /// reset the ones that dropped below it, and forget the ones that
    /// are gone.
    ///
    /// The floor is [`TIER2_PERCENT`] or [`TIER1_PERCENT`], whichever
    /// is lower: one accumulator serves both tiers, and tracking from
    /// the lower floor is what lets a process that crosses into tier 2
    /// territory already have its tier-1 duration behind it.
    ///
    /// Returns the duration map [`shadow`] reads, so a caller cannot
    /// evaluate against a map it forgot to update.
    pub fn observe(
        &mut self,
        observations: &[ProcessObservation],
        elapsed_ms: i64,
    ) -> &std::collections::HashMap<(u32, u64), f64> {
        let floor = TIER1_PERCENT.min(TIER2_PERCENT);
        // A span nobody measured credits nothing. Past GAP_MS the two
        // passes are not comparable, so the accumulated minutes are
        // dropped rather than extended -- the same refusal as
        // `sustained_above`, applied to the live side.
        let credit = if elapsed_ms > 0 && elapsed_ms <= GAP_MS {
            elapsed_ms as f64 / 60_000.0
        } else {
            self.minutes.clear();
            0.0
        };

        let mut next = std::collections::HashMap::with_capacity(observations.len());
        for p in observations {
            if p.cpu_percent < floor {
                // Dropped below the floor: the run is over, and the
                // next one starts from zero rather than resuming. A
                // process that is busy for a minute every hour has not
                // been busy for thirty minutes.
                continue;
            }
            let key = (p.pid, p.start_time);
            let so_far = self.minutes.get(&key).copied().unwrap_or(0.0);
            next.insert(key, so_far + credit);
        }
        // Replaced rather than merged, so a process that has EXITED is
        // forgotten. Left to grow, this map would be an unbounded leak
        // in a loop that runs forever -- and a recycled PID would
        // inherit a dead process's minutes, which is precisely what the
        // start_time half of the key exists to prevent.
        self.minutes = next;
        &self.minutes
    }
}

/// Reads the process table for the shadow log.
///
/// Its own `sysinfo::System` rather than a call into
/// `health::footprint`: the rules need `parent()` and `start_time()`,
/// which `footprint::Process` does not carry, and `Footprint` is being
/// reshaped by #795/#796. A refresh asks for CPU only -- not memory,
/// not tasks, not command lines -- because that is all the two rules
/// read.
///
/// Like `Footprints`, ONE instance per process and held across calls:
/// `sysinfo` reports CPU use since the previous refresh of the same
/// `System`, so a fresh instance per pass would report an idle machine
/// forever.
pub struct Table {
    system: std::sync::Mutex<sysinfo::System>,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    pub fn new() -> Self {
        Self {
            system: std::sync::Mutex::new(sysinfo::System::new()),
        }
    }

    /// One pass over the process table: the observations, and the
    /// aggregate rule's live half.
    ///
    /// Blocking -- it reads the kernel -- so callers keep it on the
    /// sampler's own thread, which is already blocking by design.
    pub fn read(&self) -> (Vec<ProcessObservation>, Aggregate) {
        let mut sys = self.system.lock().unwrap_or_else(|e| e.into_inner());
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing().with_cpu(),
        );

        let mut observations = Vec::with_capacity(sys.processes().len());
        let mut top = 0.0f64;
        for (pid, proc) in sys.processes() {
            let cpu = f64::from(proc.cpu_usage());
            // NaN is skipped rather than compared: `sysinfo` yields one
            // where a platform's accounting failed, and a NaN loses
            // every comparison silently -- so a NaN left in would make
            // `top` wrong in a way nothing would ever surface.
            if cpu.is_finite() && cpu > top {
                top = cpu;
            }
            observations.push(ProcessObservation {
                pid: pid.as_u32(),
                start_time: proc.start_time(),
                name: proc.name().to_string_lossy().to_string(),
                cpu_percent: cpu,
                parent: proc.parent().map(|p| p.as_u32()),
            });
        }
        let process_count = observations.len();
        drop(sys);

        // `available_parallelism` rather than a second `sysinfo` call:
        // the `System` above was refreshed for processes only, so its
        // CPU list is not populated, and refreshing CPUs here purely
        // for a core count would do a second kernel pass for a number
        // the standard library already has.
        //
        // `Err` becomes 0, which `Aggregate::largest_share` reads as
        // "unknown" and which makes `evaluate` stay silent -- absent is
        // not a default, the same rule as everywhere else in `health`.
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);

        (
            observations,
            Aggregate {
                top_cpu_percent: top,
                cores,
                process_count,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::{Memory, Sample};
    use std::collections::HashMap;

    /// A series at the sampler's own cadence, ending now: each entry is
    /// a machine-wide CPU percentage, oldest first, one minute apart.
    fn series(points: &[f64]) -> Vec<Sample> {
        let end = chrono::Utc::now();
        points
            .iter()
            .enumerate()
            .map(|(i, cpu)| {
                let at = end - chrono::Duration::minutes((points.len() - 1 - i) as i64);
                let mut s = bare(&at.to_rfc3339());
                s.cpu_percent = Some(*cpu);
                s
            })
            .collect()
    }

    fn bare(at: &str) -> Sample {
        Sample {
            sampled_at: at.to_string(),
            load: None,
            cpu_percent: None,
            cpu_per_core: vec![],
            memory: Memory {
                total: 1,
                used: 1,
                available: 0,
                swap_total: 0,
                swap_used: 0,
            },
            gpus: vec![],
            disks: vec![],
            battery: None,
            thermal: None,
            networks: vec![],
            uptime_secs: 1,
        }
    }

    /// A ten-core machine whose busiest process holds `percent` of ONE
    /// core, with `count` processes running.
    fn live(percent: f64, count: usize) -> Aggregate {
        Aggregate {
            top_cpu_percent: percent,
            cores: 10,
            process_count: count,
        }
    }

    /// Twenty minutes at 70% with the biggest process holding 93% of
    /// one core -- 9.3% of a ten-core machine, so nothing explains the
    /// load. This is the incident in #791, and it MUST fire: ten
    /// processes at 93% each is a different condition from one at 93%,
    /// and only an aggregate rule sees it.
    #[test]
    fn a_diffuse_load_that_nothing_explains_fires() {
        let s = series(&[70.0; 20]);
        let alerts = evaluate(&s, Some(&live(93.0, 1436)));
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert_eq!(alerts[0].key(), "diffuse_cpu");
        match &alerts[0] {
            Alert::DiffuseCpu {
                percent,
                minutes,
                process_count,
            } => {
                assert!((percent - 70.0).abs() < 0.01, "{percent}");
                assert!(*minutes >= AGGREGATE_MINUTES, "{minutes}");
                assert_eq!(*process_count, 1436);
            }
        }
    }

    /// **The false-positive test this module exists to pass.**
    ///
    /// A `yarn build` or a Rust compile pins the machine for far longer
    /// than fifteen minutes, and it is not a fault. What separates it
    /// from the incident is that ONE process is the load: here the
    /// compiler holds 600% of one core, six whole cores of a ten-core
    /// machine, well past half of it.
    ///
    /// If this test fails, the rule fires on every build on the machine
    /// the issue came from, and the alerts get ignored within a day.
    #[test]
    fn a_legitimate_build_does_not_fire() {
        let s = series(&[85.0; 40]);
        let alerts = evaluate(&s, Some(&live(600.0, 412)));
        assert!(
            alerts.is_empty(),
            "one process holding six of ten cores IS the explanation: {alerts:?}"
        );
    }

    /// **The mutation test for #791's gap requirement.**
    ///
    /// The ten `yes` processes burned for two days while the app was
    /// closed. The first sample after reopening sits next to the last
    /// one before it, and both are high -- so a rule that measures
    /// duration as "time between the oldest and newest high sample"
    /// sees two days of burn, or at best fifteen minutes of a window
    /// nobody watched.
    ///
    /// If this fails, a two-day-old condition is "detected" the instant
    /// the app reopens, with a duration the app did not measure.
    #[test]
    fn a_gap_is_not_sustained_burn() {
        let end = chrono::Utc::now();
        let at = |mins: i64| (end - chrono::Duration::minutes(mins)).to_rfc3339();
        let mut samples = Vec::new();
        // Twenty high samples, then the app was closed for two days.
        for mins in (2880..2900).rev() {
            let mut s = bare(&at(mins));
            s.cpu_percent = Some(75.0);
            samples.push(s);
        }
        // Reopened: three high samples, which is three minutes of
        // measured time and nothing like fifteen.
        for mins in [2, 1, 0] {
            let mut s = bare(&at(mins));
            s.cpu_percent = Some(75.0);
            samples.push(s);
        }

        let alerts = evaluate(&samples, Some(&live(50.0, 1436)));
        assert!(
            alerts.is_empty(),
            "a duration must never be measured across a period nobody sampled: {alerts:?}"
        );
    }

    /// And the other half of that test: the run either side of the gap
    /// is not merged, so the POST-gap run alone is what counts -- and
    /// once it is long enough on its own, it fires.
    #[test]
    fn the_run_after_a_gap_is_measured_on_its_own() {
        let end = chrono::Utc::now();
        let at = |mins: i64| (end - chrono::Duration::minutes(mins)).to_rfc3339();
        let mut samples = Vec::new();
        for mins in (2880..2890).rev() {
            let mut s = bare(&at(mins));
            s.cpu_percent = Some(75.0);
            samples.push(s);
        }
        for mins in (0..20).rev() {
            let mut s = bare(&at(mins));
            s.cpu_percent = Some(75.0);
            samples.push(s);
        }
        let alerts = evaluate(&samples, Some(&live(50.0, 900)));
        assert_eq!(alerts.len(), 1, "nineteen measured minutes: {alerts:?}");
    }

    /// A brief spike is not a runaway. Five minutes of high CPU is a
    /// test run.
    #[test]
    fn a_short_burst_says_nothing() {
        let s = series(&[90.0; 5]);
        assert!(evaluate(&s, Some(&live(20.0, 900))).is_empty());
    }

    /// A dip below the floor ends the run, so the duration restarts
    /// from the dip rather than spanning it. Otherwise a machine that
    /// is busy half the time would accumulate to fifteen minutes and
    /// report a sustained condition it never had.
    #[test]
    fn a_dip_below_the_floor_ends_the_run() {
        let mut points = vec![70.0; 20];
        // Newest three are high, but the fourth-newest dipped.
        points[16] = 20.0;
        let s = series(&points);
        assert!(
            evaluate(&s, Some(&live(20.0, 900))).is_empty(),
            "only the three newest samples are in the run"
        );
    }

    /// A missing reading breaks the run the same way a gap does:
    /// "not measured" is an unknown, not a low number.
    #[test]
    fn a_missing_cpu_reading_is_not_a_low_one() {
        let mut s = series(&[70.0; 20]);
        s[16].cpu_percent = None;
        assert!(evaluate(&s, Some(&live(20.0, 900))).is_empty());
    }

    /// No live process table means the "no single process explains it"
    /// clause cannot be checked, and that clause is the whole content
    /// of the alert. Silence, not a firing.
    #[test]
    fn an_unreadable_process_table_is_silence() {
        let s = series(&[70.0; 20]);
        assert!(evaluate(&s, None).is_empty());
    }

    /// An unknown core count is "cannot say", not "nothing explains
    /// it". A zero would otherwise divide into an infinity that clears
    /// the threshold and fires on every busy build.
    #[test]
    fn an_unknown_core_count_is_silence() {
        let s = series(&[70.0; 20]);
        let unknown = Aggregate {
            top_cpu_percent: 600.0,
            cores: 0,
            process_count: 900,
        };
        assert!(evaluate(&s, Some(&unknown)).is_empty());
        assert_eq!(unknown.largest_share(), None);
        assert_eq!(unknown.one_process_explains_it(), None);
    }

    /// An empty series is not a quiet machine, it is no information.
    #[test]
    fn an_empty_series_says_nothing() {
        assert!(evaluate(&[], Some(&live(20.0, 900))).is_empty());
        assert!(evaluate(&series(&[90.0]), Some(&live(20.0, 900))).is_empty());
    }

    /// The per-core / per-machine conversion, explicitly. 93% of one
    /// core on a ten-core machine is 9.3% of the machine, and reading
    /// the two numbers as comparable is the mistake that would have
    /// called a single `yes` process "the explanation".
    #[test]
    fn one_cores_worth_is_not_the_whole_machine() {
        let a = live(93.0, 10);
        let share = a.largest_share().expect("ten cores is known");
        assert!((share - 0.093).abs() < 0.0001, "{share}");
        assert_eq!(a.one_process_explains_it(), Some(false));

        let busy = live(600.0, 10);
        assert_eq!(busy.one_process_explains_it(), Some(true));
    }

    // ---- Shadow logging ---------------------------------------------

    fn proc(pid: u32, name: &str, cpu: f64, parent: Option<u32>) -> ProcessObservation {
        ProcessObservation {
            pid,
            start_time: 1_700_000_000,
            name: name.to_string(),
            cpu_percent: cpu,
            parent,
        }
    }

    fn durations(entries: &[(&ProcessObservation, f64)]) -> HashMap<(u32, u64), f64> {
        entries
            .iter()
            .map(|(p, m)| ((p.pid, p.start_time), *m))
            .collect()
    }

    /// Tier 1's shape: high, orphaned, long, not a known daemon.
    #[test]
    fn an_orphaned_long_burner_is_shadow_logged_as_tier_one() {
        let p = proc(4242, "yes", 99.0, Some(1));
        let out = shadow(std::slice::from_ref(&p), &durations(&[(&p, 45.0)]));
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].tier, "tier1");
        assert_eq!(out[0].severity, "alert");
        assert_eq!(out[0].name, "yes");
        assert!(out[0].orphaned);
        assert!(!out[0].allowlisted);
        assert!(!out[0].line().is_empty());
    }

    /// A known daemon is exonerated even when it matches everything
    /// else. `ollama` legitimately pins cores for hours and legitimately
    /// reparents to launchd.
    #[test]
    fn an_allowlisted_daemon_is_not_shadow_logged_as_tier_one() {
        let p = proc(900, "ollama", 140.0, Some(1));
        let out = shadow(std::slice::from_ref(&p), &durations(&[(&p, 90.0)]));
        assert!(out.is_empty(), "{out:?}");
    }

    /// A parented process is not tier 1 however hot it is -- that is
    /// what tier 2's much longer duration is the backstop for.
    #[test]
    fn a_parented_burner_is_not_tier_one() {
        let p = proc(5000, "node", 99.0, Some(412));
        let out = shadow(std::slice::from_ref(&p), &durations(&[(&p, 45.0)]));
        assert!(
            out.is_empty(),
            "forty-five minutes is not two hours: {out:?}"
        );
    }

    /// Tier 2 catches it once it has been two hours.
    #[test]
    fn a_parented_burner_past_two_hours_is_tier_two() {
        let p = proc(5000, "node", 99.0, Some(412));
        let out = shadow(std::slice::from_ref(&p), &durations(&[(&p, 130.0)]));
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].tier, "tier2");
        assert_eq!(out[0].severity, "warn");
        assert!(!out[0].orphaned);
    }

    /// A process matching both tiers is reported ONCE, as tier 1.
    /// Two lines would double-count it in the distribution the shadow
    /// week exists to produce.
    #[test]
    fn a_process_matching_both_tiers_is_logged_once() {
        let p = proc(4242, "yes", 99.0, Some(1));
        let out = shadow(std::slice::from_ref(&p), &durations(&[(&p, 200.0)]));
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].tier, "tier1");
    }

    /// A process the watcher has never seen has no duration, so it
    /// cannot satisfy a duration rule however hot it is right now.
    #[test]
    fn a_brand_new_process_has_no_duration() {
        let p = proc(4242, "yes", 400.0, Some(1));
        assert!(shadow(&[p], &HashMap::new()).is_empty());
    }

    /// A missing parent is NOT an orphan. "We could not read the
    /// parent" and "this process has no living parent" are opposite
    /// answers, and conflating them would make every unreadable
    /// process a tier-1 candidate.
    #[test]
    fn an_unreadable_parent_is_not_an_orphan() {
        let p = proc(4242, "yes", 99.0, None);
        assert!(!p.orphaned());
        assert!(shadow(std::slice::from_ref(&p), &durations(&[(&p, 45.0)])).is_empty());
    }

    // ---- The watcher -------------------------------------------------

    /// Minutes accumulate across passes while the process stays hot.
    #[test]
    fn the_watcher_accumulates_across_passes() {
        let mut w = Watcher::default();
        let p = proc(4242, "yes", 99.0, Some(1));
        for _ in 0..10 {
            w.observe(std::slice::from_ref(&p), 60_000);
        }
        let minutes = w.observe(std::slice::from_ref(&p), 60_000);
        assert!(
            (minutes[&(p.pid, p.start_time)] - 11.0).abs() < 0.001,
            "{minutes:?}"
        );
    }

    /// **The live half of the gap rule.**
    ///
    /// Across a closed lid the sampler's elapsed span is hours. If that
    /// were credited, every hot process on the machine would be handed
    /// half a day of "sustained" burn the moment the app reopens -- and
    /// tier 1 would shadow-log a fleet of firings that never happened.
    #[test]
    fn the_watcher_refuses_to_credit_a_gap() {
        let mut w = Watcher::default();
        let p = proc(4242, "yes", 99.0, Some(1));
        for _ in 0..5 {
            w.observe(std::slice::from_ref(&p), 60_000);
        }
        // Twelve hours asleep.
        let minutes = w.observe(std::slice::from_ref(&p), 12 * 60 * 60 * 1000);
        assert_eq!(
            minutes.get(&(p.pid, p.start_time)).copied(),
            Some(0.0),
            "a gap resets rather than extends: {minutes:?}"
        );
    }

    /// Dropping below the floor ends the run: the next hot spell starts
    /// from zero rather than resuming. A process busy for a minute an
    /// hour has not been busy for thirty minutes.
    #[test]
    fn the_watcher_resets_a_process_that_goes_quiet() {
        let mut w = Watcher::default();
        let hot = proc(4242, "yes", 99.0, Some(1));
        let cool = proc(4242, "yes", 3.0, Some(1));
        for _ in 0..20 {
            w.observe(std::slice::from_ref(&hot), 60_000);
        }
        w.observe(std::slice::from_ref(&cool), 60_000);
        let minutes = w.observe(std::slice::from_ref(&hot), 60_000);
        assert!(
            (minutes[&(hot.pid, hot.start_time)] - 1.0).abs() < 0.001,
            "the run restarted: {minutes:?}"
        );
    }

    /// A recycled PID does not inherit the dead process's minutes.
    /// That is the entire reason the key carries `start_time`.
    #[test]
    fn a_recycled_pid_starts_from_zero() {
        let mut w = Watcher::default();
        let old = proc(4242, "yes", 99.0, Some(1));
        for _ in 0..40 {
            w.observe(std::slice::from_ref(&old), 60_000);
        }
        let fresh = ProcessObservation {
            start_time: old.start_time + 9_999,
            ..old.clone()
        };
        let minutes = w.observe(std::slice::from_ref(&fresh), 60_000);
        assert!(
            (minutes[&(fresh.pid, fresh.start_time)] - 1.0).abs() < 0.001,
            "{minutes:?}"
        );
        assert!(
            !minutes.contains_key(&(old.pid, old.start_time)),
            "the dead process is forgotten: {minutes:?}"
        );
    }

    /// An exited process is forgotten rather than accumulating forever.
    /// The sampler loop runs for the life of the app, so a map that only
    /// ever grew would be an unbounded leak.
    #[test]
    fn the_watcher_forgets_processes_that_are_gone() {
        let mut w = Watcher::default();
        let p = proc(4242, "yes", 99.0, Some(1));
        w.observe(std::slice::from_ref(&p), 60_000);
        let minutes = w.observe(&[], 60_000);
        assert!(minutes.is_empty(), "{minutes:?}");
    }

    /// The gap rule here and the one in `alerts` are the SAME rule, not
    /// two that happen to agree. A series the charts draw as broken must
    /// not be one a duration is measured across.
    #[test]
    fn the_gap_rule_is_the_battery_modules_gap_rule() {
        assert_eq!(GAP_MS, crate::health::alerts::GAP_MS);
    }

    /// Shadow mode must have no path to a notification. If a `Shadow`
    /// ever becomes an `Alert`, a week of measurement becomes a week of
    /// interruptions -- so the absence of that conversion is asserted
    /// rather than left to review.
    #[test]
    fn nothing_converts_a_shadow_into_an_alert() {
        let src = include_str!("runaway.rs");
        let body = &src[..src.find("#[cfg(test)]").expect("tests exist")];
        assert!(
            !body.contains("impl From<Shadow>"),
            "a Shadow must not be convertible into an Alert"
        );
        // And exactly one Alert variant ships: a second one would mean
        // a deferred tier had gone live.
        let start = body.find("pub enum Alert {").expect("Alert exists");
        let end = start + body[start..].find("\n}\n").expect("Alert closes");
        let variants = body[start..end]
            .lines()
            .filter(|l| l.trim_start().starts_with("DiffuseCpu"))
            .count();
        assert_eq!(variants, 1);
    }

    /// Wording exists, since a notification with an empty body says
    /// nothing.
    #[test]
    fn the_alert_can_say_what_it_means() {
        let a = Alert::DiffuseCpu {
            percent: 71.0,
            minutes: 19.0,
            process_count: 1436,
        };
        assert!(!a.title().is_empty());
        assert!(a.body().contains("1436"));
        assert!(a.body().contains("19"));
        assert_eq!(a.key(), "diffuse_cpu");
    }

    /// The real process table, on whatever machine runs the tests. Not
    /// an assertion about any particular process -- it cannot be -- but
    /// it does prove the reader works, which a pure-arithmetic suite
    /// never would.
    #[test]
    fn a_real_process_table_reads() {
        let t = Table::new();
        // Two passes: `sysinfo` reports CPU since the previous refresh,
        // so the first one is all zeroes by construction.
        let _ = t.read();
        let (observations, aggregate) = t.read();
        assert!(!observations.is_empty(), "no processes at all");
        assert_eq!(observations.len(), aggregate.process_count);
        assert!(aggregate.top_cpu_percent.is_finite());
        // Our own process must be in there, and it has a parent.
        let own = sysinfo::get_current_pid().expect("our own pid");
        let me = observations
            .iter()
            .find(|p| p.pid == own.as_u32())
            .expect("we are running");
        assert!(!me.name.is_empty());
        assert!(me.start_time > 0, "a start time is the identity half");
    }
}
