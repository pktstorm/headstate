//! The phone's side of the step-up: build the `X-Headstate-Signature`
//! header a destructive command carries.
//!
//! The grammar and the signed bytes are not defined here and are not
//! copied from anywhere. They live in the `headstate-stepup` crate,
//! which the desktop depends on by the same path, so this module is just
//! the wiring between the device's keys and that crate's
//! [`build_header`]: a fresh nonce, [`canonical_bytes`] for the command
//! and args, and whatever signatures the keystore can produce.
//!
//! That crate is also what the tests below verify with. They call its
//! `verify` -- the desktop's actual verifier, not a re-implementation of
//! it -- so a change to the canonical form or the grammar that reaches
//! only one side cannot leave both suites green. It used to be a
//! re-implementation, whose own doc comment said "replicated"; #695 is
//! the whole story.
//!
//! Nothing about the two crates' separation changed to allow this. The
//! desktop crate still cannot be linked from here even as a
//! dev-dependency: its lock would bring octocrab and rusqlite into this
//! crate's, which `no_desktop_only_crates_in_lock` forbids for good
//! reason. `headstate-stepup` is a leaf crate over serde_json, base64,
//! p256 and ml-dsa, all of which this crate already links.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;

use crate::keys::{random_bytes, DeviceKeys, KeyError, Signatures};

// The protocol, re-exported so the client keeps spelling the header
// name `stepup::HEADER`. One implementation, two names for it.
pub use headstate_stepup::{build_header, canonical_bytes, HEADER, NONCE_LEN};

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
    let sigs: Signatures = keys.sign(&msg)?;
    Ok(build_header(
        now,
        &nonce,
        &sigs.ecdsa,
        sigs.mldsa.as_deref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{DeviceKeys, PublicKeys, SoftwareKeys, ECDSA_SIG_LEN};
    use headstate_stepup::{NoReplayCheck, PairedKeys, SignatureHeader, StepUpError};
    use serde_json::json;
    use std::sync::Arc;

    const NOW: i64 = 1788566400;
    const CMD: &str = "remove_worktree";

    fn args() -> Value {
        json!({
            "worktreePath": "/home/octocat/src/hello-world/.worktrees/feature",
            "repoPath": "/home/octocat/src/hello-world",
        })
    }

    fn keys() -> (SoftwareKeys, PublicKeys) {
        let keys = SoftwareKeys::new(Arc::new(MemoryStore::default()));
        let public = keys.generate().unwrap();
        (keys, public)
    }

    use crate::store::MemoryStore;

    /// The DESKTOP'S VERIFIER, called directly. Not a copy of its checks
    /// -- `headstate_stepup::verify` is the same function
    /// `src-tauri/src/remote/stepup.rs` calls on every destructive
    /// request, reached here because both crates depend on the one crate
    /// that defines it.
    ///
    /// The pairing's key set is what the phone registered, so
    /// [`PairedKeys`] is built from `PublicKeys` the same way the
    /// desktop builds it from a `PairedDevice` row. No replay window:
    /// that is the listener's state, and a phone has none.
    fn verify_as_desktop(
        public: &PublicKeys,
        command: &str,
        args: &Value,
        header: &str,
    ) -> Result<(), StepUpError> {
        headstate_stepup::verify(
            &PairedKeys {
                ecdsa: &public.ecdsa_p256,
                mldsa: public.mldsa_65.as_deref(),
            },
            command,
            args,
            Some(header),
            NOW,
            &NoReplayCheck,
        )
    }

    /// A header this module built is one the desktop's verifier accepts,
    /// in the shape the grammar demands.
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
        verify_as_desktop(&public, CMD, &args(), &header).unwrap();
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
        verify_as_desktop(&public, CMD, &reordered, &header).unwrap();
    }

    /// The other direction, and the reason this test is worth having on
    /// top of the one above: the desktop REFUSES what it should. A
    /// verifier that accepted everything would pass every test here.
    #[test]
    fn the_desktop_refuses_a_tampered_request() {
        let (keys, public) = keys();
        let header = sign_request(&keys, CMD, &args(), NOW).unwrap();
        let mut tampered = args();
        tampered["worktreePath"] = json!("/home/octocat");
        assert_eq!(
            verify_as_desktop(&public, CMD, &tampered, &header).unwrap_err(),
            StepUpError::BadEcdsa
        );
        assert_eq!(
            verify_as_desktop(&public, "remove_worktree_forced", &args(), &header).unwrap_err(),
            StepUpError::BadEcdsa
        );
        // Signed sixty-one seconds off the desktop's clock.
        let stale = sign_request(&keys, CMD, &args(), NOW + 61).unwrap();
        assert_eq!(
            verify_as_desktop(&public, CMD, &args(), &stale).unwrap_err(),
            StepUpError::StaleTimestamp { skew: 61 }
        );
    }

    /// Another device's signature over the same request does not
    /// verify against this pairing's keys.
    #[test]
    fn another_phones_signature_is_refused() {
        let (_, public) = keys();
        let (other_keys, _) = keys();
        let header = sign_request(&other_keys, CMD, &args(), NOW).unwrap();
        assert_eq!(
            verify_as_desktop(&public, CMD, &args(), &header).unwrap_err(),
            StepUpError::BadEcdsa
        );
    }

    #[test]
    fn each_request_gets_a_fresh_nonce() {
        let (keys, _) = keys();
        let a = sign_request(&keys, CMD, &args(), NOW).unwrap();
        let b = sign_request(&keys, CMD, &args(), NOW).unwrap();
        assert_ne!(
            SignatureHeader::parse(&a).unwrap().nonce,
            SignatureHeader::parse(&b).unwrap().nonce
        );
    }

    /// The software keys produce a hybrid signature, so the header
    /// carries both halves and the desktop's key-set check is satisfied.
    /// A pairing recorded WITHOUT an ML-DSA key refuses that same
    /// header, which is what proves the `mldsa` field is really there.
    #[test]
    fn the_header_carries_every_signature_the_device_has() {
        let (keys, public) = keys();
        let header = sign_request(&keys, CMD, &args(), NOW).unwrap();
        let parsed = SignatureHeader::parse(&header).unwrap();
        assert_eq!(parsed.ecdsa.len(), ECDSA_SIG_LEN);
        assert_eq!(public.mldsa_65.is_some(), parsed.mldsa.is_some());

        let ecdsa_only_pairing = PublicKeys {
            mldsa_65: None,
            ..public.clone()
        };
        assert_eq!(
            verify_as_desktop(&ecdsa_only_pairing, CMD, &args(), &header).unwrap_err(),
            StepUpError::UnexpectedMldsa
        );
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
