//! The switch behind the verbose `[diag]` timing log.
//!
//! Added in v3.5.3 to diagnose a slow review query on one machine, and
//! kept as a setting rather than removed: the next "it is slow on my
//! machine" report wants exactly this log, and asking someone to
//! install a special build to produce it is far worse than a checkbox.

use std::sync::atomic::{AtomicBool, Ordering};

/// A process-global flag rather than a value threaded through.
///
/// The call sites span the poll loop, the Tauri commands, and
/// `github::client` -- and the client has no `AppHandle` and no
/// business acquiring one just to decide whether to log. A single
/// atomic read is also cheap enough to sit in a per-request path, which
/// a settings lookup would not be.
///
/// This is deliberately NOT the pattern used for the refused-field
/// count, which was a global counter and had to be replaced because it
/// raced across polls and across tests. The difference is that this is
/// a single write on a settings change and a read everywhere else, with
/// no accumulation to lose.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Apply the user's preference. Called at startup and whenever settings
/// are saved.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Whether `[diag]` lines should be written.
///
/// `Relaxed` is right: nothing else is ordered against this, and a log
/// line landing one request either side of a settings change carries no
/// consequence.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Log a `[diag]` line, but only when diagnostics are on.
///
/// A macro rather than a function so the arguments are not formatted
/// when logging is off -- these lines interpolate elapsed times and
/// counts on every request, and paying that cost for output nobody
/// asked for is the thing the switch exists to avoid.
#[macro_export]
macro_rules! diag {
    ($($arg:tt)*) => {
        if $crate::diag::enabled() {
            log::info!($($arg)*);
        }
    };
}

#[cfg(test)]
mod tests {

    /// Serialises the tests that drive the global diagnostics switch.
    ///
    /// `set_enabled` is process-global and BOTH tests here toggle it, so
    /// under `--test-threads=8` one flips the switch while the other is
    /// asserting on it. The counter noise was already handled with
    /// deltas; the SWITCH itself was not.
    ///
    /// `std::sync::Mutex` is right here -- these are ordinary
    /// synchronous tests with no await in the guarded window. Recovers
    /// from poisoning so a panic in one test fails that test rather than
    /// cascading.
    fn switch_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    use super::*;

    /// Defaults OFF, so a user who never opens Settings never pays for
    /// a diagnosis they did not ask for.
    ///
    /// Asserted on `UiPrefs::default()` rather than on the static: the
    /// static is process-global and other tests in this binary flip it,
    /// so reading it here would be a race, not a guarantee. The
    /// preference is what actually decides the startup value, so it is
    /// the honest thing to pin.
    #[test]
    fn diagnostics_default_to_off() {
        assert!(!crate::poll::UiPrefs::default().diagnostic_logging);
    }

    /// The counting logger, installed at most once for this whole test
    /// binary.
    ///
    /// # Why a shared static rather than a per-test install (#853)
    ///
    /// `log::set_boxed_logger` is process-global and ONE-SHOT. The test
    /// below used to call it and `return` early when it lost the race --
    /// reporting PASS while asserting nothing at all, so the test
    /// guarding the `[diag]` switch could silently check nothing. That is
    /// the failure mode this repo rejects everywhere else
    /// (`check-privacy.sh`: "Abort loudly instead"), and it is worse than
    /// an ordinary skip because the switch it guards is a privacy-adjacent
    /// one: `[diag]` lines are what the user opted out of.
    ///
    /// The race is avoidable rather than merely detectable, which is why
    /// this is a fix and not a loud skip. There is only ever ONE logger
    /// per process, so the test does not need to own the installation --
    /// it needs the installed logger to be a counting one. Installing it
    /// from a `OnceLock` does that: whichever test arrives first installs
    /// it, every later caller gets the same counter back, and no call
    /// ever fails. `set_max_level` is set here too, since a logger that
    /// is installed but filtered out counts nothing.
    ///
    /// Returns the counter so the caller reads deltas off the same
    /// instance it installed.
    fn counting_logger() -> &'static std::sync::atomic::AtomicUsize {
        use std::sync::atomic::AtomicUsize;
        static RECORDS: AtomicUsize = AtomicUsize::new(0);
        static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

        struct Counting;
        impl log::Log for Counting {
            fn enabled(&self, _: &log::Metadata) -> bool {
                true
            }
            fn log(&self, _: &log::Record) {
                RECORDS.fetch_add(1, Ordering::Relaxed);
            }
            fn flush(&self) {}
        }

        INSTALLED.get_or_init(|| {
            // An Err here means something OUTSIDE these tests installed a
            // logger first (a harness, a dependency). That cannot be
            // counted against, and silently passing is the bug being
            // fixed -- so it fails loudly instead of returning a counter
            // that will never move.
            log::set_boxed_logger(Box::new(Counting))
                .expect("a foreign logger is already installed; this test cannot count records");
            log::set_max_level(log::LevelFilter::Info);
        });
        &RECORDS
    }

    /// The macro must actually consult the switch.
    ///
    /// Asserted by installing a REAL logger and counting records.
    /// Counting argument evaluation instead does not work: `log::info!`
    /// has its own level check and skips formatting when no logger is
    /// installed, so an ungated macro and a gated one both evaluate
    /// nothing in a bare unit test -- a version of this test that
    /// counted arguments passed even with the gate deleted.
    ///
    /// No longer returns early when it loses the logger race: see
    /// `counting_logger`, which removes the race instead (#853).
    #[test]
    fn the_macro_writes_nothing_while_off() {
        let records = counting_logger();

        // DELTAS, not absolutes. The logger is process-global and this
        // whole binary shares it, so other tests logging concurrently
        // move the counter under us -- which is exactly how the first
        // version of this test passed alone and failed in the suite.
        // A delta is still a real assertion: what matters is whether
        // THIS macro call produced a record.
        let _guard = switch_lock();
        set_enabled(false);
        let before = records.load(Ordering::Relaxed);
        crate::diag!("[diag] must not be written");
        // The gate is synchronous, so any record from this call has
        // already landed by the time the next line runs.
        let after_off = records.load(Ordering::Relaxed);

        set_enabled(true);
        crate::diag!("[diag] must be written");
        let after_on = records.load(Ordering::Relaxed);
        set_enabled(false);

        // Other tests may have logged in between, so the delta is a
        // LOWER bound on their noise and an exact bound on ours only
        // when nothing else ran. Assert the direction, which holds
        // either way: the on-call must add at least one more than the
        // off-call did.
        assert!(
            after_on - after_off >= 1,
            "a diag line was not written while diagnostics were on"
        );
        assert_eq!(
            after_off - before,
            0,
            "a diag line was written while diagnostics were off"
        );
    }

    #[test]
    fn the_switch_round_trips() {
        let _guard = switch_lock();
        set_enabled(true);
        assert!(enabled());
        set_enabled(false);
        assert!(!enabled());
    }
}
