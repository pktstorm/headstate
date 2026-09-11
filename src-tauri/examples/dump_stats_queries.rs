//! Print the stats layer's generated GraphQL documents.
//!
//! Exists so the documents this layer BUILDS can be validated against the
//! live API by hand (`gh api graphql -F query=@file`), which a
//! brace-balance unit test cannot do: a document can be perfectly
//! balanced and still name a field that does not exist or misuse an
//! argument, and the first time anyone would find out is at runtime.
//!
//! Run: `cargo run --example dump_stats_queries -- probe|detail|connection`

use headstate_lib::github::stats::query::{
    probe_query, slice_detail_query, Slice, REPO_CONNECTION_QUERY,
};
use headstate_lib::github::stats::scope::{Measure, Scope, StatsQuery, Subject};

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "probe".into());
    let q = StatsQuery::new(
        Some(Subject::Login("pktstorm".into())),
        Scope::Org("FNX-Labs".into()),
        Measure::Merged,
    );
    let slices = vec![
        Slice::new("2026-07-01", "2026-07-31"),
        Slice::new("2026-08-01", "2026-08-31"),
        Slice::new("2026-09-01", "2026-09-11"),
    ];
    match which.as_str() {
        "detail" => print!(
            "{}",
            slice_detail_query(
                &q,
                &slices,
                0,
                headstate_lib::github::stats::fetch::SLICE_PAGE_FULL
            )
        ),
        "connection" => print!("{REPO_CONNECTION_QUERY}"),
        _ => print!("{}", probe_query(&q, &slices, 0)),
    }
}
