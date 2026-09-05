//! The client commands' logic, behind the Tauri wrappers in `lib.rs`:
//! one [`Companion`] holds the store, the keys, the connection state,
//! and -- while paired -- the client and the running event subscriber.
//!
//! Separate from `lib.rs` so every command can be driven in a test
//! against the loopback server with no `AppHandle`: the sink and the
//! task spawner are injected.

use chrono::Utc;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::client::{Client, ClientError};
use crate::connection::{Connection, EventSink, Report, State};
use crate::events;
use crate::keys::DeviceKeys;
use crate::pairing::{self, Desktop};
use crate::stepup;
use crate::store::Store;
use crate::surface::{self, Class};

/// How a background task is started: `tauri::async_runtime::spawn` in
/// the app, `tokio::spawn` in tests.
pub type Spawner = Arc<dyn Fn(Pin<Box<dyn Future<Output = ()> + Send>>) + Send + Sync>;

struct Live {
    desktop: Desktop,
    client: Arc<Client>,
    events: events::Handle,
}

pub struct Companion {
    store: Arc<dyn Store>,
    keys: Arc<dyn DeviceKeys>,
    sink: Arc<dyn EventSink>,
    conn: Arc<Connection>,
    spawn: Spawner,
    live: Mutex<Option<Live>>,
}

impl Companion {
    pub fn new(
        store: Arc<dyn Store>,
        keys: Arc<dyn DeviceKeys>,
        sink: Arc<dyn EventSink>,
        spawn: Spawner,
    ) -> Self {
        Self {
            conn: Arc::new(Connection::new(sink.clone())),
            store,
            keys,
            sink,
            spawn,
            live: Mutex::new(None),
        }
    }

    /// Restore a pairing from the store at startup and start the
    /// subscriber. A record without usable keys is reported and left
    /// in place rather than deleted: the name still belongs in the
    /// banner, and re-pairing replaces it.
    pub fn load(&self) -> Result<(), String> {
        let list = pairing::load_desktops(self.store.as_ref()).map_err(|e| e.to_string())?;
        let Some(desktop) = list.into_iter().next() else {
            return Ok(());
        };
        let last_poll = events::cached_snapshot(self.store.as_ref())
            .ok()
            .flatten()
            .and_then(|s| s.received_at());
        self.conn.set_desktop(Some(desktop.name.clone()), last_poll);
        let identity = match self.keys.session_identity() {
            Ok(id) => id,
            Err(e) => {
                log::warn!(
                    "companion: paired with {} but the keys are unusable: {e}",
                    desktop.name
                );
                self.conn.set_state(State::Revoked);
                return Ok(());
            }
        };
        let client = Client::new(&identity, &desktop.fp, desktop.addrs.clone(), desktop.port)
            .map_err(|e| e.to_string())?;
        self.attach(desktop, Arc::new(client));
        Ok(())
    }

    /// `pair_from_qr`: the flow in `pairing.rs`, then the subscriber.
    /// Returns the desktop's name.
    pub async fn pair(&self, payload: &str, device_name: Option<String>) -> Result<String, String> {
        self.detach();
        let name = device_name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| pairing::default_device_name().to_string());
        let (desktop, client) = pairing::pair(
            self.store.as_ref(),
            self.keys.as_ref(),
            payload,
            &name,
            Utc::now(),
        )
        .await
        .map_err(|e| e.to_string())?;
        let name = desktop.name.clone();
        self.attach(desktop, client);
        Ok(name)
    }

    /// `unpair`: forget every desktop and the snapshot, destroy the
    /// keys, stop the subscriber.
    pub fn unpair(&self) -> Result<(), String> {
        self.detach();
        let mut problems = vec![];
        if let Err(e) = pairing::forget_all(self.store.as_ref()) {
            problems.push(e.to_string());
        }
        if let Err(e) = events::forget_snapshot(self.store.as_ref()) {
            problems.push(e.to_string());
        }
        if let Err(e) = self.keys.destroy() {
            problems.push(e.to_string());
        }
        self.conn.set_desktop(None, None);
        self.conn.set_state(State::Unpaired);
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
    }

    /// `connection_state`.
    pub fn connection_state(&self) -> Report {
        self.conn.report()
    }

    /// `subscribe_events`: start the subscriber if it is not running,
    /// wake it if it is. The frontend calls this on first listen and on
    /// every return to the foreground.
    pub fn subscribe(&self) -> Result<(), String> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        match live.as_ref() {
            None => Err("not paired with a desktop".into()),
            Some(l) => {
                if l.events.is_stopped() && self.conn.state() != State::Revoked {
                    // Stopped by a handshake failure on a call; a resume
                    // is a request to find out whether that still holds.
                    self.start_events(l);
                }
                l.events.resume();
                Ok(())
            }
        }
    }

    /// `remote_call`.
    pub async fn call(&self, command: &str, args: Value) -> Result<Value, String> {
        let class = surface::admit(command).map_err(|e| e.to_string())?;
        let (client, events, desktop_name) = {
            let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            let l = live.as_ref().ok_or("not paired with a desktop")?;
            (l.client.clone(), l.events.clone(), l.desktop.name.clone())
        };
        // Reads go through whatever the state -- the attempt is how the
        // phone finds out the desktop is back, and `get_cached` has the
        // snapshot to fall back on. Anything that changes something is
        // refused while the desktop is away, has revoked this phone, or
        // is too old to be driven from here.
        if matches!(class, Class::Write | Class::Destructive) {
            if let Some(why) = self.conn.actions_blocked() {
                return Err(format!("{desktop_name} {why}"));
            }
        }
        let signature = if class == Class::Destructive {
            Some(
                stepup::sign_request(self.keys.as_ref(), command, &args, Utc::now().timestamp())
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        match client.call(command, &args, signature.as_deref()).await {
            Ok(value) => {
                if self.conn.state() == State::Unreachable {
                    // Back, evidently; let the subscriber confirm and
                    // fill in the protocol version.
                    events.resume();
                }
                Ok(value)
            }
            Err(e) if e.is_handshake() => {
                log::warn!("companion: the desktop refused this phone on {command}: {e}");
                events.stop();
                self.conn.set_state(State::Revoked);
                Err(format!(
                    "{desktop_name} no longer recognises this phone; pair again"
                ))
            }
            Err(ClientError::Unreachable(m)) => {
                self.conn.set_state(State::Unreachable);
                events.resume();
                if command == "get_cached" {
                    if let Ok(Some(snap)) = events::cached_snapshot(self.store.as_ref()) {
                        log::info!(
                            "companion: {desktop_name} unreachable; serving the cached list"
                        );
                        return serde_json::from_str(snap.prs.get()).map_err(|e| e.to_string());
                    }
                }
                Err(format!("{desktop_name} is unreachable: {m}"))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn attach(&self, desktop: Desktop, client: Arc<Client>) {
        self.conn.set_desktop(
            Some(desktop.name.clone()),
            events::cached_snapshot(self.store.as_ref())
                .ok()
                .flatten()
                .and_then(|s| s.received_at()),
        );
        self.conn.set_state(State::Connecting);
        let live = Live {
            desktop,
            client,
            events: events::Handle::new(),
        };
        self.start_events(&live);
        *self.live.lock().unwrap_or_else(|e| e.into_inner()) = Some(live);
    }

    fn start_events(&self, live: &Live) {
        let sub = events::Subscriber {
            client: live.client.clone(),
            sink: self.sink.clone(),
            store: self.store.clone(),
            conn: self.conn.clone(),
            desktop_fp: live.desktop.fp.clone(),
        };
        (self.spawn)(Box::pin(events::run(sub, live.events.clone())));
    }

    fn detach(&self) {
        if let Some(l) = self.live.lock().unwrap_or_else(|e| e.into_inner()).take() {
            l.events.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::tests::Recorder;
    use crate::keys::{KeyError, SoftwareKeys};
    use crate::store::MemoryStore;
    use crate::testing::{Reply, TestServer};
    use base64::Engine;
    use serde_json::json;
    use std::time::Duration;

    fn companion(store: Arc<MemoryStore>, rec: Arc<Recorder>) -> Companion {
        Companion::new(
            store.clone(),
            Arc::new(SoftwareKeys::new(store)),
            rec,
            Arc::new(|f| {
                tokio::spawn(f);
            }),
        )
    }

    async fn until(mut cond: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !cond() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition within 10s");
    }

    fn token() -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32])
    }

    async fn paired() -> (TestServer, Arc<MemoryStore>, Arc<Recorder>, Companion) {
        let store = Arc::new(MemoryStore::default());
        let rec = Arc::new(Recorder::default());
        let c = companion(store.clone(), rec.clone());
        let server = TestServer::start().await;
        server.open_window(true);
        server.reply(
            "/v1/events",
            Reply::sse(&[("prs-updated", r#"[{"number":1347}]"#)], true),
        );
        let qr = server.qr(&token(), Utc::now().timestamp() + 120);
        assert_eq!(c.pair(&qr, None).await.unwrap(), "octocat's laptop");
        // The test server pairs whoever the window admitted.
        let fp = server.requests()[0].peer_fp.clone();
        server.pair(&fp);
        server.open_window(false);
        // Connected is set after `hello`; the snapshot frame that fills in
        // `last_poll` arrives a moment later on the stream. Wait for both, or
        // a busy runner reads the report between the two.
        until(|| {
            let r = c.connection_state();
            r.state == State::Connected && r.last_poll.is_some()
        })
        .await;
        (server, store, rec, c)
    }

    #[tokio::test]
    async fn unpaired_commands_say_so() {
        let c = companion(
            Arc::new(MemoryStore::default()),
            Arc::new(Recorder::default()),
        );
        assert_eq!(c.connection_state().state, State::Unpaired);
        assert_eq!(c.subscribe().unwrap_err(), "not paired with a desktop");
        assert_eq!(
            c.call("get_cached", json!({})).await.unwrap_err(),
            "not paired with a desktop"
        );
        assert_eq!(
            c.call("reveal_log", json!({})).await.unwrap_err(),
            "`reveal_log` is only available on the desktop"
        );
        assert_eq!(
            c.call("drop_database", json!({})).await.unwrap_err(),
            "`drop_database` is not a Headstate command"
        );
        assert!(c.unpair().is_ok(), "unpair is idempotent");
    }

    #[tokio::test]
    async fn pairing_connects_and_reports_the_desktop() {
        let (server, store, rec, c) = paired().await;
        let report = c.connection_state();
        assert_eq!(report.desktop.as_deref(), Some("octocat's laptop"));
        assert_eq!(report.protocol_version, Some(1));
        assert!(report.last_poll.is_some());
        assert_eq!(rec.last("prs-updated").unwrap(), r#"[{"number":1347}]"#);
        assert_eq!(
            server.requests()[0].header("content-type"),
            Some("application/json")
        );
        assert_eq!(pairing::load_desktops(store.as_ref()).unwrap().len(), 1);
        assert!(c.subscribe().is_ok());
    }

    #[tokio::test]
    async fn calls_are_forwarded_and_destructive_ones_carry_the_signature() {
        let (server, _, _, c) = paired().await;
        server.reply(
            "/v1/call/get_stats",
            Reply::json(200, json!({"merged_week": 3})),
        );
        server.reply("/v1/call/remove_orphan", Reply::json(200, json!(null)));
        assert_eq!(
            c.call("get_stats", json!({})).await.unwrap(),
            json!({"merged_week": 3})
        );
        assert_eq!(
            c.call("remove_orphan", json!({"path": "/srv/x"}))
                .await
                .unwrap(),
            json!(null)
        );
        let reqs = server.requests();
        let stats = reqs
            .iter()
            .find(|r| r.path == "/v1/call/get_stats")
            .unwrap();
        assert_eq!(stats.header("x-headstate-signature"), None);
        let rm = reqs
            .iter()
            .find(|r| r.path == "/v1/call/remove_orphan")
            .unwrap();
        let sig = rm.header("x-headstate-signature").unwrap();
        assert!(sig.starts_with("v1;ts="));
        assert!(sig.contains(";ecdsa=") && sig.contains(";mldsa="));
        assert_eq!(
            serde_json::from_str::<Value>(&rm.body).unwrap(),
            json!({"path": "/srv/x"})
        );
    }

    #[tokio::test]
    async fn a_desktop_error_is_passed_through_verbatim() {
        let (server, _, _, c) = paired().await;
        server.reply(
            "/v1/call/refresh_now",
            Reply::text(500, "gh auth status failed"),
        );
        assert_eq!(
            c.call("refresh_now", json!({})).await.unwrap_err(),
            "gh auth status failed"
        );
    }

    #[tokio::test]
    async fn while_unreachable_the_list_comes_from_the_cache_and_actions_are_refused() {
        let (server, _, _, c) = paired().await;
        drop(server);
        until(|| c.connection_state().state == State::Unreachable).await;
        assert_eq!(
            c.call("get_cached", json!({})).await.unwrap(),
            json!([{"number": 1347}]),
            "the cached snapshot, marked by the connection state"
        );
        assert_eq!(c.connection_state().protocol_version, None);
        let err = c
            .call("act_on_pr", json!({"id": "PR_1"}))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            "octocat's laptop is unreachable; actions are disabled until it is back"
        );
        let err = c
            .call("remove_orphan", json!({"path": "/x"}))
            .await
            .unwrap_err();
        assert!(err.contains("actions are disabled"));
        // Other reads are attempted and fail honestly.
        let err = c.call("get_stats", json!({})).await.unwrap_err();
        assert!(err.starts_with("octocat's laptop is unreachable:"), "{err}");
    }

    #[tokio::test]
    async fn a_handshake_refusal_on_a_call_is_revocation() {
        let (server, store, _, c) = paired().await;
        let fp = server.requests()[0].peer_fp.clone();
        server.revoke(&fp);
        server.end_streams();
        // Whichever notices first -- the subscriber's next hello or this
        // call -- the outcome is the same state.
        let err = loop {
            match c.call("get_stats", json!({})).await {
                Err(e) => break e,
                Ok(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };
        until(|| c.connection_state().state == State::Revoked).await;
        assert!(
            err.contains("no longer recognises this phone")
                || err.contains("no longer paired with this phone"),
            "{err}"
        );
        assert_eq!(
            c.call("get_stats", json!({})).await.unwrap_err(),
            "octocat's laptop no longer recognises this phone; pair again"
        );
        // The record and the name stay for the banner until re-pairing.
        assert_eq!(
            c.connection_state().desktop.as_deref(),
            Some("octocat's laptop")
        );
        assert_eq!(pairing::load_desktops(store.as_ref()).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unpair_forgets_everything() {
        let (_server, store, _, c) = paired().await;
        c.unpair().unwrap();
        let report = c.connection_state();
        assert_eq!(report.state, State::Unpaired);
        assert_eq!(report.desktop, None);
        assert!(pairing::load_desktops(store.as_ref()).unwrap().is_empty());
        assert!(events::cached_snapshot(store.as_ref()).unwrap().is_none());
        assert_eq!(
            SoftwareKeys::new(store.clone()).public_keys().unwrap_err(),
            KeyError::NoKeys
        );
        assert_eq!(c.subscribe().unwrap_err(), "not paired with a desktop");
    }

    #[tokio::test]
    async fn an_older_desktop_is_read_only() {
        let store = Arc::new(MemoryStore::default());
        let rec = Arc::new(Recorder::default());
        let c = companion(store, rec);
        let server = TestServer::start().await;
        server.open_window(true);
        server.reply(
            "/v1/hello",
            Reply::json(
                200,
                json!({"desktop_version": "4.0.0", "protocol_version": 0, "viewer_login": null}),
            ),
        );
        server.reply("/v1/events", Reply::sse(&[("prs-updated", "[]")], true));
        server.reply(
            "/v1/call/get_stats",
            Reply::json(200, json!({"merged_week": 1})),
        );
        let qr = server.qr(&token(), Utc::now().timestamp() + 120);
        c.pair(&qr, None).await.unwrap();
        let fp = server.requests()[0].peer_fp.clone();
        server.pair(&fp);
        server.open_window(false);
        until(|| c.connection_state().state == State::Connected).await;
        let report = c.connection_state();
        assert_eq!(report.protocol_version, Some(0));
        assert!(report.stale, "old desktop: shown as stale");
        assert_eq!(
            c.call("get_stats", json!({})).await.unwrap(),
            json!({"merged_week": 1})
        );
        let err = c
            .call("act_on_pr", json!({"id": "PR_1"}))
            .await
            .unwrap_err();
        assert!(
            err.starts_with("octocat's laptop runs an older Headstate"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_pairing_is_restored_from_the_store_at_startup() {
        let (server, store, _, c) = paired().await;
        c.detach();
        let rec = Arc::new(Recorder::default());
        let again = companion(store, rec);
        again.load().unwrap();
        assert_eq!(
            again.connection_state().desktop.as_deref(),
            Some("octocat's laptop")
        );
        assert!(
            again.connection_state().last_poll.is_some(),
            "from the snapshot"
        );
        until(|| again.connection_state().state == State::Connected).await;
        assert!(
            server
                .requests()
                .iter()
                .filter(|r| r.path == "/v1/events")
                .count()
                >= 2
        );
    }
}
