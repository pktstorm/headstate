//! A software stand-in for the Swift and Kotlin sides, for tests.
//!
//! It speaks the same JSON as the native plugins (`wire.rs`), so every
//! test drives the real decode-and-validate path in `lib.rs`, and it
//! signs with the same crates the desktop verifies with. What it does
//! NOT do is prompt: the "biometric" is a flag that makes the next
//! `sign` reject with the `cancelled` code, the way a dismissed Face ID
//! sheet does.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use ml_dsa::{MlDsa65, Seed};
use p256::ecdsa::signature::Signer;
use serde_json::{json, Value};

use crate::bridge::Bridge;
use crate::error::{codes, from_rejection};
use crate::wire::{cmd, SignArgs, WirePublicKeys, WireSession, WireSignatures};
use crate::{PublicKeys, Result, SessionIdentity, Signatures};

/// Ways the fake can misbehave, to prove the Rust side refuses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tamper {
    ShortEcdsaSig,
    ShortMldsaSig,
    CompressedEcdsaKey,
    ShortMldsaKey,
    NotBase64,
}

struct Held {
    ecdsa: p256::ecdsa::SigningKey,
    mldsa: Option<ml_dsa::SigningKey<MlDsa65>>,
    session: Option<SessionIdentity>,
}

pub struct Fake {
    with_mldsa: bool,
    held: Mutex<Option<Held>>,
    /// Bumped per `generate` so each key set differs, without an RNG.
    seed: AtomicU8,
    cancel_next: Mutex<bool>,
    tamper: Option<Tamper>,
}

impl Fake {
    pub fn new(with_mldsa: bool) -> Self {
        Self {
            with_mldsa,
            held: Mutex::new(None),
            seed: AtomicU8::new(1),
            cancel_next: Mutex::new(false),
            tamper: None,
        }
    }

    pub fn cancel_next_sign(self) -> Self {
        *self.cancel_next.lock().unwrap() = true;
        self
    }

    pub fn tamper(mut self, t: Tamper) -> Self {
        self.tamper = Some(t);
        self
    }

    fn reject(code: &str, message: &str) -> Result<Value> {
        Err(from_rejection(Some(code), message.into()))
    }

    fn public_keys(held: &Held) -> PublicKeys {
        PublicKeys {
            ecdsa_p256: held
                .ecdsa
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
                .to_vec(),
            mldsa_65: held
                .mldsa
                .as_ref()
                .map(|k| k.expanded_key().verifying_key().encode().to_vec()),
        }
    }

    fn generate(&self) -> Result<Value> {
        let seed = self.seed.fetch_add(1, Ordering::Relaxed);
        let ecdsa = p256::ecdsa::SigningKey::from_bytes(&[seed; 32].into()).unwrap();
        let mldsa = self
            .with_mldsa
            .then(|| ml_dsa::SigningKey::<MlDsa65>::from_seed(&Seed::from([seed; 32])));
        let held = Held {
            ecdsa,
            mldsa,
            session: None,
        };
        let mut wire = WirePublicKeys::from_public(&Self::public_keys(&held));
        match self.tamper {
            Some(Tamper::CompressedEcdsaKey) => {
                let compressed = held.ecdsa.verifying_key().to_encoded_point(true);
                wire.ecdsa_p256 = crate::wire::encode(compressed.as_bytes());
            }
            Some(Tamper::ShortMldsaKey) => {
                let mut k = held
                    .mldsa
                    .as_ref()
                    .unwrap()
                    .expanded_key()
                    .verifying_key()
                    .encode()
                    .to_vec();
                k.pop();
                wire.mldsa_65 = Some(crate::wire::encode(&k));
            }
            _ => {}
        }
        *self.held.lock().unwrap() = Some(held);
        Ok(serde_json::to_value(wire).unwrap())
    }

    fn sign(&self, args: Value) -> Result<Value> {
        let args: SignArgs = serde_json::from_value(args).unwrap();
        assert!(!args.reason.is_empty(), "the prompt needs a reason");
        let guard = self.held.lock().unwrap();
        let Some(held) = guard.as_ref() else {
            return Self::reject(codes::NOT_GENERATED, "no keys");
        };
        if std::mem::take(&mut *self.cancel_next.lock().unwrap()) {
            return Self::reject(codes::CANCELLED, "user cancelled");
        }
        let message =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &args.message)
                .unwrap();
        let ecdsa: p256::ecdsa::Signature = held.ecdsa.sign(&message);
        let mut sigs = Signatures {
            ecdsa: ecdsa.to_bytes().to_vec(),
            mldsa: held.mldsa.as_ref().map(|k| {
                k.expanded_key()
                    .sign_deterministic(&message, b"")
                    .unwrap()
                    .encode()
                    .to_vec()
            }),
        };
        match self.tamper {
            Some(Tamper::ShortEcdsaSig) => {
                sigs.ecdsa.pop();
            }
            Some(Tamper::ShortMldsaSig) => {
                sigs.mldsa.as_mut().unwrap().pop();
            }
            _ => {}
        }
        let mut wire = WireSignatures::from_signatures(&sigs);
        if self.tamper == Some(Tamper::NotBase64) {
            wire.ecdsa = "not base64!".into();
        }
        Ok(serde_json::to_value(wire).unwrap())
    }
}

impl Bridge for Fake {
    fn call(&self, command: &str, args: Value) -> Result<Value> {
        match command {
            cmd::GENERATE => self.generate(),
            cmd::PUBLIC_KEYS => match self.held.lock().unwrap().as_ref() {
                Some(held) => Ok(serde_json::to_value(WirePublicKeys::from_public(
                    &Self::public_keys(held),
                ))
                .unwrap()),
                None => Self::reject(codes::NOT_GENERATED, "no keys"),
            },
            cmd::SIGN => self.sign(args),
            cmd::DESTROY => {
                *self.held.lock().unwrap() = None;
                Ok(Value::Null)
            }
            cmd::STORE_SESSION => {
                let session: WireSession = serde_json::from_value(args).unwrap();
                let mut guard = self.held.lock().unwrap();
                let held = guard.as_mut().expect("storeSession follows generate");
                held.session = Some(session.into_identity().unwrap());
                Ok(Value::Null)
            }
            cmd::LOAD_SESSION => match self.held.lock().unwrap().as_ref() {
                Some(Held {
                    session: Some(session),
                    ..
                }) => Ok(serde_json::to_value(WireSession::from_identity(session)).unwrap()),
                _ => Self::reject(codes::NOT_GENERATED, "no session"),
            },
            other => Ok(json!({ "unknownCommand": other })),
        }
    }
}
