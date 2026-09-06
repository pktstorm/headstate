//! The seam between the Rust API and whatever holds the keys.
//!
//! [`Bridge`] is one method: run a named native command with a JSON
//! payload and get JSON back. [`Native`] is the real thing over Tauri's
//! mobile plugin handle; [`Unavailable`] is what the desktop host gets,
//! so `cargo test` on a laptop compiles and every call reports the
//! honest error; the fake in `fake.rs` is the third implementation and
//! exists only under `cfg(test)`.

use serde_json::Value;

use crate::{Error, Result};

pub trait Bridge: Send + Sync {
    fn call(&self, command: &str, args: Value) -> Result<Value>;
}

/// A platform with no keystore behind this plugin: the desktop host.
#[cfg(any(not(mobile), test))]
pub struct Unavailable;

#[cfg(any(not(mobile), test))]
impl Bridge for Unavailable {
    fn call(&self, _command: &str, _args: Value) -> Result<Value> {
        Err(Error::Unavailable(
            "the keys plugin has no native side on this platform".into(),
        ))
    }
}

#[cfg(mobile)]
pub struct Native<R: tauri::Runtime>(pub tauri::plugin::PluginHandle<R>);

#[cfg(mobile)]
impl<R: tauri::Runtime> Bridge for Native<R> {
    fn call(&self, command: &str, args: Value) -> Result<Value> {
        use tauri::plugin::mobile::PluginInvokeError;
        self.0
            .run_mobile_plugin::<Value>(command, args)
            .map_err(|e| match e {
                PluginInvokeError::InvokeRejected(r) => {
                    crate::error::from_rejection(r.code.as_deref(), r.message.unwrap_or_default())
                }
                other => Error::Plugin(other.to_string()),
            })
    }
}
