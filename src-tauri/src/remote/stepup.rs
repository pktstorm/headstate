//! Step-up signatures for destructive commands: the desktop side of
//! "Step-up for destructive commands" in the design spec.
//!
//! A phone that wants to delete something proves, per request, that it
//! still holds the biometric-gated signing keys it registered at pairing.
//! [`verify`] is the whole check and is transport-agnostic: the
//! `POST /v1/call/{command}` handler in `remote/listener.rs` calls it with
//! the paired device, the parsed body, and the raw header value, BEFORE
//! `surface::dispatch`, and only when `surface::class_of` says the
//! command is destructive. Read and write commands carry no signature.
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
//!     stepup::verify(&device, &command, &args, header, Utc::now().timestamp(), &nonces)
//!         .map_err(|e| (e.http_status(), e.to_string()))?;
//! }
//! let out = surface::dispatch(&app, &command, args, &device.name).await?;
//! if class == Class::Destructive {
//!     stepup::notify_destructive(&app, &device.name, &command);
//! }
//! ```
//!
//! `nonces` is one [`NonceWindow`] for the whole listener, managed in
//! Tauri state or held in the router; it is `Send + Sync` and takes
//! `&self`.
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
//!   audited. This module only VERIFIES. The properties formal
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
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::AppHandle;

use crate::store::devices::PairedDevice;

/// The request header carrying the step-up signature. HTTP header names
/// are case-insensitive; this is the canonical spelling.
pub const HEADER: &str = "X-Headstate-Signature";

/// How far the phone's timestamp may sit from the desktop's clock, in
/// either direction. "Sixty seconds" in the spec.
pub const MAX_SKEW_SECS: i64 = 60;

/// Length of the nonce in bytes.
pub const NONCE_LEN: usize = 16;

/// Raw `r || s`, 32 bytes each.
const ECDSA_SIG_LEN: usize = 64;
/// FIPS 204 table 2, ML-DSA-65.
const MLDSA_SIG_LEN: usize = 3309;

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

/// base64url without padding, decoding to exactly `len` bytes. The
/// engine refuses padding and non-canonical trailing bits, so every
/// value has one spelling and the replay set cannot be dodged by
/// re-encoding.
fn decode_exact(value: &str, len: usize) -> Option<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    (bytes.len() == len).then_some(bytes)
}

/// The bytes both signatures cover. See "The signed bytes" in the
/// module docs; `tests::canonical_bytes_test_vector` pins the output.
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
/// crate's, which is what the module docs promise; the walk exists
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

/// Nonces seen from each device, remembered for as long as their
/// timestamp could still pass the skew check.
///
/// One per listener. Keyed by device fingerprint so two phones choosing
/// the same nonce do not block each other, and pruned by time on every
/// insert, so it holds at most one window's worth of destructive calls,
/// which is a handful.
#[derive(Debug, Default)]
pub struct NonceWindow {
    /// `(device fingerprint, nonce) -> the request's timestamp`.
    seen: Mutex<HashMap<(String, [u8; NONCE_LEN]), i64>>,
}

impl NonceWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a nonce for a device. `false` if this device already used
    /// it and that use is still inside the window; `true` if it is new
    /// and is now remembered.
    ///
    /// Check and insert happen under one lock, so two concurrent replays
    /// cannot both be admitted.
    fn admit(&self, device_fp: &str, nonce: [u8; NONCE_LEN], timestamp: i64, now: i64) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        // Anything whose timestamp can no longer pass the skew check can
        // no longer be replayed, so forget it.
        seen.retain(|_, ts| (now - *ts).abs() <= MAX_SKEW_SECS);
        use std::collections::hash_map::Entry;
        match seen.entry((device_fp.to_string(), nonce)) {
            Entry::Occupied(_) => false,
            Entry::Vacant(slot) => {
                slot.insert(timestamp);
                true
            }
        }
    }
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
    device: &PairedDevice,
    command: &str,
    args: &Value,
    header: Option<&str>,
    now: i64,
    nonces: &NonceWindow,
) -> Result<(), StepUpError> {
    let header = SignatureHeader::parse(header.ok_or(StepUpError::Missing)?)?;

    let skew = header.timestamp - now;
    if skew.abs() > MAX_SKEW_SECS {
        return Err(StepUpError::StaleTimestamp { skew });
    }

    // The signature set must be exactly what the pairing record expects,
    // decided before any verification so the refusal names the mismatch
    // rather than a signature that was never going to be checked.
    match (&device.mldsa_pubkey, &header.mldsa) {
        (Some(_), None) => return Err(StepUpError::MissingMldsa),
        (None, Some(_)) => return Err(StepUpError::UnexpectedMldsa),
        _ => {}
    }

    let msg = canonical_bytes(command, args, &header.nonce, header.timestamp);
    verify_ecdsa(&device.ecdsa_pubkey, &msg, &header.ecdsa)?;
    if let (Some(key), Some(sig)) = (&device.mldsa_pubkey, &header.mldsa) {
        verify_mldsa(key, &msg, sig)?;
    }

    if !nonces.admit(&device.cert_fp, header.nonce_bytes, header.timestamp, now) {
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

/// The notification text for a destructive command run for a phone:
/// `(title, body)`. Separate from [`notify_destructive`] so the wording
/// is testable without the plugin.
///
/// The title carries both facts a glance needs, which device and which
/// command, in the command's own name so it matches the log line
/// `dispatch` wrote. The body says what to do if it was not you.
pub fn destructive_notice(device_name: &str, command: &str) -> (String, String) {
    (
        format!("{device_name} ran {command}"),
        "A destructive command from a paired phone. If this was not you, revoke the device in Settings."
            .to_string(),
    )
}

/// Post the native notification for a destructive command that was just
/// executed on behalf of `device_name`. The second, independent signal
/// the spec asks for; call it after `dispatch` returns, whether or not
/// the command itself succeeded, because the attempt is the news.
///
/// Failure is logged and swallowed, as with every other notification in
/// the app: the command has already run, and the notification is an
/// affordance.
pub fn notify_destructive(app: &AppHandle, device_name: &str, command: &str) {
    use tauri_plugin_notification::NotificationExt;

    if !crate::poll::notification_allowed(app) {
        return;
    }
    let (title, body) = destructive_notice(device_name, command);
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("failed to show notification: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_dsa::{MlDsa65, Seed};
    use p256::ecdsa::signature::Signer;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    const NOW: i64 = 1_788_566_400;
    const NONCE_B64: &str = "AAECAwQFBgcICQoLDA0ODw";

    /// A phone's step-up keys, made deterministically so no test needs
    /// an RNG.
    struct Phone {
        ecdsa: p256::ecdsa::SigningKey,
        mldsa: Option<ml_dsa::SigningKey<MlDsa65>>,
        fp: String,
    }

    impl Phone {
        fn new(with_mldsa: bool, seed: u8) -> Self {
            let ecdsa = p256::ecdsa::SigningKey::from_bytes(&[seed; 32].into()).unwrap();
            let mldsa = with_mldsa
                .then(|| ml_dsa::SigningKey::<MlDsa65>::from_seed(&Seed::from([seed; 32])));
            Self {
                ecdsa,
                mldsa,
                fp: format!("{:064x}", seed),
            }
        }

        fn paired(&self) -> PairedDevice {
            PairedDevice {
                id: 1,
                name: "Octocat's phone".into(),
                cert_fp: self.fp.clone(),
                cert_der: vec![0x30],
                ecdsa_pubkey: self
                    .ecdsa
                    .verifying_key()
                    .to_encoded_point(false)
                    .as_bytes()
                    .to_vec(),
                mldsa_pubkey: self
                    .mldsa
                    .as_ref()
                    .map(|k| k.expanded_key().verifying_key().encode().to_vec()),
                paired_at: "2026-09-05T00:00:00Z".into(),
                last_seen: None,
            }
        }

        fn ecdsa_sig(&self, msg: &[u8]) -> String {
            let sig: p256::ecdsa::Signature = self.ecdsa.sign(msg);
            URL_SAFE_NO_PAD.encode(sig.to_bytes())
        }

        fn mldsa_sig(&self, msg: &[u8]) -> String {
            let sig = self
                .mldsa
                .as_ref()
                .expect("phone has ML-DSA")
                .expanded_key()
                .sign_deterministic(msg, b"")
                .unwrap();
            URL_SAFE_NO_PAD.encode(sig.encode())
        }

        /// A well-formed header for `command`/`args`, with every
        /// signature this phone can produce.
        fn header(&self, command: &str, args: &Value, nonce: &str, ts: i64) -> String {
            let msg = canonical_bytes(command, args, nonce, ts);
            let mut h = format!("v1;ts={ts};nonce={nonce};ecdsa={}", self.ecdsa_sig(&msg));
            if self.mldsa.is_some() {
                h.push_str(&format!(";mldsa={}", self.mldsa_sig(&msg)));
            }
            h
        }
    }

    fn args() -> Value {
        json!({
            "worktreePath": "/home/octocat/src/hello-world/.worktrees/feature",
            "repoPath": "/home/octocat/src/hello-world",
        })
    }

    const CMD: &str = "remove_worktree";

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
            format!("{:x}", Sha256::digest(&bytes)),
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
        let err = verify(
            &phone.paired(),
            CMD,
            &args(),
            None,
            NOW,
            &NonceWindow::new(),
        )
        .unwrap_err();
        assert_eq!(err, StepUpError::Missing);
        assert_eq!(err.http_status(), 403);
        let err = verify(
            &phone.paired(),
            CMD,
            &args(),
            Some(""),
            NOW,
            &NonceWindow::new(),
        )
        .unwrap_err();
        assert!(matches!(err, StepUpError::Malformed(_)));
        assert_eq!(err.http_status(), 400);
    }

    #[test]
    fn stale_timestamp_refused_in_both_directions() {
        let phone = Phone::new(true, 1);
        let device = phone.paired();
        let nonces = NonceWindow::new();
        for (ts, skew) in [(NOW - 61, -61), (NOW + 61, 61), (NOW - 3600, -3600)] {
            let h = phone.header(CMD, &args(), NONCE_B64, ts);
            let err = verify(&device, CMD, &args(), Some(&h), NOW, &nonces).unwrap_err();
            assert_eq!(err, StepUpError::StaleTimestamp { skew }, "ts={ts}");
            assert_eq!(err.http_status(), 403);
        }
        // Exactly sixty seconds either side is still inside.
        for ts in [NOW - 60, NOW + 60] {
            let nonce = URL_SAFE_NO_PAD.encode(ts.to_le_bytes().repeat(2));
            let h = phone.header(CMD, &args(), &nonce, ts);
            verify(&device, CMD, &args(), Some(&h), NOW, &nonces).unwrap();
        }
    }

    #[test]
    fn reused_nonce_refused_and_a_failed_request_burns_none() {
        let phone = Phone::new(true, 1);
        let device = phone.paired();
        let nonces = NonceWindow::new();
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);

        // A request that fails verification leaves the nonce unspent.
        let tampered = json!({"repoPath": "/home/octocat/src/hello-world", "worktreePath": "/"});
        assert_eq!(
            verify(&device, CMD, &tampered, Some(&h), NOW, &nonces).unwrap_err(),
            StepUpError::BadEcdsa
        );

        verify(&device, CMD, &args(), Some(&h), NOW, &nonces).unwrap();
        let err = verify(&device, CMD, &args(), Some(&h), NOW + 30, &nonces).unwrap_err();
        assert_eq!(err, StepUpError::NonceReused);
        assert_eq!(err.http_status(), 403);

        // Another device is free to use the same nonce.
        let other = Phone::new(true, 2);
        let h2 = other.header(CMD, &args(), NONCE_B64, NOW);
        verify(&other.paired(), CMD, &args(), Some(&h2), NOW, &nonces).unwrap();

        // Once the timestamp itself is outside the window a replay is
        // refused for staleness, which is the same refusal with a more
        // useful message, and the next admitted request prunes the entry.
        let err = verify(&device, CMD, &args(), Some(&h), NOW + 61, &nonces).unwrap_err();
        assert_eq!(err, StepUpError::StaleTimestamp { skew: -61 });
        assert_eq!(
            nonces.seen.lock().unwrap().len(),
            2,
            "pruning waits for an admit"
        );
        let later = phone.header(CMD, &args(), "AAECAwQFBgcICQoLDA0OHw", NOW + 61);
        verify(&device, CMD, &args(), Some(&later), NOW + 61, &nonces).unwrap();
        assert_eq!(
            nonces.seen.lock().unwrap().len(),
            1,
            "pruned to the live entry"
        );
    }

    #[test]
    fn nonce_window_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NonceWindow>();
    }

    #[test]
    fn valid_ecdsa_only_accepted_for_an_ecdsa_only_pairing() {
        let phone = Phone::new(false, 1);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        verify(
            &phone.paired(),
            CMD,
            &args(),
            Some(&h),
            NOW,
            &NonceWindow::new(),
        )
        .unwrap();
    }

    #[test]
    fn valid_hybrid_accepted() {
        let phone = Phone::new(true, 1);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        assert!(h.contains(";mldsa="));
        verify(
            &phone.paired(),
            CMD,
            &args(),
            Some(&h),
            NOW,
            &NonceWindow::new(),
        )
        .unwrap();
    }

    #[test]
    fn ecdsa_only_refused_when_mldsa_was_paired() {
        let phone = Phone::new(true, 1);
        let full = phone.header(CMD, &args(), NONCE_B64, NOW);
        let ecdsa_only = full.split(";mldsa=").next().unwrap();
        let err = verify(
            &phone.paired(),
            CMD,
            &args(),
            Some(ecdsa_only),
            NOW,
            &NonceWindow::new(),
        )
        .unwrap_err();
        assert_eq!(err, StepUpError::MissingMldsa);
        assert_eq!(err.http_status(), 403);
    }

    #[test]
    fn mldsa_refused_when_the_pairing_has_no_mldsa_key() {
        let phone = Phone::new(true, 1);
        let full = phone.header(CMD, &args(), NONCE_B64, NOW);
        let mut device = phone.paired();
        device.mldsa_pubkey = None;
        let err = verify(&device, CMD, &args(), Some(&full), NOW, &NonceWindow::new()).unwrap_err();
        assert_eq!(err, StepUpError::UnexpectedMldsa);
    }

    #[test]
    fn mldsa_only_refused() {
        // The grammar requires ecdsa, so an ML-DSA-only header cannot
        // even be parsed; and a hybrid header whose ECDSA half is wrong
        // is refused on that half regardless of the ML-DSA half.
        let phone = Phone::new(true, 1);
        let full = phone.header(CMD, &args(), NONCE_B64, NOW);
        let (before, mldsa) = full.split_once(";ecdsa=").unwrap();
        let (_, mldsa) = mldsa.split_once(";mldsa=").unwrap();
        let mldsa_only = format!("{before};mldsa={mldsa}");
        assert!(matches!(
            verify(
                &phone.paired(),
                CMD,
                &args(),
                Some(&mldsa_only),
                NOW,
                &NonceWindow::new()
            ),
            Err(StepUpError::Malformed(_))
        ));

        let other = Phone::new(true, 2);
        let msg = canonical_bytes(CMD, &args(), NONCE_B64, NOW);
        let wrong_ecdsa = format!("{before};ecdsa={};mldsa={mldsa}", other.ecdsa_sig(&msg));
        assert_eq!(
            verify(
                &phone.paired(),
                CMD,
                &args(),
                Some(&wrong_ecdsa),
                NOW,
                &NonceWindow::new()
            )
            .unwrap_err(),
            StepUpError::BadEcdsa
        );
    }

    #[test]
    fn wrong_mldsa_refused_even_with_a_good_ecdsa() {
        let phone = Phone::new(true, 1);
        let other = Phone::new(true, 2);
        let msg = canonical_bytes(CMD, &args(), NONCE_B64, NOW);
        let h = format!(
            "v1;ts={NOW};nonce={NONCE_B64};ecdsa={};mldsa={}",
            phone.ecdsa_sig(&msg),
            other.mldsa_sig(&msg)
        );
        let err = verify(
            &phone.paired(),
            CMD,
            &args(),
            Some(&h),
            NOW,
            &NonceWindow::new(),
        )
        .unwrap_err();
        assert_eq!(err, StepUpError::BadMldsa);
        assert_eq!(err.http_status(), 403);
    }

    #[test]
    fn tampered_args_command_nonce_or_timestamp_refused() {
        let phone = Phone::new(true, 1);
        let device = phone.paired();
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);

        let mut tampered = args();
        tampered["worktreePath"] = json!("/home/octocat");
        assert_eq!(
            verify(&device, CMD, &tampered, Some(&h), NOW, &NonceWindow::new()).unwrap_err(),
            StepUpError::BadEcdsa
        );
        assert_eq!(
            verify(
                &device,
                "remove_worktree_forced",
                &args(),
                Some(&h),
                NOW,
                &NonceWindow::new()
            )
            .unwrap_err(),
            StepUpError::BadEcdsa
        );
        let other_nonce = h.replace(NONCE_B64, "AAECAwQFBgcICQoLDA0OHw");
        assert_eq!(
            verify(
                &device,
                CMD,
                &args(),
                Some(&other_nonce),
                NOW,
                &NonceWindow::new()
            )
            .unwrap_err(),
            StepUpError::BadEcdsa
        );
        let other_ts = h.replace(&format!("ts={NOW}"), &format!("ts={}", NOW + 1));
        assert_eq!(
            verify(
                &device,
                CMD,
                &args(),
                Some(&other_ts),
                NOW,
                &NonceWindow::new()
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
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        let reordered: Value = serde_json::from_str(
            "{ \"repoPath\" : \"/home/octocat/src/hello-world\",\n \"worktreePath\": \"/home/octocat/src/hello-world/.worktrees/feature\" }",
        )
        .unwrap();
        verify(
            &phone.paired(),
            CMD,
            &reordered,
            Some(&h),
            NOW,
            &NonceWindow::new(),
        )
        .unwrap();
    }

    #[test]
    fn high_s_ecdsa_signature_accepted() {
        // CryptoKit does not normalise s; the verifier must not demand
        // low-S.
        use p256::elliptic_curve::ops::Reduce;
        let phone = Phone::new(false, 1);
        let msg = canonical_bytes(CMD, &args(), NONCE_B64, NOW);
        let sig: p256::ecdsa::Signature = phone.ecdsa.sign(&msg);
        let s = p256::Scalar::reduce_bytes(&sig.s().to_bytes());
        let flipped =
            p256::ecdsa::Signature::from_scalars(sig.r().to_bytes(), (-s).to_bytes()).unwrap();
        assert_ne!(flipped, sig);
        let h = format!(
            "v1;ts={NOW};nonce={NONCE_B64};ecdsa={}",
            URL_SAFE_NO_PAD.encode(flipped.to_bytes())
        );
        verify(
            &phone.paired(),
            CMD,
            &args(),
            Some(&h),
            NOW,
            &NonceWindow::new(),
        )
        .unwrap();
    }

    #[test]
    fn unreadable_stored_keys_are_the_desktops_fault() {
        let phone = Phone::new(true, 1);
        let h = phone.header(CMD, &args(), NONCE_B64, NOW);
        let mut device = phone.paired();
        device.ecdsa_pubkey = vec![0x04; 65];
        let err = verify(&device, CMD, &args(), Some(&h), NOW, &NonceWindow::new()).unwrap_err();
        assert_eq!(err, StepUpError::BadStoredKey("ECDSA P-256"));
        assert_eq!(err.http_status(), 500);

        let mut device = phone.paired();
        device.mldsa_pubkey = Some(vec![0x11; 100]);
        let err = verify(&device, CMD, &args(), Some(&h), NOW, &NonceWindow::new()).unwrap_err();
        assert_eq!(err, StepUpError::BadStoredKey("ML-DSA-65"));
    }

    #[test]
    fn destructive_notice_names_the_device_and_the_command() {
        let (title, body) = destructive_notice("Octocat's phone", "remove_worktree");
        assert_eq!(title, "Octocat's phone ran remove_worktree");
        assert_eq!(
            body,
            "A destructive command from a paired phone. If this was not you, revoke the device in Settings."
        );
    }
}
