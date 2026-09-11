//! The execution layer: bounded concurrency, a wall-clock ceiling,
//! connection-first routing, and the degradation ladder inherited from
//! `client.rs` rather than reinvented.
//!
//! `slice.rs` decides WHAT to ask and `budget.rs` records what it cost.
//! This module is the part that actually issues requests, and everything
//! in it exists because its absence has already caused a shipped bug
//! here -- see the table in #824.

use super::budget::Budget;
use super::query::{probe_query, slice_detail_query, Slice, ALIAS_CHUNK, REPO_CONNECTION_QUERY};
use super::scope::{Scope, StatsQuery};
use super::slice::{self, Plan};
use crate::github::client::{ClientError, GitHubClient};
use serde_json::json;

/// How many stats reads may be in flight at once.
///
/// # Why reads need a cap at all
///
/// `BATCH_CONCURRENCY = 4` in `commands.rs:437-444` caps MUTATIONS,
/// because "GitHub applies secondary rate limits to concurrent
/// mutations". Reads have had no equivalent, and the shape that needs one
/// already exists: `fetch_history_values` (`client.rs:947-986`) spawns
/// `days / HISTORY_CHUNK_DAYS` concurrent POSTs, which is **18** at the
/// 90-day clamp. That has been fine because 18 is the ceiling and it is
/// reached only by one user action.
///
/// This feature has no such ceiling. An org-scope leaderboard fans out
/// over probe rounds, then over slices, then potentially over members --
/// the product of three counts that all depend on how much activity the
/// scope contains. Unbounded, a busy org is exactly the "many concurrent
/// requests from one client" shape that secondary limits exist to stop,
/// and a secondary limit is not a clean 403: GitHub's documented
/// behaviour is to start refusing, and octocrab is configured
/// `HandleRateLimits { max_retries: 3, min_wait_seconds: 60 }`
/// (`client.rs:1070-1076`), so each refusal becomes a 60-second wait
/// inside a request that the layer above simply waits on.
///
/// # Why 6
///
/// Bounded from below by the shipped evidence and from above by the
/// mutation cap's reasoning:
///
/// - **Not below 4.** `fetch_history_values` has shipped at 18 concurrent
///   reads without a reported secondary-limit failure, so the real
///   threshold is above 18 and a cap of 1 or 2 would be slower than what
///   already works for no measured reason.
/// - **Not 18.** That number is an accident of `HISTORY_CHUNK_DAYS` and
///   the 90-day clamp, not a measurement of anything. Inheriting it would
///   be inheriting a coincidence.
/// - **6, because the latency it has to hide is ~2-11s per request.** I
///   measured a 10-alias probe document at 1.78-1.81s and the expensive
///   node-heavy document at 3.69-5.21s for 12 aliases (live, 2026-09-11).
///   Six in flight covers 60 slices per round-trip at the probe shape,
///   which plans the whole 1,822-PR FNX-Labs history in one round-trip's
///   worth of wall clock. Going wider buys less and less -- the planner
///   needs at most `ALIAS_CHUNK` x 6 = 60 slices per round and rarely
///   that many -- while the risk it adds is a secondary limit whose
///   penalty is a 60-second stall, not a retry.
///
/// Deliberately HIGHER than the mutation cap of 4 and deliberately
/// stated: a mutation that trips a secondary limit may have partially
/// applied and has to be reasoned about, where a refused read is just a
/// read to do again. The asymmetry is the justification for the two
/// numbers differing rather than being unified.
pub const READ_CONCURRENCY: usize = 6;

/// Wall-clock ceiling on one whole scope load.
///
/// # Why this is not `poll::FETCH_TIMEOUT`
///
/// `poll::FETCH_TIMEOUT` is 30s and bounds ONE fetch. Its own doc
/// (`poll.rs:709-736`) explains the property that sets it: it must be
/// below `MIN_FOCUSED_SECS` so a hung poll lands strictly before the next
/// tick, or two fetches overlap and each spends budget. A stats load has
/// no cadence and no successor to collide with, so that constraint does
/// not apply here and copying 30s would be copying a number for a reason
/// that is not present.
///
/// What DOES apply is the reason it exists at all, and it applies more
/// strongly: octocrab is `HandleRateLimits { max_retries: 3,
/// min_wait_seconds: 60 }` (`client.rs:1070-1076`), so **one POST can
/// legitimately block for minutes** while every layer above it waits. A
/// load issuing dozens of POSTs inherits that per-POST hazard dozens of
/// times over. Only a wall-clock ceiling around the whole thing bounds
/// what the user is actually waiting on -- the same argument
/// `commands::get_pr_detail` makes at `commands.rs:604-633`, and the same
/// one that #790 shipped without.
///
/// # Why 60s
///
/// Derived from what a load legitimately needs, at measured latencies:
///
/// - A probe round is one request per [`ALIAS_CHUNK`] slices, measured at
///   1.78-1.81s for 10 aliases. The FNX-Labs history plans in 4 rounds,
///   so planning is ~8s even when every round is serial (they are, by
///   construction -- each round's slices depend on the last round's
///   counts).
/// - A detail fetch is one request per chunk, measured at 3.42-3.68s for
///   3 aliases at the default 50-node page over the densest slices in this
///   account, and those ARE parallel at [`READ_CONCURRENCY`]. The
///   FNX-Labs plan lands ~16 slices, which is two waves: ~10s.
///
/// So a worst realistic load is ~20-25s, and 60s is between two and three
/// times that. Generous on purpose, for the reason `get_pr_detail`'s
/// comment gives: "the budget exists to convert an unbounded hang into an
/// actionable error, not to tighten a latency target". A real load that
/// needs 40 seconds should succeed.
///
/// Bounded ABOVE by what a user will sit in front of. This is behind an
/// explicit load (#823 item 4), so the user asked and is watching; a
/// ceiling past a minute is a ceiling nobody waits for.
pub const LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Nodes per slice on the first attempt, and after degrading.
///
/// The ladder is `client.rs:1024-1050`'s, inherited rather than
/// reinvented: `MERGED_DETAIL_QUERY` halves `first: 100` to 50 when the
/// server gives up, because the three diff-statistic fields are ~60% of
/// its measured 6.5s and a reported log showed 124
/// `RESOURCE_LIMITS_EXCEEDED` errors on that shape.
///
/// # The default is 50, not 100, and that is a MEASUREMENT
///
/// This layer multiplies that query by its alias count, so I measured the
/// real document against dense real data -- three month-slices of
/// `org:FNX-Labs` holding 416 / 581 / 79 merged PRs, 2026-09-11:
///
/// | Document            | Wall clock         | Result      |
/// |---------------------|--------------------|-------------|
/// | 3 aliases x 100     | 11.0s, then 8.1s   | **502**, ok |
/// | 3 aliases x 50      | 3.42-3.68s         | ok 2/2      |
/// | 1 alias x 100       | 0.52s              | ok 2/2      |
///
/// THREE node-heavy aliases at `first: 100` straddle the ~11s deadline on
/// real data -- intermittently failing where 36 aliases over SPARSE ranges
/// succeeded in 9.22-10.84s. The difference is the number of nodes
/// actually materialised, not the number of aliases: the sparse ranges
/// returned a handful of PRs each and these return the full page.
///
/// Halving the page took 11.0s to 3.5s, a 3x improvement, which makes it
/// by far the most effective knob. So 50 is the DEFAULT rather than the
/// first rung of the ladder. Going out at 100 would mean a load that
/// intermittently 502s on its first attempt and recovers only via the
/// ladder -- paying a wasted request and a ~11s stall for it, every time,
/// on exactly the dense scopes this feature is for.
///
/// The cost of a 50-node page is more slices, not less data: the slicer
/// already subdivides until each slice is under 800, and the nodes are
/// paged per slice either way.
pub const SLICE_PAGE_FULL: u32 = 50;
pub const SLICE_PAGE_REDUCED: u32 = 25;

/// One rung down the ladder: a smaller page, then a smaller chunk.
///
/// # Page before aliases, which is the opposite of my first guess
///
/// I initially shed aliases first, reasoning that the 502 is a ~11s
/// deadline and alias count drives elapsed time toward it. The sparse
/// measurements supported that -- 48 node-heavy aliases failed where 80
/// count-only ones succeeded. The dense measurements above overturned it:
/// at three aliases, halving the PAGE took the document from 11.0s to
/// 3.5s, where halving the aliases could at best have taken it to ~7s.
///
/// The reason is that the work is per NODE, not per alias. An alias over
/// a sparse range is nearly free; an alias returning a full page of 100
/// PRs with per-PR diff statistics is not. Shedding aliases first would
/// have thrown away parallelism to fix a problem caused by page size, and
/// would have needed two or three rungs to achieve what one page halving
/// does.
///
/// Returns `None` at the bottom. A single alias at 25 nodes is the
/// smallest useful request; if the server gives up on that, asking for
/// less is not the problem and the error belongs to the caller.
pub fn degrade(chunk: usize, page: u32) -> Option<(usize, u32)> {
    if page > SLICE_PAGE_REDUCED {
        return Some((chunk, SLICE_PAGE_REDUCED));
    }
    if chunk > 1 {
        return Some(((chunk / 2).max(1), page));
    }
    None
}

/// What a scope load returned, and how complete it is.
///
/// #824 item 8 in the type system: there is no way to read a total out of
/// this without also being handed the facts about whether it is exact.
/// A caller that wants a bare number has to call [`Outcome::total`] and
/// can see [`Outcome::is_complete`] right beside it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The exact count, summed from `issueCount` across every slice.
    /// Exact even when `retrievable` is false -- the 1,000 cap limits
    /// retrieval, not counting.
    pub total: u64,
    /// Whether every PR in the window could be RETRIEVED, not merely
    /// counted. False means per-PR detail is over a sample.
    pub retrievable: bool,
    /// How many PRs sit in slices whose nodes could not all be fetched.
    pub unretrievable: u64,
    /// How many slices the window was cut into. > 1 means the total is
    /// assembled, which #824 item 8 requires be visible.
    pub slices: usize,
    /// Probe rounds the plan took.
    pub rounds: u32,
    /// Whether the answer came from a connection (exact, uncapped) or
    /// from search (capped, sliced). Item 5's routing decision, reported
    /// so a reader can tell which guarantee they have.
    pub via_connection: bool,
    /// What it cost. See `budget.rs`.
    pub spend: super::budget::Spend,
    /// Fields GitHub refused on the responses behind this total, summed.
    /// Non-zero means some data is missing -- inherited from
    /// `client.rs:1094-1175`'s partial-success handling.
    pub refused_fields: usize,
}

impl Outcome {
    /// Whether this total can be presented as a plain number.
    ///
    /// False if anything is capped, refused, or unmetered. The UI in #826
    /// must branch on this rather than on a truthiness check of
    /// `unretrievable`, so that a new partiality channel added later
    /// cannot be forgotten at one call site.
    pub fn is_complete(&self) -> bool {
        self.retrievable && self.refused_fields == 0
    }

    /// Whether the count was assembled from more than one request.
    pub fn is_assembled(&self) -> bool {
        self.slices > 1
    }
}

/// Fetch a count for a window, completely.
///
/// The routing decision of #824 item 5 is the first thing here: a
/// single-repo scope goes to the connection, which has no 1,000-result
/// cap and needs no slicing at all. Everything else is sliced.
///
/// Bounded by [`LOAD_TIMEOUT`] around the WHOLE load, not per request --
/// see its docs for why a per-request bound is not enough.
pub async fn load_count(
    client: &GitHubClient,
    q: &StatsQuery,
    window: Slice,
    budget: &Budget,
) -> Result<Outcome, ClientError> {
    match tokio::time::timeout(LOAD_TIMEOUT, load_count_inner(client, q, window, budget)).await {
        Ok(r) => r,
        Err(_) => Err(ClientError::Timeout(LOAD_TIMEOUT.as_secs())),
    }
}

async fn load_count_inner(
    client: &GitHubClient,
    q: &StatsQuery,
    window: Slice,
    budget: &Budget,
) -> Result<Outcome, ClientError> {
    // CONNECTION-FIRST. `repository.pullRequests` has no 1,000-result
    // cap (VERIFIED: pktstorm/headstate, 337 merged, cost 1), so a
    // single-repo scope is exact with no slicing and no probe rounds.
    if !q.scope.needs_search() {
        if let Some((owner, name)) = q.scope.owner_name() {
            return connection_count(client, owner, name, budget).await;
        }
        // A `Scope::Repo` whose value is not `owner/name` falls through
        // to search rather than failing: search still answers it
        // correctly, and a malformed scope is a caller bug that should
        // not take the feature down.
        log::warn!("single-repo scope is not owner/name; falling back to search");
    }

    let plan = plan(client, q, window, budget).await?;
    Ok(Outcome {
        total: plan.total(),
        retrievable: plan.is_retrievable(),
        unretrievable: plan.unretrievable(),
        slices: plan.slices.len(),
        rounds: plan.rounds,
        via_connection: false,
        spend: budget.snapshot(),
        refused_fields: 0,
    })
}

/// Build a complete plan for a window, running each probe round's chunks
/// concurrently at [`READ_CONCURRENCY`].
///
/// The ROUNDS are serial by construction -- each round's slices are
/// chosen from the previous round's counts, which is what "probe-driven"
/// means -- and the chunks WITHIN a round are parallel. That is the only
/// parallelism available here, and it is the one that matters: a round is
/// one request per `ALIAS_CHUNK` slices.
async fn plan(
    client: &GitHubClient,
    q: &StatsQuery,
    window: Slice,
    budget: &Budget,
) -> Result<Plan, ClientError> {
    // `plan_with` is synchronous and takes a closure, because that is
    // what makes the completeness property testable without a network
    // (see `slice.rs`). Bridging to async here rather than making the
    // planner async keeps the test-only path honest: the code under test
    // is the code that ships.
    let mut pending = vec![window];
    let mut done = Vec::new();
    let mut irreducible = Vec::new();
    let mut rounds = 0;

    while !pending.is_empty() && rounds < slice::MAX_DEPTH {
        rounds += 1;
        let counts = probe_round(client, q, &pending, budget).await?;
        let mut next = Vec::new();
        for (s, count) in pending.into_iter().zip(counts) {
            let probed = slice::ProbedSlice { slice: s, count };
            if !probed.too_big() {
                done.push(probed);
                continue;
            }
            if slice::is_one_day(&probed.slice) {
                irreducible.push(probed.clone());
                done.push(probed);
                continue;
            }
            let pieces = slice::subdivide(&probed.slice, slice::split_factor(probed.count));
            if pieces.len() < 2 {
                irreducible.push(probed.clone());
                done.push(probed);
                continue;
            }
            next.extend(pieces);
        }
        pending = next;
    }
    for s in pending {
        let probed = slice::ProbedSlice { slice: s, count: 0 };
        irreducible.push(probed.clone());
        done.push(probed);
    }
    done.sort_by(|a, b| a.slice.from.cmp(&b.slice.from));
    Ok(Plan {
        slices: done,
        irreducible,
        rounds,
    })
}

/// One probe round: every slice's `issueCount`, in chunks of
/// [`ALIAS_CHUNK`] run at [`READ_CONCURRENCY`].
///
/// Counts come back in the SAME ORDER as the slices went in, which the
/// caller relies on to pair them. Order is preserved by indexing into a
/// pre-sized vector rather than by collection order, because the chunks
/// complete out of order -- the same hazard `query.rs:667-678` records
/// for the history series, solved the same way: absolute indices.
async fn probe_round(
    client: &GitHubClient,
    q: &StatsQuery,
    slices: &[Slice],
    budget: &Budget,
) -> Result<Vec<u64>, ClientError> {
    let mut counts = vec![0u64; slices.len()];
    for wave in slices.chunks(ALIAS_CHUNK * READ_CONCURRENCY) {
        let base = offset_of(slices, wave);
        let mut set = tokio::task::JoinSet::new();
        for (n, chunk) in wave.chunks(ALIAS_CHUNK).enumerate() {
            let first_index = base + n * ALIAS_CHUNK;
            let doc = probe_query(q, chunk, first_index);
            let client = client.clone();
            let budget = budget.clone();
            let len = chunk.len();
            set.spawn(async move {
                let v = client.stats_graphql(&json!({ "query": doc })).await?;
                budget.record(&v);
                let mut out = Vec::with_capacity(len);
                for i in 0..len {
                    let alias = super::query::slice_alias(first_index + i);
                    // A missing alias is NOT zero. GitHub returning
                    // partial data (`client.rs:1094-1175`) would leave
                    // the alias absent, and defaulting it to 0 would
                    // make a slice that failed look empty -- which is
                    // the exact shape that turns a truncation into a
                    // confident wrong number. Reported as an error so
                    // the load fails rather than under-counting.
                    let c = v[&alias]["issueCount"].as_u64().ok_or_else(|| {
                        ClientError::Graphql(format!(
                            "probe response missing issueCount for slice {i} of {len}"
                        ))
                    })?;
                    out.push(c);
                }
                Ok::<(usize, Vec<u64>), ClientError>((first_index, out))
            });
        }
        while let Some(joined) = set.join_next().await {
            // A panicked task surfaced rather than swallowed: a dropped
            // chunk would leave zeros in `counts`, which reads as "no
            // activity in that range" and silently shrinks the total.
            // Same reasoning as `client.rs:975-977`.
            let (first_index, out) = joined.map_err(|e| ClientError::Join(e.to_string()))??;
            for (i, c) in out.into_iter().enumerate() {
                counts[first_index + i] = c;
            }
        }
    }
    Ok(counts)
}

/// Where `wave` starts within `all`, by pointer identity.
///
/// `chunks` yields subslices of the original, so this is exact and does
/// not depend on the contents being distinct -- two identical date ranges
/// in one round would break a content-based search.
fn offset_of(all: &[Slice], wave: &[Slice]) -> usize {
    let base = all.as_ptr() as usize;
    let here = wave.as_ptr() as usize;
    (here - base) / std::mem::size_of::<Slice>()
}

/// An exact count for ONE repository, through the uncapped connection.
///
/// `totalCount` on `repository.pullRequests` is the repository's real
/// total with no 1,000 ceiling -- VERIFIED live at 337 for
/// pktstorm/headstate, cost 1. One request, no probe rounds, no slices.
async fn connection_count(
    client: &GitHubClient,
    owner: &str,
    name: &str,
    budget: &Budget,
) -> Result<Outcome, ClientError> {
    let v = client
        .stats_graphql(&json!({
            "query": REPO_CONNECTION_QUERY,
            // `first: 1` because only `totalCount` is wanted here. The
            // page is not the cost -- `query.rs:480-484` records that
            // GitHub charges the connection, not the page -- so this is
            // about bytes on the wire, not points.
            "variables": { "owner": owner, "name": name, "first": 1, "after": null },
        }))
        .await?;
    budget.record(&v);
    let total = v["repository"]["pullRequests"]["totalCount"]
        .as_u64()
        .ok_or_else(|| {
            // Not defaulted to 0: a repository the token cannot see
            // returns a null `repository`, and reporting that as "no
            // pull requests" is the dishonest-empty-state failure #823
            // item 9 names.
            ClientError::Graphql(format!(
                "no pullRequests total for {owner}/{name} (private, renamed, or gone?)"
            ))
        })?;
    Ok(Outcome {
        total,
        // The connection has no cap, so a connection answer is
        // retrievable by construction. This is the whole reason item 5
        // prefers it.
        retrievable: true,
        unretrievable: 0,
        slices: 1,
        rounds: 0,
        via_connection: true,
        spend: budget.snapshot(),
        refused_fields: crate::github::client::refused_fields_of(&v),
    })
}

/// Fetch per-PR detail for a planned set of slices, degrading on 5xx.
///
/// The ladder is `client.rs:1024-1050`'s, with the extra rung my
/// measurements call for -- see [`degrade`]. Returns the raw alias map so
/// #826 can map it into whatever a leaderboard needs; mapping belongs
/// with the view that defines the shape, not here.
pub async fn load_detail(
    client: &GitHubClient,
    q: &StatsQuery,
    slices: &[Slice],
    budget: &Budget,
) -> Result<serde_json::Value, ClientError> {
    match tokio::time::timeout(
        LOAD_TIMEOUT,
        detail_with_ladder(client, q, slices, budget, ALIAS_CHUNK, SLICE_PAGE_FULL),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => Err(ClientError::Timeout(LOAD_TIMEOUT.as_secs())),
    }
}

async fn detail_with_ladder(
    client: &GitHubClient,
    q: &StatsQuery,
    slices: &[Slice],
    budget: &Budget,
    chunk: usize,
    page: u32,
) -> Result<serde_json::Value, ClientError> {
    match detail_round(client, q, slices, budget, chunk, page).await {
        Ok(v) => Ok(v),
        Err(e) if crate::github::client::server_gave_up_on(&e) => {
            let Some((next_chunk, next_page)) = degrade(chunk, page) else {
                // The bottom of the ladder. One alias, 50 nodes, and
                // GitHub still gave up: asking for less is not the
                // problem, so the error is the answer.
                return Err(e);
            };
            log::warn!(
                "GitHub could not answer a {chunk}-alias/{page}-node stats document ({e}); \
                 retrying at {next_chunk}/{next_page} -- the load will take longer"
            );
            Box::pin(detail_with_ladder(
                client, q, slices, budget, next_chunk, next_page,
            ))
            .await
        }
        Err(e) => Err(e),
    }
}

async fn detail_round(
    client: &GitHubClient,
    q: &StatsQuery,
    slices: &[Slice],
    budget: &Budget,
    chunk: usize,
    page: u32,
) -> Result<serde_json::Value, ClientError> {
    let mut merged = serde_json::Map::new();
    for wave in slices.chunks(chunk * READ_CONCURRENCY) {
        let base = offset_of(slices, wave);
        let mut set = tokio::task::JoinSet::new();
        for (n, part) in wave.chunks(chunk).enumerate() {
            let first_index = base + n * chunk;
            let doc = slice_detail_query(q, part, first_index, page);
            let client = client.clone();
            let budget = budget.clone();
            set.spawn(async move {
                let v = client.stats_graphql(&json!({ "query": doc })).await?;
                budget.record(&v);
                Ok::<serde_json::Value, ClientError>(v)
            });
        }
        while let Some(joined) = set.join_next().await {
            let v = joined.map_err(|e| ClientError::Join(e.to_string()))??;
            if let Some(obj) = v.as_object() {
                // Absolute alias indices, so chunks merge in any
                // completion order without clobbering each other --
                // `query.rs:667-678`'s rule.
                for (k, val) in obj {
                    merged.insert(k.clone(), val.clone());
                }
            }
        }
    }
    Ok(serde_json::Value::Object(merged))
}

/// Which scopes route through the connection rather than search.
///
/// Exposed so a caller can report WHICH guarantee it has before issuing
/// the request, and so the routing rule has one name.
pub fn routes_through_connection(scope: &Scope) -> bool {
    !scope.needs_search() && scope.owner_name().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads are capped, and the number is not inherited from the
    /// mutation cap by accident. Both halves asserted: a cap must exist,
    /// and it must be a DECISION -- equal to 4 would mean someone copied
    /// `BATCH_CONCURRENCY` without reading why it is 4.
    #[test]
    fn reads_have_their_own_justified_cap() {
        // `const` blocks, so these bounds fail the BUILD rather than a
        // test run -- the values are compile-time constants, and a guard
        // on a constant may as well be checked where it cannot be
        // skipped by running a subset of the suite.
        const {
            assert!(READ_CONCURRENCY >= 1);
            // Bounded above by the reasoning in the doc comment: the
            // penalty for tripping a secondary limit is a 60-second
            // stall (octocrab's min_wait_seconds), not a cheap retry.
            assert!(
                READ_CONCURRENCY <= 8,
                "concurrent reads past what the doc comment justifies"
            );
        }
        // And the mutation cap is still its own number, for its own
        // reason. If it moves, the asymmetry argument in the doc above
        // needs re-reading.
        let src = include_str!("../../commands.rs");
        assert!(
            src.contains("const BATCH_CONCURRENCY: usize = 4"),
            "the mutation cap moved; re-read why reads differ from it"
        );
    }

    /// A whole-load ceiling exists, and it is NOT `poll::FETCH_TIMEOUT`
    /// -- that number is set by a property (staying under
    /// `MIN_FOCUSED_SECS`) that a user-initiated load does not have.
    #[test]
    fn the_load_has_a_wall_clock_ceiling_of_its_own() {
        assert!(LOAD_TIMEOUT >= std::time::Duration::from_secs(30));
        // A ceiling past a minute is one nobody waits for, and this is
        // behind an explicit load so the user is watching.
        assert!(LOAD_TIMEOUT <= std::time::Duration::from_secs(120));
        // It must exceed a realistic load: ~8s of serial probe rounds
        // plus ~10s of parallel detail waves, measured. Three times that
        // is the headroom `get_pr_detail` argues for.
        assert!(LOAD_TIMEOUT >= std::time::Duration::from_secs(50));
    }

    /// The ladder sheds NODES before aliases, because the work is per
    /// node: on dense real data, halving the page took the document from
    /// 11.0s to 3.5s where halving the aliases could at best have
    /// reached ~7s. See [`degrade`] for the measurements and for why my
    /// first guess was the other way round.
    #[test]
    fn the_ladder_sheds_nodes_before_aliases() {
        let (chunk, page) = degrade(ALIAS_CHUNK, SLICE_PAGE_FULL).expect("first rung");
        assert_eq!(page, SLICE_PAGE_REDUCED);
        assert_eq!(chunk, ALIAS_CHUNK, "parallelism is kept on rung one");
    }

    /// The DEFAULT page is already the reduced one, because 3 node-heavy
    /// aliases at `first: 100` intermittently 502'd on real data
    /// (11.0s then 8.1s). Going out at 100 would mean paying a wasted
    /// request and a ~11s stall on exactly the dense scopes this feature
    /// exists for.
    #[test]
    fn the_default_page_is_sized_for_dense_real_data() {
        const {
            assert!(SLICE_PAGE_FULL == 50);
            assert!(
                SLICE_PAGE_FULL < 100,
                "100 straddles the ~11s deadline on dense slices; re-measure first"
            );
        }
    }

    /// The ladder terminates, and the bottom rung is the smallest
    /// useful request rather than an infinite regress.
    #[test]
    fn the_ladder_reaches_a_bottom_and_stops() {
        let mut rung = (ALIAS_CHUNK, SLICE_PAGE_FULL);
        let mut steps = 0;
        while let Some(next) = degrade(rung.0, rung.1) {
            assert!(
                next.0 < rung.0 || next.1 < rung.1,
                "a rung that reduces nothing would loop: {rung:?} -> {next:?}"
            );
            rung = next;
            steps += 1;
            assert!(steps < 20, "the ladder does not terminate");
        }
        assert_eq!(rung, (1, SLICE_PAGE_REDUCED), "the smallest useful request");
    }

    /// The ladder this inherits from still exists, and this layer's pages
    /// are DELIBERATELY smaller than its.
    ///
    /// `MERGED_DETAIL_QUERY` goes out at 100 and degrades to 50, which is
    /// right for ONE search. This layer sends several searches in one
    /// document, so the same page size materialises several times the
    /// nodes -- which is why the measurements put the default a rung
    /// lower rather than copying the numbers across.
    #[test]
    fn this_layers_pages_are_deliberately_smaller_than_the_single_search_ladder() {
        let src = include_str!("../client.rs");
        assert!(
            src.contains("const PAGE_FULL: u32 = 100")
                && src.contains("const PAGE_REDUCED: u32 = 50"),
            "client.rs's page ladder moved; re-read client.rs:1024-1050"
        );
        const {
            assert!(
                SLICE_PAGE_FULL < 100,
                "an aliased document materialises its page once PER ALIAS"
            );
            assert!(SLICE_PAGE_REDUCED < SLICE_PAGE_FULL);
        }
    }

    /// Item 5's routing: only a well-formed single-repo scope takes the
    /// uncapped connection. A malformed one falls back to search, which
    /// still answers correctly.
    #[test]
    fn only_a_well_formed_single_repo_scope_takes_the_connection() {
        assert!(routes_through_connection(&Scope::Repo(
            "pktstorm/headstate".into()
        )));
        assert!(!routes_through_connection(&Scope::Repo("nameless".into())));
        assert!(!routes_through_connection(&Scope::Org("FNX-Labs".into())));
        assert!(!routes_through_connection(&Scope::Personal(
            "pktstorm".into()
        )));
        assert!(!routes_through_connection(&Scope::All));
    }

    /// An `Outcome` cannot report a total without also reporting whether
    /// it is trustworthy. #824 item 8 in the type system.
    #[test]
    fn an_outcome_states_its_own_completeness() {
        let complete = Outcome {
            total: 337,
            retrievable: true,
            unretrievable: 0,
            slices: 1,
            rounds: 0,
            via_connection: true,
            spend: Budget::new().snapshot(),
            refused_fields: 0,
        };
        assert!(complete.is_complete());
        assert!(!complete.is_assembled());

        let capped = Outcome {
            retrievable: false,
            unretrievable: 260,
            slices: 12,
            ..complete.clone()
        };
        assert!(!capped.is_complete());
        assert!(capped.is_assembled());

        // A refusal is its own partiality channel, independent of the
        // cap -- inherited from `client.rs`'s partial-success handling.
        let refused = Outcome {
            refused_fields: 3,
            ..complete
        };
        assert!(
            !refused.is_complete(),
            "refused fields mean data is missing even when nothing was capped"
        );
    }

    /// Chunk offsets are computed by pointer identity, so two identical
    /// date ranges in one round cannot confuse the pairing.
    #[test]
    fn wave_offsets_survive_duplicate_ranges() {
        let all: Vec<Slice> = (0..25)
            .map(|_| Slice::new("2026-08-01", "2026-08-31"))
            .collect();
        let waves: Vec<&[Slice]> = all.chunks(10).collect();
        assert_eq!(offset_of(&all, waves[0]), 0);
        assert_eq!(offset_of(&all, waves[1]), 10);
        assert_eq!(offset_of(&all, waves[2]), 20);
    }

    /// One wave is `ALIAS_CHUNK * READ_CONCURRENCY` slices, which is the
    /// number of slices one round-trip's worth of wall clock covers.
    /// Pinned because the doc comment on `READ_CONCURRENCY` reasons
    /// about exactly this product.
    #[test]
    fn a_wave_covers_sixty_slices() {
        const { assert!(ALIAS_CHUNK * READ_CONCURRENCY == 60) };
    }
}
