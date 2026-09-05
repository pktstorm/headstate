//! The feature gate: **Allow phone connections**.
//!
//! Off by default. The setting is read once at startup to decide whether
//! the listener comes up with the app, and flipped by the Settings
//! toggle, which starts or stops the listener on the spot. The desktop
//! identity is loaded from the keychain -- and created, the first time
//! -- when the listener first starts, never earlier, so a user who never
//! enables the feature never gets a keychain item.
//!
//! Managed as Tauri state so the pairing task (#507) can reach the
//! running listener, the identity's fingerprint for the QR code, and
//! swap the stub [`NoPairedDevices`] for the real store in [`setup`].

use crate::commands::db_path;
use crate::remote::events::{Hub, SnapshotSource};
use crate::remote::identity::{self, Identity, PlatformStore};
use crate::remote::listener::{self, Handle, ListenerConfig, PairedCerts, ViewerLookup, PORT};
use crate::remote::pairing::PairingState;
use crate::store::{self, settings};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Manager, State};

/// Nothing is paired and no pairing window is ever open: the listener
/// refuses every handshake. What production runs until the pairing task
/// lands; replaced in [`setup`] by an implementation over
/// `paired_devices`.
pub struct NoPairedDevices;

impl PairedCerts for NoPairedDevices {
    fn is_paired(&self, _sha256_fp_hex: &str) -> bool {
        false
    }
    fn pairing_window_open(&self) -> bool {
        false
    }
}

/// The remote feature's live state.
pub struct Remote {
    listener: tokio::sync::Mutex<Option<Handle>>,
    paired: Arc<dyn PairedCerts>,
    /// Loaded once per process; the keychain may prompt on macOS, and
    /// asking on every enable would prompt on every enable.
    identity: OnceLock<Identity>,
    /// The event fan-out for `/v1/events`. Outlives any one listener so
    /// toggling the setting off and on does not re-tap the app's events.
    events: Arc<Hub>,
}

impl Remote {
    pub fn new(paired: Arc<dyn PairedCerts>, events: Arc<Hub>) -> Self {
        Self {
            listener: tokio::sync::Mutex::new(None),
            paired,
            identity: OnceLock::new(),
            events,
        }
    }

    /// The desktop certificate's fingerprint, once the identity has been
    /// loaded -- which happens on the first start. `None` before that.
    pub fn fingerprint(&self) -> Option<String> {
        self.identity.get().map(Identity::fingerprint)
    }

    /// Whether the listener is up right now. The live answer, not the
    /// stored setting: the two differ when the port was taken at
    /// startup, and "phones can connect" is the question being asked.
    pub async fn is_running(&self) -> bool {
        self.listener.lock().await.is_some()
    }
}

/// The process-level rustls default. Called first thing in `run`.
///
/// Two providers are compiled in -- ring for octocrab and reqwest,
/// aws-lc-rs for the listener -- and with two, rustls refuses to guess:
/// any `ClientConfig::builder()` without an installed default panics.
/// `ring`, so the GitHub client keeps the exact provider it has used
/// since it was added; the listener never consults the default and
/// picks aws-lc-rs per config (`listener::provider`).
///
/// `Err` from `install_default` means one is already installed, which
/// is the end state wanted either way.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Manage the state and bring the listener up if the setting says so.
/// Called from Tauri's `setup`, which is synchronous and outside the
/// runtime, hence `block_on`.
pub fn setup(app: &AppHandle) {
    // Tapped whether or not the listener is on: the events are cheap
    // to forward to nobody, and attaching lazily would mean a poll
    // that fired between "enable" and "attached" reached no phone.
    let events = Arc::new(Hub::new(snapshot_source(app)));
    events.attach(app);
    app.manage(Remote::new(Arc::new(NoPairedDevices), events));
    if !read_enabled(app) {
        return;
    }
    match tauri::async_runtime::block_on(start(app)) {
        Ok(addr) => log::info!("phone connections: on ({addr})"),
        // The toggle will read as off, which is the truth; the setting
        // stays on so the next launch tries again.
        Err(e) => log::warn!("phone connections: enabled but could not start: {e}"),
    }
}

fn read_enabled(app: &AppHandle) -> bool {
    store::open_db(&db_path(app))
        .ok()
        .and_then(|c| settings::get::<bool>(&c, settings::keys::ALLOW_PHONE_CONNECTIONS).ok())
        .flatten()
        .unwrap_or(false)
}

fn persist_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let conn = store::open_db(&db_path(app)).map_err(|e| e.to_string())?;
    settings::set(&conn, settings::keys::ALLOW_PHONE_CONNECTIONS, &enabled)
        .map_err(|e| e.to_string())
}

/// `/v1/hello`'s login, through the GitHub client the app already
/// holds. `None` when not signed in.
fn viewer_lookup(app: &AppHandle) -> ViewerLookup {
    let client = app
        .try_state::<crate::commands::GhClient>()
        .and_then(|s| s.0.clone());
    Arc::new(move || {
        let client = client.clone();
        Box::pin(async move {
            match client {
                Some(c) => c.fetch_viewer().await.ok(),
                None => None,
            }
        })
    })
}

/// The opening `prs-updated` frame of `/v1/events`: the list the
/// webview's `get_cached` returns, through the same function, so the
/// phone's first paint is the desktop's first paint. rusqlite is
/// synchronous, so it runs off the async workers.
fn snapshot_source(app: &AppHandle) -> SnapshotSource {
    let app = app.clone();
    Arc::new(move || {
        let app = app.clone();
        Box::pin(async move {
            let loaded =
                tauri::async_runtime::spawn_blocking(move || crate::commands::get_cached(app))
                    .await;
            match loaded {
                Ok(Ok(prs)) => serde_json::to_string(&prs).ok(),
                Ok(Err(e)) => {
                    log::warn!("remote: could not load the snapshot for a phone: {e}");
                    None
                }
                Err(e) => {
                    log::warn!("remote: the snapshot task failed: {e}");
                    None
                }
            }
        })
    })
}

/// Where the Linux fallback file goes; see `PlatformStore`.
fn fallback_path(app: &AppHandle) -> std::path::PathBuf {
    db_path(app).with_file_name("remote-identity.json")
}

async fn load_identity(app: &AppHandle) -> Result<Identity, String> {
    let remote = app.state::<Remote>();
    if let Some(id) = remote.identity.get() {
        return Ok(id.clone());
    }
    let store = PlatformStore::new(fallback_path(app));
    // Keychain access is synchronous and may wait on a user prompt;
    // keep it off the async workers.
    let id = tauri::async_runtime::spawn_blocking(move || identity::load_or_create(&store))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    // A concurrent load may have won; either copy is the same identity.
    Ok(remote.identity.get_or_init(|| id).clone())
}

/// Start the listener if it is not already running. Returns where it is
/// listening. Public so the pairing task can make sure the listener is
/// up before showing a QR code.
pub async fn start(app: &AppHandle) -> Result<SocketAddr, String> {
    let remote = app.state::<Remote>();
    let mut guard = remote.listener.lock().await;
    if let Some(h) = guard.as_ref() {
        return Ok(h.local_addr());
    }
    let identity = load_identity(app).await?;
    let handle = listener::start(ListenerConfig {
        bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, PORT)),
        identity,
        paired: remote.paired.clone(),
        desktop_version: app.package_info().version.to_string(),
        viewer_login: viewer_lookup(app),
        events: remote.events.clone(),
        // Managed in `lib.rs` before this module is set up.
        revocations: app.state::<Arc<PairingState>>().subscribe_revocations(),
    })
    .await
    .map_err(|e| e.to_string())?;
    let addr = handle.local_addr();
    *guard = Some(handle);
    Ok(addr)
}

/// Stop the listener, dropping every open connection. A no-op when it
/// is not running.
pub async fn stop(app: &AppHandle) {
    let remote = app.state::<Remote>();
    let handle = remote.listener.lock().await.take();
    if let Some(h) = handle {
        h.stop().await;
    }
}

/// Whether phones can connect right now.
#[tauri::command]
pub async fn get_remote_enabled(remote: State<'_, Remote>) -> Result<bool, String> {
    Ok(remote.is_running().await)
}

/// Turn phone connections on or off, and remember the choice.
///
/// The change happens BEFORE it is persisted: a port that cannot be
/// bound or a keychain that refuses is reported and nothing is saved,
/// so the toggle never claims a state the app is not in.
#[tauri::command]
pub async fn set_remote_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        let addr = start(&app).await?;
        log::info!("phone connections: on ({addr})");
    } else {
        stop(&app).await;
        log::info!("phone connections: off");
    }
    persist_enabled(&app, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_pairs_nothing_and_never_opens_the_window() {
        let stub = NoPairedDevices;
        assert!(!stub.is_paired("ab"));
        assert!(!stub.is_paired(""));
        assert!(!stub.pairing_window_open());
    }

    #[tokio::test]
    async fn remote_starts_with_no_listener_and_no_identity() {
        let remote = Remote::new(
            Arc::new(NoPairedDevices),
            Arc::new(Hub::new(Arc::new(|| Box::pin(async { None })))),
        );
        assert!(!remote.is_running().await);
        assert_eq!(remote.fingerprint(), None);
    }

    /// The setting is off by default: an absent key reads as `false`.
    #[test]
    fn allow_phone_connections_is_off_by_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = store::open_db(&dir.path().join("t.db")).unwrap();
        let v: Option<bool> =
            settings::get(&conn, settings::keys::ALLOW_PHONE_CONNECTIONS).unwrap();
        assert!(!v.unwrap_or(false));
    }

    /// Installing twice must not panic and must not error out loud:
    /// `registry.rs` installs the same provider lazily and either may
    /// run first.
    #[test]
    fn installing_the_crypto_provider_is_idempotent() {
        install_crypto_provider();
        install_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
