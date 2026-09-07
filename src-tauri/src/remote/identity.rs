//! The desktop's own TLS identity: one ML-DSA-65 key pair and one
//! self-signed certificate, generated the first time phone connections
//! are enabled and kept for ten years.
//!
//! The phone never validates a chain. At pairing it receives this
//! certificate's SHA256 fingerprint out of band (in the QR code) and pins
//! it, so the certificate carries no hostname and no CA -- the
//! fingerprint IS the identity. That is also why the identity must never
//! be silently regenerated: a new certificate is a new fingerprint, and
//! every paired phone would refuse the desktop until re-paired.
//!
//! # ML-DSA-65, on rustls' unstable path
//!
//! rustls 0.23 names the ML-DSA signature schemes but ships neither a
//! signing key nor a verifier for them on that line. Both come from
//! `rustls-post-quantum` built with its `aws-lc-rs-unstable` feature,
//! and the key and certificate are minted by rcgen behind its
//! `aws_lc_rs_unstable` feature. "Unstable" is a statement about the
//! crate API, which may move between minor versions, not about the
//! algorithm: FIPS 204 is final. The plain aws-lc-rs provider cannot
//! load an ML-DSA key at all, so every TLS config on both ends is built
//! on the post-quantum provider and on nothing else.
//!
//! # What is stored, and where
//!
//! Two things, in two places. The **32-byte FIPS 204 seed** goes in the
//! platform keychain: the key pair derives from it deterministically
//! (`PqdsaKeyPair::from_seed`), and 32 bytes fit every keychain. That
//! matters on Windows, where Credential Manager caps a credential blob
//! at `CRED_MAX_CREDENTIAL_BLOB_SIZE` (5 * 512 = 2560 bytes, `wincred.h`).
//! The private key's PKCS#8 is not the problem -- aws-lc-rs 1.18 writes
//! the seed-form `OneAsymmetricKey`, 54 bytes -- the **certificate** is:
//! an ML-DSA-65 certificate is 5,482 bytes, over twice the cap.
//!
//! So the certificate DER lives in a plain file beside the database, on
//! every platform. It is public -- every phone receives it at the
//! handshake -- so nothing is lost by keeping it outside the keychain.
//! What it is NOT is reproducible: ML-DSA signing is hedged (randomised)
//! and the serial number is random, so re-minting from the seed would
//! give a new fingerprint and unpair every phone. A seed without its
//! certificate is therefore reported as a corrupt identity, never
//! papered over with a fresh one.
//!
//! An identity from before 5.1 (ECDSA P-256, stored whole in the
//! keychain as version 1) is replaced on the first enable after the
//! upgrade. Every phone re-pairs for protocol 2 regardless, so there is
//! nothing a kept P-256 identity could still be paired with.
//!
//! Where the seed lives is the point of this module. Headstate has never
//! stored a credential of its own -- the GitHub token is read from `gh`
//! -- and this is the first entry in the platform keychain. The
//! keychain, not SQLite, because the seed is the only thing standing
//! between an attacker on the same network and every command a paired
//! phone can run, and SQLite is a plain file in the app data directory.

use aws_lc_rs::signature::{PqdsaKeyPair, ML_DSA_65_SIGNING};
use base64::Engine;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
    KeyUsagePurpose, PKCS_ML_DSA_65,
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

/// The FIPS 204 seed, `xi`: the whole secret. All three ML-DSA
/// parameter sets use 32 bytes.
pub const SEED_LEN: usize = 32;
pub type Seed = [u8; SEED_LEN];

/// What aws-lc-rs 1.18 writes before the seed in the seed-form PKCS#8
/// of an ML-DSA-65 key: `SEQUENCE { version 0, AlgorithmIdentifier
/// { id-ml-dsa-65 }, OCTET STRING { [0] seed } }`. rcgen hands the key
/// back only in this encoding (it can mint an ML-DSA key but not load
/// one), so this is where the seed is read out of it, and the exact
/// bytes are pinned so a future encoding change is caught at generate
/// time rather than stored and misread later.
const ML_DSA_65_SEED_PKCS8_PREFIX: [u8; 22] = [
    0x30, 0x34, // SEQUENCE, 52 bytes
    0x02, 0x01, 0x00, // INTEGER 0 (v1)
    0x30, 0x0b, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x03,
    0x12, // AlgorithmIdentifier { 2.16.840.1.101.3.4.3.18 }
    0x04, 0x22, // OCTET STRING, 34 bytes
    0x80, 0x20, // [0] IMPLICIT, 32 bytes: the seed
];

/// Keychain coordinates. The service is the bundle identifier so the
/// item is recognisably Headstate's in Keychain Access; the user names
/// what the item is, since one app may hold several one day.
const KEYCHAIN_SERVICE: &str = "com.pktstorm.headstate";
const KEYCHAIN_USER: &str = "remote-identity";

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("could not generate the desktop identity: {0}")]
    Generate(#[from] rcgen::Error),
    /// The key came back in a shape this build cannot store or reload.
    /// A dependency change, not a user condition; refused rather than
    /// stored, because a seed read from the wrong bytes would derive a
    /// key that does not match the certificate on the next start.
    #[error("the TLS library produced an ML-DSA-65 key in a form this build cannot store ({0})")]
    Unsupported(String),
    /// The stored identity exists but cannot be used. Deliberately NOT
    /// recovered by regenerating: that would invalidate every pairing
    /// without telling anyone. The user sees this and decides.
    #[error(
        "the stored desktop identity is unreadable ({0}); remove the keychain item and the certificate file to start over"
    )]
    Corrupt(String),
    #[error("{0}")]
    Store(String),
}

/// Where a blob is kept.
///
/// Two of these make an identity: one for the seed, which must be a
/// secret store, and one for the certificate, which need not be. A
/// trait so the listener tests never touch a real keychain: a unit test
/// that writes to the macOS keychain leaves an item behind on the
/// developer's machine and can hang a CI runner on an access prompt.
/// Production uses [`PlatformStore`] for the seed and [`FileStore`] for
/// the certificate; tests use an in-memory store.
pub trait BlobStore: Send + Sync {
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

/// What the secret store holds. Versioned so an older shape is told
/// apart from this one rather than mis-parsed: version 1 was the whole
/// P-256 identity (`key_pkcs8` and `cert_der`), which this build
/// replaces; version 2 is the seed alone.
#[derive(Serialize, Deserialize)]
struct Stored {
    v: u32,
    #[serde(default)]
    seed: String,
}

const STORED_VERSION: u32 = 2;
const LEGACY_P256_VERSION: u32 = 1;

/// What a read of the secret store found.
enum Found {
    Seed(Seed),
    LegacyP256,
}

impl Identity {
    /// A fresh ML-DSA-65 key and a self-signed certificate valid from
    /// today for [`VALIDITY_YEARS`].
    pub fn generate() -> Result<Self, IdentityError> {
        Self::generate_with_seed().map(|(id, _)| id)
    }

    /// [`Identity::generate`], plus the seed the secret store keeps.
    fn generate_with_seed() -> Result<(Self, Seed), IdentityError> {
        let key = KeyPair::generate_for(&PKCS_ML_DSA_65)?;
        let key_pkcs8 = key.serialize_der();
        let seed = seed_from_pkcs8(&key_pkcs8)?;
        let cert_der = self_signed(&key)?;
        Ok((
            Self {
                cert_der,
                key_pkcs8,
            },
            seed,
        ))
    }

    /// The identity a stored seed and certificate describe, or
    /// [`IdentityError::Corrupt`] when the certificate is not the seed's
    /// -- the same key-matches-certificate check rustls makes when the
    /// listener starts, made here so a swapped or stale certificate file
    /// stops the enable with a message rather than failing every
    /// handshake afterwards.
    fn from_seed_and_cert(seed: &Seed, cert_der: Vec<u8>) -> Result<Self, IdentityError> {
        let key_pkcs8 = pkcs8_from_seed(seed)?;
        let provider = rustls_post_quantum::provider();
        rustls::sign::CertifiedKey::from_der(
            vec![CertificateDer::from(cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pkcs8.clone())),
            &provider,
        )
        .map_err(|e| IdentityError::Corrupt(format!("certificate does not match the key: {e}")))?;
        Ok(Self {
            cert_der,
            key_pkcs8,
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

    /// The private key, for rustls: the seed-form PKCS#8, which only the
    /// post-quantum provider can load.
    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_pkcs8.clone()))
    }
}

/// The self-signed certificate for `key`: ten years from today, a
/// common name, digital signature, server auth.
fn self_signed(key: &KeyPair) -> Result<Vec<u8>, IdentityError> {
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
    Ok(params.self_signed(key)?.der().to_vec())
}

/// The seed out of the seed-form PKCS#8 rcgen returns; see
/// [`ML_DSA_65_SEED_PKCS8_PREFIX`].
fn seed_from_pkcs8(der: &[u8]) -> Result<Seed, IdentityError> {
    let expected = ML_DSA_65_SEED_PKCS8_PREFIX.len() + SEED_LEN;
    if der.len() != expected
        || der[..ML_DSA_65_SEED_PKCS8_PREFIX.len()] != ML_DSA_65_SEED_PKCS8_PREFIX
    {
        return Err(IdentityError::Unsupported(format!(
            "{} bytes, not the {expected}-byte seed form",
            der.len()
        )));
    }
    let mut seed = [0u8; SEED_LEN];
    seed.copy_from_slice(&der[ML_DSA_65_SEED_PKCS8_PREFIX.len()..]);
    Ok(seed)
}

/// The seed-form PKCS#8 of the key `seed` derives -- byte for byte what
/// rcgen produced when the seed was generated, so rustls loads the same
/// key on every start.
fn pkcs8_from_seed(seed: &Seed) -> Result<Vec<u8>, IdentityError> {
    let key = PqdsaKeyPair::from_seed(&ML_DSA_65_SIGNING, seed)
        .map_err(|e| IdentityError::Corrupt(format!("seed: {e}")))?;
    key.to_pkcs8v1()
        .map(|doc| doc.as_ref().to_vec())
        .map_err(|e| IdentityError::Corrupt(format!("seed: {e}")))
}

fn stored_bytes(seed: &Seed) -> Vec<u8> {
    let stored = Stored {
        v: STORED_VERSION,
        seed: base64::engine::general_purpose::STANDARD.encode(seed),
    };
    // A number and a string cannot fail to serialise.
    serde_json::to_vec(&stored).expect("identity serialises")
}

fn parse_stored(bytes: &[u8]) -> Result<Found, IdentityError> {
    let stored: Stored =
        serde_json::from_slice(bytes).map_err(|e| IdentityError::Corrupt(e.to_string()))?;
    match stored.v {
        STORED_VERSION => {}
        LEGACY_P256_VERSION => return Ok(Found::LegacyP256),
        other => {
            return Err(IdentityError::Corrupt(format!(
                "version {other} (this build reads {STORED_VERSION})"
            )))
        }
    }
    let seed = base64::engine::general_purpose::STANDARD
        .decode(&stored.seed)
        .map_err(|e| IdentityError::Corrupt(format!("seed: {e}")))?;
    let seed: Seed = seed
        .try_into()
        .map_err(|v: Vec<u8>| IdentityError::Corrupt(format!("seed is {} bytes", v.len())))?;
    Ok(Found::Seed(seed))
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

/// The identity from the stores, or a new one written to them.
///
/// The only place an identity is ever created. Anything unreadable is
/// an error, never a regeneration -- see [`IdentityError::Corrupt`].
/// The one exception is a version-1 (P-256) secret from before this
/// build, which is replaced: no phone can be paired with it under
/// protocol 2, and the replacement is logged.
pub fn load_or_create(
    secret: &dyn BlobStore,
    certificate: &dyn BlobStore,
) -> Result<Identity, IdentityError> {
    match secret.read()?.as_deref().map(parse_stored).transpose()? {
        Some(Found::Seed(seed)) => {
            let cert_der = certificate.read()?.ok_or_else(|| {
                IdentityError::Corrupt(
                    "the seed is stored but the certificate file is missing".into(),
                )
            })?;
            Identity::from_seed_and_cert(&seed, cert_der)
        }
        Some(Found::LegacyP256) => {
            log::warn!(
                "the stored desktop identity is the pre-5.1 P-256 kind; replacing it with an \
                 ML-DSA-65 identity (every phone must pair again)"
            );
            create(secret, certificate)
        }
        None => create(secret, certificate),
    }
}

/// Certificate first, then the seed: a crash between the two leaves an
/// orphan certificate that the next start overwrites, never a seed
/// whose certificate is gone.
fn create(secret: &dyn BlobStore, certificate: &dyn BlobStore) -> Result<Identity, IdentityError> {
    let (identity, seed) = Identity::generate_with_seed()?;
    certificate.write(&identity.cert_der)?;
    secret.write(&stored_bytes(&seed))?;
    log::info!(
        "generated the desktop identity (ML-DSA-65), fingerprint {}",
        identity.fingerprint()
    );
    Ok(identity)
}

/// The platform keychain, via the `keyring` crate: Keychain Services on
/// macOS, Credential Manager on Windows, the freedesktop Secret Service
/// on Linux. Holds the seed; see the module docs for why only that.
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

impl BlobStore for PlatformStore {
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

/// A file that only the owning user can read. The certificate's home
/// on every platform, and the seed's Linux fallback; see
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

impl BlobStore for FileStore {
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
    //! An in-memory store for tests, so nothing touches a keychain, and
    //! the certificate checks the listener tests share.

    use super::{BlobStore, Identity, IdentityError};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryStore {
        bytes: Mutex<Option<Vec<u8>>>,
    }

    impl BlobStore for MemoryStore {
        fn read(&self) -> Result<Option<Vec<u8>>, IdentityError> {
            Ok(self.bytes.lock().unwrap().clone())
        }

        fn write(&self, bytes: &[u8]) -> Result<(), IdentityError> {
            *self.bytes.lock().unwrap() = Some(bytes.to_vec());
            Ok(())
        }
    }

    /// `id-ml-dsa-65`, 2.16.840.1.101.3.4.3.18, as it appears in DER.
    const ML_DSA_65_OID: [u8; 11] = [
        0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x03, 0x12,
    ];

    /// Whether a certificate is an ML-DSA-65 one: the algorithm
    /// identifier appears exactly three times in a self-signed
    /// certificate -- the subject public key's, the TBSCertificate's
    /// signature field, and the outer signatureAlgorithm -- and in no
    /// other kind at all.
    pub(crate) fn is_ml_dsa_65_certificate(der: &[u8]) -> bool {
        der.windows(ML_DSA_65_OID.len())
            .filter(|w| *w == ML_DSA_65_OID)
            .count()
            == 3
    }

    impl Identity {
        /// What a phone from before protocol 2 presents: an ECDSA P-256
        /// session certificate. Exists only so the listener can prove
        /// it refuses one.
        pub(crate) fn p256_for_tests() -> Identity {
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            Identity {
                cert_der: super::self_signed(&key).unwrap(),
                key_pkcs8: key.serialize_der(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{is_ml_dsa_65_certificate, MemoryStore};
    use super::*;
    use chrono::NaiveDate;

    /// Windows Credential Manager's `CRED_MAX_CREDENTIAL_BLOB_SIZE`.
    const WINDOWS_CREDENTIAL_BLOB_MAX: usize = 5 * 512;

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

    /// The key is ML-DSA-65 and the certificate is its own: the
    /// post-quantum provider loads the key (the plain aws-lc-rs one
    /// cannot), rustls' key-matches-certificate check passes, and the
    /// certificate names id-ml-dsa-65 in all three places a self-signed
    /// one does.
    #[test]
    fn the_key_is_ml_dsa_65_and_the_certificate_is_self_signed() {
        let id = Identity::generate().unwrap();
        let pq = rustls_post_quantum::provider();
        let certified =
            rustls::sign::CertifiedKey::from_der(vec![id.cert()], id.key(), &pq).unwrap();
        assert_eq!(
            certified
                .key
                .choose_scheme(&[rustls::SignatureScheme::ML_DSA_65])
                .map(|s| s.scheme()),
            Some(rustls::SignatureScheme::ML_DSA_65)
        );
        assert!(certified
            .key
            .choose_scheme(&[rustls::SignatureScheme::ECDSA_NISTP256_SHA256])
            .is_none());
        assert!(is_ml_dsa_65_certificate(id.cert().as_ref()));
        assert!(!is_ml_dsa_65_certificate(
            Identity::p256_for_tests().cert().as_ref()
        ));
        // NOT asserted: that plain aws-lc-rs cannot LOAD this key.
        // True when written and false since rustls 0.23.44. The
        // assertions above -- ML-DSA-65 chosen, ECDSA refused, the
        // certificate ML-DSA-65 -- are the ones about this identity, and
        // they still hold. See #588.
    }

    /// The seed rcgen's PKCS#8 carries derives the same key aws-lc-rs
    /// wrote it from: same PKCS#8 bytes, so rustls loads the same key
    /// on every start.
    #[test]
    fn the_seed_reproduces_the_key() {
        let key = KeyPair::generate_for(&PKCS_ML_DSA_65).unwrap();
        let der = key.serialize_der();
        assert_eq!(der.len(), ML_DSA_65_SEED_PKCS8_PREFIX.len() + SEED_LEN);
        let seed = seed_from_pkcs8(&der).unwrap();
        assert_eq!(pkcs8_from_seed(&seed).unwrap(), der);

        // Not the seed form: refused at generate time, never stored.
        let p256 = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        assert!(matches!(
            seed_from_pkcs8(&p256.serialize_der()),
            Err(IdentityError::Unsupported(_))
        ));
        let mut wrong_prefix = der.clone();
        wrong_prefix[17] = 0x11; // id-ml-dsa-44
        assert!(matches!(
            seed_from_pkcs8(&wrong_prefix),
            Err(IdentityError::Unsupported(_))
        ));
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
        let (secret, cert) = (MemoryStore::default(), MemoryStore::default());
        let first = load_or_create(&secret, &cert).unwrap();
        let second = load_or_create(&secret, &cert).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.fingerprint(), second.fingerprint());
        // Loaded from the stores, not merely cached: a fresh pair of
        // stores holding the same bytes gives the same identity.
        let (again_secret, again_cert) = (MemoryStore::default(), MemoryStore::default());
        again_secret
            .write(&secret.read().unwrap().unwrap())
            .unwrap();
        again_cert.write(&cert.read().unwrap().unwrap()).unwrap();
        assert_eq!(load_or_create(&again_secret, &again_cert).unwrap(), first);
    }

    /// The split the module docs describe: the keychain holds a seed
    /// small enough for every platform's keychain, the certificate --
    /// which is not -- lives in the file.
    #[test]
    fn the_secret_is_the_seed_and_the_certificate_is_the_file() {
        let (secret, cert) = (MemoryStore::default(), MemoryStore::default());
        let id = load_or_create(&secret, &cert).unwrap();
        let raw = secret.read().unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["v"], 2);
        let seed = base64::engine::general_purpose::STANDARD
            .decode(v["seed"].as_str().unwrap())
            .unwrap();
        assert_eq!(seed.len(), SEED_LEN);
        assert!(v.get("key_pkcs8").is_none() && v.get("cert_der").is_none());
        assert!(
            raw.len() < WINDOWS_CREDENTIAL_BLOB_MAX,
            "{} bytes would not fit a Windows credential",
            raw.len()
        );
        assert_eq!(cert.read().unwrap().unwrap(), id.cert_der);
        assert!(
            id.cert_der.len() > WINDOWS_CREDENTIAL_BLOB_MAX,
            "the certificate ({} bytes) is why the two are stored apart",
            id.cert_der.len()
        );
    }

    /// The pre-5.1 identity: replaced, once, with a log line, since no
    /// phone can be paired with it under protocol 2.
    #[test]
    fn a_legacy_p256_secret_is_replaced_not_kept_and_not_an_error() {
        let (secret, cert) = (MemoryStore::default(), MemoryStore::default());
        secret
            .write(br#"{"v":1,"key_pkcs8":"AAAA","cert_der":"AAAA"}"#)
            .unwrap();
        let id = load_or_create(&secret, &cert).unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&secret.read().unwrap().unwrap()).unwrap();
        assert_eq!(v["v"], 2);
        assert!(is_ml_dsa_65_certificate(&id.cert_der));
        assert_eq!(load_or_create(&secret, &cert).unwrap(), id);
    }

    /// The one rule that matters most: a broken identity is an ERROR,
    /// not a fresh one. Regenerating would unpair every phone silently.
    #[test]
    fn a_corrupt_identity_is_an_error_not_a_regeneration() {
        let corrupt = |secret_bytes: &[u8], cert_bytes: Option<&[u8]>| {
            let (secret, cert) = (MemoryStore::default(), MemoryStore::default());
            secret.write(secret_bytes).unwrap();
            if let Some(c) = cert_bytes {
                cert.write(c).unwrap();
            }
            let err = load_or_create(&secret, &cert).unwrap_err();
            // Still there, untouched, for the user to inspect or remove.
            assert_eq!(secret.read().unwrap().unwrap(), secret_bytes);
            match err {
                IdentityError::Corrupt(m) => m,
                other => panic!("expected Corrupt, got {other:?}"),
            }
        };
        corrupt(b"{not json", None);
        assert!(corrupt(br#"{"v":3,"seed":""}"#, None).contains("version 3"));
        assert!(corrupt(br#"{"v":2,"seed":"AAAA"}"#, None).contains("3 bytes"));
        assert!(corrupt(br#"{"v":2,"seed":"not base64!"}"#, None).contains("seed"));

        let (secret, cert) = (MemoryStore::default(), MemoryStore::default());
        let id = load_or_create(&secret, &cert).unwrap();
        let seed_blob = secret.read().unwrap().unwrap();
        // The seed without its certificate: it cannot be re-minted with
        // the same fingerprint, so this is corrupt, not fresh.
        assert!(corrupt(&seed_blob, None).contains("certificate file is missing"));
        // A certificate that is not this seed's.
        let other = Identity::generate().unwrap();
        assert!(corrupt(&seed_blob, Some(&other.cert_der)).contains("does not match"));
        assert!(corrupt(&seed_blob, Some(b"\x30\x03\x02\x01\x00")).contains("does not match"));
        // And with its own certificate back, the same identity again.
        assert_eq!(load_or_create(&secret, &cert).unwrap(), id);
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
