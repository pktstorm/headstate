//! The mTLS listener a paired phone talks to.
//!
//! axum on a tokio task, port [`PORT`] on every interface, TLS 1.3 only,
//! rustls on the aws-lc-rs provider. Client certificates are required at
//! the handshake and checked by fingerprint against the paired devices
//! -- no CA, no chain -- so an unpaired client never reaches HTTP.
//!
//! # The pairing seam
//!
//! The verifier does not know how devices are stored. It asks a
//! [`PairedCerts`] -- "is this fingerprint paired?" and "is a pairing
//! window open?" -- and the pairing task (#507) supplies the answer from
//! the `paired_devices` table and its live token. Until it lands,
//! `gate.rs` wires in a stub that pairs nothing and never opens the
//! window, so the listener can be enabled and refuses everyone.
//!
//! # What the handshake admits, and what HTTP then allows
//!
//! At the TLS layer a certificate is admitted if it is paired, OR if a
//! pairing window is open -- the phone about to pair has no row yet and
//! must get as far as `POST /v1/pair`. Above TLS, [`require_paired`]
//! lets an unpaired peer reach [`PAIR_PATH`] and nothing else, and
//! re-asks `is_paired` on EVERY request rather than trusting the
//! handshake, so a device revoked mid-connection is refused on its next
//! request, not only its next handshake. Handlers find who is calling
//! in the [`Peer`] request extension.
//!
//! The one request that outlives that check is `GET /v1/events`, a
//! server-sent event stream that stays open for as long as the phone
//! is; `remote/events.rs` watches the pairing task's revocation
//! broadcast to cut it.

use crate::remote::events::{self, Hub};
use crate::remote::identity::{fingerprint_of, Identity};
use axum::extract::{Extension, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use hyper_util::rt::TokioIo;
use rustls::crypto::{aws_lc_rs, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, DigitallySignedStruct, DistinguishedName, Error as TlsError, ServerConfig,
    SignatureScheme,
};
use serde::Serialize;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio_rustls::TlsAcceptor;

/// Fixed, from the dynamic range so it never collides with a well-known
/// service. The phone learns it from the pairing QR.
pub const PORT: u16 = 41919;

/// Bumped deliberately, in the spec, when the surface or the pairing
/// payload changes shape. Returned by `/v1/hello` and embedded in the QR.
pub const PROTOCOL_VERSION: u32 = 1;

/// The one path an unpaired peer may reach. The pairing task mounts its
/// handler here.
pub const PAIR_PATH: &str = "/v1/pair";

/// What the verifier needs to know about paired devices.
///
/// Implemented over `paired_devices` by the pairing task; over a
/// `HashSet` in the tests below. Both methods are called on the TLS
/// handshake path and on every request, so they must be cheap and must
/// not block: read an in-memory set, not the database.
pub trait PairedCerts: Send + Sync {
    /// Whether a device with this certificate fingerprint (lowercase hex
    /// SHA256 of its DER) is currently paired. Revocation is this
    /// returning `false` where it returned `true` before.
    fn is_paired(&self, sha256_fp_hex: &str) -> bool;
    /// Whether a pairing token is live, so an unpaired certificate may
    /// be admitted to reach `/v1/pair`.
    fn pairing_window_open(&self) -> bool;
}

/// Who is on the other end of the connection. Inserted into every
/// request's extensions by the connection task, from the certificate
/// the handshake verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Lowercase hex SHA256 of the peer's certificate DER.
    pub fingerprint: String,
}

/// How `/v1/hello` learns the signed-in GitHub login. A closure rather
/// than a client handle so the listener neither depends on octocrab nor
/// needs a network in tests. `None` when not signed in or unreachable;
/// the first `Some` is cached for the listener's lifetime, as the
/// frontend already caches the same answer.
pub type ViewerLookup =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

/// Everything `start` needs.
pub struct ListenerConfig {
    /// `0.0.0.0:41919` in production; `127.0.0.1:0` in tests.
    pub bind: SocketAddr,
    pub identity: Identity,
    pub paired: Arc<dyn PairedCerts>,
    /// The RUNTIME version from `package_info()`, not `CARGO_PKG_VERSION`,
    /// for the reason `latest_release` gives.
    pub desktop_version: String,
    pub viewer_login: ViewerLookup,
    /// The event fan-out `/v1/events` subscribers hang off. Lives in
    /// `gate::Remote` across listener restarts.
    pub events: Arc<Hub>,
    /// `PairingState::subscribe_revocations()`. Each event stream
    /// `resubscribe`s so a revocation reaches every open stream.
    pub revocations: broadcast::Receiver<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error("could not listen on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error("could not build the TLS configuration: {0}")]
    Tls(#[from] TlsError),
}

/// The body of `GET /v1/hello`.
#[derive(Serialize, Debug)]
pub struct Hello {
    pub desktop_version: String,
    pub protocol_version: u32,
    pub viewer_login: Option<String>,
}

struct AppState {
    paired: Arc<dyn PairedCerts>,
    desktop_version: String,
    viewer_login: ViewerLookup,
    viewer_cache: tokio::sync::OnceCell<String>,
    events: Arc<Hub>,
    revocations: broadcast::Receiver<String>,
}

/// A running listener. Dropping it stops accepting; [`Handle::stop`]
/// additionally waits for the accept loop to finish and every open
/// connection to be torn down, which is what "Allow phone connections:
/// off" should mean.
pub struct Handle {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl Handle {
    /// Where the listener actually bound -- matters when `bind` asked
    /// for port 0.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop accepting, drop every open connection, and wait for both.
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = (&mut self.task).await;
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// The provider the listener uses, regardless of the process default.
///
/// aws-lc-rs, because it is the only rustls provider with a
/// post-quantum key exchange, and X25519MLKEM768 placed FIRST
/// explicitly. rustls only orders it first under its
/// `prefer-post-quantum` cargo feature, which this crate does not enable
/// (rustls is declared with default features off, for the reasons in
/// Cargo.toml); spelling the order out here means the preference does
/// not depend on a feature flag anyone can drop by accident, and the
/// test `key_exchange_is_hybrid_post_quantum` holds it.
fn provider() -> CryptoProvider {
    CryptoProvider {
        kx_groups: vec![
            aws_lc_rs::kx_group::X25519MLKEM768,
            aws_lc_rs::kx_group::X25519,
            aws_lc_rs::kx_group::SECP256R1,
        ],
        ..aws_lc_rs::default_provider()
    }
}

/// Accepts a client certificate by fingerprint alone.
///
/// No chain building: the certificate is self-signed by the phone and
/// the desktop learned its fingerprint at pairing. The one thing this
/// still verifies cryptographically is the handshake signature -- that
/// the peer HOLDS the private key for the certificate it presented --
/// because without that a fingerprint is a public value anyone could
/// replay.
struct PairedVerifier {
    paired: Arc<dyn PairedCerts>,
    algs: WebPkiSupportedAlgorithms,
}

impl std::fmt::Debug for PairedVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairedVerifier").finish_non_exhaustive()
    }
}

impl ClientCertVerifier for PairedVerifier {
    /// Empty: sending no CA hints tells the phone "any certificate",
    /// which is right, since it has a self-signed one.
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        let fp = fingerprint_of(end_entity.as_ref());
        if self.paired.is_paired(&fp) || self.paired.pairing_window_open() {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(TlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    /// Never reached: the config offers TLS 1.3 only. Refuse rather than
    /// accept, so a future config change cannot quietly widen this.
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 is not offered".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// TLS 1.3 only, the desktop's identity, client certificates required
/// and checked by [`PairedVerifier`].
fn server_config(
    identity: &Identity,
    paired: Arc<dyn PairedCerts>,
) -> Result<ServerConfig, TlsError> {
    let provider = Arc::new(provider());
    let verifier = PairedVerifier {
        paired,
        algs: provider.signature_verification_algorithms,
    };
    ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(verifier))
        .with_single_cert(vec![identity.cert()], identity.key())
}

/// The path-level gate above TLS: an unpaired peer reaches `/v1/pair`
/// and nothing else. Re-checked per request so revocation takes effect
/// on the next request of an already-open connection.
async fn require_paired(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let Some(peer) = req.extensions().get::<Peer>() else {
        // The connection task always inserts one; its absence is a bug
        // in this file, not a client condition, and must not pass.
        log::error!("remote: a request arrived without a peer fingerprint");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if req.uri().path() == PAIR_PATH || st.paired.is_paired(&peer.fingerprint) {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "not paired").into_response()
    }
}

async fn hello(State(st): State<Arc<AppState>>) -> Json<Hello> {
    let lookup = st.viewer_login.clone();
    let viewer_login = st
        .viewer_cache
        .get_or_try_init(|| async move { lookup().await.ok_or(()) })
        .await
        .ok()
        .cloned();
    Json(Hello {
        desktop_version: st.desktop_version.clone(),
        protocol_version: PROTOCOL_VERSION,
        viewer_login,
    })
}

/// `GET /v1/events`: the stream described in `remote/events.rs`.
async fn events(State(st): State<Arc<AppState>>, Extension(peer): Extension<Peer>) -> Response {
    let sub = events::Subscriber {
        fingerprint: peer.fingerprint,
        revocations: st.revocations.resubscribe(),
        paired: st.paired.clone(),
    };
    match events::subscribe(&st.events, sub).await {
        Some(sse) => sse.into_response(),
        None => (StatusCode::FORBIDDEN, "not paired").into_response(),
    }
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/hello", get(hello))
        .route(events::PATH, get(events))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_paired,
        ))
        .with_state(state)
}

/// Bind, then serve on a spawned task until the handle is stopped.
///
/// Binding happens here rather than on the task so a port already in
/// use is reported to the caller -- and to the Settings toggle -- as an
/// error, not logged from a task nobody is watching.
pub async fn start(cfg: ListenerConfig) -> Result<Handle, ListenerError> {
    let tls = server_config(&cfg.identity, cfg.paired.clone())?;
    let listener = TcpListener::bind(cfg.bind)
        .await
        .map_err(|source| ListenerError::Bind {
            addr: cfg.bind,
            source,
        })?;
    let addr = listener
        .local_addr()
        .map_err(|source| ListenerError::Bind {
            addr: cfg.bind,
            source,
        })?;

    let state = Arc::new(AppState {
        paired: cfg.paired,
        desktop_version: cfg.desktop_version,
        viewer_login: cfg.viewer_login,
        viewer_cache: tokio::sync::OnceCell::new(),
        events: cfg.events,
        revocations: cfg.revocations,
    });
    let app = router(state);
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(accept_loop(listener, acceptor, app, shutdown_rx));

    log::info!("remote: listening on {addr}");
    Ok(Handle {
        addr,
        shutdown: Some(shutdown_tx),
        task,
    })
}

async fn accept_loop(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    app: Router,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut conns: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok((tcp, from)) => {
                    conns.spawn(serve_connection(tcp, from, acceptor.clone(), app.clone()));
                }
                Err(e) => {
                    // Out of file descriptors, most likely. Do not spin.
                    log::warn!("remote: accept failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            },
        }
        // Reap finished connections so the set does not grow without
        // bound over a long session.
        while conns.try_join_next().is_some() {}
    }
    // Abort every open connection: "off" means off, not "no new ones".
    conns.shutdown().await;
    log::info!("remote: listener stopped");
}

/// One connection: handshake, then HTTP/1.1 until it closes.
async fn serve_connection(tcp: TcpStream, from: SocketAddr, acceptor: TlsAcceptor, app: Router) {
    let tls = match acceptor.accept(tcp).await {
        Ok(tls) => tls,
        Err(e) => {
            // The expected outcome for an unpaired device, and the
            // signal a user wants to see when "something keeps trying".
            log::info!("remote: refused a connection from {from}: {e}");
            return;
        }
    };
    // The verifier has already approved this certificate; here it is
    // only named. Client auth is mandatory, so a handshake that
    // completed presented one.
    let Some(fp) = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|chain| chain.first())
        .map(|cert| fingerprint_of(cert.as_ref()))
    else {
        log::error!("remote: a handshake completed without a client certificate");
        return;
    };
    let peer = Peer { fingerprint: fp };

    let service = hyper::service::service_fn(move |mut req: Request<hyper::body::Incoming>| {
        req.extensions_mut().insert(peer.clone());
        let mut app = app.clone();
        async move { tower_service::Service::call(&mut app, req).await }
    });
    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(tls), service)
        .await
    {
        // A phone going out of range closes mid-request; that is
        // ordinary and not worth a warning.
        log::debug!("remote: connection from {from} ended: {e}");
    }
}

/// `pub(crate)`: `remote/events.rs` drives its stream tests through the
/// same in-memory pairing store and pinned client as the tests here.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::remote::events::SnapshotSource;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, NamedGroup};
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The in-memory `PairedCerts` the spec's verifier tests run against.
    #[derive(Default)]
    pub(crate) struct MemoryCerts {
        paired: Mutex<HashSet<String>>,
        window: AtomicBool,
    }

    impl MemoryCerts {
        pub(crate) fn pair(&self, fp: &str) {
            self.paired.lock().unwrap().insert(fp.to_string());
        }
        pub(crate) fn revoke(&self, fp: &str) {
            self.paired.lock().unwrap().remove(fp);
        }
        fn open_window(&self, open: bool) {
            self.window.store(open, Ordering::SeqCst);
        }
    }

    impl PairedCerts for MemoryCerts {
        fn is_paired(&self, fp: &str) -> bool {
            self.paired.lock().unwrap().contains(fp)
        }
        fn pairing_window_open(&self) -> bool {
            self.window.load(Ordering::SeqCst)
        }
    }

    /// The phone's side of the pin: accept the server certificate whose
    /// fingerprint matches, verify it holds the key, refuse anything
    /// else. Mirrors what `src-mobile/client.rs` will do.
    #[derive(Debug)]
    struct PinnedServer {
        fp: String,
        algs: WebPkiSupportedAlgorithms,
    }

    impl ServerCertVerifier for PinnedServer {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, TlsError> {
            if fingerprint_of(end_entity.as_ref()) == self.fp {
                Ok(ServerCertVerified::assertion())
            } else {
                Err(TlsError::InvalidCertificate(
                    CertificateError::ApplicationVerificationFailure,
                ))
            }
        }
        fn verify_tls12_signature(
            &self,
            _m: &[u8],
            _c: &CertificateDer<'_>,
            _d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            Err(TlsError::General("TLS 1.2 is not offered".into()))
        }
        fn verify_tls13_signature(
            &self,
            m: &[u8],
            c: &CertificateDer<'_>,
            d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            rustls::crypto::verify_tls13_signature(m, c, d, &self.algs)
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.algs.supported_schemes()
        }
    }

    pub(crate) struct Server {
        pub(crate) handle: Handle,
        pub(crate) fp: String,
        pub(crate) certs: Arc<MemoryCerts>,
        /// The pairing task's side of the revocation broadcast.
        pub(crate) revocations: broadcast::Sender<String>,
    }

    /// A hub with nothing to snapshot, for tests that never open the
    /// event stream.
    pub(crate) fn no_snapshot() -> SnapshotSource {
        Arc::new(|| Box::pin(async { None }))
    }

    async fn serve(certs: Arc<MemoryCerts>) -> Server {
        serve_with(certs, Arc::new(Hub::new(no_snapshot()))).await
    }

    pub(crate) async fn serve_with(certs: Arc<MemoryCerts>, events: Arc<Hub>) -> Server {
        let identity = Identity::generate().unwrap();
        let fp = identity.fingerprint();
        let (revocations, rx) = broadcast::channel(16);
        let handle = start(ListenerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            identity,
            paired: certs.clone(),
            desktop_version: "9.9.9".into(),
            viewer_login: Arc::new(|| Box::pin(async { Some("octocat".to_string()) })),
            events,
            revocations: rx,
        })
        .await
        .unwrap();
        Server {
            handle,
            fp,
            certs,
            revocations,
        }
    }

    pub(crate) fn client_config(phone: Option<&Identity>, server_fp: &str) -> Arc<ClientConfig> {
        let provider = Arc::new(provider());
        let verifier = PinnedServer {
            fp: server_fp.to_string(),
            algs: provider.signature_verification_algorithms,
        };
        let builder = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier));
        let cfg = match phone {
            Some(id) => builder
                .with_client_auth_cert(vec![id.cert()], id.key())
                .unwrap(),
            None => builder.with_no_client_auth(),
        };
        Arc::new(cfg)
    }

    struct Reply {
        status: u16,
        body: String,
        kx: Option<NamedGroup>,
    }

    /// One HTTP/1.1 GET over a fresh mTLS connection. `Err` for any
    /// failure at any layer -- a refused handshake shows up as the
    /// server's alert on the first read, after the client believed its
    /// half of the TLS 1.3 handshake was complete.
    async fn get(
        addr: SocketAddr,
        path: &str,
        phone: Option<&Identity>,
        server_fp: &str,
    ) -> Result<Reply, String> {
        let connector = tokio_rustls::TlsConnector::from(client_config(phone, server_fp));
        let tcp = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        let name = ServerName::try_from("localhost").unwrap();
        let mut tls = connector
            .connect(name, tcp)
            .await
            .map_err(|e| format!("handshake: {e}"))?;
        tls.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .map_err(|e| format!("write: {e}"))?;
        let mut raw = Vec::new();
        // The server closes after the response (Connection: close); a
        // refusal closes without one.
        let _ = tls.read_to_end(&mut raw).await;
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, body) = text
            .split_once("\r\n\r\n")
            .ok_or_else(|| format!("no response (got {} bytes)", raw.len()))?;
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("bad status line: {head}"))?;
        let kx = tls
            .get_ref()
            .1
            .negotiated_key_exchange_group()
            .map(|g| g.name());
        Ok(Reply {
            status,
            body: body.to_string(),
            kx,
        })
    }

    #[tokio::test]
    async fn a_paired_phone_gets_hello() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        let phone = Identity::generate().unwrap();
        server.certs.pair(&phone.fingerprint());

        let reply = get(
            server.handle.local_addr(),
            "/v1/hello",
            Some(&phone),
            &server.fp,
        )
        .await
        .unwrap();
        assert_eq!(reply.status, 200);
        let v: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(v["desktop_version"], "9.9.9");
        assert_eq!(v["protocol_version"], 1);
        assert_eq!(v["viewer_login"], "octocat");
        server.handle.stop().await;
    }

    #[tokio::test]
    async fn an_unpaired_phone_fails_the_handshake() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        let phone = Identity::generate().unwrap();

        let reply = get(
            server.handle.local_addr(),
            "/v1/hello",
            Some(&phone),
            &server.fp,
        )
        .await;
        assert!(
            reply.is_err(),
            "an unpaired certificate must not reach HTTP"
        );
        server.handle.stop().await;
    }

    #[tokio::test]
    async fn a_phone_without_a_certificate_is_refused() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        server.certs.open_window(true);

        let reply = get(server.handle.local_addr(), PAIR_PATH, None, &server.fp).await;
        assert!(
            reply.is_err(),
            "client auth is mandatory even while pairing"
        );
        server.handle.stop().await;
    }

    #[tokio::test]
    async fn a_revoked_phone_is_refused_on_its_next_handshake() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        let phone = Identity::generate().unwrap();
        server.certs.pair(&phone.fingerprint());
        let addr = server.handle.local_addr();

        assert_eq!(
            get(addr, "/v1/hello", Some(&phone), &server.fp)
                .await
                .unwrap()
                .status,
            200
        );
        server.certs.revoke(&phone.fingerprint());
        assert!(get(addr, "/v1/hello", Some(&phone), &server.fp)
            .await
            .is_err());
        // And a different, still-paired phone is unaffected.
        let other = Identity::generate().unwrap();
        server.certs.pair(&other.fingerprint());
        assert_eq!(
            get(addr, "/v1/hello", Some(&other), &server.fp)
                .await
                .unwrap()
                .status,
            200
        );
        server.handle.stop().await;
    }

    /// While a pairing window is open the handshake admits an unpaired
    /// certificate -- and the path gate then allows it `/v1/pair` only.
    /// 404 rather than 403 on the pair path proves it got past the gate
    /// (no handler is mounted there yet; that is the pairing task's).
    #[tokio::test]
    async fn an_open_pairing_window_admits_an_unpaired_phone_to_pair_and_nothing_else() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        let phone = Identity::generate().unwrap();
        server.certs.open_window(true);
        let addr = server.handle.local_addr();

        let hello = get(addr, "/v1/hello", Some(&phone), &server.fp)
            .await
            .unwrap();
        assert_eq!(hello.status, 403);

        let pair = get(addr, PAIR_PATH, Some(&phone), &server.fp)
            .await
            .unwrap();
        assert_eq!(pair.status, 404);

        // Window closed again: back to failing the handshake.
        server.certs.open_window(false);
        assert!(get(addr, "/v1/hello", Some(&phone), &server.fp)
            .await
            .is_err());
        server.handle.stop().await;
    }

    /// The spec's reason for aws-lc-rs: PR titles recorded off the wire
    /// today must stay private, so the key exchange is hybrid
    /// post-quantum. Held here explicitly rather than via a rustls
    /// feature flag; see `provider()`.
    #[tokio::test]
    async fn key_exchange_is_hybrid_post_quantum() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        let phone = Identity::generate().unwrap();
        server.certs.pair(&phone.fingerprint());

        let reply = get(
            server.handle.local_addr(),
            "/v1/hello",
            Some(&phone),
            &server.fp,
        )
        .await
        .unwrap();
        assert_eq!(reply.kx, Some(NamedGroup::X25519MLKEM768));
        server.handle.stop().await;
    }

    /// The pin on the phone's side is real: a server presenting some
    /// other certificate is refused by the test client. Without this the
    /// tests above could pass against any server at all.
    #[tokio::test]
    async fn the_client_refuses_a_server_whose_fingerprint_does_not_match() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        let phone = Identity::generate().unwrap();
        server.certs.pair(&phone.fingerprint());
        let wrong_fp = Identity::generate().unwrap().fingerprint();

        let reply = get(
            server.handle.local_addr(),
            "/v1/hello",
            Some(&phone),
            &wrong_fp,
        )
        .await;
        assert!(reply.is_err());
        server.handle.stop().await;
    }

    #[tokio::test]
    async fn hello_reports_no_login_when_not_signed_in() {
        let certs = Arc::new(MemoryCerts::default());
        let identity = Identity::generate().unwrap();
        let fp = identity.fingerprint();
        let handle = start(ListenerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            identity,
            paired: certs.clone(),
            desktop_version: "9.9.9".into(),
            viewer_login: Arc::new(|| Box::pin(async { None })),
            events: Arc::new(Hub::new(no_snapshot())),
            revocations: broadcast::channel(1).1,
        })
        .await
        .unwrap();
        let phone = Identity::generate().unwrap();
        certs.pair(&phone.fingerprint());

        let reply = get(handle.local_addr(), "/v1/hello", Some(&phone), &fp)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert!(v["viewer_login"].is_null());
        handle.stop().await;
    }

    #[tokio::test]
    async fn stop_closes_the_port() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs).await;
        let addr = server.handle.local_addr();
        assert!(TcpStream::connect(addr).await.is_ok());

        server.handle.stop().await;
        assert!(
            TcpStream::connect(addr).await.is_err(),
            "the port must be closed once the toggle is off"
        );
    }

    #[tokio::test]
    async fn a_port_in_use_is_reported_not_logged() {
        let certs = Arc::new(MemoryCerts::default());
        let server = serve(certs.clone()).await;
        let err = start(ListenerConfig {
            bind: server.handle.local_addr(),
            identity: Identity::generate().unwrap(),
            paired: certs,
            desktop_version: "9.9.9".into(),
            viewer_login: Arc::new(|| Box::pin(async { None })),
            events: Arc::new(Hub::new(no_snapshot())),
            revocations: broadcast::channel(1).1,
        })
        .await
        .err()
        .expect("binding a taken port must fail");
        assert!(matches!(err, ListenerError::Bind { .. }));
        assert!(err.to_string().contains("could not listen on"));
        server.handle.stop().await;
    }
}
