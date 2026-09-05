//! What the phone does when it is in the background: a single refresh
//! per OS-granted window, and nothing else.
//!
//! The spec's decision ("What the phone does when it is in the
//! background") is that the phone does nothing while suspended and
//! catches up on resume, which is the event subscriber's reconnect
//! (`subscribe_events`, #514) and is not duplicated here. The one cheap
//! improvement it allows is [`BackgroundRefresh`]: when
//! `tauri-plugin-headstate-refresh` is granted a window, `GET /v1/hello`
//! and then the cached list, exactly what a connect does before the
//! stream, store the list as the snapshot, and return. On any error give
//! up quietly; nothing retries inside the window.
//!
//! **Never `/v1/events`.** The snapshot the subscriber gets is the first
//! frame of the stream; the background path must not open the stream,
//! so it asks for the same list through `get_cached` instead (the
//! desktop's `remote/events.rs` documents them as the same data). The
//! [`Desktop`] seam has no way to open the stream at all, and the tests
//! pin the request sequence to those two.
//!
//! [`Desktop`] and [`SnapshotSink`] are the seam to the client (#514):
//! the wiring is `hello` -> `Client::hello`, `get_cached` ->
//! `Client::call("get_cached", {}, None)`, `save` ->
//! `events::save_snapshot`. Until that lands [`install`] manages the
//! plugin's [`NoopRefresher`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_headstate_refresh::{NoopRefresher, RefreshFuture, Refresher};

/// A boxed request to the desktop.
pub type DesktopFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

/// The two requests the background path may make. Deliberately nothing
/// else: no stream, no commands.
pub trait Desktop: Send + Sync {
    /// `GET /v1/hello`. Proves the desktop is there and still knows us.
    fn hello(&self) -> DesktopFuture<()>;
    /// The cached PR list as `get_cached` returns it, JSON verbatim.
    fn get_cached(&self) -> DesktopFuture<String>;
}

/// Where the list goes: the snapshot store, so the app opens fresh.
pub trait SnapshotSink: Send + Sync {
    fn save(&self, prs_json: &str) -> Result<(), String>;
}

/// The [`Refresher`] the plugin runs in a window.
pub struct BackgroundRefresh {
    desktop: Arc<dyn Desktop>,
    sink: Arc<dyn SnapshotSink>,
}

impl BackgroundRefresh {
    pub fn new(desktop: Arc<dyn Desktop>, sink: Arc<dyn SnapshotSink>) -> Self {
        Self { desktop, sink }
    }
}

impl Refresher for BackgroundRefresh {
    fn refresh(&self) -> RefreshFuture {
        let desktop = self.desktop.clone();
        let sink = self.sink.clone();
        Box::pin(async move {
            desktop.hello().await?;
            let prs = desktop.get_cached().await?;
            sink.save(&prs)
        })
    }
}

/// Put the app's refresher in Tauri state for the plugin to find.
///
/// TODO(#514): once the client is on main, build a [`BackgroundRefresh`]
/// over `Companion`'s client and store here instead of the no-op.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    let refresher: Arc<dyn Refresher> = Arc::new(NoopRefresher);
    app.manage(refresher);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Every request the fake desktop saw, by kind. The enum has two
    /// variants because the seam has two methods; a third request kind
    /// cannot be recorded because it cannot be made.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Request {
        Hello,
        GetCached,
    }

    struct FakeDesktop {
        log: Mutex<Vec<Request>>,
        hello: Result<(), String>,
        cached: Result<String, String>,
    }

    impl FakeDesktop {
        fn new(hello: Result<(), String>, cached: Result<String, String>) -> Arc<Self> {
            Arc::new(Self {
                log: Mutex::new(vec![]),
                hello,
                cached,
            })
        }
        fn requests(&self) -> Vec<Request> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Desktop for FakeDesktop {
        fn hello(&self) -> DesktopFuture<()> {
            self.log.lock().unwrap().push(Request::Hello);
            let r = self.hello.clone();
            Box::pin(async move { r })
        }
        fn get_cached(&self) -> DesktopFuture<String> {
            self.log.lock().unwrap().push(Request::GetCached);
            let r = self.cached.clone();
            Box::pin(async move { r })
        }
    }

    #[derive(Default)]
    struct FakeSink {
        saved: Mutex<Vec<String>>,
    }

    impl SnapshotSink for FakeSink {
        fn save(&self, prs_json: &str) -> Result<(), String> {
            self.saved.lock().unwrap().push(prs_json.to_string());
            Ok(())
        }
    }

    const LIST: &str = r#"[{"number":1347,"title":"Add spoon"}]"#;

    fn run(desktop: &Arc<FakeDesktop>) -> (Result<(), String>, Arc<FakeSink>) {
        let sink = Arc::new(FakeSink::default());
        let refresh = BackgroundRefresh::new(desktop.clone(), sink.clone());
        let result = tauri::async_runtime::block_on(refresh.refresh());
        (result, sink)
    }

    /// The whole background path: hello, the list, the store. Exactly
    /// two requests, in that order, and never the stream.
    #[test]
    fn a_window_is_hello_then_the_cached_list_then_the_store() {
        let desktop = FakeDesktop::new(Ok(()), Ok(LIST.into()));
        let (result, sink) = run(&desktop);
        assert_eq!(result, Ok(()));
        assert_eq!(desktop.requests(), vec![Request::Hello, Request::GetCached]);
        assert_eq!(*sink.saved.lock().unwrap(), vec![LIST.to_string()]);
    }

    #[test]
    fn an_unreachable_desktop_is_given_up_on_after_hello() {
        let desktop = FakeDesktop::new(Err("desktop unreachable".into()), Ok(LIST.into()));
        let (result, sink) = run(&desktop);
        assert_eq!(result, Err("desktop unreachable".into()));
        assert_eq!(
            desktop.requests(),
            vec![Request::Hello],
            "no list fetch, no retry"
        );
        assert!(sink.saved.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_list_fetch_stores_nothing() {
        let desktop = FakeDesktop::new(Ok(()), Err("desktop answered HTTP 500".into()));
        let (result, sink) = run(&desktop);
        assert_eq!(result, Err("desktop answered HTTP 500".into()));
        assert_eq!(desktop.requests(), vec![Request::Hello, Request::GetCached]);
        assert!(sink.saved.lock().unwrap().is_empty());
    }

    /// The seam is the proof: `Desktop` has no method that could open
    /// the stream, so the background path cannot. Pinned here so a
    /// later "convenience" method is a deliberate change to this test.
    #[test]
    fn the_seam_offers_only_hello_and_get_cached() {
        let src = include_str!("background.rs");
        let start = src.find("pub trait Desktop").unwrap();
        let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
        let methods: Vec<&str> = body
            .lines()
            .filter_map(|l| l.trim().strip_prefix("fn "))
            .map(|l| l.split('(').next().unwrap())
            .collect();
        assert_eq!(methods, vec!["hello", "get_cached"]);
        assert!(!body.contains("events"), "no stream on the seam");
    }
}
