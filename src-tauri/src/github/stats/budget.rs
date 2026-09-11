//! What a scope load COST, accumulated and actually read.
//!
//! The app has always asked GitHub what its requests cost. `PRS_QUERY`,
//! `MERGED_DETAIL_QUERY` and `COUNT_QUERY` all select
//! `rateLimit { cost remaining resetAt }`, and three of the four stats
//! queries do not select it at all -- but the deeper problem is that
//! NOTHING READ `cost`, anywhere, on any query. The one consumer is
//! `client.rs:906-912`, which reads `remaining` and logs a warning below
//! 500. So the app could tell you it was nearly out of budget, and never
//! what had spent it.
//!
//! That is survivable for a poll loop whose spend is a known constant --
//! `poll.rs:1503-1580` pins `PRS_QUERY`'s cost in a CI test precisely so
//! it stays knowable without being measured at runtime. It is not
//! survivable for this feature, where one sidebar click issues a number
//! of requests that depends on how much activity the scope contains. The
//! probe-driven slicer in `slice.rs` cannot be costed in advance by
//! construction: how many slices it takes IS the thing it discovers.
//!
//! # Why an accumulator and not a log line
//!
//! The unit that matters to a user is the CLICK, not the request. "That
//! org cost you 23 points" is actionable; twenty-three separate log lines
//! each saying "cost 1" are not, and at the fan-out this feature
//! reaches they are noise that buries the one line that mattered.
//!
//! So [`Budget`] is handed to a load, threaded through every request it
//! makes, and read once at the end. `remaining` and `reset_at` are kept
//! as the LOWEST and LATEST seen rather than the last: requests run
//! concurrently (see `fetch.rs`), so "the last response to arrive" is a
//! race, and the pessimistic value is the one a refusal should be based
//! on.
//!
//! # On the cost model
//!
//! Read `poll.rs:1503-1580` before adding a field to any stats query. The
//! short version, which that test learned the hard way: cost is driven by
//! NESTED CONNECTIONS, not by the number of searches, and a connection
//! nested inside another is INVISIBLE to a substring count of its parent
//! -- adding `contexts(` inside `statusCheckRollup` took the live cost
//! from 3 to 4 while the guard's count did not move.
//!
//! For this feature specifically: `additions`, `deletions` and
//! `changedFiles` are scalars and cost nothing extra, while
//! `reviews { totalCount }` is a CONNECTION and is priced per search. A
//! leaderboard over lines changed is therefore affordable; one that adds
//! a review count is not free, and `MEASURED_*` below records what I
//! measured rather than what I assumed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// GitHub's hourly GraphQL budget, from the `rateLimit` object itself.
///
/// Not a constant the app should rely on -- `Budget` reads the real
/// `remaining` off every response -- but it is the denominator a
/// projection needs before the first response has arrived.
pub const HOURLY_BUDGET: u64 = 5_000;

/// Below this many remaining points, a new scope load is refused.
///
/// The poll loop is the thing being protected, not this feature. It is
/// the only part of the app with a standing obligation: it must keep
/// answering every 60s or the user's PR list goes stale and the tray
/// badge lies. `poll.rs:1503-1580` measures that obligation at well
/// under 500 points/hour for both cadences, and `client.rs:906-912`
/// already treats 500 remaining as the warning line.
///
/// 500 is therefore that same line, reused deliberately: one number for
/// "the budget is nearly gone" rather than two that can disagree. A
/// stats load refused here costs the user a leaderboard they can retry
/// in an hour; a poll loop starved by one costs them the feature the app
/// exists for.
pub const RESERVE: u64 = 500;

/// MEASURED, 2026-09-11, against the live API with `gh api graphql`.
///
/// An aliased `search { issueCount }` document costs 1 point in total
/// regardless of alias count. Confirmed at 36, 60 and 80 aliases, each
/// 3 runs, all `cost: 1` -- which is the fact the whole batching strategy
/// rests on and matches what `query.rs:212-219` recorded at 66 aliases.
pub const MEASURED_PROBE_COST: u64 = 1;

/// MEASURED, 2026-09-11, live: an aliased document of searches taking
/// `first: 100` nodes with `additions deletions changedFiles
/// author { login }` costs **1 point** in total. Confirmed at 12, 24 and
/// 36 aliases over sparse ranges, and at 3 aliases over dense ones (279
/// real PR nodes) -- `cost: 1` every time.
///
/// This is the number that makes a complete leaderboard affordable, and
/// it was worth measuring rather than assuming: 1,200 PR nodes with
/// per-PR diff statistics for one point is not what the
/// `MERGED_DETAIL_QUERY` experience suggests. The difference is that
/// those three fields are SCALARS. `MERGED_DETAIL_QUERY` costs what it
/// costs (and measures 6.5s for 100 nodes, `client.rs:1024-1050`)
/// because of its `reviews { totalCount }` and `comments { totalCount }`
/// connections, not because of the diff statistics.
///
/// The LATENCY, not the cost, is what bounds this shape, and it is bounded
/// by NODES materialised rather than by aliases -- 3 aliases at `first:
/// 100` over dense ranges intermittently 502'd at 11.0s while the same 3
/// at `first: 50` took 3.4s. See `query::ALIAS_CHUNK` and
/// `fetch::SLICE_PAGE_FULL`.
pub const MEASURED_DETAIL_CHUNK_COST: u64 = 1;

/// What one scope load spent.
///
/// Cheap to clone: the counters are in an `Arc`, so every concurrent
/// request in a load shares one accumulator rather than reporting into
/// separate ones that somebody then has to remember to add up.
#[derive(Debug, Clone)]
pub struct Budget {
    spent: Arc<AtomicU64>,
    requests: Arc<AtomicU64>,
    /// The LOWEST `remaining` any response reported, or `u64::MAX` if
    /// none has. The pessimistic value, because requests are concurrent
    /// and a refusal should be based on the worst thing GitHub said
    /// rather than on whichever response happened to land last.
    lowest_remaining: Arc<AtomicU64>,
    /// Requests that reported no cost -- see [`Budget::unmetered`].
    unmetered: Arc<AtomicU64>,
    /// The LATEST `resetAt` seen, as epoch seconds, or 0.
    ///
    /// Latest rather than first for the same reason: if the window rolled
    /// over mid-load, the later reset is the one a "try again at" message
    /// must quote, or the message tells the user to retry at a time that
    /// has already passed.
    reset_at: Arc<AtomicU64>,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

impl Budget {
    pub fn new() -> Self {
        Self {
            spent: Arc::new(AtomicU64::new(0)),
            requests: Arc::new(AtomicU64::new(0)),
            lowest_remaining: Arc::new(AtomicU64::new(u64::MAX)),
            unmetered: Arc::new(AtomicU64::new(0)),
            reset_at: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Record one response's `rateLimit` object.
    ///
    /// Takes the whole `data` value rather than three numbers so there is
    /// exactly one place that knows where `rateLimit` sits in a response,
    /// and so a query that FORGOT to select it is visible: the request is
    /// still counted, and `unmetered()` reports the gap.
    ///
    /// A missing `rateLimit` does NOT default `cost` to 1. That would be
    /// a guess recorded as a measurement, which is the failure mode this
    /// whole module exists to fix -- the accumulated total would look
    /// authoritative while containing invented numbers. It counts as
    /// unmetered instead, and says so.
    pub fn record(&self, data: &serde_json::Value) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let rl = &data["rateLimit"];
        let Some(cost) = rl["cost"].as_u64() else {
            self.unmetered.fetch_add(1, Ordering::Relaxed);
            return;
        };
        self.spent.fetch_add(cost, Ordering::Relaxed);
        if let Some(remaining) = rl["remaining"].as_u64() {
            self.lowest_remaining
                .fetch_min(remaining, Ordering::Relaxed);
        }
        if let Some(reset) = rl["resetAt"].as_str() {
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(reset) {
                let secs = t.timestamp().max(0) as u64;
                self.reset_at.fetch_max(secs, Ordering::Relaxed);
            }
        }
    }

    /// Points spent by this load so far.
    pub fn spent(&self) -> u64 {
        self.spent.load(Ordering::Relaxed)
    }

    /// Requests issued by this load so far, metered or not.
    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// The lowest `remaining` GitHub reported during this load.
    ///
    /// `None` before any response has carried a `rateLimit` -- which is
    /// genuinely different from "zero remaining" and must not render as
    /// it. The same NULL-not-0 rule `store/schema.rs` migration 8 states
    /// for health samples: "not measured" and "measured as zero" are
    /// opposite answers.
    pub fn remaining(&self) -> Option<u64> {
        match self.lowest_remaining.load(Ordering::Relaxed) {
            u64::MAX => None,
            n => Some(n),
        }
    }

    /// When the hourly window rolls over, if a response said.
    pub fn reset_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        match self.reset_at.load(Ordering::Relaxed) {
            0 => None,
            secs => chrono::DateTime::from_timestamp(secs as i64, 0),
        }
    }

    /// Requests that carried no `rateLimit` object.
    ///
    /// Should be zero: every stats query selects it, and
    /// `tests::every_stats_query_meters_itself` fails CI if one stops.
    /// Non-zero at runtime means a request failed before GitHub answered,
    /// or a query was added without the field -- and either way the
    /// accumulated spend UNDERSTATES the truth, which is the direction
    /// that matters. A reported total must say so rather than look exact.
    ///
    /// COUNTED, not derived. An earlier version of this derived it as
    /// `requests - min(spent, requests)`, on the reasoning that a metered
    /// request costs at least one point so the count of metered requests
    /// cannot exceed the points spent. The bound is true and the
    /// derivation is still wrong: it compares a number of POINTS against a
    /// number of REQUESTS, so one request costing 3 points masks two that
    /// reported nothing, and the load claims to be exactly metered while
    /// two thirds of it was not. A unit confusion is not the thing to
    /// economise a counter on.
    pub fn unmetered(&self) -> u64 {
        self.unmetered.load(Ordering::Relaxed)
    }

    /// Whether there is budget for a load projected to cost `projected`.
    ///
    /// Refuses when the spend would leave less than [`RESERVE`], so the
    /// poll loop keeps working. Item 1 of #824 asks for "refuse, or warn
    /// hard" -- this is the refusal, and [`Spend::pressure`] is the warning.
    ///
    /// `None` remaining means no response has reported one yet, which is
    /// the state at the START of the first load of a session. Permitted:
    /// refusing on the absence of information would make a cold start
    /// fail, and the first response will supply the real number before
    /// the load has spent anything meaningful.
    pub fn permits(&self, projected: u64) -> bool {
        match self.remaining() {
            None => true,
            Some(remaining) => remaining.saturating_sub(projected) >= RESERVE,
        }
    }

    /// A snapshot for the UI and the logs.
    pub fn snapshot(&self) -> Spend {
        Spend {
            points: self.spent(),
            requests: self.requests(),
            unmetered: self.unmetered(),
            remaining: self.remaining(),
            reset_at: self.reset_at(),
        }
    }
}

/// What a load cost, in the form the frontend and the log both read.
///
/// Serialised camelCase to match every other command return type in this
/// app.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    /// Rate-limit points this load spent.
    pub points: u64,
    /// Requests it issued.
    pub requests: u64,
    /// Requests whose cost could not be read -- see [`Budget::unmetered`].
    /// Non-zero means `points` is a FLOOR, not a total.
    pub unmetered: u64,
    /// The lowest remaining budget GitHub reported. `None` means nothing
    /// reported one, which is not the same as zero.
    pub remaining: Option<u64>,
    pub reset_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Spend {
    /// Whether `points` is exact or a floor.
    ///
    /// #824's item 8 applied to the budget itself: "anything partial,
    /// capped or assembled from slices says so in its return value".
    /// A spend total assembled from responses, some of which never
    /// reported a cost, is exactly that shape.
    pub fn is_exact(&self) -> bool {
        self.unmetered == 0
    }

    /// How close the budget is to the reserve, 0.0 to 1.0.
    ///
    /// `None` when nothing reported a remaining figure. 1.0 means the
    /// reserve is reached -- the point at which [`Budget::permits`]
    /// starts refusing.
    pub fn pressure(&self) -> Option<f64> {
        let remaining = self.remaining?;
        let usable = HOURLY_BUDGET.saturating_sub(RESERVE) as f64;
        if usable <= 0.0 {
            return Some(1.0);
        }
        let used = HOURLY_BUDGET.saturating_sub(remaining) as f64;
        Some((used / usable).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rl(cost: u64, remaining: u64, reset: &str) -> serde_json::Value {
        json!({ "rateLimit": { "cost": cost, "remaining": remaining, "resetAt": reset } })
    }

    #[test]
    fn accumulates_cost_across_requests() {
        let b = Budget::new();
        b.record(&rl(1, 4999, "2026-09-11T16:52:14Z"));
        b.record(&rl(4, 4995, "2026-09-11T16:52:14Z"));
        assert_eq!(b.spent(), 5);
        assert_eq!(b.requests(), 2);
        assert!(b.snapshot().is_exact());
    }

    /// Requests in a load are CONCURRENT, so "the last response" is a
    /// race. A refusal has to be based on the worst thing GitHub said,
    /// not on whichever reply won.
    #[test]
    fn remaining_is_the_lowest_seen_not_the_last() {
        let b = Budget::new();
        b.record(&rl(1, 900, "2026-09-11T16:52:14Z"));
        b.record(&rl(1, 400, "2026-09-11T16:52:14Z"));
        // Arrives last, reports a HIGHER figure -- a response that was
        // issued earlier and overtaken.
        b.record(&rl(1, 890, "2026-09-11T16:52:14Z"));
        assert_eq!(b.remaining(), Some(400));
    }

    /// If the window rolled over mid-load, the LATER reset is the one a
    /// "try again at" message must quote -- the earlier one has already
    /// passed by the time the user reads it.
    #[test]
    fn reset_at_is_the_latest_seen() {
        let b = Budget::new();
        b.record(&rl(1, 100, "2026-09-11T16:00:00Z"));
        b.record(&rl(1, 100, "2026-09-11T17:00:00Z"));
        b.record(&rl(1, 100, "2026-09-11T16:30:00Z"));
        assert_eq!(
            b.reset_at().map(|t| t.to_rfc3339()),
            Some("2026-09-11T17:00:00+00:00".to_string())
        );
    }

    /// "Nothing reported a remaining figure" and "zero remaining" are
    /// opposite answers. Rendering the first as the second would claim
    /// the budget is gone on a cold start.
    #[test]
    fn no_response_yet_is_not_zero_remaining() {
        let b = Budget::new();
        assert_eq!(b.remaining(), None);
        assert_eq!(b.reset_at(), None);
        // And the absence does not refuse the first load of a session.
        assert!(b.permits(50));
    }

    /// A query that forgot `rateLimit` must NOT have its cost guessed at
    /// one point. The guess would be indistinguishable from a measurement
    /// in the accumulated total, which is the exact defect this module
    /// exists to fix.
    #[test]
    fn an_unmetered_response_is_counted_but_not_costed() {
        let b = Budget::new();
        b.record(&rl(3, 4997, "2026-09-11T16:52:14Z"));
        b.record(&json!({ "merged_week": { "issueCount": 7 } }));
        assert_eq!(b.spent(), 3, "no invented cost for the unmetered request");
        assert_eq!(b.requests(), 2);
        assert_eq!(b.unmetered(), 1);
        let s = b.snapshot();
        assert!(
            !s.is_exact(),
            "a total containing an unmetered request is a floor, not a total"
        );
    }

    /// The poll loop must keep working. It is the only part of the app
    /// with a standing obligation, and a leaderboard is never worth
    /// starving it.
    #[test]
    fn refuses_a_load_that_would_eat_the_poll_loops_reserve() {
        let b = Budget::new();
        b.record(&rl(1, 600, "2026-09-11T16:52:14Z"));
        assert!(b.permits(100), "600 - 100 = 500, exactly the reserve");
        assert!(!b.permits(101), "601 would breach it");
        assert!(!b.permits(10_000));
    }

    /// A load that has already driven the budget under the reserve
    /// refuses even a single further point.
    #[test]
    fn refuses_everything_once_under_the_reserve() {
        let b = Budget::new();
        b.record(&rl(1, 499, "2026-09-11T16:52:14Z"));
        assert!(!b.permits(1));
        assert!(!b.permits(0) || RESERVE == 499);
    }

    #[test]
    fn pressure_rises_from_zero_to_one_at_the_reserve() {
        let b = Budget::new();
        b.record(&rl(1, HOURLY_BUDGET, "2026-09-11T16:52:14Z"));
        assert_eq!(b.snapshot().pressure(), Some(0.0));

        let b = Budget::new();
        b.record(&rl(1, RESERVE, "2026-09-11T16:52:14Z"));
        assert_eq!(b.snapshot().pressure(), Some(1.0));

        // Past the reserve it clamps rather than exceeding 1.0, so a
        // gauge cannot render past full.
        let b = Budget::new();
        b.record(&rl(1, 0, "2026-09-11T16:52:14Z"));
        assert_eq!(b.snapshot().pressure(), Some(1.0));

        // And unknown stays unknown rather than becoming 0.0, which
        // would render as "plenty of budget" on no information at all.
        assert_eq!(Budget::new().snapshot().pressure(), None);
    }

    /// One accumulator shared across concurrent requests, which is the
    /// whole reason the counters are atomics in an `Arc`.
    #[test]
    fn clones_share_one_accumulator() {
        let b = Budget::new();
        let c = b.clone();
        b.record(&rl(2, 4998, "2026-09-11T16:52:14Z"));
        c.record(&rl(3, 4995, "2026-09-11T16:52:14Z"));
        assert_eq!(b.spent(), 5);
        assert_eq!(c.spent(), 5);
        assert_eq!(b.requests(), 2);
    }

    /// The reserve is the number `client.rs` already warns at. Two
    /// numbers for "the budget is nearly gone" could disagree, and the
    /// user would get a warning from one and a refusal from the other at
    /// different moments.
    #[test]
    fn the_reserve_matches_the_existing_low_budget_warning() {
        let src = include_str!("../client.rs");
        assert!(
            src.contains("remaining < 500"),
            "client.rs no longer warns at 500; RESERVE must move with it"
        );
        assert_eq!(RESERVE, 500);
    }

    /// Item 1 of #824: `rateLimit` on EVERY stats query, not three of
    /// seven. Asserted on the query source, because no mapper test can
    /// tell the difference -- they feed JSON literals and pass happily
    /// against a document that never asked for the field. This is the
    /// same reasoning `query.rs`'s own
    /// `the_detail_query_asks_for_every_thread_and_its_true_count` gives.
    #[test]
    fn every_stats_query_meters_itself() {
        let src = include_str!("../query.rs");
        // ALL FOUR that #824 names as missing it, not just the const.
        // `STATS_QUERY` is a document; the other three BUILD documents,
        // and each was hardcoded `author:@me` with no `rateLimit` -- so
        // each had to be changed, and each can be changed back.
        //
        // Scoped to the text between this name and the next top-level
        // `pub`, so the field has to be inside the right document rather
        // than merely somewhere in a file that has many.
        // Anchored on the DEFINITION, not the first mention of the name.
        // An earlier version searched for the bare name and found
        // `history_query_range`'s call site inside `history_query` three
        // lines above the function, then scanned the wrong body and
        // failed on a document that does select the field. A test that
        // reads the wrong region is worse than none: it reports a defect
        // at a location that does not have one.
        for (name, anchor) in [
            ("STATS_QUERY", "pub const STATS_QUERY"),
            ("history_query_range", "pub fn history_query_range("),
            ("periods_query", "pub fn periods_query("),
            ("cycle_trend_query", "pub fn cycle_trend_query("),
        ] {
            let from = src
                .find(anchor)
                .unwrap_or_else(|| panic!("{name} definition not found in query.rs"));
            let body = &src[from..];
            // To the next top-level item, so the field has to be inside
            // THIS document rather than merely somewhere below it.
            let end = body[anchor.len()..]
                .find("\npub ")
                .map_or(body.len(), |i| i + anchor.len());
            assert!(
                body[..end].contains("rateLimit"),
                "{name} does not select rateLimit; its cost cannot be read"
            );
        }
        // And the builders in this module's own query file.
        let stats = include_str!("query.rs");
        for name in ["probe_query", "slice_detail_query"] {
            let from = stats
                .find(&format!("pub fn {name}"))
                .unwrap_or_else(|| panic!("{name} not found"));
            let body = &stats[from..];
            let end = body[1..].find("\npub fn ").map_or(body.len(), |i| i + 1);
            assert!(
                body[..end].contains("rateLimit"),
                "{name} must select rateLimit so its cost can be read"
            );
        }
    }
}
