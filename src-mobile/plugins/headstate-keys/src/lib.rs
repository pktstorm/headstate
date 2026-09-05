//! `tauri-plugin-headstate-keys`: the phone's keys, in the phone's
//! hardware.
//!
//! The mobile companion (see "Step-up for destructive commands" and
//! "Post-quantum posture" in docs/superpowers/specs/2026-09-05-mobile-companion-design.md)
//! holds three keys, all made at pairing:
//!
//! - a **session key**, ECDSA P-256, whose self-signed certificate is the
//!   TLS client identity the desktop pins;
//! - a **step-up signing key**, ECDSA P-256, in the Secure Enclave or
//!   the Android Keystore, usable only after a biometric or device
//!   passcode check;
//! - an **ML-DSA-65 signing key** beside it, when the keystore can hold
//!   one (Secure Enclave on iOS 26, a KeyMint 5 TEE on Android 17).
//!
//! [`DeviceKeys`] is the whole Rust API; [`HeadstateKeys`] is the
//! implementation the app gets from Tauri state after registering
//! [`init`]. The Swift side (`ios/Sources/HeadstateKeysPlugin.swift`)
//! and the Kotlin side (`android/src/main/java/HeadstateKeysPlugin.kt`)
//! hold the signing keys and never export them; the module docs in
//! `wire.rs` pin what crosses the bridge.
//!
//! # The step-up keys are in hardware; the session key is not
//!
//! rustls presents a client identity from the private key's bytes: there
//! is no hook for "sign this handshake with a key I cannot read". So the
//! session key cannot live in the Secure Enclave or the Keystore, and
//! this plugin does not pretend it does. It is generated HERE, in Rust,
//! with the same `rcgen` call the desktop uses for its own identity, and
//! handed to the native side to keep: on iOS a Keychain item with
//! `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` (no iCloud Keychain,
//! no restore onto another phone); on Android an app-private file
//! encrypted under an AES-GCM key that IS in the Keystore. The spec only
//! requires the step-up keys to be non-exportable, and it is the
//! step-up keys that gate anything destructive; a stolen session key
//! alone can read PR titles until the pairing is revoked, which is why
//! `SECURITY.md` tells you to revoke a lost phone.
//!
//! Generating the session key in Rust on both platforms, rather than in
//! CryptoKit on iOS and Rust on Android, keeps one code path and one
//! PKCS#8 encoder (`rcgen`'s, which rustls is known to load).
//!
//! # One prompt per destructive command
//!
//! Access control is on the keys themselves, not on a prompt the app
//! shows: iOS `SecAccessControl` with `.privateKeyUsage` and
//! `.userPresence`, Android `setUserAuthenticationRequired(true)`. With
//! two signing keys that would be two prompts. Each platform file
//! explains how it collapses them to one; the short version is that iOS
//! reuses one `LAContext` for both keys and Android binds the prompt to
//! the ECDSA operation and gives the ML-DSA key a ten-second window.
//!
//! # ML-DSA is optional and decided at `generate`
//!
//! [`PublicKeys::mldsa_65`] is `None` when the keystore could not make
//! the key: the platform is too old, the initialiser threw, or (Android)
//! the key came back below TEE security level. The pairing record on the
//! desktop remembers which keys the phone offered, and a later
//! [`DeviceKeys::sign`] returns exactly that set. Re-pairing after an OS
//! upgrade is how a phone adds ML-DSA.

use std::fmt;

use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use serde_json::Value;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

mod bridge;
mod error;
#[cfg(test)]
mod fake;
pub mod wire;

pub use error::{Error, Result};
pub use wire::{ECDSA_P256_LEN, ECDSA_SIG_LEN, MLDSA_65_LEN, MLDSA_SIG_LEN};

use bridge::Bridge;
use wire::{cmd, SignArgs, WirePublicKeys, WireSession, WireSignatures};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.pktstorm.headstate.keys";

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_headstate_keys);

/// What the biometric prompt says. One string for both platforms; the
/// native side shows it as the reason (iOS) or subtitle (Android).
pub const PROMPT_REASON: &str = "Confirm this change on your desktop";

/// The step-up public keys, in the encodings the desktop stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKeys {
    /// 65 bytes, SEC1 uncompressed (`0x04 || x || y`).
    pub ecdsa_p256: Vec<u8>,
    /// 1952 bytes, FIPS 204; `None` when the keystore has no ML-DSA key.
    pub mldsa_65: Option<Vec<u8>>,
}

/// One step-up signature set over one message.
#[derive(Clone, PartialEq, Eq)]
pub struct Signatures {
    /// Raw `r || s`, 64 bytes, over the SHA256 of the message.
    pub ecdsa: Vec<u8>,
    /// 3309 bytes, pure ML-DSA-65 with the empty context; present
    /// exactly when [`PublicKeys::mldsa_65`] was.
    pub mldsa: Option<Vec<u8>>,
}

impl fmt::Debug for Signatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signatures")
            .field("ecdsa", &self.ecdsa.len())
            .field("mldsa", &self.mldsa.as_ref().map(Vec::len))
            .finish()
    }
}

/// The TLS client identity, in the shapes rustls takes.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    /// The self-signed certificate, DER.
    pub cert_der: Vec<u8>,
    /// The private key, PKCS#8 DER.
    pub key_pkcs8: Vec<u8>,
}

/// No key bytes in logs: `Debug` prints lengths.
impl fmt::Debug for SessionIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionIdentity")
            .field("cert_der", &self.cert_der.len())
            .field("key_pkcs8", &"[elided]")
            .finish()
    }
}

/// The phone's keys. The mobile client (`src-mobile/src/keys.rs`, #514)
/// programs against this trait; [`HeadstateKeys`] is the hardware
/// implementation and the client's software fallback is the other.
pub trait DeviceKeys: Send + Sync {
    /// Make a fresh session identity and step-up key set, replacing any
    /// existing ones. Called once per pairing. Returns the step-up
    /// public keys for the pair request.
    fn generate(&self) -> Result<PublicKeys>;
    /// The step-up public keys, or [`Error::NotGenerated`].
    fn public_keys(&self) -> Result<PublicKeys>;
    /// Sign `bytes` with every step-up key this phone holds. This is
    /// where the biometric prompt happens; [`Error::Cancelled`] when the
    /// user dismisses it.
    fn sign(&self, bytes: &[u8]) -> Result<Signatures>;
    /// Forget every key. Idempotent: a phone that was never paired is
    /// not an error.
    fn destroy(&self) -> Result<()>;
    /// The TLS client identity, or [`Error::NotGenerated`].
    fn session_identity(&self) -> Result<SessionIdentity>;
}

/// The hardware-backed [`DeviceKeys`], managed in Tauri state by
/// [`init`]. Reach it with [`HeadstateKeysExt::headstate_keys`].
pub struct HeadstateKeys {
    bridge: Box<dyn Bridge>,
}

impl HeadstateKeys {
    fn new(bridge: Box<dyn Bridge>) -> Self {
        Self { bridge }
    }

    fn call<T: serde::de::DeserializeOwned>(&self, command: &str, args: Value) -> Result<T> {
        let value = self.bridge.call(command, args)?;
        serde_json::from_value(value)
            .map_err(|e| Error::Malformed(format!("{command} response: {e}")))
    }
}

/// A fresh P-256 key and a self-signed certificate for it. The same
/// shape as the desktop's identity (`src-tauri/src/remote/identity.rs`)
/// with `ClientAuth` in place of `ServerAuth`. Validity is rcgen's
/// default window (1975 to 4096): the desktop pins the fingerprint and
/// checks no dates, and the alternative would make the certificate
/// depend on the phone's clock at pairing time for no gain.
fn generate_session() -> Result<SessionIdentity> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Headstate Companion");
    params.distinguished_name = dn;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let cert = params.self_signed(&key)?;
    Ok(SessionIdentity {
        cert_der: cert.der().to_vec(),
        key_pkcs8: key.serialize_der(),
    })
}

impl DeviceKeys for HeadstateKeys {
    fn generate(&self) -> Result<PublicKeys> {
        // The native `generate` deletes whatever it held first, so a
        // failure part-way leaves either no keys or only signing keys;
        // both read as NotGenerated to `session_identity`, and the next
        // `generate` starts over. Nothing is ever half-trusted.
        let keys: WirePublicKeys = self.call(cmd::GENERATE, Value::Null)?;
        let keys = keys.into_public()?;
        let session = generate_session()?;
        let stored = serde_json::to_value(WireSession::from_identity(&session))
            .expect("two strings serialise");
        let _: Value = self.call(cmd::STORE_SESSION, stored)?;
        log::info!(
            "generated device keys (ML-DSA-65: {})",
            if keys.mldsa_65.is_some() { "yes" } else { "no" }
        );
        Ok(keys)
    }

    fn public_keys(&self) -> Result<PublicKeys> {
        let keys: WirePublicKeys = self.call(cmd::PUBLIC_KEYS, Value::Null)?;
        keys.into_public()
    }

    fn sign(&self, bytes: &[u8]) -> Result<Signatures> {
        let args = serde_json::to_value(SignArgs {
            message: wire::encode(bytes),
            reason: PROMPT_REASON.into(),
        })
        .expect("two strings serialise");
        let sigs: WireSignatures = self.call(cmd::SIGN, args)?;
        sigs.into_signatures()
    }

    fn destroy(&self) -> Result<()> {
        let _: Value = self.call(cmd::DESTROY, Value::Null)?;
        Ok(())
    }

    fn session_identity(&self) -> Result<SessionIdentity> {
        let session: WireSession = self.call(cmd::LOAD_SESSION, Value::Null)?;
        session.into_identity()
    }
}

/// Access to the keys from anything that implements [`Manager`].
pub trait HeadstateKeysExt<R: Runtime> {
    fn headstate_keys(&self) -> &HeadstateKeys;
}

impl<R: Runtime, T: Manager<R>> HeadstateKeysExt<R> for T {
    fn headstate_keys(&self) -> &HeadstateKeys {
        self.state::<HeadstateKeys>().inner()
    }
}

/// Registers the plugin. On iOS and Android this loads the native side;
/// on a desktop host it manages a [`HeadstateKeys`] whose every call is
/// [`Error::Unavailable`], so the app compiles and tests there.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("headstate-keys")
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            let bridge: Box<dyn Bridge> = Box::new(bridge::Native(
                _api.register_android_plugin(PLUGIN_IDENTIFIER, "HeadstateKeysPlugin")?,
            ));
            #[cfg(target_os = "ios")]
            let bridge: Box<dyn Bridge> = Box::new(bridge::Native(
                _api.register_ios_plugin(init_plugin_headstate_keys)?,
            ));
            #[cfg(not(mobile))]
            let bridge: Box<dyn Bridge> = Box::new(bridge::Unavailable);
            app.manage(HeadstateKeys::new(bridge));
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake::{Fake, Tamper};
    use sha2::{Digest, Sha256};

    fn keys(fake: Fake) -> HeadstateKeys {
        HeadstateKeys::new(Box::new(fake))
    }

    /// `tests::canonical_bytes_test_vector` in the desktop's
    /// `src-tauri/src/remote/stepup.rs`, byte for byte: the message a
    /// phone signs for `remove_worktree` with nonce `00 01 .. 0f` at
    /// timestamp 1788566400. 203 bytes.
    const CANONICAL: &[u8] = concat!(
        r#"{"args":{"repoPath":"/home/octocat/src/hello-world","#,
        r#""worktreePath":"/home/octocat/src/hello-world/.worktrees/feature"},"#,
        r#""command":"remove_worktree","nonce":"AAECAwQFBgcICQoLDA0ODw","timestamp":1788566400}"#,
    )
    .as_bytes();
    const CANONICAL_SHA256: &str =
        "ebd1a4f4f78ff1f55f7bf642cc8d72262b6a77ab14164bbf4f95135a6e0f79ff";

    #[test]
    fn the_test_vector_is_the_desktops() {
        assert_eq!(CANONICAL.len(), 203);
        assert_eq!(format!("{:x}", Sha256::digest(CANONICAL)), CANONICAL_SHA256);
    }

    #[test]
    fn before_generate_there_are_no_keys() {
        let k = keys(Fake::new(true));
        assert_eq!(k.public_keys().unwrap_err(), Error::NotGenerated);
        assert_eq!(k.session_identity().unwrap_err(), Error::NotGenerated);
        assert_eq!(k.sign(CANONICAL).unwrap_err(), Error::NotGenerated);
        k.destroy().expect("destroy is idempotent");
    }

    #[test]
    fn generate_returns_the_wire_encodings_and_public_keys_agrees() {
        let k = keys(Fake::new(true));
        let public = k.generate().unwrap();
        assert_eq!(public.ecdsa_p256.len(), ECDSA_P256_LEN);
        assert_eq!(public.ecdsa_p256[0], 0x04);
        assert_eq!(public.mldsa_65.as_ref().map(Vec::len), Some(MLDSA_65_LEN));
        assert_eq!(k.public_keys().unwrap(), public);
    }

    #[test]
    fn without_mldsa_the_option_is_none_everywhere() {
        let k = keys(Fake::new(false));
        let public = k.generate().unwrap();
        assert_eq!(public.mldsa_65, None);
        let sigs = k.sign(CANONICAL).unwrap();
        assert_eq!(sigs.ecdsa.len(), ECDSA_SIG_LEN);
        assert_eq!(sigs.mldsa, None);
    }

    /// The signatures verify the way `stepup.rs` verifies them:
    /// P-256 from the SEC1 key over the SHA256 prehash, raw `r || s`,
    /// and ML-DSA-65 pure with the empty context.
    #[test]
    fn signatures_over_the_test_vector_verify_as_the_desktop_checks_them() {
        let k = keys(Fake::new(true));
        let public = k.generate().unwrap();
        let sigs = k.sign(CANONICAL).unwrap();
        assert_eq!(sigs.ecdsa.len(), ECDSA_SIG_LEN);
        assert_eq!(sigs.mldsa.as_ref().map(Vec::len), Some(MLDSA_SIG_LEN));

        use p256::ecdsa::signature::Verifier;
        let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&public.ecdsa_p256).unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&sigs.ecdsa).unwrap();
        vk.verify(CANONICAL, &sig).unwrap();
        assert!(vk.verify(b"{\"args\":{}}", &sig).is_err());

        use ml_dsa::{EncodedVerifyingKey, MlDsa65, Signature, VerifyingKey};
        let enc =
            EncodedVerifyingKey::<MlDsa65>::try_from(public.mldsa_65.unwrap().as_slice()).unwrap();
        let vk = VerifyingKey::<MlDsa65>::decode(&enc);
        let sig = Signature::<MlDsa65>::try_from(sigs.mldsa.unwrap().as_slice()).unwrap();
        assert!(vk.verify_with_context(CANONICAL, b"", &sig));
        assert!(!vk.verify_with_context(CANONICAL, b"headstate", &sig));
        assert!(!vk.verify_with_context(b"{\"args\":{}}", b"", &sig));
    }

    #[test]
    fn generate_replaces_the_keys() {
        let k = keys(Fake::new(true));
        let first = k.generate().unwrap();
        let second = k.generate().unwrap();
        assert_ne!(first, second);
        assert_eq!(k.public_keys().unwrap(), second);
    }

    #[test]
    fn destroy_forgets_everything() {
        let k = keys(Fake::new(true));
        k.generate().unwrap();
        k.destroy().unwrap();
        assert_eq!(k.public_keys().unwrap_err(), Error::NotGenerated);
        assert_eq!(k.session_identity().unwrap_err(), Error::NotGenerated);
        assert_eq!(k.sign(CANONICAL).unwrap_err(), Error::NotGenerated);
    }

    /// rustls accepts the identity: the key loads through the provider
    /// and its public half matches the certificate's.
    #[test]
    fn the_session_identity_is_a_rustls_client_identity() {
        let k = keys(Fake::new(true));
        k.generate().unwrap();
        let id = k.session_identity().unwrap();
        assert!(!format!("{id:?}").contains("key_pkcs8: ["));

        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let key = provider
            .key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(id.key_pkcs8.clone().into()))
            .unwrap();
        let certified =
            rustls::sign::CertifiedKey::new(vec![CertificateDer::from(id.cert_der.clone())], key);
        certified.keys_match().unwrap();
    }

    #[test]
    fn a_second_session_identity_differs_from_the_first() {
        let k = keys(Fake::new(true));
        k.generate().unwrap();
        let a = k.session_identity().unwrap();
        k.generate().unwrap();
        let b = k.session_identity().unwrap();
        assert_ne!(a.key_pkcs8, b.key_pkcs8);
        assert_ne!(a.cert_der, b.cert_der);
    }

    #[test]
    fn a_dismissed_prompt_is_cancelled_not_a_signature() {
        let k = keys(Fake::new(true).cancel_next_sign());
        k.generate().unwrap();
        assert_eq!(k.sign(CANONICAL).unwrap_err(), Error::Cancelled);
        // The keys are still there; only that one prompt was refused.
        assert!(k.sign(CANONICAL).is_ok());
    }

    #[test]
    fn wrong_length_material_from_the_native_side_is_refused() {
        for (tamper, expect) in [
            (
                Tamper::ShortEcdsaSig,
                "ECDSA signature is 63 bytes; expected 64",
            ),
            (
                Tamper::ShortMldsaSig,
                "ML-DSA signature is 3308 bytes; expected 3309",
            ),
            (
                Tamper::CompressedEcdsaKey,
                "ECDSA public key is 33 bytes; expected 65",
            ),
            (
                Tamper::ShortMldsaKey,
                "ML-DSA public key is 1951 bytes; expected 1952",
            ),
            (Tamper::NotBase64, "ECDSA signature is not base64"),
        ] {
            let k = keys(Fake::new(true).tamper(tamper));
            let result = match tamper {
                Tamper::CompressedEcdsaKey | Tamper::ShortMldsaKey => k.generate().map(|_| ()),
                _ => k.generate().and_then(|_| k.sign(CANONICAL)).map(|_| ()),
            };
            match result {
                Err(Error::Malformed(msg)) => {
                    assert!(msg.starts_with(expect), "{tamper:?}: {msg}")
                }
                other => panic!("{tamper:?}: expected Malformed, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_native_rejection_maps_by_its_code() {
        use error::{codes, from_rejection};
        assert_eq!(
            from_rejection(Some(codes::NOT_GENERATED), "x".into()),
            Error::NotGenerated
        );
        assert_eq!(
            from_rejection(Some(codes::CANCELLED), "x".into()),
            Error::Cancelled
        );
        assert_eq!(
            from_rejection(Some(codes::AUTH_FAILED), "lockout".into()),
            Error::AuthFailed("lockout".into())
        );
        assert_eq!(
            from_rejection(Some(codes::UNAVAILABLE), "no SE".into()),
            Error::Unavailable("no SE".into())
        );
        assert_eq!(
            from_rejection(None, "boom".into()),
            Error::Plugin("boom".into())
        );
        assert_eq!(
            from_rejection(Some("somethingElse"), "boom".into()),
            Error::Plugin("boom".into())
        );
    }

    #[test]
    fn the_desktop_host_reports_unavailable() {
        let k = HeadstateKeys::new(Box::new(bridge::Unavailable));
        assert!(matches!(k.generate(), Err(Error::Unavailable(_))));
        assert!(matches!(k.public_keys(), Err(Error::Unavailable(_))));
        assert!(matches!(k.sign(b"x"), Err(Error::Unavailable(_))));
        assert!(matches!(k.destroy(), Err(Error::Unavailable(_))));
        assert!(matches!(k.session_identity(), Err(Error::Unavailable(_))));
    }

    /// The bridge's JSON shape, pinned: camelCase keys, standard base64
    /// with padding, `null` and absent both meaning "no ML-DSA".
    #[test]
    fn the_wire_shape_is_camel_case_base64_with_optional_mldsa() {
        let with_null: WirePublicKeys =
            serde_json::from_str(r#"{"ecdsaP256":"BAE=","mldsa65":null}"#).unwrap();
        assert_eq!(with_null.mldsa_65, None);
        let absent: WirePublicKeys = serde_json::from_str(r#"{"ecdsaP256":"BAE="}"#).unwrap();
        assert_eq!(absent.mldsa_65, None);
        let sigs = serde_json::to_string(&WireSignatures::from_signatures(&Signatures {
            ecdsa: vec![1, 2, 3],
            mldsa: None,
        }))
        .unwrap();
        assert_eq!(sigs, r#"{"ecdsa":"AQID","mldsa":null}"#);
        let args = serde_json::to_value(SignArgs {
            message: "AQID".into(),
            reason: PROMPT_REASON.into(),
        })
        .unwrap();
        assert_eq!(args["message"], "AQID");
        assert_eq!(args["reason"], PROMPT_REASON);
    }
}
