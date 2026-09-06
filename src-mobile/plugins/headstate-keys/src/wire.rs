//! What crosses the bridge, and the checks on the way back.
//!
//! The Swift and Kotlin sides return JSON with camelCase keys and every
//! byte string as standard base64 with padding. This module owns that
//! shape so the two native files and the fake backend agree by
//! construction, and it refuses anything of the wrong length before it
//! can reach the desktop: a 70-byte "ECDSA signature" is a plugin bug
//! and should fail here, loudly, not as a 403 from the other end.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::{
    session_from_seed, Error, PublicKeys, Result, SessionIdentity, SessionSeed, Signatures,
    SESSION_SEED_LEN,
};

/// ECDSA P-256 public key: SEC1 uncompressed, `0x04 || x || y`.
pub const ECDSA_P256_LEN: usize = 65;
/// ML-DSA-65 public key, FIPS 204 table 2.
pub const MLDSA_65_LEN: usize = 1952;
/// ECDSA signature: raw `r || s` (IEEE P1363), 32 bytes each.
pub const ECDSA_SIG_LEN: usize = 64;
/// ML-DSA-65 signature, FIPS 204 table 2.
pub const MLDSA_SIG_LEN: usize = 3309;

/// Native command names. Swift dispatches on the Objective-C selector
/// `<name>:` and Kotlin on the `@Command` method name, so these are
/// method names, not snake_case.
pub mod cmd {
    pub const GENERATE: &str = "generate";
    pub const PUBLIC_KEYS: &str = "publicKeys";
    pub const SIGN: &str = "sign";
    pub const DESTROY: &str = "destroy";
    pub const STORE_SESSION: &str = "storeSession";
    pub const LOAD_SESSION: &str = "loadSession";
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WirePublicKeys {
    pub ecdsa_p256: String,
    /// Absent or `null` when the keystore holds no ML-DSA key.
    #[serde(default)]
    pub mldsa_65: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSignatures {
    pub ecdsa: String,
    #[serde(default)]
    pub mldsa: Option<String>,
}

/// Both directions: `storeSession` sends it, `loadSession` returns it.
/// The certificate DER and the 32-byte seed of the ML-DSA-65 session
/// key -- never a PKCS#8; the native sides keep both as opaque bytes.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSession {
    pub cert_der: String,
    pub key_seed: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignArgs {
    pub message: String,
    /// What the biometric prompt says. Passed from Rust so the wording
    /// lives in one place for both platforms.
    pub reason: String,
}

pub fn encode(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

fn decode(what: &str, b64: &str) -> Result<Vec<u8>> {
    BASE64
        .decode(b64)
        .map_err(|e| Error::Malformed(format!("{what} is not base64: {e}")))
}

fn decode_exact(what: &str, b64: &str, len: usize) -> Result<Vec<u8>> {
    let bytes = decode(what, b64)?;
    if bytes.len() != len {
        return Err(Error::Malformed(format!(
            "{what} is {} bytes; expected {len}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

impl WirePublicKeys {
    pub fn from_public(keys: &PublicKeys) -> Self {
        Self {
            ecdsa_p256: encode(&keys.ecdsa_p256),
            mldsa_65: keys.mldsa_65.as_deref().map(encode),
        }
    }

    pub fn into_public(self) -> Result<PublicKeys> {
        let ecdsa_p256 = decode_exact("ECDSA public key", &self.ecdsa_p256, ECDSA_P256_LEN)?;
        if ecdsa_p256[0] != 0x04 {
            return Err(Error::Malformed(
                "ECDSA public key is not an uncompressed SEC1 point".into(),
            ));
        }
        let mldsa_65 = self
            .mldsa_65
            .as_deref()
            .map(|b64| decode_exact("ML-DSA public key", b64, MLDSA_65_LEN))
            .transpose()?;
        Ok(PublicKeys {
            ecdsa_p256,
            mldsa_65,
        })
    }
}

impl WireSignatures {
    pub fn from_signatures(sigs: &Signatures) -> Self {
        Self {
            ecdsa: encode(&sigs.ecdsa),
            mldsa: sigs.mldsa.as_deref().map(encode),
        }
    }

    pub fn into_signatures(self) -> Result<Signatures> {
        Ok(Signatures {
            ecdsa: decode_exact("ECDSA signature", &self.ecdsa, ECDSA_SIG_LEN)?,
            mldsa: self
                .mldsa
                .as_deref()
                .map(|b64| decode_exact("ML-DSA signature", b64, MLDSA_SIG_LEN))
                .transpose()?,
        })
    }
}

/// The session seed out of its base64, exactly [`SESSION_SEED_LEN`]
/// bytes.
pub fn decode_seed(b64: &str) -> Result<SessionSeed> {
    let bytes = decode_exact("session seed", b64, SESSION_SEED_LEN)?;
    let mut seed = [0u8; SESSION_SEED_LEN];
    seed.copy_from_slice(&bytes);
    Ok(seed)
}

impl WireSession {
    pub fn from_parts(cert_der: &[u8], seed: &SessionSeed) -> Self {
        Self {
            cert_der: encode(cert_der),
            key_seed: encode(seed),
        }
    }

    pub fn into_identity(self) -> Result<SessionIdentity> {
        let cert_der = decode("session certificate", &self.cert_der)?;
        // DER SEQUENCE, the outermost tag of a Certificate. Cheap, and
        // it catches a swapped or empty field.
        if cert_der.first() != Some(&0x30) {
            return Err(Error::Malformed("session certificate is not DER".into()));
        }
        let seed = decode_seed(&self.key_seed)?;
        session_from_seed(&seed, cert_der)
    }
}
