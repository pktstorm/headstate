//! The phone's side of the step-up: build the `X-Headstate-Signature`
//! header a destructive command carries.
//!
//! The desktop's `remote/stepup.rs` defines the grammar and the signed
//! bytes; this module produces exactly what its `verify` checks:
//!
//! ```text
//! X-Headstate-Signature: v1;ts=<unix secs>;nonce=<b64url 16B>;ecdsa=<b64url 64B>[;mldsa=<b64url 3309B>]
//! ```
//!
//! - `;`-separated, no whitespace, `v1` first, each key once.
//! - Every value base64url WITHOUT padding.
//! - Both signatures are over [`canonical_bytes`]: the JSON object
//!   `{args, command, nonce, timestamp}` with keys sorted by their UTF-8
//!   bytes at every level (inside `args` too), no whitespace, and
//!   `serde_json`'s escaping. `timestamp` is an integer; `nonce` the
//!   exact header string.
//!
//! Pinned by [`tests::canonical_bytes_test_vector`], the same 203-byte
//! vector the desktop pins (SHA256 `ebd1a4f4…79ff`), so the two sides
//! agree by construction. The desktop crate itself cannot be linked from
//! here even as a dev-dependency: its lock would bring octocrab and
//! rusqlite into this crate's, which `no_desktop_only_crates_in_lock`
//! forbids for good reason. The vector is the bridge instead.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;

use crate::keys::{random_bytes, DeviceKeys, KeyError, Signatures};

/// The request header. HTTP header names are case-insensitive; this is
/// the canonical spelling, matching the desktop's constant.
pub const HEADER: &str = "X-Headstate-Signature";

/// Nonce length in bytes.
pub const NONCE_LEN: usize = 16;

/// The bytes both signatures cover. See the module docs.
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

/// Object keys in byte order at every level, no whitespace, scalars
/// through `serde_json` so the escaping is the crate's. The explicit walk
/// exists because `serde_json::Map` keeps insertion order when the
/// `preserve_order` feature is on anywhere in the build, and a signature
/// must not depend on a dependency's feature flags.
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

/// The header value from its parts.
pub fn build_header(timestamp: i64, nonce: &str, sigs: &Signatures) -> String {
    let mut h = format!(
        "v1;ts={timestamp};nonce={nonce};ecdsa={}",
        URL_SAFE_NO_PAD.encode(&sigs.ecdsa)
    );
    if let Some(mldsa) = &sigs.mldsa {
        h.push_str(";mldsa=");
        h.push_str(&URL_SAFE_NO_PAD.encode(mldsa));
    }
    h
}

/// Sign one destructive request: a fresh nonce, the canonical bytes for
/// `command`/`args` at `now`, every signature the device can produce,
/// and the header carrying them. `now` is Unix seconds by the phone's
/// clock; the desktop allows sixty seconds of skew.
pub fn sign_request(
    keys: &dyn DeviceKeys,
    command: &str,
    args: &Value,
    now: i64,
) -> Result<String, KeyError> {
    let nonce = URL_SAFE_NO_PAD.encode(random_bytes::<NONCE_LEN>()?);
    let msg = canonical_bytes(command, args, &nonce, now);
    let sigs = keys.sign(&msg)?;
    Ok(build_header(now, &nonce, &sigs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{DeviceKeys, PublicKeys, SoftwareKeys, ECDSA_SIG_LEN, MLDSA_SIG_LEN};
    use crate::store::MemoryStore;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;

    const NONCE_B64: &str = "AAECAwQFBgcICQoLDA0ODw";
    const NOW: i64 = 1788566400;
    const CMD: &str = "remove_worktree";

    fn args() -> Value {
        json!({
            "worktreePath": "/home/octocat/src/hello-world/.worktrees/feature",
            "repoPath": "/home/octocat/src/hello-world",
        })
    }

    /// The desktop's vector, byte for byte.
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

    /// A strict parser in the desktop's grammar, reduced to what the
    /// tests need: the fields, in any order, each once, no unknowns.
    /// `(timestamp, nonce, ecdsa, mldsa)`.
    type ParsedHeader = (i64, String, Vec<u8>, Option<Vec<u8>>);

    fn parse(header: &str) -> Result<ParsedHeader, String> {
        let mut fields = header.split(';');
        if fields.next() != Some("v1") {
            return Err("version".into());
        }
        let (mut ts, mut nonce, mut ecdsa, mut mldsa) = (None, None, None, None);
        for f in fields {
            let (k, v) = f.split_once('=').ok_or("kv")?;
            let slot = match k {
                "ts" => &mut ts,
                "nonce" => &mut nonce,
                "ecdsa" => &mut ecdsa,
                "mldsa" => &mut mldsa,
                _ => return Err(format!("unknown {k}")),
            };
            if slot.replace(v).is_some() {
                return Err(format!("twice {k}"));
            }
        }
        let dec = |v: &str, n: usize| {
            let b = URL_SAFE_NO_PAD.decode(v).map_err(|e| e.to_string())?;
            (b.len() == n).then_some(b).ok_or(format!("len {n}"))
        };
        let nonce = nonce.ok_or("nonce")?;
        dec(nonce, NONCE_LEN)?;
        Ok((
            ts.ok_or("ts")?.parse().map_err(|_| "ts")?,
            nonce.to_string(),
            dec(ecdsa.ok_or("ecdsa")?, ECDSA_SIG_LEN)?,
            match mldsa {
                None => None,
                Some(v) => Some(dec(v, MLDSA_SIG_LEN)?),
            },
        ))
    }

    fn keys() -> (SoftwareKeys, PublicKeys) {
        let keys = SoftwareKeys::new(Arc::new(MemoryStore::default()));
        let public = keys.generate().unwrap();
        (keys, public)
    }

    /// The desktop's checks, replicated: parse the header, rebuild the
    /// canonical bytes from the parsed nonce and timestamp, verify both
    /// signatures against the keys the pair request carried.
    fn verify_as_desktop(public: &PublicKeys, command: &str, args: &Value, header: &str) {
        let (ts, nonce, ecdsa, mldsa) = parse(header).unwrap();
        let msg = canonical_bytes(command, args, &nonce, ts);

        use p256::ecdsa::signature::Verifier;
        let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&public.ecdsa_p256).unwrap();
        vk.verify(&msg, &p256::ecdsa::Signature::from_slice(&ecdsa).unwrap())
            .expect("ECDSA verifies");

        match (&public.mldsa_65, mldsa) {
            (Some(key), Some(sig)) => {
                use ml_dsa::{EncodedVerifyingKey, MlDsa65, Signature, VerifyingKey};
                let enc = EncodedVerifyingKey::<MlDsa65>::try_from(key.as_slice()).unwrap();
                let vk = VerifyingKey::<MlDsa65>::decode(&enc);
                let sig = Signature::<MlDsa65>::try_from(sig.as_slice()).unwrap();
                assert!(vk.verify_with_context(&msg, b"", &sig), "ML-DSA verifies");
            }
            (None, None) => {}
            _ => panic!("signature set must match the pairing's key set"),
        }
    }

    #[test]
    fn the_header_is_in_the_desktop_grammar_and_verifies() {
        let (keys, public) = keys();
        let header = sign_request(&keys, CMD, &args(), NOW).unwrap();
        assert!(header.starts_with(&format!("v1;ts={NOW};nonce=")));
        assert!(!header.contains(' '));
        for field in header.split(';').skip(1) {
            let (_, value) = field.split_once('=').unwrap();
            assert!(!value.contains('='), "no padding anywhere: {field}");
        }
        verify_as_desktop(&public, CMD, &args(), &header);
    }

    #[test]
    fn a_reordered_body_signs_the_same_bytes() {
        // The desktop canonicalises the PARSED body, so key order on the
        // wire is free: the same signature must verify for both.
        let (keys, public) = keys();
        let header = sign_request(&keys, CMD, &args(), NOW).unwrap();
        let reordered = json!({
            "repoPath": "/home/octocat/src/hello-world",
            "worktreePath": "/home/octocat/src/hello-world/.worktrees/feature",
        });
        verify_as_desktop(&public, CMD, &reordered, &header);
    }

    #[test]
    fn each_request_gets_a_fresh_nonce() {
        let (keys, _) = keys();
        let a = sign_request(&keys, CMD, &args(), NOW).unwrap();
        let b = sign_request(&keys, CMD, &args(), NOW).unwrap();
        assert_ne!(parse(&a).unwrap().1, parse(&b).unwrap().1);
    }

    #[test]
    fn build_header_omits_mldsa_when_the_device_has_none() {
        let sigs = Signatures {
            ecdsa: vec![1; ECDSA_SIG_LEN],
            mldsa: None,
        };
        let h = build_header(7, NONCE_B64, &sigs);
        assert_eq!(
            h,
            format!(
                "v1;ts=7;nonce={NONCE_B64};ecdsa={}",
                URL_SAFE_NO_PAD.encode([1u8; ECDSA_SIG_LEN])
            )
        );
        assert_eq!(parse(&h).unwrap().3, None);
    }

    #[test]
    fn signing_without_keys_is_the_no_keys_error() {
        let keys = SoftwareKeys::new(Arc::new(MemoryStore::default()));
        assert_eq!(
            sign_request(&keys, CMD, &args(), NOW).unwrap_err(),
            KeyError::NoKeys
        );
    }
}
