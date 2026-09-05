use serde::{ser::Serializer, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

/// Why a key operation failed. Every message is safe to show the user
/// and never contains key material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// `generate` has not been called, or `destroy` has. The phone is
    /// not paired.
    #[error("this phone has no device keys; pair with a desktop first")]
    NotGenerated,
    /// The user dismissed the biometric or passcode prompt.
    #[error("the confirmation prompt was cancelled")]
    Cancelled,
    /// The prompt ran and the platform refused: wrong biometric too
    /// many times, lockout, or the key's authorisation is gone (Android
    /// invalidates a key when the device credential is removed).
    #[error("the confirmation failed: {0}")]
    AuthFailed(String),
    /// There is no keystore to hold the keys: the desktop host, a
    /// simulator without a Secure Enclave, or a platform side that
    /// refused to create the ECDSA key. ML-DSA being unavailable is
    /// NOT an error; it is `mldsa_65: None`.
    #[error("hardware-backed keys are not available on this device: {0}")]
    Unavailable(String),
    /// The native side returned something in the wrong shape: bad
    /// base64, or a key or signature of the wrong length. A plugin bug,
    /// not a user-facing condition, but refused rather than forwarded so
    /// the desktop never sees it.
    #[error("the keystore returned malformed data: {0}")]
    Malformed(String),
    /// rcgen could not make the session certificate.
    #[error("could not create the session certificate: {0}")]
    Certificate(String),
    /// Any other failure crossing the bridge.
    #[error("device keys: {0}")]
    Plugin(String),
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

impl From<rcgen::Error> for Error {
    fn from(e: rcgen::Error) -> Self {
        Error::Certificate(e.to_string())
    }
}

/// The `code` values the Swift and Kotlin sides put on a rejection.
/// Anything else, or no code at all, is [`Error::Plugin`]. Only the
/// native bridge and the test fake consult these, so a desktop host
/// build has no reader for them.
#[cfg_attr(not(mobile), allow(dead_code))]
pub(crate) mod codes {
    pub const NOT_GENERATED: &str = "notGenerated";
    pub const CANCELLED: &str = "cancelled";
    pub const AUTH_FAILED: &str = "authFailed";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const MALFORMED: &str = "malformed";
}

/// Maps a native rejection to an [`Error`] by its code.
#[cfg_attr(not(mobile), allow(dead_code))]
pub(crate) fn from_rejection(code: Option<&str>, message: String) -> Error {
    match code {
        Some(codes::NOT_GENERATED) => Error::NotGenerated,
        Some(codes::CANCELLED) => Error::Cancelled,
        Some(codes::AUTH_FAILED) => Error::AuthFailed(message),
        Some(codes::UNAVAILABLE) => Error::Unavailable(message),
        Some(codes::MALFORMED) => Error::Malformed(message),
        _ => Error::Plugin(message),
    }
}
