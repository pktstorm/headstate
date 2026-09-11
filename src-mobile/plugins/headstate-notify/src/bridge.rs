//! The seam between the Rust core and the platform's notification API.
//!
//! Same shape as `headstate-refresh`'s bridge and for the same reason:
//! [`Native`] is the real thing over Tauri's mobile plugin handle,
//! [`Unavailable`] is what every non-iOS target gets -- so `cargo test`
//! on a laptop compiles and posts nothing -- and the fake in the tests
//! is the third implementation, under `cfg(test)` only.
//!
//! The one difference is that this bridge RETURNS a value. The refresh
//! plugin's native commands are fire-and-forget (`register`,
//! `complete`), so its `call` discards the reply; `permission` and
//! `requestPermission` are questions, and a bridge that could not carry
//! an answer back would force the permission gate into the native side
//! where it could not be tested.
//!
//! The reply crosses as a JSON STRING rather than a typed value, which
//! looks like an extra hop and is deliberate: `run_mobile_plugin` is
//! generic over `DeserializeOwned`, and a trait with a generic method is
//! not object-safe -- so `Box<dyn Bridge>` would not exist. A string is
//! the narrowest thing that carries any reply, and [`Bridge::call`]
//! above it is a non-generic convenience that does the decode, so no
//! caller writes `serde_json::from_str` by hand.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Error;

pub trait Bridge: Send + Sync {
    /// Run a native command and return its reply as JSON text.
    fn call_json(&self, command: &str, args: Value) -> Result<String, Error>;
}

impl dyn Bridge {
    /// [`call_json`], decoded.
    ///
    /// A decode failure is [`Error::Native`] rather than a panic: a
    /// native side that answered with the wrong shape is a plugin bug,
    /// and the place it must surface is the log of the window it broke
    /// -- not a crash inside a background task iOS will simply record
    /// as a termination.
    ///
    /// [`call_json`]: Bridge::call_json
    pub fn call<T: DeserializeOwned>(&self, command: &str, args: Value) -> Result<T, Error> {
        let text = self.call_json(command, args)?;
        serde_json::from_str(&text)
            .map_err(|e| Error::Native(format!("`{command}` answered unexpectedly: {e}")))
    }
}

/// A platform with no local-notification bridge: the desktop host, and
/// Android (see `build.rs` on why Android is absent by decision).
///
/// Every call is [`Error::Unavailable`], never a silent success. A
/// bridge that accepted and dropped would make "notifications do not
/// work on this platform" indistinguishable from "there was nothing to
/// say", which is the failure mode that takes longest to notice.
pub struct Unavailable;

impl Bridge for Unavailable {
    fn call_json(&self, _command: &str, _args: Value) -> Result<String, Error> {
        Err(Error::Unavailable)
    }
}

#[cfg(target_os = "ios")]
pub struct Native<R: tauri::Runtime>(pub tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "ios")]
impl<R: tauri::Runtime> Bridge for Native<R> {
    fn call_json(&self, command: &str, args: Value) -> Result<String, Error> {
        // Decoded as `Value` and re-serialised rather than handed
        // straight through: `run_mobile_plugin` is generic and this
        // method is not, so the concrete type has to be chosen here.
        // `Value` is the one that accepts any reply the native side can
        // produce, and the caller's `call` is what gives it a type.
        self.0
            .run_mobile_plugin::<Value>(command, args)
            .map(|v| v.to_string())
            .map_err(|e| Error::Native(e.to_string()))
    }
}
