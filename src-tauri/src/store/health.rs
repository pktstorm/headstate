//! The system-health series: writing samples, trimming to 24 hours, and
//! handing back a bounded history.
//!
//! # Why the history is downsampled here
//!
//! At one sample a minute the table holds ~1440 rows. Sending all of
//! them to draw a chart a few hundred pixels wide is waste on the
//! desktop and a real cost on the phone, which reads this over the LAN:
//! `size_worktrees` taught that lesson when a slow command over the
//! remote surface timed out at two minutes (#661). So the SQL picks a
//! bounded number of buckets and the caller cannot ask for more.

use super::schema::StoreError;
use crate::health::Sample;
use rusqlite::{params, Connection};

/// How long a sample is kept.
pub const RETENTION_HOURS: i64 = 24;

/// The most points `history` will ever return.
///
/// A chart is a few hundred pixels wide; more points than that draw on
/// top of each other. 120 is one every twelve minutes across a day,
/// which is enough to see a shape without being enough to be slow.
pub const MAX_POINTS: usize = 120;

/// Write one sample and drop anything older than [`RETENTION_HOURS`].
///
/// The trim runs on every write rather than on a timer: a timer is a
/// second thing to schedule and to get wrong, and deleting a handful of
/// rows beside an insert costs nothing. Without it the table is a slow
/// leak that only shows up weeks later.
pub fn record(conn: &Connection, s: &Sample) -> Result<(), StoreError> {
    let detail = serde_json::to_string(s)?;
    conn.execute(
        "INSERT OR REPLACE INTO health_samples
           (sampled_at, load_1, load_5, load_15, cpu_percent,
            mem_total, mem_used, mem_available,
            battery_percent, on_ac, thermal, uptime_secs, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            s.sampled_at,
            s.load.map(|l| l[0]),
            s.load.map(|l| l[1]),
            s.load.map(|l| l[2]),
            s.cpu_percent,
            s.memory.total as i64,
            s.memory.used as i64,
            s.memory.available as i64,
            s.battery.as_ref().map(|b| b.percent),
            s.battery.as_ref().map(|b| i64::from(b.on_ac)),
            s.thermal,
            s.uptime_secs as i64,
            detail,
        ],
    )?;
    // Compared as strings, which is sound because every timestamp here
    // is RFC 3339 in UTC: that format sorts lexicographically in time
    // order, and mixing offsets would break far more than this query.
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(RETENTION_HOURS)).to_rfc3339();
    conn.execute(
        "DELETE FROM health_samples WHERE sampled_at < ?1",
        params![cutoff],
    )?;
    Ok(())
}

/// The series, at most [`MAX_POINTS`] rows, oldest first.
///
/// Bucketed by row number rather than by time, so a period when the app
/// was closed does not become a run of empty buckets the caller has to
/// filter. The GAP is still visible -- consecutive rows are simply far
/// apart in time -- which is what lets the chart draw it as a gap rather
/// than interpolating a flat line through hours nobody measured.
pub fn history(conn: &Connection) -> Result<Vec<Sample>, StoreError> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM health_samples", [], |r| r.get(0))?;
    // `max(1)`: with fewer rows than buckets every row is its own
    // bucket, and a stride of 0 would divide by zero.
    let stride = (total / MAX_POINTS as i64).max(1);
    let mut stmt = conn.prepare(
        "SELECT detail FROM (
             SELECT detail, ROW_NUMBER() OVER (ORDER BY sampled_at) - 1 AS n
             FROM health_samples
         ) WHERE n % ?1 = 0 ORDER BY n",
    )?;
    let rows = stmt.query_map(params![stride], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        // A row that does not parse is skipped rather than failing the
        // whole history: one bad sample from an older shape must not
        // cost the user every chart.
        if let Ok(s) = serde_json::from_str::<Sample>(&row?) {
            out.push(s);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::{Memory, Sample};

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::store::migrate(&c).unwrap();
        c
    }

    fn sample(at: &str) -> Sample {
        Sample {
            sampled_at: at.to_string(),
            load: Some([1.0, 2.0, 3.0]),
            cpu_percent: Some(12.5),
            cpu_per_core: vec![10.0, 15.0],
            memory: Memory {
                total: 100,
                used: 40,
                available: 60,
                swap_total: 0,
                swap_used: 0,
            },
            disks: vec![],
            battery: None,
            thermal: Some("nominal".into()),
            networks: vec![],
            uptime_secs: 42,
        }
    }

    #[test]
    fn a_sample_round_trips() {
        let c = conn();
        // Dated NOW, not a fixed date: `record` trims anything outside
        // the retention window, so a hardcoded timestamp deletes itself
        // the moment it falls out of the last 24 hours.
        record(&c, &sample(&chrono::Utc::now().to_rfc3339())).unwrap();
        let got = history(&c).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cpu_percent, Some(12.5));
        assert_eq!(got[0].thermal.as_deref(), Some("nominal"));
    }

    /// Anything older than the window is gone, so the table cannot grow
    /// without bound.
    #[test]
    fn samples_older_than_a_day_are_dropped() {
        let c = conn();
        let old = (chrono::Utc::now() - chrono::Duration::hours(RETENTION_HOURS + 1)).to_rfc3339();
        let now = chrono::Utc::now().to_rfc3339();
        record(&c, &sample(&old)).unwrap();
        record(&c, &sample(&now)).unwrap();
        let got = history(&c).unwrap();
        assert_eq!(got.len(), 1, "the day-old sample must be gone");
        assert_eq!(got[0].sampled_at, now);
    }

    /// The history is bounded however long the app has been running.
    ///
    /// This is the property the phone depends on: an unbounded series
    /// over the LAN is the mistake that made `size_worktrees` time out
    /// (#661).
    #[test]
    fn the_history_is_bounded_and_stays_in_order() {
        let c = conn();
        let base = chrono::Utc::now() - chrono::Duration::hours(20);
        for i in 0..1440 {
            let at = (base + chrono::Duration::minutes(i)).to_rfc3339();
            record(&c, &sample(&at)).unwrap();
        }
        let got = history(&c).unwrap();
        assert!(
            got.len() <= MAX_POINTS,
            "{} points is more than the cap",
            got.len()
        );
        assert!(got.len() > 1, "a full day should not collapse to one point");
        // Oldest first, and strictly increasing.
        for w in got.windows(2) {
            assert!(w[0].sampled_at < w[1].sampled_at, "out of order");
        }
    }

    /// Fewer samples than buckets: every row comes back, and nothing
    /// divides by zero.
    #[test]
    fn a_short_history_is_returned_whole() {
        let c = conn();
        let base = chrono::Utc::now() - chrono::Duration::minutes(5);
        for i in 0..5 {
            record(
                &c,
                &sample(&(base + chrono::Duration::minutes(i)).to_rfc3339()),
            )
            .unwrap();
        }
        assert_eq!(history(&c).unwrap().len(), 5);
    }

    /// An empty table is an empty history, not an error.
    #[test]
    fn no_samples_is_not_a_failure() {
        assert!(history(&conn()).unwrap().is_empty());
    }
}
