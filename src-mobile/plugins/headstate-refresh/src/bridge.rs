//! The seam between the Rust core and the OS scheduler that grants the
//! windows.
//!
//! [`Bridge`] is one method: run a named native command with a JSON
//! payload. [`Native`] is the real thing over Tauri's mobile plugin
//! handle; [`Unavailable`] is what the desktop host gets, so `cargo
//! test` on a laptop compiles and nothing is ever scheduled there; the
//! recorder in the tests is the third implementation and exists only
//! under `cfg(test)`.

use serde_json::Value;

pub trait Bridge: Send + Sync {
    /// Whether there is a native side to call at all.
    fn available(&self) -> bool;
    fn call(&self, command: &str, args: Value) -> Result<(), String>;
}

/// A platform with no background scheduler behind this plugin: the
/// desktop host.
#[cfg(any(not(mobile), test))]
pub struct Unavailable;

#[cfg(any(not(mobile), test))]
impl Bridge for Unavailable {
    fn available(&self) -> bool {
        false
    }
    fn call(&self, _command: &str, _args: Value) -> Result<(), String> {
        Err("the refresh plugin has no native side on this platform".into())
    }
}

#[cfg(mobile)]
pub struct Native<R: tauri::Runtime>(pub tauri::plugin::PluginHandle<R>);

#[cfg(mobile)]
impl<R: tauri::Runtime> Bridge for Native<R> {
    fn available(&self) -> bool {
        true
    }
    fn call(&self, command: &str, args: Value) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<Value>(command, args)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
