//! The step-up signature protocol: the bytes a phone signs before a
//! destructive command, the header that carries the signatures, and the
//! verification the desktop performs.
//!
//! # Why this is a crate and not two modules
//!
//! The phone signs; the desktop verifies. That signature is the whole
//! guard on destructive commands. `src-tauri` and `src-mobile` are
//! separate crates with separate lockfiles, deliberately -- see
//! `src-mobile/Cargo.toml` for what must never reach the phone -- so
//! until this crate existed the two ends could not link, and the mobile
//! suite verified its own signatures with a hand-written copy of the
//! desktop's checks. Its doc comment said so: "the desktop's checks,
//! replicated". Both suites could be green while each agreed only with
//! its own copy of the rules, and the first thing to notice a divergence
//! would be a real phone whose destructive commands were refused (#695).
//!
//! A fixture would have made that drift detectable. One crate makes it
//! impossible: there is one implementation, and the mobile tests call
//! [`verify`] itself.
//!
//! What lives here is the contract -- anything whose bytes both ends
//! must agree on. What does NOT live here is anything only one end has:
//! the desktop's replay window and its `PairedDevice` row, the phone's
//! keystore. [`NonceLog`] is the seam: this crate decides WHEN a nonce
//! is spent, the desktop decides WHERE that is remembered.
//!
//! # The header
//!
//! ```text
//! X-Headstate-Signature: v1;ts=<unix>;nonce=<b64url>;ecdsa=<b64url>[;mldsa=<b64url>]
//! ```
//!
//! - Fields are separated by `;` with no whitespace anywhere. The first
//!   field is the literal version `v1`; every other field is `key=value`.
//!   Each key appears exactly once, in any order; an unknown key is a
//!   malformed header rather than something to skip, so a future `v2`
//!   cannot be half-understood by a `v1` desktop.
//! - `ts`: the phone's clock as whole seconds since the Unix epoch, in
//!   decimal with an optional leading `-`. Refused when it differs from
//!   the desktop's clock by more than [`MAX_SKEW_SECS`] in either
//!   direction.
//! - `nonce`: 16 random bytes. Refused if the same device has already
//!   used it for a request whose timestamp is still inside the window.
//! - `ecdsa`: the ECDSA P-256 signature, as the raw 64-byte `r || s`
//!   (IEEE P1363), NOT DER. Both platforms can produce it directly:
//!   CryptoKit's `ECDSASignature.rawRepresentation`, Android's
//!   `SHA256withECDSAinP1363Format`. The message is hashed with SHA256
//!   as usual for ECDSA. Either `s` value is accepted; there is no low-S
//!   rule, because CryptoKit does not normalise.
//! - `mldsa`: the ML-DSA-65 signature, 3309 bytes, over the same bytes
//!   with the empty context string, i.e. plain FIPS 204 `ML-DSA.Sign`
//!   with `ctx = ""` and no pre-hash. Present exactly when the pairing
//!   record has an ML-DSA key: a pairing with one refuses a request
//!   without it, and a pairing without one refuses a request that has it,
//!   because a phone whose key set differs from its pairing record should
//!   re-pair rather than be half-trusted.
//! - Every `b64url` value is base64url without padding (RFC 4648 §5),
//!   the same alphabet the pairing token uses.
//!
//! # The signed bytes
//!
//! Both signatures are over [`canonical_bytes`]: the JSON object
//! `{command, args, nonce, timestamp}` in canonical form. Canonical means:
//!
//! - object keys sorted by their UTF-8 bytes, at every level, including
//!   inside `args`;
//! - no whitespace;
//! - strings escaped the way `serde_json` does: only `"` and `\` and the
//!   control characters U+0000..U+001F are escaped, as `\"`, `\\`, `\b`,
//!   `\f`, `\n`, `\r`, `\t`, or otherwise `\u00XX` with lowercase hex.
//!   Everything else, including non-ASCII, is emitted as raw UTF-8;
//! - `timestamp` is a JSON integer, `nonce` the exact base64url string
//!   from the header, `command` the path segment, `args` the request body
//!   as the phone sent it. Integers print without fraction or exponent.
//!   No destructive command takes a floating-point argument, and a phone
//!   must not send one: float formatting is not pinned here.
//!
//! The desktop canonicalises the PARSED body, so the body on the wire may
//! use any whitespace and key order; only the values must match what the
//! phone signed. Pinned by `tests::canonical_bytes_test_vector`:
//!
//! ```text
//! command   remove_worktree
//! args      {"worktreePath":"/home/octocat/src/hello-world/.worktrees/feature","repoPath":"/home/octocat/src/hello-world"}
//! nonce     AAECAwQFBgcICQoLDA0ODw   (the bytes 00 01 02 .. 0f)
//! timestamp 1788566400
//!
//! {"args":{"repoPath":"/home/octocat/src/hello-world","worktreePath":"/home/octocat/src/hello-world/.worktrees/feature"},"command":"remove_worktree","nonce":"AAECAwQFBgcICQoLDA0ODw","timestamp":1788566400}
//!
//! 203 bytes; SHA256 ebd1a4f4f78ff1f55f7bf642cc8d72262b6a77ab14164bbf4f95135a6e0f79ff
//! ```
//!
//! # Order of checks
//!
//! Header present, header well-formed, timestamp in window, signature
//! set matches the pairing record, ECDSA verifies, ML-DSA verifies, and
//! only then is the nonce recorded. Recording last means a request that
//! failed to verify never burns a nonce, so a phone with a clock problem
//! can retry the same signed request after fixing it; a request that
//! verified is remembered for the rest of its timestamp window, which is
//! exactly as long as a replay of it could pass the timestamp check.
//!
//! # Mounting
//!
//! ```ignore
//! // In the /v1/call/{command} handler, after the client certificate has
//! // been mapped to its PairedDevice row and before dispatch:
//! if class == Class::Destructive {
//!     let header = headers.get(HEADER).and_then(|v| v.to_str().ok());
//!     let keys = PairedKeys { ecdsa: &device.ecdsa_pubkey, mldsa: device.mldsa_pubkey.as_deref() };
//!     stepup::verify(&keys, &command, &args, header, Utc::now().timestamp(), &nonces.for_device(&device.cert_fp))
//!         .map_err(|e| (e.http_status(), e.to_string()))?;
//! }
//! ```
//!
//! # Why RustCrypto `ml-dsa` and not `libcrux-ml-dsa`
//!
//! Both were evaluated against this tree on 2026-09-05, both claim final
//! FIPS 204, and a scratch cross-check confirmed it: a signature made by
//! either verifies under the other, and both derive the same public key
//! from the same seed. The differences:
//!
//! - Build: `ml-dsa` has no build script and every dependency is
//!   RustCrypto. `libcrux-ml-dsa` 0.0.10 carries three `build.rs` files
//!   (they only emit SIMD cfgs, but they run on every platform, and on
//!   x86_64 they switch on AVX2 code paths with runtime detection), and
//!   its `libcrux-secrets` dependency lists `crabgrind` -> `bindgen` ->
//!   `clang-sys` under a `cfg(valgrind_ct_test)` target. That is never
//!   compiled, but it lands in `Cargo.lock` and in any full-graph
//!   supply-chain scan.
//! - Lock growth beyond what `p256` already adds: 15 entries for
//!   `ml-dsa` (including a second `digest` stack at 0.11) against 29 for
//!   `libcrux-ml-dsa` (`hax-lib` proc macros, `tls_codec`, `bindgen` and
//!   its parser stack, `libloading`).
//! - Audit: `libcrux`'s core is formally verified; `ml-dsa` is not
//!   audited. This crate only VERIFIES. The properties formal
//!   verification buys most -- no secret-dependent timing, no key
//!   leakage -- protect a signer, and the desktop never holds an ML-DSA
//!   private key. What a verifier needs is to implement the standard
//!   exactly, which the cross-check and `ml-dsa`'s Wycheproof tests
//!   (added in 0.1.0) cover.
//! - API: `ml-dsa` is on a 0.1 line with the `signature` traits shared
//!   by `p256`; `libcrux-ml-dsa` is 0.0.x.
//!
//! The verify-only role tips it: the audit gap is at the signer, and the
//! signer is the phone's secure hardware.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;

/// The request header carrying the step-up signature. HTTP header names
/// are case-insensitive; this is the canonical spelling.
pub const HEADER: &str = "X-Headstate-Signature";

/// How far the phone's timestamp may sit from the desktop's clock, in
/// either direction. "Sixty seconds" in the spec.
pub const MAX_SKEW_SECS: i64 = 60;

/// Length of the nonce in bytes.
pub const NONCE_LEN: usize = 16;

/// Raw `r || s`, 32 bytes each.
pub const ECDSA_SIG_LEN: usize = 64;

/// FIPS 204 table 2, ML-DSA-65.
pub const MLDSA_SIG_LEN: usize = 3309;

/// Why a destructive request was refused. [`StepUpError::http_status`]
/// is the listener's mapping; every message is safe to send back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StepUpError {
    /// No `X-Headstate-Signature` header at all.
    #[error("destructive commands require an X-Headstate-Signature header")]
    Missing,
    /// The header was present but not in the documented grammar.
    #[error("malformed X-Headstate-Signature: {0}")]
    Malformed(String),
    /// `ts` is more than [`MAX_SKEW_SECS`] from the desktop's clock.
    /// `skew` is phone minus desktop, so a positive value means the
    /// phone's clock is ahead.
    #[error(
        "signature timestamp is {skew}s from the desktop clock; the limit is {MAX_SKEW_SECS}s"
    )]
    StaleTimestamp { skew: i64 },
    /// This device already used this nonce inside the window.
    #[error("signature nonce was already used")]
    NonceReused,
    /// The pairing recorded an ML-DSA key and the header has no `mldsa`.
    #[error("this pairing requires an ML-DSA-65 signature and none was sent")]
    MissingMldsa,
    /// The header has `mldsa` but the pairing recorded no ML-DSA key.
    #[error("this pairing has no ML-DSA-65 key; re-pair to add one")]
    UnexpectedMldsa,
    /// The ECDSA signature did not verify against the paired key.
    #[error("ECDSA signature did not verify")]
    BadEcdsa,
    /// The ML-DSA signature did not verify against the paired key.
    #[error("ML-DSA-65 signature did not verify")]
    BadMldsa,
    /// The key stored at pairing does not decode. A desktop-side fault,
    /// not the phone's: the pairing flow validated the length, so this
    /// means the row was damaged after the fact.
    #[error("the paired {0} key on this desktop is unreadable; re-pair")]
    BadStoredKey(&'static str),
}

impl StepUpError {
    /// 400 for a header the phone built wrong, 500 for a key the desktop
    /// cannot read, 403 for everything else. The same collapse pairing
    /// uses: the message says what failed, the status does not.
    pub fn http_status(&self) -> u16 {
        match self {
            StepUpError::Malformed(_) => 400,
            StepUpError::BadStoredKey(_) => 500,
            _ => 403,
        }
    }
}

/// The step-up public keys a pairing recorded, borrowed from wherever
/// the caller keeps them.
///
/// The desktop's are columns on a `PairedDevice` row; the phone's tests
/// take them straight from the keystore's `generate`. Neither of those
/// types belongs here -- one is a rusqlite row, the other a plugin type
/// -- so the verifier takes the two byte strings and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedKeys<'a> {
    /// P-256 step-up key, SEC1 uncompressed: 65 bytes starting `0x04`.
    pub ecdsa: &'a [u8],
    /// ML-DSA-65 step-up key, 1952 bytes; `None` when the phone had none.
    pub mldsa: Option<&'a [u8]>,
}

/// Where a verified nonce is remembered, so it cannot be replayed.
///
/// The replay window itself is desktop state: it is per-listener, keyed
/// by device fingerprint, and lives as long as the process. This crate
/// owns the RULE -- record last, only after every signature has
/// verified, so a failed request never burns a nonce -- and the desktop
/// owns the storage. A phone verifying its own signature in a test has
/// no replay window at all, which is what [`NoReplayCheck`] is for.
pub trait NonceLog {
    /// Record `nonce` as spent at `timestamp`. `false` if it was already
    /// spent and that use can still be replayed; `true` if it is new and
    /// is now remembered.
    ///
    /// Implementations must make the check and the insert atomic, or two
    /// concurrent replays can both be admitted.
    fn admit(&self, nonce: [u8; NONCE_LEN], timestamp: i64) -> bool;
}

/// A [`NonceLog`] that admits everything.
///
/// For callers with no replay state to protect: the phone verifying a
/// header it just built, and any test whose subject is the signature
/// rather than the window. Named rather than a bare closure so a call
/// site that skips replay protection says so out loud.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoReplayCheck;

impl NonceLog for NoReplayCheck {
    fn admit(&self, _nonce: [u8; NONCE_LEN], _timestamp: i64) -> bool {
        true
    }
}

/// The parsed `X-Headstate-Signature` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHeader {
    /// `ts`, seconds since the epoch by the phone's clock.
    pub timestamp: i64,
    /// `nonce` exactly as sent, so the canonical bytes can quote it.
    pub nonce: String,
    /// The decoded nonce, what the replay window remembers.
    pub nonce_bytes: [u8; NONCE_LEN],
    /// Raw `r || s`.
    pub ecdsa: Vec<u8>,
    /// The ML-DSA-65 signature, when the phone sent one.
    pub mldsa: Option<Vec<u8>>,
}

impl SignatureHeader {
    /// Parse the header value. Strict: the grammar in the module docs and
    /// nothing else.
    pub fn parse(header: &str) -> Result<Self, StepUpError> {
        let malformed = |why: &str| StepUpError::Malformed(why.to_string());
        let mut fields = header.split(';');
        if fields.next() != Some("v1") {
            return Err(malformed("expected version `v1` first"));
        }
        let mut ts = None;
        let mut nonce = None;
        let mut ecdsa = None;
        let mut mldsa = None;
        for field in fields {
            let Some((key, value)) = field.split_once('=') else {
                return Err(malformed(&format!("field {field:?} is not key=value")));
            };
            let slot = match key {
                "ts" => &mut ts,
                "nonce" => &mut nonce,
                "ecdsa" => &mut ecdsa,
                "mldsa" => &mut mldsa,
                _ => return Err(malformed(&format!("unknown field {key:?}"))),
            };
            if slot.replace(value).is_some() {
                return Err(malformed(&format!("field {key:?} given twice")));
            }
        }

        let timestamp = ts
            .ok_or_else(|| malformed("missing ts"))?
            .parse::<i64>()
            .map_err(|_| malformed("ts is not a whole number of seconds"))?;
        let nonce = nonce.ok_or_else(|| malformed("missing nonce"))?;
        let nonce_bytes: [u8; NONCE_LEN] = decode_exact(nonce, NONCE_LEN)
            .ok_or_else(|| malformed(&format!("nonce is not {NONCE_LEN} bytes of base64url")))?
            .try_into()
            .expect("decode_exact checked the length");
        let ecdsa = decode_exact(
            ecdsa.ok_or_else(|| malformed("missing ecdsa"))?,
            ECDSA_SIG_LEN,
        )
        .ok_or_else(|| malformed(&format!("ecdsa is not {ECDSA_SIG_LEN} bytes of base64url")))?;
        let mldsa = match mldsa {
            None => None,
            Some(v) => Some(decode_exact(v, MLDSA_SIG_LEN).ok_or_else(|| {
                malformed(&format!("mldsa is not {MLDSA_SIG_LEN} bytes of base64url"))
            })?),
        };
        Ok(Self {
            timestamp,
            nonce: nonce.to_string(),
            nonce_bytes,
            ecdsa,
            mldsa,
        })
    }
}

/// The header value from its parts: the phone's side of the grammar
/// [`SignatureHeader::parse`] reads.
///
/// It lives beside the parser rather than in the phone's crate because
/// that is the point of this crate -- a change to the grammar cannot
/// touch one direction and miss the other.
pub fn build_header(timestamp: i64, nonce: &str, ecdsa: &[u8], mldsa: Option<&[u8]>) -> String {
    let mut header = format!(
        "v1;ts={timestamp};nonce={nonce};ecdsa={}",
        URL_SAFE_NO_PAD.encode(ecdsa)
    );
    if let Some(mldsa) = mldsa {
        header.push_str(";mldsa=");
        header.push_str(&URL_SAFE_NO_PAD.encode(mldsa));
    }
    header
}

/// base64url without padding, decoding to exactly `len` bytes. The
/// engine refuses padding and non-canonical trailing bits, so every
/// value has one spelling and the replay set cannot be dodged by
/// re-encoding.
fn decode_exact(value: &str, len: usize) -> Option<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    (bytes.len() == len).then_some(bytes)
}

/// The bytes both signatures cover. See "The signed bytes" in the
/// crate docs; `tests::canonical_bytes_test_vector` pins the output.
pub fn canonical_bytes(command: &str, args: &Value, nonce: &str, timestamp: i64) -> Vec<u8> {
    let mut out = Vec::new();
    write_canonical(
        &serde_json::json!({
            "command": command,
            "args": args,
            "nonce": nonce,
            "timestamp": timestamp,
        }),
        &mut out,
    );
    out
}

/// Serialise `value` with object keys in byte order at every level and
/// no whitespace. Scalars go through `serde_json` so the escaping is the
/// crate's, which is what the crate docs promise; the walk exists
/// because `serde_json::Map` keeps insertion order when the
/// `preserve_order` feature is on anywhere in the build, and a signature
/// must not depend on which features a dependency happened to enable.
fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_scalar(&Value::String(key.clone()), out);
                out.push(b':');
                write_canonical(&map[key], out);
            }
            out.push(b'}');
        }
        scalar => write_scalar(scalar, out),
    }
}

fn write_scalar(value: &Value, out: &mut Vec<u8>) {
    serde_json::to_writer(&mut *out, value).expect("writing JSON scalars to a Vec cannot fail");
}

/// Verify the step-up on a destructive request.
///
/// `header` is the raw `X-Headstate-Signature` value, or `None` when the
/// request had no such header. `args` is the parsed JSON body exactly as
/// it will be handed to `surface::dispatch`. `now` is the desktop's
/// clock in Unix seconds, passed in rather than read so the window is
/// testable. On `Ok(())` the nonce has been recorded in `nonces`; on any
/// `Err` nothing was recorded.
pub fn verify(
    keys: &PairedKeys<'_>,
    command: &str,
    args: &Value,
    header: Option<&str>,
    now: i64,
    nonces: &dyn NonceLog,
) -> Result<(), StepUpError> {
    let header = SignatureHeader::parse(header.ok_or(StepUpError::Missing)?)?;

    let skew = header.timestamp - now;
    if skew.abs() > MAX_SKEW_SECS {
        return Err(StepUpError::StaleTimestamp { skew });
    }

    // The signature set must be exactly what the pairing record expects,
    // decided before any verification so the refusal names the mismatch
    // rather than a signature that was never going to be checked.
    match (keys.mldsa, &header.mldsa) {
        (Some(_), None) => return Err(StepUpError::MissingMldsa),
        (None, Some(_)) => return Err(StepUpError::UnexpectedMldsa),
        _ => {}
    }

    let msg = canonical_bytes(command, args, &header.nonce, header.timestamp);
    verify_ecdsa(keys.ecdsa, &msg, &header.ecdsa)?;
    if let (Some(key), Some(sig)) = (keys.mldsa, &header.mldsa) {
        verify_mldsa(key, &msg, sig)?;
    }

    if !nonces.admit(header.nonce_bytes, header.timestamp) {
        return Err(StepUpError::NonceReused);
    }
    Ok(())
}

/// ECDSA P-256 over SHA256 of `msg`, signature as raw `r || s`.
fn verify_ecdsa(key: &[u8], msg: &[u8], sig: &[u8]) -> Result<(), StepUpError> {
    use p256::ecdsa::signature::Verifier;
    let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(key)
        .map_err(|_| StepUpError::BadStoredKey("ECDSA P-256"))?;
    let sig = p256::ecdsa::Signature::from_slice(sig).map_err(|_| StepUpError::BadEcdsa)?;
    key.verify(msg, &sig).map_err(|_| StepUpError::BadEcdsa)
}

/// ML-DSA-65 over `msg` itself with the empty context string.
fn verify_mldsa(key: &[u8], msg: &[u8], sig: &[u8]) -> Result<(), StepUpError> {
    use ml_dsa::{EncodedVerifyingKey, MlDsa65, Signature, VerifyingKey};
    let encoded = EncodedVerifyingKey::<MlDsa65>::try_from(key)
        .map_err(|_| StepUpError::BadStoredKey("ML-DSA-65"))?;
    let key = VerifyingKey::<MlDsa65>::decode(&encoded);
    let sig = Signature::<MlDsa65>::try_from(sig).map_err(|_| StepUpError::BadMldsa)?;
    if key.verify_with_context(msg, b"", &sig) {
        Ok(())
    } else {
        Err(StepUpError::BadMldsa)
    }
}

#[cfg(test)]
mod guard {
    /// This crate is linked into the PHONE, so its dependency list is
    /// the phone's too. `src-mobile`'s `no_desktop_only_crates_in_lock`
    /// reads that crate's lock and would catch a desktop-only crate
    /// arriving through here -- but it would catch it in the wrong
    /// place, blaming the companion for something added three
    /// directories away. This says it where it happens.
    ///
    /// The manifest, not the lock: what matters is what this crate
    /// ASKS for. A transitive arrival is the mobile guard's business.
    #[test]
    fn this_crate_stays_a_leaf() {
        let manifest = include_str!("../Cargo.toml");
        let deps = manifest
            .split("[dependencies]")
            .nth(1)
            .expect("a [dependencies] section");
        for forbidden in [
            "tauri", "octocrab", "rusqlite", "reqwest", "hyper", "axum", "rcgen", "rustls",
            "keyring", "tokio",
        ] {
            assert!(
                !deps.contains(&format!("\n{forbidden} ")),
                "{forbidden} is a dependency of headstate-stepup; this crate is linked into the \
                 phone and must stay a leaf over serde_json, base64 and the two signature crates"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_dsa::{MlDsa65, Seed};
    use p256::ecdsa::signature::Signer;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;

    /// Lowercase hex. `sha2` 0.11 returns a `hybrid-array` `Array`, which
    /// -- unlike the 0.10 `GenericArray` -- has no `LowerHex`, so `{:x}`
    /// no longer formats a digest.
    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    const NOW: i64 = 1_788_566_400;
    const NONCE_B64: &str = "AAECAwQFBgcICQoLDA0ODw";
    const CMD: &str = "remove_worktree";

    /// The smallest thing that can spend a nonce: enough to prove the
    /// verifier records last and only on success, without the desktop's
    /// per-device, time-pruned window.
    #[derive(Default)]
    struct OneDeviceLog(RefCell<Vec<[u8; NONCE_LEN]>>);

    impl NonceLog for OneDeviceLog {
        fn admit(&self, nonce: [u8; NONCE_LEN], _timestamp: i64) -> bool {
            let mut seen = self.0.borrow_mut();
            if seen.contains(&nonce) {
                return false;
            }
            seen.push(nonce);
            true
        }
    }

    /// A phone's step-up keys, made deterministically so no test needs
    /// an RNG.
    struct Phone {
        ecdsa: p256::ecdsa::SigningKey,
        mldsa: Option<ml_dsa::SigningKey<MlDsa65>>,
    }

    impl Phone {
        fn new(with_mldsa: bool, seed: u8) -> Self {
            let ecdsa = p256::ecdsa::SigningKey::from_bytes(&[seed; 32].into()).unwrap();
            let mldsa = with_mldsa
                .then(|| ml_dsa::SigningKey::<MlDsa65>::from_seed(&Seed::from([seed; 32])));
            Self { ecdsa, mldsa }
        }

        fn ecdsa_pubkey(&self) -> Vec<u8> {
            // `to_sec1_point` is `to_encoded_point` renamed in
            // elliptic-curve 0.14; `false` is still "uncompressed", so
            // this is the same 65 bytes starting 0x04.
            self.ecdsa
                .verifying_key()
                .to_sec1_point(false)
                .as_bytes()
                .to_vec()
        }

        fn mldsa_pubkey(&self) -> Option<Vec<u8>> {
            self.mldsa
                .as_ref()
                .map(|k| k.expanded_key().verifying_key().encode().to_vec())
        }

        fn ecdsa_sig(&self, msg: &[u8]) -> Vec<u8> {
            let sig: p256::ecdsa::Signature = self.ecdsa.sign(msg);
            sig.to_bytes().to_vec()
        }

        fn mldsa_sig(&self, msg: &[u8]) -> Vec<u8> {
            self.mldsa
                .as_ref()
                .expect("phone has ML-DSA")
                .expanded_key()
                .sign_deterministic(msg, b"")
                .unwrap()
                .encode()
                .to_vec()
        }

        /// A well-formed header for `command`/`args`, with every
        /// signature this phone can produce. Built through
        /// [`build_header`], the same function the companion calls.
        fn header(&self, command: &str, args: &Value, nonce: &str, ts: i64) -> String {
            let msg = canonical_bytes(command, args, nonce, ts);
            build_header(
                ts,
                nonce,
                &self.ecdsa_sig(&msg),
                self.mldsa.as_ref().map(|_| self.mldsa_sig(&msg)).as_deref(),
            )
        }
    }

    /// Borrowed keys for a phone, held in locals so the borrows live.
    macro_rules! paired {
        ($phone:expr) => {{
            let ecdsa = $phone.ecdsa_pubkey();
            let mldsa = $phone.mldsa_pubkey();
            (ecdsa, mldsa)
        }};
    }

    fn keys<'a>(ecdsa: &'a [u8], mldsa: &'a Option<Vec<u8>>) -> PairedKeys<'a> {
        PairedKeys {
            ecdsa,
            mldsa: mldsa.as_deref(),
        }
    }

    fn args() -> Value {
        json!({
            "worktreePath": "/home/octocat/src/hello-world/.worktrees/feature",
            "repoPath": "/home/octocat/src/hello-world",
        })
    }

    #[test]
    fn canonical_bytes_test_vector() {
        let bytes = canonical_bytes(CMD, &args(), NONCE_B64, NOW);
        let expected = concat!(
            r#"{"args":{"repoPath":"/home/octocat/src/hello-world","#,
            r#""worktreePath":"/home/octocat/src/hello-world/.worktrees/feature"},"#,
            r#""command":"remove_worktree","nonce":"AAECAwQFBgcICQoLDA0ODw","timestamp":1788566400}"#,
        );
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), expected);
        assert_eq!(bytes.len(), 203);
        assert_eq!(
            hex(&Sha256::digest(&bytes)),
            "ebd1a4f4f78ff1f55f7bf642cc8d72262b6a77ab14164bbf4f95135a6e0f79ff"
        );
    }

    #[test]
    fn canonical_bytes_sorts_nested_keys_and_escapes_like_serde() {
        let args = json!({
            "b": [1, true, null, {"y": "x", "x": "y"}],
            "a": {"z": "quote\" backslash\\ nl\n tab\t ctl\u{1}", "é": "ünïcode"},
        });
        let bytes = canonical_bytes("cmd", &args, "n", -5);
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            concat!(
                r#"{"args":{"a":{"z":"quote\" backslash\\ nl\n tab\t ctl\u0001","é":"ünïcode"},"#,
                r#""b":[1,true,null,{"x":"y","y":"x"}]},"command":"cmd","nonce":"n","timestamp":-5}"#,
            )
        );
    }

    #[test]
    fn header_parses_and_round_trips() {
        let phone = Phone::new(true, 1);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        let parsed = SignatureHeader::parse(&h).unwrap();
        assert_eq!(parsed.timestamp, NOW);
        assert_eq!(parsed.nonce, NONCE_B64);
        assert_eq!(parsed.nonce_bytes, core::array::from_fn(|i| i as u8));
        assert_eq!(parsed.ecdsa.len(), ECDSA_SIG_LEN);
        assert_eq!(parsed.mldsa.as_ref().map(Vec::len), Some(MLDSA_SIG_LEN));

        // Field order is free.
        let reordered = {
            let mut parts: Vec<&str> = h.split(';').collect();
            parts[1..].reverse();
            parts.join(";")
        };
        assert_eq!(SignatureHeader::parse(&reordered).unwrap(), parsed);
    }

    /// `build_header` and `SignatureHeader::parse` are the two halves of
    /// one grammar, and this is the test that could not exist before
    /// this crate did: the phone's builder feeding the desktop's parser
    /// in one process.
    #[test]
    fn build_header_omits_mldsa_when_the_device_has_none() {
        let h = build_header(7, NONCE_B64, &[1u8; ECDSA_SIG_LEN], None);
        assert_eq!(
            h,
            format!(
                "v1;ts=7;nonce={NONCE_B64};ecdsa={}",
                URL_SAFE_NO_PAD.encode([1u8; ECDSA_SIG_LEN])
            )
        );
        assert_eq!(SignatureHeader::parse(&h).unwrap().mldsa, None);

        let with = build_header(
            7,
            NONCE_B64,
            &[1u8; ECDSA_SIG_LEN],
            Some(&[2u8; MLDSA_SIG_LEN]),
        );
        assert_eq!(
            SignatureHeader::parse(&with).unwrap().mldsa,
            Some(vec![2u8; MLDSA_SIG_LEN])
        );
    }

    #[test]
    fn build_header_never_pads_and_never_spaces() {
        let phone = Phone::new(true, 1);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        assert!(h.starts_with(&format!("v1;ts={NOW};nonce=")));
        assert!(!h.contains(' '));
        for field in h.split(';').skip(1) {
            let (_, value) = field.split_once('=').unwrap();
            assert!(!value.contains('='), "no padding anywhere: {field}");
        }
    }

    #[test]
    fn header_rejects_every_deviation_from_the_grammar() {
        let phone = Phone::new(false, 1);
        let good = phone.header(CMD, &args(), NONCE_B64, NOW);
        let bad = [
            ("", "empty"),
            ("v2;ts=1;nonce=AAECAwQFBgcICQoLDA0ODw;ecdsa=AA", "version"),
            (&good.replace("v1;", "v1; "), "whitespace"),
            (&good.replace("ts=", "ts=x"), "ts not a number"),
            (
                &good.replace("nonce=AAECAwQFBgcICQoLDA0ODw", "nonce=AAEC"),
                "short nonce",
            ),
            (
                &good.replace(
                    "nonce=AAECAwQFBgcICQoLDA0ODw",
                    "nonce=AAECAwQFBgcICQoLDA0ODw==",
                ),
                "padded nonce",
            ),
            (
                &good.replace(
                    "nonce=AAECAwQFBgcICQoLDA0ODw",
                    "nonce=AAECAwQFBgcICQoLDA0OD+",
                ),
                "non-url alphabet",
            ),
            (&good.replace("ecdsa=", "ecdsa=AAAA"), "ecdsa wrong length"),
            (&format!("{good};mldsa=AAAA"), "mldsa wrong length"),
            (&format!("{good};ts=1"), "duplicate key"),
            (&format!("{good};extra=1"), "unknown key"),
            (&format!("{good};"), "trailing separator"),
            (&good.replace(";ecdsa=", ";"), "field without ="),
            (
                &good
                    .rsplit_once(";ecdsa=")
                    .map(|(a, _)| a.to_string())
                    .unwrap(),
                "missing ecdsa",
            ),
            (&good.replace(";nonce=", ";NONCE="), "key case"),
        ];
        for (h, why) in bad {
            assert!(
                matches!(SignatureHeader::parse(h), Err(StepUpError::Malformed(_))),
                "{why}: {h:?}"
            );
        }
        assert!(SignatureHeader::parse(&good).is_ok());
    }

    #[test]
    fn no_signature_refused() {
        let phone = Phone::new(false, 1);
        let (e, m) = paired!(phone);
        let err = verify(&keys(&e, &m), CMD, &args(), None, NOW, &NoReplayCheck).unwrap_err();
        assert_eq!(err, StepUpError::Missing);
        assert_eq!(err.http_status(), 403);
        let err = verify(&keys(&e, &m), CMD, &args(), Some(""), NOW, &NoReplayCheck).unwrap_err();
        assert!(matches!(err, StepUpError::Malformed(_)));
        assert_eq!(err.http_status(), 400);
    }

    #[test]
    fn stale_timestamp_refused_in_both_directions() {
        let phone = Phone::new(true, 1);
        let (e, m) = paired!(phone);
        let log = OneDeviceLog::default();
        for (ts, skew) in [(NOW - 61, -61), (NOW + 61, 61), (NOW - 3600, -3600)] {
            let h = phone.header(CMD, &args(), NONCE_B64, ts);
            let err = verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW, &log).unwrap_err();
            assert_eq!(err, StepUpError::StaleTimestamp { skew }, "ts={ts}");
            assert_eq!(err.http_status(), 403);
        }
        // Exactly sixty seconds either side is still inside.
        for ts in [NOW - 60, NOW + 60] {
            let nonce = URL_SAFE_NO_PAD.encode(ts.to_le_bytes().repeat(2));
            let h = phone.header(CMD, &args(), &nonce, ts);
            verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW, &log).unwrap();
        }
    }

    #[test]
    fn a_failed_request_burns_no_nonce_and_a_replay_is_refused() {
        let phone = Phone::new(true, 1);
        let (e, m) = paired!(phone);
        let log = OneDeviceLog::default();
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);

        // A request that fails verification leaves the nonce unspent.
        let tampered = json!({"repoPath": "/home/octocat/src/hello-world", "worktreePath": "/"});
        assert_eq!(
            verify(&keys(&e, &m), CMD, &tampered, Some(&h), NOW, &log).unwrap_err(),
            StepUpError::BadEcdsa
        );

        verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW, &log).unwrap();
        let err = verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW + 30, &log).unwrap_err();
        assert_eq!(err, StepUpError::NonceReused);
        assert_eq!(err.http_status(), 403);
    }

    #[test]
    fn valid_ecdsa_only_accepted_for_an_ecdsa_only_pairing() {
        let phone = Phone::new(false, 1);
        let (e, m) = paired!(phone);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW, &NoReplayCheck).unwrap();
    }

    #[test]
    fn valid_hybrid_accepted() {
        let phone = Phone::new(true, 1);
        let (e, m) = paired!(phone);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        assert!(h.contains(";mldsa="));
        verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW, &NoReplayCheck).unwrap();
    }

    #[test]
    fn ecdsa_only_refused_when_mldsa_was_paired() {
        let phone = Phone::new(true, 1);
        let (e, m) = paired!(phone);
        let full = phone.header(CMD, &args(), NONCE_B64, NOW);
        let ecdsa_only = full.split(";mldsa=").next().unwrap();
        let err = verify(
            &keys(&e, &m),
            CMD,
            &args(),
            Some(ecdsa_only),
            NOW,
            &NoReplayCheck,
        )
        .unwrap_err();
        assert_eq!(err, StepUpError::MissingMldsa);
        assert_eq!(err.http_status(), 403);
    }

    #[test]
    fn mldsa_refused_when_the_pairing_has_no_mldsa_key() {
        let phone = Phone::new(true, 1);
        let (e, _) = paired!(phone);
        let full = phone.header(CMD, &args(), NONCE_B64, NOW);
        let err = verify(
            &keys(&e, &None),
            CMD,
            &args(),
            Some(&full),
            NOW,
            &NoReplayCheck,
        )
        .unwrap_err();
        assert_eq!(err, StepUpError::UnexpectedMldsa);
    }

    #[test]
    fn mldsa_only_refused() {
        // The grammar requires ecdsa, so an ML-DSA-only header cannot
        // even be parsed; and a hybrid header whose ECDSA half is wrong
        // is refused on that half regardless of the ML-DSA half.
        let phone = Phone::new(true, 1);
        let (e, m) = paired!(phone);
        let full = phone.header(CMD, &args(), NONCE_B64, NOW);
        let (before, mldsa) = full.split_once(";ecdsa=").unwrap();
        let (_, mldsa) = mldsa.split_once(";mldsa=").unwrap();
        let mldsa_only = format!("{before};mldsa={mldsa}");
        assert!(matches!(
            verify(
                &keys(&e, &m),
                CMD,
                &args(),
                Some(&mldsa_only),
                NOW,
                &NoReplayCheck
            ),
            Err(StepUpError::Malformed(_))
        ));

        let other = Phone::new(true, 2);
        let msg = canonical_bytes(CMD, &args(), NONCE_B64, NOW);
        let wrong_ecdsa = format!(
            "{before};ecdsa={};mldsa={mldsa}",
            URL_SAFE_NO_PAD.encode(other.ecdsa_sig(&msg))
        );
        assert_eq!(
            verify(
                &keys(&e, &m),
                CMD,
                &args(),
                Some(&wrong_ecdsa),
                NOW,
                &NoReplayCheck
            )
            .unwrap_err(),
            StepUpError::BadEcdsa
        );
    }

    #[test]
    fn wrong_mldsa_refused_even_with_a_good_ecdsa() {
        let phone = Phone::new(true, 1);
        let other = Phone::new(true, 2);
        let (e, m) = paired!(phone);
        let msg = canonical_bytes(CMD, &args(), NONCE_B64, NOW);
        let h = build_header(
            NOW,
            NONCE_B64,
            &phone.ecdsa_sig(&msg),
            Some(&other.mldsa_sig(&msg)),
        );
        let err = verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW, &NoReplayCheck).unwrap_err();
        assert_eq!(err, StepUpError::BadMldsa);
        assert_eq!(err.http_status(), 403);
    }

    #[test]
    fn tampered_args_command_nonce_or_timestamp_refused() {
        let phone = Phone::new(true, 1);
        let (e, m) = paired!(phone);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);

        let mut tampered = args();
        tampered["worktreePath"] = json!("/home/octocat");
        assert_eq!(
            verify(&keys(&e, &m), CMD, &tampered, Some(&h), NOW, &NoReplayCheck).unwrap_err(),
            StepUpError::BadEcdsa
        );
        assert_eq!(
            verify(
                &keys(&e, &m),
                "remove_worktree_forced",
                &args(),
                Some(&h),
                NOW,
                &NoReplayCheck
            )
            .unwrap_err(),
            StepUpError::BadEcdsa
        );
        let other_nonce = h.replace(NONCE_B64, "AAECAwQFBgcICQoLDA0OHw");
        assert_eq!(
            verify(
                &keys(&e, &m),
                CMD,
                &args(),
                Some(&other_nonce),
                NOW,
                &NoReplayCheck
            )
            .unwrap_err(),
            StepUpError::BadEcdsa
        );
        let other_ts = h.replace(&format!("ts={NOW}"), &format!("ts={}", NOW + 1));
        assert_eq!(
            verify(
                &keys(&e, &m),
                CMD,
                &args(),
                Some(&other_ts),
                NOW,
                &NoReplayCheck
            )
            .unwrap_err(),
            StepUpError::BadEcdsa
        );
    }

    #[test]
    fn body_formatting_does_not_matter_only_values_do() {
        // The desktop canonicalises the parsed body, so a phone may send
        // keys in any order and with any whitespace.
        let phone = Phone::new(false, 1);
        let (e, m) = paired!(phone);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        let reordered: Value = serde_json::from_str(
            "{ \"repoPath\" : \"/home/octocat/src/hello-world\",\n \"worktreePath\": \"/home/octocat/src/hello-world/.worktrees/feature\" }",
        )
        .unwrap();
        verify(
            &keys(&e, &m),
            CMD,
            &reordered,
            Some(&h),
            NOW,
            &NoReplayCheck,
        )
        .unwrap();
    }

    #[test]
    fn high_s_ecdsa_signature_accepted() {
        // CryptoKit does not normalise s; the verifier must not demand
        // low-S.
        // `Reduce` moved to crypto-bigint and `reduce_bytes` became
        // `reduce` in the 0.14 line; still the `Reduce<FieldBytes>` impl,
        // so this is the same reduction of the same bytes.
        use p256::elliptic_curve::bigint::Reduce;
        let phone = Phone::new(false, 1);
        let (e, m) = paired!(phone);
        let msg = canonical_bytes(CMD, &args(), NONCE_B64, NOW);
        let sig: p256::ecdsa::Signature = phone.ecdsa.sign(&msg);
        let s = <p256::Scalar as Reduce<p256::FieldBytes>>::reduce(&sig.s().to_bytes());
        let flipped =
            p256::ecdsa::Signature::from_scalars(sig.r().to_bytes(), (-s).to_bytes()).unwrap();
        assert_ne!(flipped, sig);
        let h = build_header(NOW, NONCE_B64, &flipped.to_bytes(), None);
        verify(&keys(&e, &m), CMD, &args(), Some(&h), NOW, &NoReplayCheck).unwrap();
    }

    #[test]
    fn unreadable_stored_keys_are_the_desktops_fault() {
        let phone = Phone::new(true, 1);
        let (_, m) = paired!(phone);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        let bad_ecdsa = vec![0x04; 65];
        let err = verify(
            &keys(&bad_ecdsa, &m),
            CMD,
            &args(),
            Some(&h),
            NOW,
            &NoReplayCheck,
        )
        .unwrap_err();
        assert_eq!(err, StepUpError::BadStoredKey("ECDSA P-256"));
        assert_eq!(err.http_status(), 500);

        let (e, _) = paired!(phone);
        let bad_mldsa = Some(vec![0x11; 100]);
        let err = verify(
            &keys(&e, &bad_mldsa),
            CMD,
            &args(),
            Some(&h),
            NOW,
            &NoReplayCheck,
        )
        .unwrap_err();
        assert_eq!(err, StepUpError::BadStoredKey("ML-DSA-65"));
    }
}
