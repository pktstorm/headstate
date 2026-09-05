//! Database schema and migrations.
//!
//! Migrations are numbered from the first commit, so v0.1 installs stay
//! upgradable rather than needing the database deleted -- this matters even
//! though there is only one migration today.

use rusqlite::Connection;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialisation error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Numbered migrations from the first commit, so v0.1 installs stay
/// upgradable rather than needing the database deleted.
const MIGRATIONS: &[&str] = &[
    // 1: the snapshot cache and the merge history.
    "CREATE TABLE IF NOT EXISTS snapshot (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        payload TEXT NOT NULL,
        fetched_at TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS merge_history (
        repo TEXT NOT NULL,
        number INTEGER NOT NULL,
        merged_at TEXT NOT NULL,
        PRIMARY KEY (repo, number)
     );",
    // `merge_history` was never written to -- see `store/mod.rs`. Dropped
    // rather than left as a permanently-empty table implying a feature
    // that does not exist. Additive-only migrations elsewhere; this one is
    // safe because nothing ever read it either.
    "DROP TABLE IF EXISTS merge_history;",
    // 3: user settings.
    //
    // Key-value rather than a column per setting: settings are read and
    // written one at a time, and a new one should not need a migration.
    // Values are JSON so a setting can grow from a scalar to a list --
    // `worktree_dirs` in particular starts as one path and will not stay
    // that way.
    //
    // Lives in SQLite rather than localStorage because the POLL LOOP and
    // the worktree scanner both need these values, and neither can read
    // the webview's storage.
    "CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
     );",
    // 4: let the snapshot table hold MORE THAN ONE list.
    //
    // The original `CHECK (id = 1)` allowed exactly one cached list, the
    // authored one. To review had no cache at all, so opening it always
    // waited on a live query -- ~20s on a 60-PR queue, with an empty
    // panel until it returned.
    //
    // SQLite cannot drop a CHECK constraint, so the table is rebuilt.
    // The existing row is carried over rather than discarded: throwing
    // away a valid cache on upgrade would give every user one slow
    // launch for no reason.
    "CREATE TABLE snapshot_new (
        id INTEGER PRIMARY KEY,
        payload TEXT NOT NULL,
        fetched_at TEXT NOT NULL
     );
     INSERT INTO snapshot_new (id, payload, fetched_at)
        SELECT id, payload, fetched_at FROM snapshot;
     DROP TABLE snapshot;
     ALTER TABLE snapshot_new RENAME TO snapshot;",
    // 5: the cleanup ledger.
    //
    // A TABLE rather than a settings key because this is append-only
    // history queried by time, where `settings` holds values read and
    // written whole.
    //
    // Written on EVERY run including preview runs, which is what keeps
    // it from becoming the second `merge_history` -- a permanently-empty
    // table implying a feature that does not exist. It is also the only
    // way a user can audit work the app did while nobody was watching,
    // and that auditability is what makes an unattended feature
    // trustworthy rather than merely convenient.
    //
    // `action` records refusals too: when the delete-time re-check
    // declines something, that is the guard working and the user should
    // be able to see it work.
    "CREATE TABLE IF NOT EXISTS cleanup_log (
        id INTEGER PRIMARY KEY,
        at TEXT NOT NULL,
        kind TEXT NOT NULL,
        target TEXT NOT NULL,
        detail TEXT,
        bytes INTEGER,
        action TEXT NOT NULL,
        error TEXT
     );
     CREATE INDEX IF NOT EXISTS cleanup_log_at ON cleanup_log (at DESC);",
    // 6: phones paired with this desktop (mobile companion, Storage
    // section of the design spec).
    //
    // `cert_fp` is the lowercase hex SHA256 of `cert_der`, UNIQUE
    // because the TLS client-certificate verifier looks a presented
    // certificate up by exactly this string and one certificate can only
    // belong to one device. `name` is deliberately NOT unique: the spec
    // lets two devices with the same name coexist unless the user
    // chooses to replace the old one at re-pairing.
    //
    // `ecdsa_pubkey` is the P-256 step-up key, SEC1 uncompressed (65
    // bytes). `mldsa_pubkey` is the ML-DSA-65 step-up key (1952 bytes),
    // NULL when the phone's keystore could not produce one -- the
    // desktop verifies exactly the signatures this row says to expect.
    //
    // Timestamps are RFC 3339 text, matching `cleanup_log.at`.
    "CREATE TABLE IF NOT EXISTS paired_devices (
        id              INTEGER PRIMARY KEY,
        name            TEXT NOT NULL,
        cert_fp         TEXT NOT NULL UNIQUE,
        cert_der        BLOB NOT NULL,
        ecdsa_pubkey    BLOB NOT NULL,
        mldsa_pubkey    BLOB,
        paired_at       TEXT NOT NULL,
        last_seen       TEXT
     );",
];

pub fn migrate(conn: &Connection) -> Result<(), StoreError> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    Ok(())
}

pub fn open_db(path: &Path) -> Result<Connection, StoreError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let conn = Connection::open(path)?;
    // WAL lets a reader proceed while a writer holds the file, and
    // busy_timeout replaces rusqlite's effectively-zero default with a
    // real wait. Contention is near-impossible today -- one autocommit
    // UPSERT of one row, from a loop whose only other writer is offset by
    // construction -- so this is cheap hardening against a future second
    // writer, not a fix for an observed failure.
    //
    // Non-fatal: a read-only volume or an older SQLite should degrade to
    // the previous behaviour rather than refuse to open the cache.
    if let Err(e) = conn.pragma_update(None, "journal_mode", "WAL") {
        log::warn!("could not enable WAL: {e}");
    }
    if let Err(e) = conn.busy_timeout(std::time::Duration::from_secs(5)) {
        log::warn!("could not set busy_timeout: {e}");
    }
    migrate(&conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_table(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    /// An existing install sits at user_version 1 with the empty
    /// merge_history table. The DROP must run for THOSE databases, not
    /// only for fresh ones -- otherwise the dead table lingers forever on
    /// every machine that already installed the app.
    #[test]
    fn upgrading_an_existing_db_drops_the_dead_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE snapshot (id INTEGER PRIMARY KEY CHECK (id = 1),
                payload TEXT NOT NULL, fetched_at TEXT NOT NULL);
             CREATE TABLE merge_history (repo TEXT NOT NULL, number INTEGER NOT NULL,
                merged_at TEXT NOT NULL, PRIMARY KEY (repo, number));",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();

        migrate(&conn).unwrap();

        assert!(
            !has_table(&conn, "merge_history"),
            "dead table must be dropped"
        );
        assert!(has_table(&conn, "snapshot"), "the real cache must survive");
    }

    /// Migration 6 adds `paired_devices` to a database that stopped at
    /// version 5, which is every install that predates the mobile
    /// companion. Checked from a real v5 state rather than a fresh
    /// database, so a migration that only works when it runs first in
    /// the list would be caught.
    #[test]
    fn migration_six_adds_paired_devices_to_a_v5_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE snapshot (id INTEGER PRIMARY KEY, payload TEXT NOT NULL,
                fetched_at TEXT NOT NULL);
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE cleanup_log (id INTEGER PRIMARY KEY, at TEXT NOT NULL,
                kind TEXT NOT NULL, target TEXT NOT NULL, detail TEXT, bytes INTEGER,
                action TEXT NOT NULL, error TEXT);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 5i64).unwrap();

        migrate(&conn).unwrap();

        assert!(has_table(&conn, "paired_devices"));
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 6);

        // The fingerprint is the verifier's lookup key; two rows with the
        // same one would make "which device is this" ambiguous.
        let insert = "INSERT INTO paired_devices
            (name, cert_fp, cert_der, ecdsa_pubkey, paired_at)
            VALUES (?1, 'ab', x'00', x'04', '2026-01-01T00:00:00Z')";
        conn.execute(insert, ["a"]).unwrap();
        assert!(
            conn.execute(insert, ["b"]).is_err(),
            "cert_fp must be unique"
        );
    }

    #[test]
    fn a_fresh_db_ends_up_with_only_the_snapshot_table() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert!(has_table(&conn, "snapshot"));
        assert!(!has_table(&conn, "merge_history"));
    }

    /// Migrations are applied once and are idempotent on re-open, which
    /// every call to `open_db` relies on.
    #[test]
    fn migrate_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let v1: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        migrate(&conn).unwrap();
        let v2: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v1, v2);
        assert_eq!(v1 as usize, MIGRATIONS.len());
    }

    /// Migration 4 REBUILDS the snapshot table to drop `CHECK (id = 1)`.
    ///
    /// SQLite cannot drop a constraint in place, so the rebuild is the
    /// only route -- and a rebuild that forgot to copy the rows would
    /// give every upgrading user one slow, cache-less launch. Verified
    /// against a database built at the old version rather than a
    /// round-trip of the current one.
    #[test]
    fn migration_four_keeps_an_existing_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        let conn = Connection::open(&path).unwrap();

        // The schema exactly as version 3 left it, including the CHECK
        // that made a second cached list impossible.
        conn.execute_batch(
            "CREATE TABLE snapshot (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                payload TEXT NOT NULL,
                fetched_at TEXT NOT NULL
             );
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO snapshot (id, payload, fetched_at)
                VALUES (1, '[{\"number\":42}]', '2026-01-01T00:00:00Z');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 3i64).unwrap();

        migrate(&conn).expect("the upgrade must succeed on a real v3 database");

        let payload: String = conn
            .query_row("SELECT payload FROM snapshot WHERE id = 1", [], |r| {
                r.get(0)
            })
            .expect("the cached list must survive the rebuild");
        assert!(payload.contains("42"));

        // And the constraint is gone, which is the point of the change.
        conn.execute(
            "INSERT INTO snapshot (id, payload, fetched_at) VALUES (2, '[]', 'now')",
            [],
        )
        .expect("a second cached list must now be allowed");
    }
}
