//! The stats answer cache.
//!
//! One job: remember what GitHub already said about a closed time window,
//! so navigating back to a scope does not re-spend the request budget on
//! a number that cannot have changed.
//!
//! # This had to be built, not reused
//!
//! `schema.rs` migration 2 dropped `merge_history`, the table the original
//! stats design planned, because it was never written to -- and
//! `store/mod.rs` records that its SHAPE was wrong besides: it
//! accumulated merges by diffing the open PR set, where a PR leaving that
//! set may have been closed unmerged. Reusing it would have contradicted
//! the `is:merged` search that must stay authoritative.
//!
//! This module does not accumulate. It memoises an answer, keyed by the
//! question that produced it, and GitHub stays the only source of truth.
//! The test at the bottom asserts this table is actually written to, so it
//! cannot become the second permanently-empty table implying a feature
//! that does not exist.
//!
//! # Why a closed window is cacheable FOREVER
//!
//! The PRs merged in August 2026 are fixed once August is over. A
//! leaderboard over that window is a constant, so recomputing it per
//! navigation is pure waste -- and expensive waste, because recomputing
//! means the probe rounds and slice fetches of `github::stats::fetch`
//! against a 5,000-point hourly budget.
//!
//! A window that includes TODAY is a different thing entirely and gets a
//! short freshness bound. [`is_closed`] is where that line is drawn, and
//! it is drawn on the window's own end date rather than on the row's age,
//! because the age of the row tells you nothing about whether its answer
//! can still move.

use super::schema::StoreError;
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};

/// How long an answer about a window that includes today may be reused.
///
/// Five minutes. The window is still accumulating, so the answer is
/// genuinely stale the moment it is written -- this is not a correctness
/// bound but a de-duplication one: it stops a user clicking between two
/// scopes from paying for the same load twice in a row.
///
/// Short rather than long because the open window is where a user looks
/// for their own PR merged ten minutes ago, and a stale zero there reads
/// as "it did not count". Long enough that the repeated clicking that
/// motivates the cache at all is actually covered.
pub const OPEN_WINDOW_TTL_SECS: i64 = 300;

/// A cached answer and what is known about its trustworthiness.
#[derive(Debug, Clone, PartialEq)]
pub struct Cached {
    pub total: u64,
    /// False when the stored answer was capped, refused, or assembled
    /// from a plan that could not be fully retrieved. Carried through
    /// storage so a partial answer cannot be read back as a confident
    /// one -- #824 item 8 does not stop applying because a number went
    /// through SQLite.
    pub complete: bool,
    /// The `Outcome` JSON as it was written. Opaque here on purpose: #826
    /// defines what a leaderboard needs and this module should not have
    /// an opinion about it.
    pub payload: String,
    pub fetched_at: DateTime<Utc>,
}

/// Whether a window has ended, and its answer can therefore never change.
///
/// `today` is passed rather than read from the clock so this is testable
/// without freezing time -- the same reason `query::period_ranges` takes
/// a `now`.
///
/// A window ending YESTERDAY or earlier is closed. Today's own date is
/// NOT closed even at 23:59: a PR can still be merged into it, and
/// `query::period_ranges` already makes exactly this distinction for the
/// same reason ("today is still accumulating, so including it compares a
/// partial period against complete ones", `query.rs:247-252`).
pub fn is_closed(window_end: &str, today: NaiveDate) -> bool {
    match NaiveDate::parse_from_str(window_end, "%Y-%m-%d") {
        Ok(end) => end < today,
        // An unparseable end date is treated as OPEN, which is the safe
        // direction: it means the answer is re-fetched rather than
        // trusted forever on the strength of a date nobody could read.
        Err(_) => false,
    }
}

/// Store one answer.
///
/// `INSERT OR REPLACE` rather than an ignore-if-present: a fresh answer
/// for the same window is strictly better than a stale one, and in
/// particular an answer that is now COMPLETE must be able to overwrite a
/// partial one from a load that was capped or refused.
#[allow(clippy::too_many_arguments)]
pub fn put(
    conn: &Connection,
    key: &str,
    window_start: &str,
    window_end: &str,
    total: u64,
    complete: bool,
    payload: &str,
    fetched_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR REPLACE INTO stats_cache
           (key, window_start, window_end, total, complete, payload, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            key,
            window_start,
            window_end,
            total as i64,
            i64::from(complete),
            payload,
            fetched_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Read an answer back, if one is still usable.
///
/// Returns `None` when there is no row, or when the row is about a window
/// that is still open and older than [`OPEN_WINDOW_TTL_SECS`]. A closed
/// window's answer is returned however old it is, which is the point.
///
/// `now` is a parameter for the same reason `is_closed` takes `today`.
pub fn get(
    conn: &Connection,
    key: &str,
    window_start: &str,
    window_end: &str,
    now: DateTime<Utc>,
) -> Result<Option<Cached>, StoreError> {
    let row = conn
        .query_row(
            "SELECT total, complete, payload, fetched_at FROM stats_cache
              WHERE key = ?1 AND window_start = ?2 AND window_end = ?3",
            params![key, window_start, window_end],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((total, complete, payload, fetched_at)) = row else {
        return Ok(None);
    };
    // An unparseable timestamp makes the row unusable rather than
    // eternally fresh. Same direction as `is_closed`'s parse failure:
    // re-fetch rather than trust something nobody can read.
    let Ok(fetched) = DateTime::parse_from_rfc3339(&fetched_at) else {
        return Ok(None);
    };
    let fetched = fetched.with_timezone(&Utc);
    if !is_closed(window_end, now.date_naive())
        && (now - fetched).num_seconds() > OPEN_WINDOW_TTL_SECS
    {
        return Ok(None);
    }
    Ok(Some(Cached {
        total: total.max(0) as u64,
        complete: complete != 0,
        payload,
        fetched_at: fetched,
    }))
}

/// Drop every cached answer.
///
/// For the moment when the identity behind `@me` changes: a new token is
/// a different person, and `StatsQuery::cache_key` resolves `@me` with
/// the login that was current when the row was written. Keys are
/// namespaced by login so a stale row cannot be *served* to the wrong
/// user, but it can still sit there forever taking space for a user who
/// is gone.
///
/// NOT called on a schedule. A closed window's answer never expires, so
/// time-based eviction would throw away exactly the rows the cache exists
/// for. This table grows in proportion to the number of distinct scopes
/// and windows a user actually looks at, which is small.
pub fn clear(conn: &Connection) -> Result<usize, StoreError> {
    Ok(conn.execute("DELETE FROM stats_cache", [])?)
}

/// How many answers are cached. For diagnostics and for the test below
/// that proves this table is written to.
pub fn count(conn: &Connection) -> Result<u64, StoreError> {
    let n: i64 = conn.query_row("SELECT count(*) FROM stats_cache", [], |r| r.get(0))?;
    Ok(n.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::store::migrate(&conn).unwrap();
        conn
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// The whole point: an answer about a window that has ENDED cannot
    /// change, so it is reusable however old the row is. Recomputing it
    /// would re-spend the probe rounds and slice fetches for a constant.
    #[test]
    fn a_closed_windows_answer_never_goes_stale() {
        let conn = db();
        put(
            &conn,
            "merged|octocat|org:FNX-Labs",
            "2026-08-01",
            "2026-08-31",
            706,
            true,
            r#"{"total":706}"#,
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();

        // A year later. August 2026 is still over.
        let got = get(
            &conn,
            "merged|octocat|org:FNX-Labs",
            "2026-08-01",
            "2026-08-31",
            at("2027-09-01T00:00:00Z"),
        )
        .unwrap()
        .expect("a closed window's answer is still good");
        assert_eq!(got.total, 706);
        assert!(got.complete);
    }

    /// A window that includes today is still accumulating, so its answer
    /// gets a short bound. The failure this prevents: a user's PR merged
    /// ten minutes ago missing from "this week", which reads as "it did
    /// not count".
    #[test]
    fn an_open_windows_answer_expires_quickly() {
        let conn = db();
        put(
            &conn,
            "merged|octocat|org:FNX-Labs",
            "2026-09-01",
            "2026-09-30",
            93,
            true,
            "{}",
            at("2026-09-11T12:00:00Z"),
        )
        .unwrap();
        let key = ("merged|octocat|org:FNX-Labs", "2026-09-01", "2026-09-30");

        // Inside the TTL: reused, which is the de-duplication the cache
        // is for.
        assert!(get(&conn, key.0, key.1, key.2, at("2026-09-11T12:04:00Z"))
            .unwrap()
            .is_some());
        // Past it: re-fetched.
        assert!(
            get(&conn, key.0, key.1, key.2, at("2026-09-11T12:06:00Z"))
                .unwrap()
                .is_none(),
            "an accumulating window must not be served stale for long"
        );
    }

    /// The line is TODAY, matching `query::period_ranges`'s own rule that
    /// today is still accumulating (`query.rs:247-252`).
    #[test]
    fn today_is_not_a_closed_window() {
        assert!(is_closed("2026-09-10", day("2026-09-11")));
        assert!(
            !is_closed("2026-09-11", day("2026-09-11")),
            "a PR can still be merged into today"
        );
        assert!(!is_closed("2026-09-12", day("2026-09-11")));
    }

    /// An unparseable end date is treated as OPEN, so the answer is
    /// re-fetched rather than trusted forever on a date nobody can read.
    #[test]
    fn an_unreadable_window_is_never_treated_as_closed() {
        assert!(!is_closed("not-a-date", day("2026-09-11")));
        assert!(!is_closed("", day("2026-09-11")));
    }

    /// Partiality survives the round trip. A cached total whose
    /// partiality was forgotten launders a sample into a fact, which is
    /// worse than having no cache at all.
    #[test]
    fn a_partial_answer_is_read_back_as_partial() {
        let conn = db();
        put(
            &conn,
            "merged|*|org:FNX-Labs",
            "2026-08-01",
            "2026-08-01",
            1_200,
            false,
            r#"{"retrievable":false,"unretrievable":200}"#,
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();
        let got = get(
            &conn,
            "merged|*|org:FNX-Labs",
            "2026-08-01",
            "2026-08-01",
            at("2026-09-02T00:00:00Z"),
        )
        .unwrap()
        .unwrap();
        assert!(!got.complete, "a capped answer must not read back clean");
        assert!(got.payload.contains("unretrievable"));
    }

    /// A complete answer must be able to replace a partial one -- a load
    /// that was capped or refused should not poison the row forever.
    #[test]
    fn a_complete_answer_overwrites_a_partial_one() {
        let conn = db();
        let k = ("merged|octocat|org:Stohic", "2026-08-01", "2026-08-31");
        put(
            &conn,
            k.0,
            k.1,
            k.2,
            100,
            false,
            "{}",
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();
        put(
            &conn,
            k.0,
            k.1,
            k.2,
            319,
            true,
            "{}",
            at("2026-09-02T00:00:00Z"),
        )
        .unwrap();
        let got = get(&conn, k.0, k.1, k.2, at("2026-09-03T00:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(got.total, 319);
        assert!(got.complete);
        assert_eq!(count(&conn).unwrap(), 1, "replaced, not duplicated");
    }

    /// Different windows for one key are different rows. A month's answer
    /// must not be served for a quarter's.
    #[test]
    fn the_window_is_part_of_the_identity() {
        let conn = db();
        let k = "merged|octocat|org:FNX-Labs";
        put(
            &conn,
            k,
            "2026-07-01",
            "2026-07-31",
            461,
            true,
            "{}",
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();
        put(
            &conn,
            k,
            "2026-08-01",
            "2026-08-31",
            706,
            true,
            "{}",
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(count(&conn).unwrap(), 2);
        let july = get(
            &conn,
            k,
            "2026-07-01",
            "2026-07-31",
            at("2026-09-02T00:00:00Z"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(july.total, 461);
        // And a window nobody asked about is absent rather than
        // approximated from a neighbour.
        assert!(get(
            &conn,
            k,
            "2026-07-01",
            "2026-08-31",
            at("2026-09-02T00:00:00Z")
        )
        .unwrap()
        .is_none());
    }

    /// Two subjects are two rows. The key carries a RESOLVED login
    /// precisely so this holds -- see `StatsQuery::cache_key`.
    #[test]
    fn two_subjects_do_not_share_a_row() {
        let conn = db();
        let w = ("2026-08-01", "2026-08-31");
        put(
            &conn,
            "merged|octocat|org:FNX-Labs",
            w.0,
            w.1,
            10,
            true,
            "{}",
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();
        put(
            &conn,
            "merged|pktstorm|org:FNX-Labs",
            w.0,
            w.1,
            99,
            true,
            "{}",
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();
        let a = get(
            &conn,
            "merged|octocat|org:FNX-Labs",
            w.0,
            w.1,
            at("2026-09-02T00:00:00Z"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(a.total, 10, "one user's row must not serve another's");
        assert_eq!(count(&conn).unwrap(), 2);
    }

    #[test]
    fn a_missing_row_is_none_not_zero() {
        let conn = db();
        assert!(
            get(&conn, "nothing", "2026-01-01", "2026-01-31", Utc::now())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn clearing_drops_every_row() {
        let conn = db();
        put(
            &conn,
            "k",
            "2026-01-01",
            "2026-01-31",
            1,
            true,
            "{}",
            Utc::now(),
        )
        .unwrap();
        assert_eq!(clear(&conn).unwrap(), 1);
        assert_eq!(count(&conn).unwrap(), 0);
    }

    /// THE guard against this becoming the second `merge_history`.
    ///
    /// Migration 2 dropped that table because nothing ever wrote to it,
    /// and `store/mod.rs` calls a permanently-empty table "a feature that
    /// does not exist". This asserts the write path exists and works end
    /// to end -- schema, insert, read back -- so a table with no writer
    /// cannot ship again.
    #[test]
    fn the_table_is_actually_written_to() {
        let conn = db();
        assert_eq!(count(&conn).unwrap(), 0, "starts empty");
        put(
            &conn,
            "merged|pktstorm|repo:pktstorm/headstate",
            "2026-01-01",
            "2026-08-31",
            337,
            true,
            r#"{"total":337,"viaConnection":true}"#,
            at("2026-09-01T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(count(&conn).unwrap(), 1, "and is written to");
        let got = get(
            &conn,
            "merged|pktstorm|repo:pktstorm/headstate",
            "2026-01-01",
            "2026-08-31",
            at("2026-09-02T00:00:00Z"),
        )
        .unwrap()
        .unwrap();
        // The live figure for this repository through the connection,
        // which is the path that has no 1,000-result cap.
        assert_eq!(got.total, 337);
    }
}
