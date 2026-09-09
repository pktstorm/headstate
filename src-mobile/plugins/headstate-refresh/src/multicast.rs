//! Holding an Android `WifiManager.MulticastLock` around an mDNS browse.
//!
//! # Why this exists
//!
//! Android's Wi-Fi driver drops inbound multicast that is not addressed
//! to this device unless something in the process holds a
//! `MulticastLock`. It is a power optimisation, and it is silent: the
//! socket binds, the browse joins the group, the query goes out, and
//! the replies are filtered before they ever reach userspace. From
//! `src-mobile/src/discovery.rs` the result is indistinguishable from
//! "no desktop on this network" -- which the module doc there
//! explicitly calls a benign `None`. A phone whose desktop had merely
//! changed its DHCP lease was therefore left on stale addresses, with
//! nothing in the log saying why (#610).
//!
//! # Why this plugin rather than a third one
//!
//! A `headstate-multicast` plugin of its own was considered and
//! rejected. `CHANGE_WIFI_MULTICAST_STATE` is already declared in this
//! plugin's `AndroidManifest.xml`, which AGP merges into the app's, and
//! this plugin is already the one that owns Android-side lifecycle
//! (WorkManager). A third plugin would add a third Gradle module, a
//! third permission set, a third Swift package and a third member of
//! the mobile crate's workspace, all to wrap one `WifiManager` call --
//! and it would put the lock in a different place from the permission
//! that makes it legal, which is the shape of the bug #610 is about.
//!
//! # Why a process-global hook and not a plugin handle
//!
//! `discovery::browse` is a free function called from
//! `Client::rediscover`, and `Client` holds no `AppHandle` -- it is
//! built from a stored pairing record in a dozen places, every one of
//! them a desktop-host test. Threading a handle through all of that for
//! one Android-only call would put a mobile concern in every test's
//! constructor and in `Client`'s own shape. Instead [`install`] is
//! called once from the plugin's `setup`, where the handle is already
//! in hand, and [`hold`] reads it. The global is written exactly once
//! at app start and only read afterwards, which is the narrow case
//! `OnceLock` exists for.
//!
//! # Scope: per browse, never the process
//!
//! A held multicast lock keeps the Wi-Fi chip accepting frames it would
//! otherwise drop, and that costs battery for as long as it is held. So
//! the lock is taken per browse and released when the
//! [`MulticastGuard`] drops -- on every exit path, including the early
//! `return None`s inside the browse, a panic unwinding out of the mDNS
//! daemon, and the blocking task being dropped underneath us. Holding
//! it for the process lifetime would be one line shorter and would
//! drain a phone in a pocket, invisibly, forever.
//!
//! # Failure is loud
//!
//! Every path that fails to take the lock logs at `warn`. This whole
//! bug was a silent empty browse that read as "no desktops found"; a
//! fix whose own failure was equally silent would only move the
//! problem one layer down.

use std::sync::OnceLock;

use serde_json::json;

use crate::bridge::Bridge;

/// The native command names for the lock.
///
/// camelCase, NOT snake_case: Kotlin's `PluginHandle` dispatches on the
/// literal `@Command` method name (`commands[method.name]`), so these
/// are Kotlin method names. `headstate-keys`' `wire::cmd` records the
/// same rule; this plugin's existing `register` and `complete` happen
/// to be single words and so never showed it.
pub mod cmd {
    /// Rust -> native: `{"tag": "<string>"}`. Takes the lock.
    pub const ACQUIRE_MULTICAST: &str = "acquireMulticast";
    /// Rust -> native, no arguments. Releases it.
    pub const RELEASE_MULTICAST: &str = "releaseMulticast";
}

/// The tag Android shows for this lock in `dumpsys wifi`. Diagnostic
/// only; the platform keys nothing on it.
const LOCK_TAG: &str = "headstate-mdns";

/// Set once by [`install`] from the plugin's `setup`. Empty until then,
/// and forever on a platform where there is no lock to take.
static HOLDER: OnceLock<Box<dyn Bridge>> = OnceLock::new();

/// Make [`hold`] able to take a real lock.
///
/// Called once, from `init`'s `setup`, and only on Android: iOS's
/// multicast gate is the `NSLocalNetworkUsageDescription` /
/// `NSBonjourServiceTypes` pair in `Info.ios.plist` and there is
/// nothing to acquire at runtime, and a desktop host has no such
/// concept at all. A second call is logged and ignored rather than
/// panicking -- a duplicated `init` must not take the app down over a
/// battery optimisation.
pub fn install(bridge: Box<dyn Bridge>) {
    if HOLDER.set(bridge).is_err() {
        log::warn!("headstate-refresh: the multicast lock holder was already installed");
    }
}

/// Held for as long as a browse needs inbound multicast. Releasing is
/// the `Drop`, so no exit path can skip it.
pub struct MulticastGuard {
    /// `None` when nothing was taken: no holder is installed, or the
    /// acquire failed. Dropping such a guard releases nothing, so a
    /// failed acquire cannot leave the native side unbalanced.
    holder: Option<&'static dyn Bridge>,
}

impl Drop for MulticastGuard {
    fn drop(&mut self) {
        let Some(holder) = self.holder else {
            return;
        };
        match holder.call(cmd::RELEASE_MULTICAST, json!({})) {
            Ok(()) => log::debug!("headstate-refresh: multicast lock released"),
            // Loud: a lock that will not release is a battery drain the
            // user cannot see, and cannot stop short of killing the app.
            Err(e) => log::warn!("headstate-refresh: could not release the multicast lock: {e}"),
        }
    }
}

/// Take the multicast lock for the life of the returned guard.
///
/// Always returns a guard, never an error. On iOS and on a desktop host
/// there is nothing to hold and the guard is inert. On Android a browse
/// that could not take the lock is still worth attempting -- a device
/// on Ethernet, or one whose driver does not filter, finds the desktop
/// anyway -- so the failure does not abort the browse. What must never
/// happen is failing quietly, so it is logged at `warn` with the reason.
#[must_use = "the lock is released as soon as the guard is dropped"]
pub fn hold() -> MulticastGuard {
    let Some(holder) = HOLDER.get() else {
        // Not a warning: this is every non-Android platform, plus every
        // desktop `cargo test`. Nothing is wrong.
        return MulticastGuard { holder: None };
    };
    acquire(holder.as_ref())
}

/// The body of [`hold`], over an explicit holder so the tests can drive
/// it: `HOLDER` is a write-once global and a test that installed into
/// it could not then test the other cases.
fn acquire(holder: &'static dyn Bridge) -> MulticastGuard {
    match holder.call(cmd::ACQUIRE_MULTICAST, json!({ "tag": LOCK_TAG })) {
        Ok(()) => {
            log::debug!("headstate-refresh: multicast lock acquired");
            MulticastGuard {
                holder: Some(holder),
            }
        }
        Err(e) => {
            // The exact failure #610 is about, made visible. Without
            // this line a browse that finds nothing on Android reads
            // identically to a LAN with no desktop on it.
            log::warn!(
                "headstate-refresh: could not take the Wi-Fi multicast lock ({e}); \
                 an mDNS browse may find nothing even with the desktop on this LAN"
            );
            MulticastGuard { holder: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;

    /// Records every call, and can be made to fail the acquire.
    struct Recorder {
        calls: Mutex<Vec<(String, Value)>>,
        acquire_fails: bool,
    }

    impl Bridge for Recorder {
        fn available(&self) -> bool {
            true
        }
        fn call(&self, command: &str, args: Value) -> Result<(), String> {
            self.calls.lock().unwrap().push((command.to_string(), args));
            if self.acquire_fails && command == cmd::ACQUIRE_MULTICAST {
                return Err("no CHANGE_WIFI_MULTICAST_STATE".into());
            }
            Ok(())
        }
    }

    /// `acquire` takes a `&'static dyn Bridge` because the real holder
    /// is a `OnceLock` that lives for the process; a leaked box is the
    /// cheapest way for a test to be equally long-lived.
    fn recorder(acquire_fails: bool) -> &'static Recorder {
        Box::leak(Box::new(Recorder {
            calls: Mutex::new(Vec::new()),
            acquire_fails,
        }))
    }

    fn commands(rec: &Recorder) -> Vec<String> {
        rec.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(c, _)| c.clone())
            .collect()
    }

    #[test]
    fn a_guard_acquires_on_creation_and_releases_on_drop() {
        let rec = recorder(false);
        {
            let _guard = acquire(rec);
            assert_eq!(
                rec.calls.lock().unwrap().as_slice(),
                [(
                    cmd::ACQUIRE_MULTICAST.to_string(),
                    json!({ "tag": LOCK_TAG })
                )],
                "taken, with the tag, and not yet released"
            );
        }
        assert_eq!(
            commands(rec),
            [cmd::ACQUIRE_MULTICAST, cmd::RELEASE_MULTICAST],
            "dropping the guard releases exactly once"
        );
    }

    /// The point of the `Drop`: `browse_with`'s early `return None`s
    /// must not leave the lock held.
    #[test]
    fn an_early_return_still_releases() {
        let rec = recorder(false);
        fn browse_that_gives_up(holder: &'static dyn Bridge) -> Option<()> {
            let _guard = acquire(holder);
            None
        }
        assert_eq!(browse_that_gives_up(rec), None);
        assert_eq!(
            commands(rec),
            [cmd::ACQUIRE_MULTICAST, cmd::RELEASE_MULTICAST]
        );
    }

    /// A panic unwinding out of the mDNS daemon is an exit path too.
    #[test]
    fn a_panic_still_releases() {
        let rec = recorder(false);
        let result = std::panic::catch_unwind(|| {
            let _guard = acquire(rec);
            panic!("the daemon thread died");
        });
        assert!(result.is_err());
        assert_eq!(
            commands(rec),
            [cmd::ACQUIRE_MULTICAST, cmd::RELEASE_MULTICAST]
        );
    }

    /// A failed acquire must not release a lock it never took: the
    /// native side does not reference-count (see the Kotlin plugin), so
    /// an unmatched release would throw there.
    #[test]
    fn a_failed_acquire_releases_nothing() {
        let rec = recorder(true);
        {
            let _guard = acquire(rec);
        }
        assert_eq!(
            commands(rec),
            [cmd::ACQUIRE_MULTICAST],
            "only the failed acquire"
        );
    }

    /// With no holder installed -- iOS, or a desktop `cargo test` --
    /// `hold` is inert and calls nothing.
    #[test]
    fn no_holder_is_an_inert_guard() {
        assert!(
            hold().holder.is_none(),
            "no holder is installed in a desktop test"
        );
    }

    /// These MUST equal the Kotlin `@Command` method names: Tauri's
    /// Android `PluginHandle` looks a command up by `method.name` with
    /// no case conversion, so a snake_case spelling here would be an
    /// `InvalidCommandException` at runtime and nothing at compile
    /// time -- which on this code path would look exactly like the
    /// silent empty browse #610 is about.
    #[test]
    fn the_command_names_are_pinned_to_the_kotlin_method_names() {
        assert_eq!(cmd::ACQUIRE_MULTICAST, "acquireMulticast");
        assert_eq!(cmd::RELEASE_MULTICAST, "releaseMulticast");
    }
}
