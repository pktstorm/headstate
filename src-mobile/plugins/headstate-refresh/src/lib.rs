//! `tauri-plugin-headstate-refresh`: the phone's opportunistic
//! background refresh.
//!
//! "What the phone does when it is in the background" in
//! docs/superpowers/specs/2026-09-05-mobile-companion-design.md is
//! *nothing*, by decision: iOS suspends the app soon after it leaves the
//! foreground and the event stream dies with it; the phone catches up on
//! resume. The one cheap improvement the spec allows is this plugin. Each
//! OS grants a short window every so often -- a `BGAppRefreshTask` on
//! iOS, a `PeriodicWorkRequest` on Android, fifteen minutes apart at the
//! soonest -- and inside it the app may fetch `/v1/hello` and the cached
//! PR list so it opens fresh. **It is not a stream and must not be
//! designed as one.** Nothing here opens `/v1/events`, retries inside a
//! window, or asks the OS for more time.
//!
//! # Shape
//!
//! The plugin owns the OS side and knows nothing about the desktop. What
//! to do in a window is a [`Refresher`] the app puts in Tauri state as
//! `Arc<dyn Refresher>`; [`refresh_now`] finds it and runs it. Until the
//! client (#514) is wired the app installs [`NoopRefresher`].
//!
//! # Direction of calls
//!
//! Tauri's mobile plugins are called *from* Rust; the way a native side
//! calls *into* Rust is a `Channel` handed to it as a command argument.
//! At plugin load Rust calls the native `register` command with one
//! channel. When the OS grants a window the native side sends
//! `{"kind":"begin","id":N}` on it; Rust spawns the refresh and, when
//! that finishes, calls the native `complete` command with
//! `{"id":N,"success":bool}`, which ends the OS task. If the OS takes the
//! window back first (iOS's expiration handler, or the Android worker's
//! own deadline) the native side sends `{"kind":"expire","id":N}`, Rust
//! aborts the refresh, and no `complete` follows.
//!
//! On a desktop host there is no native side: `init` manages nothing but
//! an inert core, so the app compiles and tests there.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    plugin::{Builder, TauriPlugin},
    AppHandle, Manager, Runtime,
};

mod bridge;

use bridge::Bridge;

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.pktstorm.headstate.refresh";

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_headstate_refresh);

/// The task identifier iOS knows the refresh by. Must appear in the
/// app's `BGTaskSchedulerPermittedIdentifiers` (src-mobile/Info.ios.plist)
/// or `BGTaskScheduler` refuses to schedule it. The Android unique work
/// name is the same string.
pub const TASK_IDENTIFIER: &str = "com.pktstorm.headstate.companion.refresh";

/// A boxed refresh: one window's worth of work.
pub type RefreshFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// What the app does when it is granted a background window. The app
/// manages one as `Arc<dyn Refresher>`; `src-mobile/src/background.rs`
/// is the implementation.
pub trait Refresher: Send + Sync {
    /// One attempt. Any error is "give up quietly": it is logged at
    /// debug and the window is released; nothing retries inside it.
    fn refresh(&self) -> RefreshFuture;
}

/// Does nothing and succeeds. The app's refresher until the client is
/// wired, and the behaviour when no refresher is managed at all.
pub struct NoopRefresher;

impl Refresher for NoopRefresher {
    fn refresh(&self) -> RefreshFuture {
        Box::pin(async { Ok(()) })
    }
}

/// The native command names. Same on both platforms.
pub mod cmd {
    /// Rust -> native, once at load: `{"channel": <Channel>}`.
    pub const REGISTER: &str = "register";
    /// Rust -> native, once per window: `{"id": N, "success": bool}`.
    pub const COMPLETE: &str = "complete";
}

/// What the native side sends on the channel. `kind` is the tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Message {
    /// The OS granted a window.
    Begin { id: u64 },
    /// The OS took the window back before the refresh finished.
    Expire { id: u64 },
}

/// The `complete` command's arguments.
#[derive(Debug, Serialize)]
struct Complete {
    id: u64,
    success: bool,
}

/// Runs the managed [`Refresher`], or nothing when the app has not
/// installed one. This is what a window does.
pub async fn refresh_now<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let refresher = app
        .try_state::<Arc<dyn Refresher>>()
        .map(|s| s.inner().clone());
    run_refresher(refresher).await
}

async fn run_refresher(refresher: Option<Arc<dyn Refresher>>) -> Result<(), String> {
    match refresher {
        Some(r) => r.refresh().await,
        None => {
            log::debug!("headstate-refresh: no refresher installed; nothing to do");
            Ok(())
        }
    }
}

/// The windows in flight and the bridge to end them. Managed in Tauri
/// state by [`init`] so it lives as long as the app: on a phone the
/// channel closure also holds the core, but on a desktop host this is
/// the only owner.
pub struct Scheduler {
    #[allow(dead_code)]
    core: Arc<Core>,
}

struct Core {
    bridge: Box<dyn Bridge>,
    /// Window id -> the refresh running in it. An id that is not here
    /// has finished or expired.
    open: Mutex<HashMap<u64, tauri::async_runtime::JoinHandle<()>>>,
    /// Keeps the channel alive for the life of the plugin. Tauri also
    /// holds a clone in its channel table, but that is its business.
    channel: Mutex<Option<tauri::ipc::Channel<Value>>>,
}

impl Core {
    fn new(bridge: Box<dyn Bridge>) -> Self {
        Self {
            bridge,
            open: Mutex::new(HashMap::new()),
            channel: Mutex::new(None),
        }
    }

    /// Run the refresh `make` builds in window `id`; report to the
    /// native side when it ends, unless the window expired first.
    fn begin(self: &Arc<Self>, id: u64, make: impl FnOnce() -> RefreshFuture) {
        // The lock is held across the spawn so `finish` cannot run
        // before the handle is in the map: a refresh that returns at
        // once would otherwise report on a window it never joined.
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.contains_key(&id) {
            log::warn!("headstate-refresh: window {id} began twice; ignoring the second");
            return;
        }
        log::debug!("headstate-refresh: window {id} granted");
        let refresh = make();
        let core = self.clone();
        let handle = tauri::async_runtime::spawn(async move {
            let result = refresh.await;
            core.finish(id, result);
        });
        open.insert(id, handle);
    }

    /// The OS took window `id` back: stop the refresh and forget it. The
    /// native side has already ended the task, so nothing is reported.
    fn expire(&self, id: u64) {
        let handle = self
            .open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        match handle {
            Some(h) => {
                h.abort();
                log::debug!("headstate-refresh: window {id} expired; refresh aborted");
            }
            None => log::debug!("headstate-refresh: window {id} expired after it finished"),
        }
    }

    fn finish(&self, id: u64, result: Result<(), String>) {
        let was_open = self
            .open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
            .is_some();
        if !was_open {
            log::debug!("headstate-refresh: window {id} finished after it expired");
            return;
        }
        let success = match result {
            Ok(()) => {
                log::debug!("headstate-refresh: window {id} refreshed");
                true
            }
            Err(e) => {
                // "Give up quietly": debug, not warn. The desktop being
                // away is the expected case on a phone in a pocket.
                log::debug!("headstate-refresh: window {id} gave up: {e}");
                false
            }
        };
        let args = serde_json::to_value(Complete { id, success }).expect("two scalars serialise");
        if let Err(e) = self.bridge.call(cmd::COMPLETE, args) {
            log::warn!("headstate-refresh: could not end window {id}: {e}");
        }
    }

    fn on_message(self: &Arc<Self>, msg: Message, make: impl FnOnce() -> RefreshFuture) {
        match msg {
            Message::Begin { id } => self.begin(id, make),
            Message::Expire { id } => self.expire(id),
        }
    }

    /// Hand the native side the channel windows arrive on. On a desktop
    /// host there is no native side and nothing is registered.
    fn register<R: Runtime>(self: &Arc<Self>, app: AppHandle<R>) {
        use tauri::ipc::{Channel, InvokeResponseBody};
        if !self.bridge.available() {
            log::debug!("headstate-refresh: no background scheduler on this platform");
            return;
        }
        let core = self.clone();
        let channel = Channel::<Value>::new(move |body: InvokeResponseBody| {
            match body.deserialize::<Message>() {
                Ok(msg) => {
                    let app = app.clone();
                    core.on_message(msg, move || {
                        Box::pin(async move { refresh_now(&app).await })
                    });
                }
                Err(e) => log::warn!("headstate-refresh: unreadable message from the OS side: {e}"),
            }
            Ok(())
        });
        let result = self
            .bridge
            .call(cmd::REGISTER, json!({ "channel": channel }));
        *self.channel.lock().unwrap_or_else(|e| e.into_inner()) = Some(channel);
        match result {
            Ok(()) => log::info!("headstate-refresh: background refresh registered"),
            // The app must start regardless: a phone without background
            // refresh still catches up on resume.
            Err(e) => log::warn!("headstate-refresh: could not register with the OS: {e}"),
        }
    }
}

/// Registers the plugin. On iOS and Android this loads the native side,
/// which registers the OS task and hands Rust the channel the windows
/// arrive on; on a desktop host it manages an inert [`Scheduler`], so
/// the app compiles and tests there.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("headstate-refresh")
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            let bridge: Box<dyn Bridge> = Box::new(bridge::Native(
                _api.register_android_plugin(PLUGIN_IDENTIFIER, "HeadstateRefreshPlugin")?,
            ));
            #[cfg(target_os = "ios")]
            let bridge: Box<dyn Bridge> = Box::new(bridge::Native(
                _api.register_ios_plugin(init_plugin_headstate_refresh)?,
            ));
            #[cfg(not(mobile))]
            let bridge: Box<dyn Bridge> = Box::new(bridge::Unavailable);
            let core = Arc::new(Core::new(bridge));
            core.register(app.clone());
            app.manage(Scheduler { core });
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// Collects every native call.
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl Recorder {
        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Bridge for Recorder {
        fn available(&self) -> bool {
            true
        }
        fn call(&self, command: &str, args: Value) -> Result<(), String> {
            self.calls.lock().unwrap().push((command.to_string(), args));
            Ok(())
        }
    }

    fn core() -> (Arc<Recorder>, Arc<Core>) {
        let rec = Arc::new(Recorder::default());
        let core = Arc::new(Core::new(Box::new(RecorderRef(rec.clone()))));
        (rec, core)
    }

    /// `Box<dyn Bridge>` wants ownership; the test wants to keep reading.
    struct RecorderRef(Arc<Recorder>);
    impl Bridge for RecorderRef {
        fn available(&self) -> bool {
            true
        }
        fn call(&self, command: &str, args: Value) -> Result<(), String> {
            self.0.call(command, args)
        }
    }

    fn until(mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "condition not met within 10s"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn open_windows(core: &Core) -> usize {
        core.open.lock().unwrap().len()
    }

    /// Counts calls; the app's `background.rs` tests the real one.
    struct Counting(AtomicUsize);
    impl Refresher for Counting {
        fn refresh(&self) -> RefreshFuture {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn the_wire_shapes_are_pinned() {
        assert_eq!(
            serde_json::from_str::<Message>(r#"{"kind":"begin","id":7}"#).unwrap(),
            Message::Begin { id: 7 }
        );
        assert_eq!(
            serde_json::from_str::<Message>(r#"{"kind":"expire","id":7}"#).unwrap(),
            Message::Expire { id: 7 }
        );
        assert!(serde_json::from_str::<Message>(r#"{"kind":"stream","id":7}"#).is_err());
        assert!(serde_json::from_str::<Message>(r#"{"id":7}"#).is_err());
        assert_eq!(
            serde_json::to_value(Complete {
                id: 7,
                success: true
            })
            .unwrap(),
            json!({"id": 7, "success": true})
        );
        assert_eq!(cmd::REGISTER, "register");
        assert_eq!(cmd::COMPLETE, "complete");
        assert_eq!(TASK_IDENTIFIER, "com.pktstorm.headstate.companion.refresh");
    }

    #[test]
    fn a_window_runs_the_refresh_once_and_reports_success() {
        let (rec, core) = core();
        let refresher = Arc::new(Counting(AtomicUsize::new(0)));
        let r = refresher.clone();
        core.on_message(Message::Begin { id: 1 }, move || {
            Box::pin(async move { run_refresher(Some(r)).await })
        });
        until(|| !rec.calls().is_empty());
        assert_eq!(
            rec.calls(),
            vec![("complete".to_string(), json!({"id": 1, "success": true}))]
        );
        assert_eq!(
            refresher.0.load(Ordering::SeqCst),
            1,
            "no retry in the window"
        );
        assert_eq!(open_windows(&core), 0);
    }

    #[test]
    fn a_refresh_that_gives_up_reports_failure_and_nothing_else() {
        let (rec, core) = core();
        core.on_message(Message::Begin { id: 2 }, || {
            Box::pin(async { Err("desktop unreachable".to_string()) })
        });
        until(|| !rec.calls().is_empty());
        assert_eq!(
            rec.calls(),
            vec![("complete".to_string(), json!({"id": 2, "success": false}))]
        );
        assert_eq!(open_windows(&core), 0);
    }

    #[test]
    fn an_expired_window_aborts_the_refresh_and_reports_nothing() {
        let (rec, core) = core();
        let started = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        struct OnDrop(Arc<AtomicBool>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let (s, d) = (started.clone(), dropped.clone());
        core.on_message(Message::Begin { id: 3 }, move || {
            Box::pin(async move {
                let _guard = OnDrop(d);
                s.notify_one();
                // Never finishes on its own: the OS must take it back.
                std::future::pending::<()>().await;
                Ok(())
            })
        });
        tauri::async_runtime::block_on(started.notified());
        assert_eq!(open_windows(&core), 1);
        core.on_message(Message::Expire { id: 3 }, || {
            unreachable!("expire does not refresh")
        });
        until(|| dropped.load(Ordering::SeqCst));
        assert_eq!(open_windows(&core), 0);
        std::thread::sleep(Duration::from_millis(50));
        assert!(rec.calls().is_empty(), "no complete after an expiry");
    }

    #[test]
    fn a_window_that_expires_after_finishing_is_a_no_op() {
        let (rec, core) = core();
        core.on_message(Message::Begin { id: 4 }, || Box::pin(async { Ok(()) }));
        until(|| !rec.calls().is_empty());
        core.on_message(Message::Expire { id: 4 }, || unreachable!());
        core.on_message(Message::Expire { id: 99 }, || unreachable!());
        assert_eq!(rec.calls().len(), 1);
    }

    #[test]
    fn a_duplicate_begin_is_ignored() {
        let (rec, core) = core();
        let gate = Arc::new(tokio::sync::Notify::new());
        let g = gate.clone();
        core.on_message(Message::Begin { id: 5 }, move || {
            Box::pin(async move {
                g.notified().await;
                Ok(())
            })
        });
        core.on_message(Message::Begin { id: 5 }, || unreachable!("second begin"));
        assert_eq!(open_windows(&core), 1);
        gate.notify_one();
        until(|| !rec.calls().is_empty());
        assert_eq!(rec.calls().len(), 1);
    }

    #[test]
    fn no_refresher_is_a_quiet_ok_and_the_noop_is_too() {
        assert_eq!(tauri::async_runtime::block_on(run_refresher(None)), Ok(()));
        let noop: Arc<dyn Refresher> = Arc::new(NoopRefresher);
        assert_eq!(
            tauri::async_runtime::block_on(run_refresher(Some(noop))),
            Ok(())
        );
    }

    #[test]
    fn a_failed_complete_is_logged_not_fatal() {
        let core = Arc::new(Core::new(Box::new(bridge::Unavailable)));
        core.on_message(Message::Begin { id: 6 }, || Box::pin(async { Ok(()) }));
        until(|| open_windows(&core) == 0);
    }
}
