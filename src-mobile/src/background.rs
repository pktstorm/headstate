//! What the phone does when it is in the background: a single refresh
//! per OS-granted window, and nothing else.
//!
//! The spec's decision ("What the phone does when it is in the
//! background") is that the phone does nothing while suspended and
//! catches up on resume, which is the event subscriber's reconnect
//! (`subscribe_events`, `companion.rs`) and is not duplicated here. The
//! one cheap improvement it allows is [`BackgroundRefresh`]: when
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
//! pin the request sequence to those two -- against the fake, and
//! against the loopback server with the real client.
//!
//! [`Desktop`] and [`SnapshotSink`] are implemented by [`Companion`]:
//! `hello` is `Client::hello`, `get_cached` is
//! `Client::call("get_cached", {}, None)` on the live client, `save` is
//! `events::save_snapshot` plus `Connection::mark_poll`. A window does
//! not move the connection state: the subscriber owns that, and a
//! desktop that is away is the expected case for a phone in a pocket.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_headstate_refresh::{RefreshFuture, Refresher};

use crate::companion::Companion;

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

impl Desktop for Companion {
    fn hello(&self) -> DesktopFuture<()> {
        let client = self.client();
        Box::pin(async move { client?.hello().await.map(|_| ()).map_err(|e| e.to_string()) })
    }

    fn get_cached(&self) -> DesktopFuture<String> {
        let client = self.client();
        Box::pin(async move {
            let list = client?
                .call("get_cached", &json!({}), None)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&list).map_err(|e| e.to_string())
        })
    }
}

impl SnapshotSink for Companion {
    fn save(&self, prs_json: &str) -> Result<(), String> {
        self.record_snapshot(prs_json)
    }
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

/// Put the app's refresher in Tauri state for the plugin to find: a
/// [`BackgroundRefresh`] over the managed [`Companion`], so it must run
/// after `setup` has managed one.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    let companion = app.state::<Arc<Companion>>().inner().clone();
    let refresher: Arc<dyn Refresher> =
        Arc::new(BackgroundRefresh::new(companion.clone(), companion));
    app.manage(refresher);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::tests::Recorder;
    use crate::connection::State;
    use crate::events;
    use crate::keys::SoftwareKeys;
    use crate::store::MemoryStore;
    use crate::testing::{Reply, TestServer};
    use base64::Engine;
    use chrono::Utc;
    use std::sync::Mutex;
    use std::time::Duration;

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

    // ---- The real client, against the loopback server ---------------

    async fn until(mut cond: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !cond() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition within 10s");
    }

    /// A companion paired with the loopback server, its subscriber
    /// connected and holding the stream open, as on a phone.
    async fn paired() -> (TestServer, Arc<MemoryStore>, Arc<Recorder>, Arc<Companion>) {
        let store = Arc::new(MemoryStore::default());
        let rec = Arc::new(Recorder::default());
        let c = Arc::new(Companion::new(
            store.clone(),
            Arc::new(SoftwareKeys::new(store.clone())),
            rec.clone(),
            Arc::new(|f| {
                tokio::spawn(f);
            }),
        ));
        let server = TestServer::start().await;
        server.open_window(true);
        server.reply(
            "/v1/events",
            Reply::sse(&[("prs-updated", r#"[{"number":1}]"#)], true),
        );
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32]);
        let qr = server.qr(&token, Utc::now().timestamp() + 120);
        c.pair(&qr, None).await.unwrap();
        let fp = server.requests()[0].peer_fp.clone();
        server.pair(&fp);
        server.open_window(false);
        until(|| {
            let r = c.connection_state();
            r.state == State::Connected && r.last_poll.is_some()
        })
        .await;
        (server, store, rec, c)
    }

    /// The wired refresher, end to end: one window against the loopback
    /// server makes exactly `GET /v1/hello` and `POST /v1/call/get_cached`
    /// on the real client, stores what came back as the snapshot with a
    /// fresh poll time, and opens no second stream.
    #[tokio::test]
    async fn the_wired_refresher_goes_through_the_seam_and_never_opens_the_stream() {
        let (server, store, _rec, c) = paired().await;
        server.reply(
            "/v1/call/get_cached",
            Reply::json(200, serde_json::from_str(LIST).unwrap()),
        );
        let before = server.requests().len();
        let streams = |s: &TestServer| {
            s.requests()
                .iter()
                .filter(|r| r.path == "/v1/events")
                .count()
        };
        assert_eq!(streams(&server), 1, "the subscriber's stream, held open");
        let stale = c.connection_state().last_poll.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;

        let refresh = BackgroundRefresh::new(c.clone(), c.clone());
        refresh.refresh().await.unwrap();

        let made: Vec<(String, String)> = server.requests()[before..]
            .iter()
            .map(|r| (r.method.clone(), r.path.clone()))
            .collect();
        assert_eq!(
            made,
            vec![
                ("GET".to_string(), "/v1/hello".to_string()),
                ("POST".to_string(), "/v1/call/get_cached".to_string()),
            ]
        );
        assert_eq!(streams(&server), 1, "no stream opened by the window");
        let snap = events::cached_snapshot(store.as_ref()).unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(snap.prs.get()).unwrap(),
            serde_json::from_str::<serde_json::Value>(LIST).unwrap()
        );
        let fresh = c.connection_state().last_poll.unwrap();
        assert!(fresh > stale, "mark_poll: {stale} -> {fresh}");
        assert_eq!(c.connection_state().state, State::Connected);
    }

    /// A window while the desktop is away: hello fails, nothing else is
    /// tried, the snapshot and the connection state are left alone.
    #[tokio::test]
    async fn a_window_while_the_desktop_is_away_touches_nothing() {
        let (server, store, _rec, c) = paired().await;
        let snapshot_before = events::cached_snapshot(store.as_ref())
            .unwrap()
            .unwrap()
            .received_at;
        drop(server);
        // The subscriber notices the dead stream on its own; the window
        // must not be what tells it.
        until(|| c.connection_state().state == State::Unreachable).await;

        let refresh = BackgroundRefresh::new(c.clone(), c.clone());
        let err = refresh.refresh().await.unwrap_err();
        assert!(err.contains("unreachable"), "{err}");
        assert_eq!(
            events::cached_snapshot(store.as_ref())
                .unwrap()
                .unwrap()
                .received_at,
            snapshot_before
        );
    }

    #[tokio::test]
    async fn an_unpaired_companion_has_nothing_to_refresh() {
        let store = Arc::new(MemoryStore::default());
        let c = Arc::new(Companion::new(
            store.clone(),
            Arc::new(SoftwareKeys::new(store)),
            Arc::new(Recorder::default()),
            Arc::new(|f| {
                tokio::spawn(f);
            }),
        ));
        let refresh = BackgroundRefresh::new(c.clone(), c);
        assert_eq!(
            refresh.refresh().await.unwrap_err(),
            "not paired with a desktop"
        );
    }
}
