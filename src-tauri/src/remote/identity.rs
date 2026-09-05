//! The desktop's own TLS identity: one P256 key pair and one self-signed
//! certificate, generated the first time phone connections are enabled
//! and kept for ten years.
//!
//! The phone never validates a chain. At pairing it receives this
//! certificate's SHA256 fingerprint out of band (in the QR code) and pins
//! it, so the certificate carries no hostname and no CA -- the
//! fingerprint IS the identity. That is also why the identity must never
//! be silently regenerated: a new certificate is a new fingerprint, and
//! every paired phone would refuse the desktop until re-paired.
//!
//! Where the private key lives is the point of this module. Headstate
//! has never stored a credential of its own -- the GitHub token is read
//! from `gh` -- and this is the first entry in the platform keychain.
//! The keychain, not SQLite, because the key is the only thing standing
//! between an attacker on the same network and every command a paired
//! phone can run, and SQLite is a plain file in the app data directory.

use base64::Engine;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};

/// How long the certificate is good for. Long, because expiry buys
/// nothing when the peer pins a fingerprint rather than checking dates,
/// and a lapse would silently unpair every phone.
pub const VALIDITY_YEARS: i32 = 10;

/// Keychain coordinates. The service is the bundle identifier so the
/// item is recognisably Headstate's in Keychain Access; the user names
/// what the item is, since one app may hold several one day.
const KEYCHAIN_SERVICE: &str = "com.pktstorm.headstate";
const KEYCHAIN_USER: &str = "remote-identity";

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("could not generate the desktop identity: {0}")]
    Generate(#[from] rcgen::Error),
    /// The stored blob exists but cannot be used. Deliberately NOT
    /// recovered by regenerating: that would invalidate every pairing
    /// without telling anyone. The user sees this and decides.
    #[error(
        "the stored desktop identity is unreadable ({0}); remove the keychain item to start over"
    )]
    Corrupt(String),
    #[error("{0}")]
    Store(String),
}

/// Where the identity blob is kept.
///
/// A trait so the listener tests never touch a real keychain: a unit
/// test that writes to the macOS keychain leaves an item behind on the
/// developer's machine and can hang a CI runner on an access prompt.
/// Production uses [`PlatformStore`]; tests use an in-memory store.
pub trait SecretStore: Send + Sync {
    /// The stored blob, or `None` if nothing has been stored yet.
    fn read(&self) -> Result<Option<Vec<u8>>, IdentityError>;
    /// Store the blob, replacing any previous one.
    fn write(&self, bytes: &[u8]) -> Result<(), IdentityError>;
}

/// The key pair and its certificate, both as DER.
///
/// `Clone` because the listener needs its own copy and the fingerprint
/// is wanted elsewhere (the pairing QR). No `Debug` derive: the private
/// key must never reach a log, so the manual impl prints the
/// fingerprint alone.
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    cert_der: Vec<u8>,
    key_pkcs8: Vec<u8>,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

/// What is persisted. Versioned so a future key type (the spec plans to
/// move to ML-DSA once rustls ships it) can be told apart from this one
/// rather than mis-parsed.
#[derive(Serialize, Deserialize)]
struct Stored {
    v: u32,
    key_pkcs8: String,
    cert_der: String,
}

const STORED_VERSION: u32 = 1;

impl Identity {
    /// A fresh P256 key and a self-signed certificate valid from today
    /// for [`VALIDITY_YEARS`].
    pub fn generate() -> Result<Self, IdentityError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let (from, to) = validity_window(chrono::Utc::now().date_naive());

        let mut params = CertificateParams::default();
        params.not_before = rcgen::date_time_ymd(from.0, from.1, from.2);
        params.not_after = rcgen::date_time_ymd(to.0, to.1, to.2);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Headstate desktop");
        params.distinguished_name = dn;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        // No subject alternative name, on purpose: the phone pins the
        // fingerprint, and a laptop's address changes with every network.

        let cert = params.self_signed(&key)?;
        Ok(Self {
            cert_der: cert.der().to_vec(),
            key_pkcs8: key.serialize_der(),
        })
    }

    /// Lowercase hex SHA256 of the certificate DER -- the value the phone
    /// pins, and the value shown in the pairing dialog.
    pub fn fingerprint(&self) -> String {
        fingerprint_of(&self.cert_der)
    }

    /// The certificate, for rustls.
    pub fn cert(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    /// The private key, for rustls.
    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_pkcs8.clone()))
    }

    fn to_bytes(&self) -> Vec<u8> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let stored = Stored {
            v: STORED_VERSION,
            key_pkcs8: b64.encode(&self.key_pkcs8),
            cert_der: b64.encode(&self.cert_der),
        };
        // A struct of three strings cannot fail to serialise.
        serde_json::to_vec(&stored).expect("identity serialises")
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let stored: Stored =
            serde_json::from_slice(bytes).map_err(|e| IdentityError::Corrupt(e.to_string()))?;
        if stored.v != STORED_VERSION {
            return Err(IdentityError::Corrupt(format!(
                "version {} (this build reads {STORED_VERSION})",
                stored.v
            )));
        }
        let key_pkcs8 = b64
            .decode(&stored.key_pkcs8)
            .map_err(|e| IdentityError::Corrupt(format!("key: {e}")))?;
        let cert_der = b64
            .decode(&stored.cert_der)
            .map_err(|e| IdentityError::Corrupt(format!("certificate: {e}")))?;
        // Prove the key is usable NOW rather than when the listener
        // first tries to sign: a corrupt key should stop the enable
        // with a message, not fail every handshake afterwards.
        KeyPair::from_pkcs8_der_and_sign_algo(
            &PrivatePkcs8KeyDer::from(key_pkcs8.as_slice()),
            &PKCS_ECDSA_P256_SHA256,
        )
        .map_err(|e| IdentityError::Corrupt(format!("key: {e}")))?;
        if cert_der.is_empty() {
            return Err(IdentityError::Corrupt("empty certificate".into()));
        }
        Ok(Self {
            cert_der,
            key_pkcs8,
        })
    }
}

/// Lowercase hex SHA256 of a DER certificate. Used for the desktop's own
/// certificate and, by the listener's verifier, for every phone's.
pub fn fingerprint_of(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A calendar date as `(year, month, day)`, the shape `rcgen::date_time_ymd`
/// takes. Plain tuples so the window can be computed and tested without
/// naming rcgen's `time` types.
type Ymd = (i32, u8, u8);

/// `[today, today + VALIDITY_YEARS]`, both at midnight UTC.
///
/// A 29 February start lands on the 28th ten years on when that year is
/// not a leap year; `date_time_ymd` would panic on an invalid date, and
/// a panic once every four years on one day is exactly the bug nobody
/// reproduces.
fn validity_window(today: chrono::NaiveDate) -> (Ymd, Ymd) {
    use chrono::Datelike;
    let (y, m, d) = (today.year(), today.month() as u8, today.day() as u8);
    let end_year = y + VALIDITY_YEARS;
    let end_day = if m == 2 && d == 29 && !is_leap_year(end_year) {
        28
    } else {
        d
    };
    ((y, m, d), (end_year, m, end_day))
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// The identity from the store, or a new one written to it.
///
/// The only place an identity is ever created. Anything unreadable is
/// an error, never a regeneration -- see [`IdentityError::Corrupt`].
pub fn load_or_create(store: &dyn SecretStore) -> Result<Identity, IdentityError> {
    if let Some(bytes) = store.read()? {
        return Identity::from_bytes(&bytes);
    }
    let identity = Identity::generate()?;
    store.write(&identity.to_bytes())?;
    log::info!(
        "generated the desktop identity, fingerprint {}",
        identity.fingerprint()
    );
    Ok(identity)
}

/// The platform keychain, via the `keyring` crate: Keychain Services on
/// macOS, Credential Manager on Windows, the freedesktop Secret Service
/// on Linux.
///
/// Why this crate: it is the one cross-platform keychain binding with a
/// maintained backend for all three, and its Linux backend talks D-Bus
/// through `zbus`, pure Rust -- so the Linux CI job needs no `libdbus`
/// or `libsecret` headers, which it does not install today.
///
/// What the Secret Service backend needs at RUNTIME is a daemon
/// (gnome-keyring, KWallet's bridge, KeePassXC) on the session bus. A
/// headless Linux box, a CI runner, or a bare window manager has none,
/// and `keyring` reports that as a store error rather than a missing
/// entry. On Linux only, that case falls back to a mode-0600 file in the
/// app data directory. Never on macOS or Windows: both always have a
/// keychain, so an error there is a real error and is surfaced.
///
/// The fallback is a step down -- a file readable by anything running
/// as the user -- and is logged as such at every start. It is also
/// separate from the keychain: an identity written to a Secret Service
/// that is later absent is not found in the file, and a fresh one would
/// be generated, unpairing every phone. That is the honest price of
/// having no daemon; it is logged rather than hidden.
pub struct PlatformStore {
    /// Where the Linux fallback file goes. Held, and the fallback path
    /// compiled, on every platform so the code is type-checked
    /// everywhere; the `cfg!` in `on_failure` is what keeps it Linux-only
    /// at runtime.
    fallback: FileStore,
}

impl PlatformStore {
    /// `fallback_path` is the Linux fallback file; see the type docs.
    pub fn new(fallback_path: PathBuf) -> Self {
        Self {
            fallback: FileStore::new(fallback_path),
        }
    }

    fn entry() -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
    }

    /// Whether a keyring error means "there is no usable store here",
    /// as opposed to "the store answered and said something is wrong".
    fn store_unavailable(e: &keyring::Error) -> bool {
        matches!(
            e,
            keyring::Error::NoDefaultStore
                | keyring::Error::PlatformFailure(_)
                | keyring::Error::NoStorageAccess(_)
        )
    }

    fn keychain_read() -> Result<Option<Vec<u8>>, keyring::Error> {
        match Self::entry()?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn keychain_write(bytes: &[u8]) -> Result<(), keyring::Error> {
        Self::entry()?.set_secret(bytes)
    }

    /// Decide what to do with a keychain failure: fall back on Linux
    /// when no store is available, otherwise surface it.
    fn on_failure<T>(
        &self,
        e: keyring::Error,
        fallback: impl FnOnce(&FileStore) -> Result<T, IdentityError>,
    ) -> Result<T, IdentityError> {
        if cfg!(target_os = "linux") && Self::store_unavailable(&e) {
            log::warn!(
                "no Secret Service is available ({e}); keeping the desktop identity in {} instead, \
                 which any process running as this user can read",
                self.fallback.path.display()
            );
            return fallback(&self.fallback);
        }
        Err(IdentityError::Store(format!("keychain: {e}")))
    }
}

impl SecretStore for PlatformStore {
    fn read(&self) -> Result<Option<Vec<u8>>, IdentityError> {
        match Self::keychain_read() {
            Ok(v) => Ok(v),
            Err(e) => self.on_failure(e, |f| f.read()),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), IdentityError> {
        match Self::keychain_write(bytes) {
            Ok(()) => Ok(()),
            Err(e) => self.on_failure(e, |f| f.write(bytes)),
        }
    }
}

/// A file that only the owning user can read. The Linux fallback; see
/// [`PlatformStore`].
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SecretStore for FileStore {
    fn read(&self) -> Result<Option<Vec<u8>>, IdentityError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(IdentityError::Store(format!(
                "{}: {e}",
                self.path.display()
            ))),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), IdentityError> {
        let wrap =
            |e: std::io::Error| IdentityError::Store(format!("{}: {e}", self.path.display()));
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(wrap)?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        // Owner-only from the moment the file exists, not chmod'd
        // afterwards -- there is no window where it is world-readable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        opts.open(&self.path)
            .and_then(|mut f| f.write_all(bytes))
            .map_err(wrap)
    }
}

#[cfg(test)]
pub mod testing {
    //! An in-memory store for tests, so nothing touches a keychain.

    use super::{IdentityError, SecretStore};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryStore {
        bytes: Mutex<Option<Vec<u8>>>,
    }

    impl SecretStore for MemoryStore {
        fn read(&self) -> Result<Option<Vec<u8>>, IdentityError> {
            Ok(self.bytes.lock().unwrap().clone())
        }

        fn write(&self, bytes: &[u8]) -> Result<(), IdentityError> {
            *self.bytes.lock().unwrap() = Some(bytes.to_vec());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::MemoryStore;
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn the_fingerprint_is_lowercase_hex_sha256_of_the_der() {
        let id = Identity::generate().unwrap();
        let fp = id.fingerprint();
        assert_eq!(fp.len(), 64);
        assert!(fp
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(fp, fingerprint_of(id.cert().as_ref()));
        // A known vector, so this is SHA256 and not something else.
        assert_eq!(
            fingerprint_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn two_generated_identities_differ() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn the_key_is_p256_and_the_certificate_is_self_signed() {
        let id = Identity::generate().unwrap();
        // The stored key parses as P256 -- the same check `from_bytes`
        // makes, exercised on a fresh key.
        let key = KeyPair::from_pkcs8_der_and_sign_algo(
            &PrivatePkcs8KeyDer::from(id.key_pkcs8.as_slice()),
            &PKCS_ECDSA_P256_SHA256,
        )
        .unwrap();
        assert_eq!(key.algorithm(), &PKCS_ECDSA_P256_SHA256);
        // The certificate's public key is that key's: the raw public
        // key bytes appear verbatim inside the DER's SubjectPublicKeyInfo.
        use rcgen::PublicKeyData;
        let pk = key.der_bytes();
        assert!(id.cert_der.windows(pk.len()).any(|w| w == pk));
    }

    #[test]
    fn validity_is_ten_years_from_today() {
        let (from, to) = validity_window(NaiveDate::from_ymd_opt(2026, 9, 4).unwrap());
        assert_eq!(from, (2026, 9, 4));
        assert_eq!(to, (2036, 9, 4));
        // The tuples are what rcgen is fed, so they must be dates rcgen
        // accepts -- `date_time_ymd` panics on an invalid one.
        let _ = rcgen::date_time_ymd(to.0, to.1, to.2);
    }

    #[test]
    fn a_leap_day_start_does_not_panic_ten_years_on() {
        // 2028 is a leap year; 2038 is not.
        let (_, to) = validity_window(NaiveDate::from_ymd_opt(2028, 2, 29).unwrap());
        assert_eq!(to, (2038, 2, 28));
        let _ = rcgen::date_time_ymd(to.0, to.1, to.2);
        // 2096 -> 2106: neither the 100-year nor the 400-year rule
        // applies, still not a leap year.
        let (_, to) = validity_window(NaiveDate::from_ymd_opt(2096, 2, 29).unwrap());
        assert_eq!(to, (2106, 2, 28));
        // 2020 -> 2030 is not a leap year either; 2040 -> 2050 is not;
        // but a start on any other day is carried through unchanged.
        let (_, to) = validity_window(NaiveDate::from_ymd_opt(2024, 2, 28).unwrap());
        assert_eq!(to, (2034, 2, 28));
    }

    /// The window is actually applied to the certificate, not only
    /// computed: rcgen encodes dates before 2050 as UTCTime, whose
    /// `YYMMDDHHMMSSZ` bytes appear verbatim in the DER.
    #[test]
    fn the_certificate_carries_a_ten_year_not_after() {
        let id = Identity::generate().unwrap();
        let today = chrono::Utc::now().date_naive();
        let (_, to) = validity_window(today);
        let stamp = format!("{:02}{:02}{:02}000000Z", to.0 % 100, to.1, to.2);
        assert!(
            id.cert_der
                .windows(stamp.len())
                .any(|w| w == stamp.as_bytes()),
            "notAfter {stamp} not found in the certificate"
        );
    }

    #[test]
    fn load_or_create_creates_once_and_then_loads_the_same_identity() {
        let store = MemoryStore::default();
        let first = load_or_create(&store).unwrap();
        let second = load_or_create(&store).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn the_stored_blob_holds_the_key_and_certificate_as_base64_json() {
        let store = MemoryStore::default();
        let id = load_or_create(&store).unwrap();
        let raw = store.read().unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["v"], 1);
        let b64 = base64::engine::general_purpose::STANDARD;
        assert_eq!(
            b64.decode(v["cert_der"].as_str().unwrap()).unwrap(),
            id.cert_der
        );
        assert_eq!(
            b64.decode(v["key_pkcs8"].as_str().unwrap()).unwrap(),
            id.key_pkcs8
        );
    }

    /// The one rule that matters most: a broken blob is an ERROR, not a
    /// fresh identity. Regenerating would unpair every phone silently.
    #[test]
    fn a_corrupt_blob_is_an_error_not_a_regeneration() {
        let store = MemoryStore::default();
        store.write(b"{not json").unwrap();
        assert!(matches!(
            load_or_create(&store),
            Err(IdentityError::Corrupt(_))
        ));
        // Still there, untouched, for the user to inspect or remove.
        assert_eq!(store.read().unwrap().unwrap(), b"{not json");

        let store = MemoryStore::default();
        store
            .write(br#"{"v":1,"key_pkcs8":"AAAA","cert_der":"AAAA"}"#)
            .unwrap();
        assert!(matches!(
            load_or_create(&store),
            Err(IdentityError::Corrupt(_))
        ));

        let store = MemoryStore::default();
        store
            .write(br#"{"v":2,"key_pkcs8":"","cert_der":""}"#)
            .unwrap();
        assert!(matches!(
            load_or_create(&store),
            Err(IdentityError::Corrupt(m)) if m.contains("version 2")
        ));
    }

    #[test]
    fn debug_output_never_contains_the_key() {
        let id = Identity::generate().unwrap();
        let b64 = base64::engine::general_purpose::STANDARD;
        let shown = format!("{id:?}");
        assert!(shown.contains(&id.fingerprint()));
        assert!(!shown.contains(&b64.encode(&id.key_pkcs8)));
        assert!(!shown.contains("key_pkcs8"));
    }

    #[test]
    fn the_file_store_round_trips_and_reads_absent_as_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = FileStore::new(dir.path().join("nested").join("remote-identity.json"));
        assert_eq!(store.read().unwrap(), None);
        store.write(b"hello").unwrap();
        assert_eq!(store.read().unwrap().unwrap(), b"hello");
        store.write(b"hi").unwrap();
        assert_eq!(store.read().unwrap().unwrap(), b"hi");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let store = FileStore::new(dir.path().join("remote-identity.json"));
        store.write(b"secret").unwrap();
        let mode = std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// The fallback decision, without a keychain: only "no store here"
    /// errors qualify. A store that answered and rejected the data must
    /// surface, not be papered over with a file.
    #[test]
    fn only_a_missing_store_qualifies_for_the_fallback() {
        assert!(PlatformStore::store_unavailable(
            &keyring::Error::NoDefaultStore
        ));
        assert!(!PlatformStore::store_unavailable(&keyring::Error::NoEntry));
        assert!(!PlatformStore::store_unavailable(
            &keyring::Error::BadEncoding(vec![])
        ));
        assert!(!PlatformStore::store_unavailable(&keyring::Error::Invalid(
            "x".into(),
            "y".into()
        )));
    }
}
