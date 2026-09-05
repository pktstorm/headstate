//! What the phone knows about its link to the desktop, as one value the
//! frontend polls (`connection_state`) and is told about (the
//! `connection-state` event).
//!
//! The states are the spec's five. `protocol_version` is what
//! `/v1/hello` reported on the last successful connect and is `None`
//! whenever the state is not `connected`, so a "desktop too old" banner
//! never reads a stale number. `last_poll` is when the desktop last
//! delivered a `prs-updated` (the snapshot on connect, or a poll result),
//! persisted with the snapshot so it survives a restart.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::{Arc, Mutex};

use crate::client::PROTOCOL_VERSION;

/// Where events go. `lib.rs` implements it for the Tauri `AppHandle`
/// (`app.emit`); tests collect. `json` is the payload already
/// serialised, emitted verbatim.
pub trait EventSink: Send + Sync {
    fn emit(&self, name: &str, json: &str);
}

/// The Tauri event carrying a [`Report`] on every change, so the
/// frontend's transport can react without polling.
pub const STATE_EVENT: &str = "connection-state";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Unpaired,
    Connecting,
    Connected,
    Unreachable,
    Revoked,
}

/// The `connection_state` answer. Field names are the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub state: State,
    /// The paired desktop's name from the QR; `None` while unpaired.
    pub desktop: Option<String>,
    /// ISO 8601, or `None` before the first `prs-updated`.
    pub last_poll: Option<String>,
    /// From `/v1/hello`; `None` unless `state` is `connected`.
    pub protocol_version: Option<u32>,
    /// Whether what the phone shows may be out of date and actions are
    /// refused: anything but `connected` to a desktop speaking at least
    /// this app's protocol. The UI's stale marker reads this.
    pub stale: bool,
}

#[derive(Debug)]
struct Inner {
    state: State,
    desktop: Option<String>,
    last_poll: Option<DateTime<Utc>>,
    protocol_version: Option<u32>,
}

pub struct Connection {
    inner: Mutex<Inner>,
    sink: Arc<dyn EventSink>,
}

impl Connection {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                state: State::Unpaired,
                desktop: None,
                last_poll: None,
                protocol_version: None,
            }),
            sink,
        }
    }

    pub fn report(&self) -> Report {
        report_of(&self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn state(&self) -> State {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).state
    }

    /// Move to `state`. Any state but `Connected` clears
    /// `protocol_version`; use [`Connection::connected`] to enter
    /// `Connected` with one. Emits only on an actual change.
    pub fn set_state(&self, state: State) {
        self.update(|i| {
            if i.state == state {
                return false;
            }
            i.state = state;
            if state != State::Connected {
                i.protocol_version = None;
            }
            true
        });
    }

    /// `Connected`, with what the desktop said in `/v1/hello`.
    pub fn connected(&self, protocol_version: u32) {
        self.update(|i| {
            let changed =
                i.state != State::Connected || i.protocol_version != Some(protocol_version);
            i.state = State::Connected;
            i.protocol_version = Some(protocol_version);
            changed
        });
    }

    /// The desktop this phone is paired with (or not), set at load, at
    /// pairing, and at unpairing. Does not change `state`.
    pub fn set_desktop(&self, name: Option<String>, last_poll: Option<DateTime<Utc>>) {
        self.update(|i| {
            i.desktop = name;
            i.last_poll = last_poll;
            true
        });
    }

    pub fn mark_poll(&self, at: DateTime<Utc>) {
        self.update(|i| {
            i.last_poll = Some(at);
            true
        });
    }

    fn update(&self, f: impl FnOnce(&mut Inner) -> bool) {
        let report = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if !f(&mut inner) {
                return;
            }
            report_of(&inner)
        };
        // Serialising a struct of scalars cannot fail.
        let json = serde_json::to_string(&report).expect("report serialises");
        self.sink.emit(STATE_EVENT, &json);
    }
}

/// Why write and destructive commands are refused right now, as the
/// clause that follows the desktop's name in the error, or `None` when
/// they are allowed.
fn blocked(i: &Inner) -> Option<&'static str> {
    match i.state {
        State::Unpaired => Some("is not paired with this phone"),
        State::Connecting => Some("is still being reached; try again in a moment"),
        State::Unreachable => Some("is unreachable; actions are disabled until it is back"),
        State::Revoked => Some("no longer recognises this phone; pair again"),
        State::Connected => match i.protocol_version {
            Some(v) if v >= PROTOCOL_VERSION => None,
            // #524's banner names the version and links the release.
            _ => Some("runs an older Headstate; update it before driving it from here"),
        },
    }
}

impl Connection {
    /// See [`blocked`].
    pub fn actions_blocked(&self) -> Option<&'static str> {
        blocked(&self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

fn report_of(i: &Inner) -> Report {
    Report {
        state: i.state,
        desktop: i.desktop.clone(),
        last_poll: i
            .last_poll
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        protocol_version: i.protocol_version,
        stale: blocked(i).is_some(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Collects every emitted event, for tests across the crate.
    #[derive(Default)]
    pub(crate) struct Recorder {
        pub events: Mutex<Vec<(String, String)>>,
    }

    impl Recorder {
        pub(crate) fn names(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .map(|(n, _)| n.clone())
                .collect()
        }
        pub(crate) fn last(&self, name: &str) -> Option<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, j)| j.clone())
        }
    }

    impl EventSink for Recorder {
        fn emit(&self, name: &str, json: &str) {
            self.events
                .lock()
                .unwrap()
                .push((name.to_string(), json.to_string()));
        }
    }

    fn conn() -> (Arc<Recorder>, Connection) {
        let rec = Arc::new(Recorder::default());
        (rec.clone(), Connection::new(rec))
    }

    #[test]
    fn the_report_is_the_wire_shape_the_frontend_expects() {
        let (_, c) = conn();
        let json = serde_json::to_value(c.report()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "unpaired",
                "desktop": null,
                "last_poll": null,
                "protocol_version": null,
                "stale": true
            })
        );
        c.set_desktop(Some("octocat's laptop".into()), None);
        c.connected(1);
        c.mark_poll("2026-09-05T12:00:00Z".parse().unwrap());
        assert_eq!(
            serde_json::to_value(c.report()).unwrap(),
            serde_json::json!({
                "state": "connected",
                "desktop": "octocat's laptop",
                "last_poll": "2026-09-05T12:00:00Z",
                "protocol_version": 1,
                "stale": false
            })
        );
    }

    #[test]
    fn stale_and_blocked_track_state_and_protocol() {
        let (_, c) = conn();
        assert!(c.report().stale);
        assert!(c.actions_blocked().unwrap().contains("not paired"));
        c.connected(PROTOCOL_VERSION);
        assert!(!c.report().stale);
        assert_eq!(c.actions_blocked(), None);
        c.connected(PROTOCOL_VERSION + 1);
        assert_eq!(c.actions_blocked(), None, "a newer desktop is fine");
        c.connected(PROTOCOL_VERSION - 1);
        assert!(c.report().stale);
        assert!(c.actions_blocked().unwrap().contains("older Headstate"));
        c.set_state(State::Unreachable);
        assert!(c.actions_blocked().unwrap().contains("unreachable"));
        c.set_state(State::Revoked);
        assert!(c.actions_blocked().unwrap().contains("pair again"));
    }

    #[test]
    fn protocol_version_is_null_in_every_state_but_connected() {
        let (_, c) = conn();
        c.connected(1);
        assert_eq!(c.report().protocol_version, Some(1));
        for s in [
            State::Connecting,
            State::Unreachable,
            State::Revoked,
            State::Unpaired,
        ] {
            c.connected(1);
            c.set_state(s);
            assert_eq!(c.report().protocol_version, None, "{s:?}");
            assert_eq!(c.state(), s);
        }
    }

    #[test]
    fn every_change_is_emitted_and_a_no_op_is_not() {
        let (rec, c) = conn();
        c.set_state(State::Connecting);
        c.set_state(State::Connecting);
        c.connected(1);
        c.connected(1);
        assert_eq!(rec.names(), vec![STATE_EVENT, STATE_EVENT]);
        let last: serde_json::Value =
            serde_json::from_str(&rec.last(STATE_EVENT).unwrap()).unwrap();
        assert_eq!(last["state"], "connected");
    }
}
