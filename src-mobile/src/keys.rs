//! The phone's keys: the TLS session identity and the step-up signing
//! pair (ECDSA P-256 plus, where available, ML-DSA-65).
//!
//! # The seam
//!
//! [`DeviceKeys`] is the whole contract between this crate and wherever
//! the keys actually live. Two implementations:
//!
//! - [`HardwareKeys`]: `tauri-plugin-headstate-keys` (#513,
//!   `src-mobile/plugins/headstate-keys/`) -- Secure Enclave / Android
//!   Keystore step-up keys with biometric access control, and the
//!   session identity in the keychain -- adapted to this trait, its
//!   errors mapped onto [`KeyError`].
//! - [`SoftwareKeys`]: keys derived from seeds kept in the encrypted
//!   settings store (`store.rs`). Every test uses it, and so does a
//!   phone (or the simulator, or a desktop host) whose hardware cannot
//!   hold a key.
//!
//! Production gets [`FallbackKeys`] over the two: hardware first,
//! software when the plugin says [`KeyError::Unavailable`].
//!
//! Nothing above this trait knows which one it has. `pairing.rs` calls
//! `generate` then `public_keys` and `session_identity`; `stepup.rs`
//! calls `sign`; `unpair` calls `destroy`.
//!
//! # Encodings (the wire, from the desktop's `remote/pairing.rs` and
//! `remote/stepup.rs`)
//!
//! - ECDSA P-256 public key: 65-byte SEC1 uncompressed point.
//! - ML-DSA-65 public key: 1952 bytes, FIPS 204 encoding.
//! - ECDSA signature: raw `r || s`, 64 bytes (IEEE P1363), over the
//!   SHA256 of the message, either `s` accepted.
//! - ML-DSA-65 signature: 3309 bytes, pure mode, EMPTY context string.
//! - Session identity: the certificate as DER and the private key as
//!   PKCS#8 DER, the shapes rustls takes for a client identity.
//!
//! A hardware implementation must return exactly these. On iOS,
//! `P256.Signing.ECDSASignature.rawRepresentation` and
//! `x963Representation` for the key; on Android,
//! `SHA256withECDSAinP1363Format` and the X.509-encoded key's last 65
//! bytes. ML-DSA keys and signatures come out of both platforms in the
//! FIPS encodings already.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::Arc;
// The plugin's trait, under another name: it shares its spelling with
// this crate's, and only [`HardwareKeys`] calls it.
use tauri_plugin_headstate_keys::DeviceKeys as PluginDeviceKeys;

use crate::store::{get_json, put_json, Store, StoreError};

pub const ECDSA_P256_LEN: usize = 65;
pub const MLDSA_65_LEN: usize = 1952;
pub const ECDSA_SIG_LEN: usize = 64;
pub const MLDSA_SIG_LEN: usize = 3309;

/// Session certificate validity. Long, like the desktop's, because the
/// desktop pins the fingerprint and checks no dates; an expiry would
/// silently unpair the phone.
const VALIDITY_YEARS: i32 = 10;

const SESSION_KEY: &str = "keys/session";
const STEPUP_KEY: &str = "keys/stepup";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// `generate` has not been called (or `destroy` was). The phone is
    /// not paired.
    #[error("this phone has no device keys; pair with a desktop first")]
    NoKeys,
    #[error(transparent)]
    Store(#[from] StoreError),
    /// This backend cannot hold keys on this device at all: no Secure
    /// Enclave / Keystore support, the simulator, a desktop host.
    /// [`FallbackKeys`] answers it by using software keys instead.
    #[error("hardware-backed keys are not available on this device: {0}")]
    Unavailable(String),
    #[error("device keys: {0}")]
    Crypto(String),
}

/// The step-up public keys, as raw bytes in the encodings above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKeys {
    pub ecdsa_p256: Vec<u8>,
    pub mldsa_65: Option<Vec<u8>>,
}

/// One step-up signature set over one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signatures {
    pub ecdsa: Vec<u8>,
    pub mldsa: Option<Vec<u8>>,
}

/// The TLS client identity. No `Debug` of the key: the manual impl
/// prints the fingerprint alone.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub cert_der: Vec<u8>,
    pub key_pkcs8: Vec<u8>,
}

impl fmt::Debug for SessionIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionIdentity")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

impl SessionIdentity {
    /// SHA256 of the certificate DER, lowercase hex, no prefix: the
    /// `client_fp` in the pairing proof and the desktop's
    /// `paired_devices.cert_fp`.
    pub fn fingerprint(&self) -> String {
        fingerprint_of(&self.cert_der)
    }
}

/// SHA256 of a certificate's DER as 64 lowercase hex characters, the
/// fingerprint convention shared with the desktop.
pub fn fingerprint_of(der: &[u8]) -> String {
    format!("{:x}", Sha256::digest(der))
}

/// Where the keys live. See the module docs for the two implementations.
pub trait DeviceKeys: Send + Sync {
    /// Create a fresh session identity and step-up pair, replacing any
    /// existing ones. Called once per pairing. Returns the step-up
    /// public keys for the pair request.
    fn generate(&self) -> Result<PublicKeys, KeyError>;
    /// The step-up public keys, or [`KeyError::NoKeys`].
    fn public_keys(&self) -> Result<PublicKeys, KeyError>;
    /// Sign `bytes` with every step-up key this device holds. On a
    /// hardware implementation this is where the biometric prompt
    /// happens.
    fn sign(&self, bytes: &[u8]) -> Result<Signatures, KeyError>;
    /// Forget every key. Idempotent.
    fn destroy(&self) -> Result<(), KeyError>;
    /// The TLS client identity, or [`KeyError::NoKeys`].
    fn session_identity(&self) -> Result<SessionIdentity, KeyError>;
}

/// `count` bytes from the TLS provider's CSPRNG. The provider is already
/// linked for TLS, so this adds no dependency.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], KeyError> {
    let mut out = [0u8; N];
    rustls::crypto::aws_lc_rs::default_provider()
        .secure_random
        .fill(&mut out)
        .map_err(|e| KeyError::Crypto(format!("random bytes: {e:?}")))?;
    Ok(out)
}

// ---------------------------------------------------------------------
// The software implementation
// ---------------------------------------------------------------------

/// What [`SoftwareKeys`] persists at `keys/session`.
#[derive(Serialize, Deserialize)]
pub struct StoredSession {
    pub v: u32,
    /// Standard base64.
    pub cert_der: String,
    /// Standard base64, PKCS#8 DER.
    pub key_pkcs8: String,
}

/// What [`SoftwareKeys`] persists at `keys/stepup`.
#[derive(Serialize, Deserialize)]
pub struct StoredStepUp {
    pub v: u32,
    /// Standard base64, 32 bytes: the P-256 scalar.
    pub ecdsa_seed: String,
    /// Standard base64, 32 bytes: the FIPS 204 seed `xi`. Always present
    /// from this implementation; optional in the shape because a
    /// hardware implementation that persists through the same store may
    /// have none.
    pub mldsa_seed: Option<String>,
}

const STORED_VERSION: u32 = 1;

/// Keys derived from seeds in the settings store.
pub struct SoftwareKeys {
    store: Arc<dyn Store>,
}

struct StepUpPair {
    ecdsa: p256::ecdsa::SigningKey,
    mldsa: ml_dsa::SigningKey<ml_dsa::MlDsa65>,
}

impl SoftwareKeys {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    fn load_pair(&self) -> Result<StepUpPair, KeyError> {
        let stored: StoredStepUp =
            get_json(self.store.as_ref(), STEPUP_KEY)?.ok_or(KeyError::NoKeys)?;
        let ecdsa_seed = decode(&stored.ecdsa_seed, "ecdsa seed")?;
        let ecdsa = p256::ecdsa::SigningKey::from_slice(&ecdsa_seed)
            .map_err(|e| KeyError::Crypto(format!("stored ECDSA seed: {e}")))?;
        let mldsa_seed: [u8; 32] = decode(
            stored.mldsa_seed.as_deref().ok_or(KeyError::NoKeys)?,
            "ml-dsa seed",
        )?
        .try_into()
        .map_err(|_| KeyError::Crypto("stored ML-DSA seed is not 32 bytes".into()))?;
        let mldsa =
            ml_dsa::SigningKey::<ml_dsa::MlDsa65>::from_seed(&ml_dsa::Seed::from(mldsa_seed));
        Ok(StepUpPair { ecdsa, mldsa })
    }
}

fn decode(b64: &str, what: &str) -> Result<Vec<u8>, KeyError> {
    BASE64
        .decode(b64)
        .map_err(|e| KeyError::Crypto(format!("stored {what} is not base64: {e}")))
}

fn public_keys_of(pair: &StepUpPair) -> PublicKeys {
    let keys = PublicKeys {
        ecdsa_p256: pair
            .ecdsa
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec(),
        mldsa_65: Some(pair.mldsa.expanded_key().verifying_key().encode().to_vec()),
    };
    // The encodings the desktop's pair handler measures.
    debug_assert_eq!(keys.ecdsa_p256.len(), ECDSA_P256_LEN);
    debug_assert_eq!(keys.mldsa_65.as_ref().map(Vec::len), Some(MLDSA_65_LEN));
    keys
}

/// `(not_before, not_after)` as `(year, month, day)`: yesterday, so a
/// desktop whose clock is slightly behind still sees a valid
/// certificate, through [`VALIDITY_YEARS`] from today. 29 February is
/// clamped to the 28th in the target year.
fn validity_window(today: chrono::NaiveDate) -> ((i32, u8, u8), (i32, u8, u8)) {
    use chrono::Datelike;
    let from = today.pred_opt().unwrap_or(today);
    let to_day = if today.month() == 2 && today.day() == 29 {
        28
    } else {
        today.day() as u8
    };
    (
        (from.year(), from.month() as u8, from.day() as u8),
        (today.year() + VALIDITY_YEARS, today.month() as u8, to_day),
    )
}

fn generate_session() -> Result<SessionIdentity, KeyError> {
    let crypto = |e: rcgen::Error| KeyError::Crypto(format!("session certificate: {e}"));
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(crypto)?;
    let (from, to) = validity_window(chrono::Utc::now().date_naive());
    let mut params = CertificateParams::default();
    params.not_before = rcgen::date_time_ymd(from.0, from.1, from.2);
    params.not_after = rcgen::date_time_ymd(to.0, to.1, to.2);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Headstate Companion");
    params.distinguished_name = dn;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let cert = params.self_signed(&key).map_err(crypto)?;
    Ok(SessionIdentity {
        cert_der: cert.der().to_vec(),
        key_pkcs8: key.serialize_der(),
    })
}

impl DeviceKeys for SoftwareKeys {
    fn generate(&self) -> Result<PublicKeys, KeyError> {
        let session = generate_session()?;
        // A random 32-byte scalar is a valid P-256 key unless it is zero
        // or at least the group order, odds around 2^-128 per draw; the
        // loop is correctness, not an expected path.
        let ecdsa = loop {
            let seed = random_bytes::<32>()?;
            if p256::ecdsa::SigningKey::from_slice(&seed).is_ok() {
                break seed;
            }
        };
        let mldsa = random_bytes::<32>()?;
        put_json(
            self.store.as_ref(),
            SESSION_KEY,
            &StoredSession {
                v: STORED_VERSION,
                cert_der: BASE64.encode(&session.cert_der),
                key_pkcs8: BASE64.encode(&session.key_pkcs8),
            },
        )?;
        put_json(
            self.store.as_ref(),
            STEPUP_KEY,
            &StoredStepUp {
                v: STORED_VERSION,
                ecdsa_seed: BASE64.encode(ecdsa),
                mldsa_seed: Some(BASE64.encode(mldsa)),
            },
        )?;
        self.public_keys()
    }

    fn public_keys(&self) -> Result<PublicKeys, KeyError> {
        Ok(public_keys_of(&self.load_pair()?))
    }

    fn sign(&self, bytes: &[u8]) -> Result<Signatures, KeyError> {
        use p256::ecdsa::signature::Signer;
        let pair = self.load_pair()?;
        let ecdsa: p256::ecdsa::Signature = pair.ecdsa.sign(bytes);
        let mldsa = pair
            .mldsa
            .expanded_key()
            .sign_deterministic(bytes, b"")
            .map_err(|e| KeyError::Crypto(format!("ML-DSA signing: {e:?}")))?;
        let sigs = Signatures {
            ecdsa: ecdsa.to_bytes().to_vec(),
            mldsa: Some(mldsa.encode().to_vec()),
        };
        // The encodings the desktop's step-up verifier measures.
        debug_assert_eq!(sigs.ecdsa.len(), ECDSA_SIG_LEN);
        debug_assert_eq!(sigs.mldsa.as_ref().map(Vec::len), Some(MLDSA_SIG_LEN));
        Ok(sigs)
    }

    fn destroy(&self) -> Result<(), KeyError> {
        self.store.remove(SESSION_KEY)?;
        self.store.remove(STEPUP_KEY)?;
        Ok(())
    }

    fn session_identity(&self) -> Result<SessionIdentity, KeyError> {
        let stored: StoredSession =
            get_json(self.store.as_ref(), SESSION_KEY)?.ok_or(KeyError::NoKeys)?;
        Ok(SessionIdentity {
            cert_der: decode(&stored.cert_der, "session certificate")?,
            key_pkcs8: decode(&stored.key_pkcs8, "session key")?,
        })
    }
}

// ---------------------------------------------------------------------
// The hardware implementation, through the keys plugin
// ---------------------------------------------------------------------

/// The keys plugin adapted to this crate's trait: the same five calls,
/// the same byte encodings (the plugin's `wire.rs` pins them to the
/// desktop's), its errors mapped onto [`KeyError`]. Holds the app
/// handle because the plugin keeps its state in Tauri's.
pub struct HardwareKeys {
    app: tauri::AppHandle,
}

impl HardwareKeys {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn plugin(&self) -> &tauri_plugin_headstate_keys::HeadstateKeys {
        use tauri_plugin_headstate_keys::HeadstateKeysExt;
        self.app.headstate_keys()
    }
}

/// `NotGenerated` is this crate's [`KeyError::NoKeys`]; `Unavailable`
/// keeps its meaning so [`FallbackKeys`] can act on it; everything else
/// (a cancelled or failed prompt, a malformed reply, a certificate or
/// plugin fault) is reported with the plugin's own wording.
fn plugin_error(e: tauri_plugin_headstate_keys::Error) -> KeyError {
    use tauri_plugin_headstate_keys::Error as E;
    match e {
        E::NotGenerated => KeyError::NoKeys,
        E::Unavailable(why) => KeyError::Unavailable(why),
        other => KeyError::Crypto(other.to_string()),
    }
}

impl DeviceKeys for HardwareKeys {
    fn generate(&self) -> Result<PublicKeys, KeyError> {
        let k = self.plugin().generate().map_err(plugin_error)?;
        Ok(PublicKeys {
            ecdsa_p256: k.ecdsa_p256,
            mldsa_65: k.mldsa_65,
        })
    }

    fn public_keys(&self) -> Result<PublicKeys, KeyError> {
        let k = self.plugin().public_keys().map_err(plugin_error)?;
        Ok(PublicKeys {
            ecdsa_p256: k.ecdsa_p256,
            mldsa_65: k.mldsa_65,
        })
    }

    fn sign(&self, bytes: &[u8]) -> Result<Signatures, KeyError> {
        let s = self.plugin().sign(bytes).map_err(plugin_error)?;
        Ok(Signatures {
            ecdsa: s.ecdsa,
            mldsa: s.mldsa,
        })
    }

    fn destroy(&self) -> Result<(), KeyError> {
        self.plugin().destroy().map_err(plugin_error)
    }

    fn session_identity(&self) -> Result<SessionIdentity, KeyError> {
        let s = self.plugin().session_identity().map_err(plugin_error)?;
        Ok(SessionIdentity {
            cert_der: s.cert_der,
            key_pkcs8: s.key_pkcs8,
        })
    }
}

// ---------------------------------------------------------------------
// Hardware first, software where there is none
// ---------------------------------------------------------------------

/// Hardware keys where the phone has them, software keys where it does
/// not. Decided at `generate`: the hardware is tried first and
/// [`KeyError::Unavailable`] falls back to [`SoftwareKeys`] with a
/// warning in the log. Every later call asks the hardware first and
/// takes the software answer only when the hardware has no keys or is
/// unavailable, so a phone that paired in hardware never quietly signs
/// in software; a successful hardware `generate` also clears any
/// software keys left from an earlier pairing. `destroy` clears both.
pub struct FallbackKeys<H: DeviceKeys> {
    hardware: H,
    software: SoftwareKeys,
}

impl<H: DeviceKeys> FallbackKeys<H> {
    pub fn new(hardware: H, software: SoftwareKeys) -> Self {
        Self { hardware, software }
    }

    fn either<T>(
        &self,
        op: impl Fn(&dyn DeviceKeys) -> Result<T, KeyError>,
    ) -> Result<T, KeyError> {
        match op(&self.hardware) {
            Err(KeyError::NoKeys) | Err(KeyError::Unavailable(_)) => op(&self.software),
            other => other,
        }
    }
}

impl<H: DeviceKeys> DeviceKeys for FallbackKeys<H> {
    fn generate(&self) -> Result<PublicKeys, KeyError> {
        match self.hardware.generate() {
            Ok(keys) => {
                self.software.destroy()?;
                Ok(keys)
            }
            Err(KeyError::Unavailable(why)) => {
                log::warn!("companion: hardware keys unavailable ({why}); using software keys");
                self.software.generate()
            }
            Err(e) => Err(e),
        }
    }

    fn public_keys(&self) -> Result<PublicKeys, KeyError> {
        self.either(|k| k.public_keys())
    }

    fn sign(&self, bytes: &[u8]) -> Result<Signatures, KeyError> {
        self.either(|k| k.sign(bytes))
    }

    fn destroy(&self) -> Result<(), KeyError> {
        match self.hardware.destroy() {
            Ok(()) | Err(KeyError::Unavailable(_)) => {}
            Err(e) => return Err(e),
        }
        self.software.destroy()
    }

    fn session_identity(&self) -> Result<SessionIdentity, KeyError> {
        self.either(|k| k.session_identity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    pub(crate) fn software() -> SoftwareKeys {
        SoftwareKeys::new(Arc::new(MemoryStore::default()))
    }

    #[test]
    fn plugin_errors_map_onto_this_crates() {
        use tauri_plugin_headstate_keys::Error as E;
        assert_eq!(plugin_error(E::NotGenerated), KeyError::NoKeys);
        assert_eq!(
            plugin_error(E::Unavailable("simulator".into())),
            KeyError::Unavailable("simulator".into())
        );
        assert_eq!(
            plugin_error(E::Cancelled),
            KeyError::Crypto(E::Cancelled.to_string())
        );
        assert!(matches!(
            plugin_error(E::AuthFailed("x".into())),
            KeyError::Crypto(_)
        ));
    }

    /// A stand-in hardware backend: either absent on this device, or
    /// working (software keys on a store of its own, so which backend
    /// answered is visible from the public keys).
    enum Fake {
        Absent,
        Working(SoftwareKeys),
    }

    impl Fake {
        fn call<T>(
            &self,
            op: impl FnOnce(&SoftwareKeys) -> Result<T, KeyError>,
        ) -> Result<T, KeyError> {
            match self {
                Fake::Absent => Err(KeyError::Unavailable("no secure enclave".into())),
                Fake::Working(k) => op(k),
            }
        }
    }

    impl DeviceKeys for Fake {
        fn generate(&self) -> Result<PublicKeys, KeyError> {
            self.call(|k| k.generate())
        }
        fn public_keys(&self) -> Result<PublicKeys, KeyError> {
            self.call(|k| k.public_keys())
        }
        fn sign(&self, bytes: &[u8]) -> Result<Signatures, KeyError> {
            self.call(|k| k.sign(bytes))
        }
        fn destroy(&self) -> Result<(), KeyError> {
            self.call(|k| k.destroy())
        }
        fn session_identity(&self) -> Result<SessionIdentity, KeyError> {
            self.call(|k| k.session_identity())
        }
    }

    #[test]
    fn without_hardware_the_software_keys_are_used_throughout() {
        let store = Arc::new(MemoryStore::default());
        let keys = FallbackKeys::new(Fake::Absent, SoftwareKeys::new(store.clone()));
        assert_eq!(keys.public_keys().unwrap_err(), KeyError::NoKeys);
        let public = keys.generate().unwrap();
        let software = SoftwareKeys::new(store);
        assert_eq!(software.public_keys().unwrap(), public);
        assert_eq!(
            keys.session_identity().unwrap(),
            software.session_identity().unwrap()
        );
        assert_eq!(keys.sign(b"m").unwrap().ecdsa.len(), ECDSA_SIG_LEN);
        keys.destroy().unwrap();
        assert_eq!(software.public_keys().unwrap_err(), KeyError::NoKeys);
    }

    #[test]
    fn with_hardware_the_software_keys_are_never_made_and_old_ones_are_cleared() {
        let hw_store = Arc::new(MemoryStore::default());
        let sw_store = Arc::new(MemoryStore::default());
        let stale = SoftwareKeys::new(sw_store.clone());
        stale.generate().unwrap();
        let keys = FallbackKeys::new(
            Fake::Working(SoftwareKeys::new(hw_store.clone())),
            SoftwareKeys::new(sw_store.clone()),
        );
        let public = keys.generate().unwrap();
        assert_eq!(SoftwareKeys::new(hw_store).public_keys().unwrap(), public);
        assert_eq!(
            SoftwareKeys::new(sw_store.clone())
                .public_keys()
                .unwrap_err(),
            KeyError::NoKeys,
            "the earlier software pairing is gone"
        );
        assert_eq!(keys.public_keys().unwrap(), public);
        keys.destroy().unwrap();
        assert_eq!(keys.public_keys().unwrap_err(), KeyError::NoKeys);
    }

    #[test]
    fn a_hardware_failure_other_than_unavailable_is_not_papered_over() {
        struct Broken;
        impl DeviceKeys for Broken {
            fn generate(&self) -> Result<PublicKeys, KeyError> {
                Err(KeyError::Crypto("keystore threw".into()))
            }
            fn public_keys(&self) -> Result<PublicKeys, KeyError> {
                Err(KeyError::Crypto("keystore threw".into()))
            }
            fn sign(&self, _: &[u8]) -> Result<Signatures, KeyError> {
                Err(KeyError::Crypto("cancelled".into()))
            }
            fn destroy(&self) -> Result<(), KeyError> {
                Ok(())
            }
            fn session_identity(&self) -> Result<SessionIdentity, KeyError> {
                Err(KeyError::Crypto("keystore threw".into()))
            }
        }
        let keys = FallbackKeys::new(Broken, software());
        assert!(matches!(keys.generate(), Err(KeyError::Crypto(_))));
        assert!(matches!(keys.sign(b"m"), Err(KeyError::Crypto(_))));
    }

    #[test]
    fn before_generate_there_are_no_keys() {
        let keys = software();
        assert_eq!(keys.public_keys().unwrap_err(), KeyError::NoKeys);
        assert_eq!(keys.session_identity().unwrap_err(), KeyError::NoKeys);
        assert_eq!(keys.sign(b"x").unwrap_err(), KeyError::NoKeys);
        assert!(keys.destroy().is_ok(), "destroy is idempotent");
    }

    #[test]
    fn generate_produces_the_wire_encodings() {
        let keys = software();
        let public = keys.generate().unwrap();
        assert_eq!(public.ecdsa_p256.len(), ECDSA_P256_LEN);
        assert_eq!(public.ecdsa_p256[0], 0x04, "SEC1 uncompressed");
        assert_eq!(public.mldsa_65.as_ref().map(Vec::len), Some(MLDSA_65_LEN));
        assert_eq!(keys.public_keys().unwrap(), public);

        let id = keys.session_identity().unwrap();
        assert_eq!(id.fingerprint().len(), 64);
        assert!(id.fingerprint().chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!format!("{id:?}").contains("key_pkcs8"));
        // rustls accepts the pair as a client identity.
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(id.key_pkcs8.clone().into());
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        provider.key_provider.load_private_key(key).unwrap();
    }

    /// The signatures verify the way the desktop's `stepup.rs` verifies
    /// them: P-256 from the SEC1 key over the SHA256 prehash, raw r||s;
    /// ML-DSA-65 pure with the empty context.
    #[test]
    fn signatures_verify_against_the_public_keys_as_the_desktop_checks_them() {
        let keys = software();
        let public = keys.generate().unwrap();
        let msg = b"{\"args\":{},\"command\":\"remove_orphan\",\"nonce\":\"n\",\"timestamp\":1}";
        let sigs = keys.sign(msg).unwrap();
        assert_eq!(sigs.ecdsa.len(), ECDSA_SIG_LEN);
        assert_eq!(sigs.mldsa.as_ref().map(Vec::len), Some(MLDSA_SIG_LEN));

        use p256::ecdsa::signature::Verifier;
        let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&public.ecdsa_p256).unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&sigs.ecdsa).unwrap();
        vk.verify(msg, &sig).unwrap();
        assert!(vk.verify(b"other", &sig).is_err());

        use ml_dsa::{EncodedVerifyingKey, MlDsa65, Signature, VerifyingKey};
        let enc =
            EncodedVerifyingKey::<MlDsa65>::try_from(public.mldsa_65.unwrap().as_slice()).unwrap();
        let vk = VerifyingKey::<MlDsa65>::decode(&enc);
        let sig = Signature::<MlDsa65>::try_from(sigs.mldsa.unwrap().as_slice()).unwrap();
        assert!(vk.verify_with_context(msg, b"", &sig));
        assert!(!vk.verify_with_context(b"other", b"", &sig));
    }

    #[test]
    fn generate_replaces_and_destroy_forgets() {
        let keys = software();
        let first = keys.generate().unwrap();
        let first_fp = keys.session_identity().unwrap().fingerprint();
        let second = keys.generate().unwrap();
        assert_ne!(first, second);
        assert_ne!(first_fp, keys.session_identity().unwrap().fingerprint());
        keys.destroy().unwrap();
        assert_eq!(keys.public_keys().unwrap_err(), KeyError::NoKeys);
        assert_eq!(keys.session_identity().unwrap_err(), KeyError::NoKeys);
    }

    #[test]
    fn keys_survive_a_new_handle_on_the_same_store() {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
        let public = SoftwareKeys::new(store.clone()).generate().unwrap();
        let again = SoftwareKeys::new(store);
        assert_eq!(again.public_keys().unwrap(), public);
    }

    #[test]
    fn validity_runs_from_yesterday_for_ten_years_and_clamps_leap_days() {
        let d = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(validity_window(d(2026, 9, 5)), ((2026, 9, 4), (2036, 9, 5)));
        assert_eq!(
            validity_window(d(2028, 2, 29)),
            ((2028, 2, 28), (2038, 2, 28))
        );
        assert_eq!(
            validity_window(d(2027, 1, 1)),
            ((2026, 12, 31), (2037, 1, 1))
        );
    }

    #[test]
    fn fingerprint_is_lowercase_hex_sha256() {
        assert_eq!(
            fingerprint_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
