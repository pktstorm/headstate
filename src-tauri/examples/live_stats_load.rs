//! Exercise a real scope load against the LIVE GitHub API.
//!
//! Unit tests prove the slicer is complete against a synthetic
//! distribution; they cannot prove the documents are valid GraphQL, that
//! the routing picks the right API, or that the metering reads a real
//! `rateLimit`. This does, and it is how the numbers in the doc comments
//! were obtained.
//!
//! Not a test: it needs a token, a network, and an account whose data
//! changes, so it would be a flaky CI failure. Run by hand:
//!
//!   cargo run --example live_stats_load

use headstate_lib::github::client::GitHubClient;
use headstate_lib::github::stats::{
    load_count, Budget, Measure, Scope, Slice, StatsQuery, Subject,
};

#[tokio::main]
async fn main() {
    let token = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .expect("gh auth token");
    let token = String::from_utf8_lossy(&token.stdout).trim().to_string();
    let oc = octocrab::Octocrab::builder()
        .personal_token(token)
        .build()
        .expect("client");
    let client = GitHubClient::new(oc);

    // Three scopes, chosen to exercise all three routing paths and both
    // sides of the 1,000-result cap.
    let cases = vec![
        (
            "single repo via CONNECTION (uncapped)",
            StatsQuery::new(
                Some(Subject::Login("pktstorm".into())),
                Scope::Repo("pktstorm/headstate".into()),
                Measure::Merged,
            ),
            Slice::new("2020-01-01", "2026-09-10"),
        ),
        (
            "org, whole history via SEARCH (over the cap -> sliced)",
            StatsQuery::new(None, Scope::Org("FNX-Labs".into()), Measure::Merged),
            Slice::new("2020-01-01", "2026-09-10"),
        ),
        (
            "a NON-viewer subject in an org",
            StatsQuery::new(
                Some(Subject::Login("dcd".into())),
                Scope::Org("FNX-Labs".into()),
                Measure::Merged,
            ),
            Slice::new("2026-01-01", "2026-09-10"),
        ),
    ];

    for (label, q, window) in cases {
        let budget = Budget::new();
        let started = std::time::Instant::now();
        match load_count(&client, &q, window.clone(), &budget).await {
            Ok(o) => println!(
                "{label}\n  total={} complete={} retrievable={} unretrievable={} \
                 slices={} rounds={} viaConnection={}\n  \
                 points={} requests={} unmetered={} remaining={:?} exact={}  [{:.2}s]",
                o.total,
                o.is_complete(),
                o.retrievable,
                o.unretrievable,
                o.slices,
                o.rounds,
                o.via_connection,
                o.spend.points,
                o.spend.requests,
                o.spend.unmetered,
                o.spend.remaining,
                o.spend.is_exact(),
                started.elapsed().as_secs_f32(),
            ),
            Err(e) => println!(
                "{label}\n  FAILED: {e}  [{:.2}s]",
                started.elapsed().as_secs_f32()
            ),
        }
    }
}
