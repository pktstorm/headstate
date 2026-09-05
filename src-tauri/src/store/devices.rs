//! The `paired_devices` table: one row per phone the user has approved.
//!
//! Two readers with different needs share it. The TLS client-certificate
//! verifier (`remote/listener.rs`) looks a presented certificate up by
//! fingerprint on every handshake and needs the DER and the step-up keys.
//! The Settings screen lists names and last-seen times and must never be
//! handed key material it has no use for; `remote/pairing.rs` maps rows
//! to a summary for it.
//!
//! Every function takes a `&Connection` rather than opening one: the
//! callers already hold a connection from `open_db`, and a test can run
//! the whole module on `Connection::open_in_memory()`.

use super::StoreError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};

/// A row of `paired_devices`, in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDevice {
    pub id: i64,
    pub name: String,
    /// Lowercase hex SHA256 of `cert_der`, without a `sha256:` prefix.
    pub cert_fp: String,
    pub cert_der: Vec<u8>,
    /// P-256 step-up key, SEC1 uncompressed: 65 bytes starting `0x04`.
    pub ecdsa_pubkey: Vec<u8>,
    /// ML-DSA-65 step-up key, 1952 bytes; `None` when the phone had none.
    pub mldsa_pubkey: Option<Vec<u8>>,
    /// RFC 3339.
    pub paired_at: String,
    /// RFC 3339; `None` until the device's first connection after pairing.
    pub last_seen: Option<String>,
}

/// What pairing knows about a device before it has a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDevice {
    pub name: String,
    pub cert_fp: String,
    pub cert_der: Vec<u8>,
    pub ecdsa_pubkey: Vec<u8>,
    pub mldsa_pubkey: Option<Vec<u8>>,
}

const COLUMNS: &str =
    "id, name, cert_fp, cert_der, ecdsa_pubkey, mldsa_pubkey, paired_at, last_seen";

fn from_row(r: &Row<'_>) -> rusqlite::Result<PairedDevice> {
    Ok(PairedDevice {
        id: r.get(0)?,
        name: r.get(1)?,
        cert_fp: r.get(2)?,
        cert_der: r.get(3)?,
        ecdsa_pubkey: r.get(4)?,
        mldsa_pubkey: r.get(5)?,
        paired_at: r.get(6)?,
        last_seen: r.get(7)?,
    })
}

/// Store an approved device. Returns the new row id.
///
/// `paired_at` is stamped here rather than passed in so that no caller
/// can record a pairing at a time other than when it happened. Fails on
/// a duplicate fingerprint: that is the UNIQUE constraint doing its job,
/// and the pairing flow checks for an existing row first so the error
/// only surfaces for a genuine race.
pub fn insert(conn: &Connection, device: &NewDevice) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO paired_devices
            (name, cert_fp, cert_der, ecdsa_pubkey, mldsa_pubkey, paired_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            device.name,
            device.cert_fp,
            device.cert_der,
            device.ecdsa_pubkey,
            device.mldsa_pubkey,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Every paired device, oldest pairing first.
pub fn list(conn: &Connection) -> Result<Vec<PairedDevice>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM paired_devices ORDER BY paired_at, id"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The verifier's lookup: the device presenting this certificate, if the
/// user has approved it.
pub fn find_by_fingerprint(
    conn: &Connection,
    cert_fp: &str,
) -> Result<Option<PairedDevice>, StoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM paired_devices WHERE cert_fp = ?1"),
            [cert_fp],
            from_row,
        )
        .optional()?)
}

/// Devices already paired under this name. More than one is possible:
/// the user may decline to replace at re-pairing, in which case the rows
/// coexist.
pub fn find_by_name(conn: &Connection, name: &str) -> Result<Vec<PairedDevice>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM paired_devices WHERE name = ?1 ORDER BY paired_at, id"
    ))?;
    let rows = stmt.query_map([name], from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Delete one device. Returns the row that was removed, so the caller
/// can close that certificate's open connections; `None` when there was
/// no such row, which is not an error -- the user may have clicked
/// Revoke twice.
pub fn revoke(conn: &Connection, id: i64) -> Result<Option<PairedDevice>, StoreError> {
    let existing = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM paired_devices WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?;
    if existing.is_some() {
        conn.execute("DELETE FROM paired_devices WHERE id = ?1", [id])?;
    }
    Ok(existing)
}

/// Record that a paired device connected. Called by the listener after
/// a successful handshake; a fingerprint with no row is ignored because
/// the verifier has already refused it.
pub fn touch_last_seen(conn: &Connection, cert_fp: &str) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE paired_devices SET last_seen = ?1 WHERE cert_fp = ?2",
        params![Utc::now().to_rfc3339(), cert_fp],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        super::super::schema::migrate(&conn).unwrap();
        conn
    }

    fn phone(name: &str, fp: &str) -> NewDevice {
        NewDevice {
            name: name.into(),
            cert_fp: fp.into(),
            cert_der: vec![0x30, 0x82, 0x01],
            ecdsa_pubkey: vec![0x04; 65],
            mldsa_pubkey: None,
        }
    }

    #[test]
    fn insert_then_find_by_fingerprint_round_trips_every_column() {
        let conn = db();
        let mut new = phone("Octocat's phone", "ab12");
        new.mldsa_pubkey = Some(vec![0x11; 1952]);
        let id = insert(&conn, &new).unwrap();

        let found = find_by_fingerprint(&conn, "ab12").unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.name, new.name);
        assert_eq!(found.cert_fp, new.cert_fp);
        assert_eq!(found.cert_der, new.cert_der);
        assert_eq!(found.ecdsa_pubkey, new.ecdsa_pubkey);
        assert_eq!(found.mldsa_pubkey, new.mldsa_pubkey);
        assert!(!found.paired_at.is_empty());
        assert_eq!(found.last_seen, None);
    }

    #[test]
    fn an_unknown_fingerprint_is_none_not_an_error() {
        assert_eq!(find_by_fingerprint(&db(), "nope").unwrap(), None);
    }

    #[test]
    fn a_duplicate_fingerprint_is_refused() {
        let conn = db();
        insert(&conn, &phone("a", "same")).unwrap();
        assert!(insert(&conn, &phone("b", "same")).is_err());
        assert_eq!(list(&conn).unwrap().len(), 1);
    }

    #[test]
    fn list_returns_every_device_and_same_names_may_coexist() {
        let conn = db();
        insert(&conn, &phone("Octocat's phone", "one")).unwrap();
        insert(&conn, &phone("Octocat's phone", "two")).unwrap();
        insert(&conn, &phone("Tablet", "three")).unwrap();

        let all = list(&conn).unwrap();
        assert_eq!(all.len(), 3);
        let same: Vec<_> = find_by_name(&conn, "Octocat's phone").unwrap();
        assert_eq!(same.len(), 2);
        assert!(find_by_name(&conn, "nobody").unwrap().is_empty());
    }

    #[test]
    fn revoke_removes_the_row_and_returns_it() {
        let conn = db();
        let id = insert(&conn, &phone("Octocat's phone", "gone")).unwrap();

        let removed = revoke(&conn, id).unwrap().expect("the row existed");
        assert_eq!(removed.cert_fp, "gone");
        assert_eq!(find_by_fingerprint(&conn, "gone").unwrap(), None);
        assert!(list(&conn).unwrap().is_empty());

        // A second click on Revoke is a no-op, not a failure.
        assert_eq!(revoke(&conn, id).unwrap(), None);
    }

    #[test]
    fn touch_last_seen_stamps_the_matching_row_only() {
        let conn = db();
        insert(&conn, &phone("a", "seen")).unwrap();
        insert(&conn, &phone("b", "unseen")).unwrap();

        touch_last_seen(&conn, "seen").unwrap();
        touch_last_seen(&conn, "never-paired").unwrap();

        assert!(find_by_fingerprint(&conn, "seen")
            .unwrap()
            .unwrap()
            .last_seen
            .is_some());
        assert!(find_by_fingerprint(&conn, "unseen")
            .unwrap()
            .unwrap()
            .last_seen
            .is_none());
    }
}
