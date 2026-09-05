//! The HTTPS client for the paired desktop: reqwest on rustls with the
//! aws-lc-rs provider, the session certificate as the client identity,
//! and a server verifier that accepts one certificate -- the one whose
//! SHA256 fingerprint was pinned at pairing.
//!
//! - TLS 1.3 only, X25519MLKEM768 offered first (the same provider and
//!   order as the desktop's listener).
//! - No hostname check and no chain: the desktop's certificate is
//!   self-signed and names no host, because a laptop's address changes
//!   with every network. The fingerprint IS the identity; the handshake
//!   signature proves the peer holds its key.
//! - The desktop may be reachable at several addresses (LAN, overlay);
//!   [`Client`] tries them in order with a short connect timeout each
//!   and remembers the one that answered.
//!
//! # Errors the caller acts on
//!
//! [`ClientError::Handshake`] is the one that matters: TLS refused the
//! connection. Before pairing that means the QR's fingerprint does not
//! match what the server presented. After pairing it means the desktop
//! refused OUR certificate, which is what revocation looks like from
//! here -- the spec's "the phone learns it is revoked on its next
//! handshake failure". Everything network-shaped is
//! [`ClientError::Unreachable`], which the next address, or the next
//! retry, may fix.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{aws_lc_rs, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::keys::{fingerprint_of, SessionIdentity};
use crate::stepup;

/// The protocol this client speaks; `/v1/hello` reports the desktop's.
pub const PROTOCOL_VERSION: u32 = 1;

/// Per address. A LAN address that has gone away fails fast; an overlay
/// address on a slow link still connects within this.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// A whole command, connect included. `size_worktrees` on a large disk
/// is the slowest thing on the surface.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Silence on the event stream before it is treated as dead. The desktop
/// writes a keep-alive every 15 seconds, so three missed is a broken
/// path, not a quiet one.
pub const STREAM_READ_TIMEOUT: Duration = Duration::from_secs(45);

/// The body of `GET /v1/hello`. Stored with the pairing record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub desktop_version: String,
    pub protocol_version: u32,
    pub viewer_login: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// TLS refused: the server's fingerprint is not the pinned one, or
    /// the server refused this phone's certificate.
    #[error("TLS handshake failed: {0}")]
    Handshake(String),
    /// Could not reach the desktop at any address.
    #[error("desktop unreachable: {0}")]
    Unreachable(String),
    /// The desktop answered with a non-2xx status; `message` is its body.
    #[error("{message}")]
    Status { status: u16, message: String },
    /// The desktop answered something this client cannot use.
    #[error("bad reply from the desktop: {0}")]
    Protocol(String),
}

impl ClientError {
    pub fn is_handshake(&self) -> bool {
        matches!(self, ClientError::Handshake(_))
    }
}

/// aws-lc-rs with X25519MLKEM768 first, spelled out rather than left to
/// rustls's `prefer-post-quantum` feature, exactly as the desktop does.
pub(crate) fn provider() -> CryptoProvider {
    CryptoProvider {
        kx_groups: vec![
            aws_lc_rs::kx_group::X25519MLKEM768,
            aws_lc_rs::kx_group::X25519,
            aws_lc_rs::kx_group::SECP256R1,
        ],
        ..aws_lc_rs::default_provider()
    }
}

/// Accepts the one certificate whose fingerprint matches; verifies the
/// handshake signature so the fingerprint cannot be replayed by someone
/// without the key.
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

    /// Never reached: TLS 1.3 only. Refuse rather than accept, so a
    /// config change cannot quietly widen this.
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 is not offered".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// TLS 1.3 only, the pinned verifier, and the session certificate as
/// the client identity. `server_fp` is lowercase hex, no prefix.
pub fn tls_config(identity: &SessionIdentity, server_fp: &str) -> Result<ClientConfig, TlsError> {
    let provider = Arc::new(provider());
    let verifier = PinnedServer {
        fp: server_fp.to_string(),
        algs: provider.signature_verification_algorithms,
    };
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_auth_cert(
            vec![CertificateDer::from(identity.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_pkcs8.clone())),
        )?;
    // The desktop serves HTTP/1.1; say so rather than let ALPN guess.
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    // No session resumption. A resumed TLS 1.3 session skips the
    // certificate exchange, so a phone holding a ticket from before its
    // revocation would get past the desktop's verifier on the strength
    // of the old handshake. Every connection is a full handshake, which
    // is what "the phone learns it is revoked on its next handshake
    // failure" needs to be true. (Observed, not theoretical: with
    // resumption on, the revocation test reached the path gate.)
    cfg.resumption = rustls::client::Resumption::disabled();
    Ok(cfg)
}

/// One paired desktop, reachable at any of `addrs` on `port`.
pub struct Client {
    calls: reqwest::Client,
    stream: reqwest::Client,
    addrs: Vec<String>,
    port: u16,
    /// Index into `addrs` of the last address that answered; tried
    /// first next time.
    preferred: Mutex<Option<usize>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("addrs", &self.addrs)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(
        identity: &SessionIdentity,
        server_fp: &str,
        addrs: Vec<String>,
        port: u16,
    ) -> Result<Self, ClientError> {
        if addrs.is_empty() {
            return Err(ClientError::Unreachable("no addresses to try".into()));
        }
        let tls = tls_config(identity, server_fp)
            .map_err(|e| ClientError::Protocol(format!("TLS configuration: {e}")))?;
        let base = || {
            reqwest::Client::builder()
                .tls_backend_preconfigured(tls.clone())
                .connect_timeout(CONNECT_TIMEOUT)
                .http1_only()
                .no_proxy()
        };
        let build = |b: reqwest::ClientBuilder| {
            b.build()
                .map_err(|e| ClientError::Protocol(format!("HTTP client: {e}")))
        };
        Ok(Self {
            calls: build(base().timeout(CALL_TIMEOUT))?,
            stream: build(base().read_timeout(STREAM_READ_TIMEOUT))?,
            addrs,
            port,
            preferred: Mutex::new(None),
        })
    }

    /// `https://<addr>:<port>`, with IPv6 literals bracketed.
    pub fn base_url(addr: &str, port: u16) -> String {
        if addr.contains(':') && !addr.starts_with('[') {
            format!("https://[{addr}]:{port}")
        } else {
            format!("https://{addr}:{port}")
        }
    }

    /// Address indices in the order to try: the last one that answered,
    /// then the rest in the QR's order.
    fn order(&self) -> Vec<usize> {
        let preferred = *self.preferred.lock().unwrap_or_else(|e| e.into_inner());
        let mut order: Vec<usize> = preferred.into_iter().collect();
        order.extend((0..self.addrs.len()).filter(|i| Some(*i) != preferred));
        order
    }

    /// Run `f` against each base URL until one answers. An address that
    /// cannot be reached is skipped for the next; any other outcome --
    /// a handshake refusal, a status, a bad reply -- is the desktop's
    /// answer and is returned at once, since the other addresses lead
    /// to the same desktop.
    async fn try_each<T, F, Fut>(&self, f: F) -> Result<T, ClientError>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<T, ClientError>>,
    {
        let mut last = None;
        for i in self.order() {
            match f(Self::base_url(&self.addrs[i], self.port)).await {
                Ok(v) => {
                    *self.preferred.lock().unwrap_or_else(|e| e.into_inner()) = Some(i);
                    return Ok(v);
                }
                Err(ClientError::Unreachable(m)) => {
                    log::info!("companion: {} did not answer: {m}", self.addrs[i]);
                    last = Some(m);
                }
                Err(e) => return Err(e),
            }
        }
        Err(ClientError::Unreachable(
            last.unwrap_or_else(|| "no addresses to try".into()),
        ))
    }

    /// `GET /v1/hello`.
    pub async fn hello(&self) -> Result<Hello, ClientError> {
        self.try_each(|base| async move {
            let resp = self
                .calls
                .get(format!("{base}/v1/hello"))
                .send()
                .await
                .map_err(classify)?;
            json_body(resp).await
        })
        .await
    }

    /// `POST /v1/pair`. `body` is the pair request; the reply is the
    /// desktop's 200 body, or [`ClientError::Status`] with its refusal.
    pub async fn pair<Req: Serialize, Out: for<'de> Deserialize<'de>>(
        &self,
        body: &Req,
    ) -> Result<Out, ClientError> {
        self.try_each(|base| async move {
            let resp = self
                .calls
                .post(format!("{base}/v1/pair"))
                .json(body)
                .send()
                .await
                .map_err(classify)?;
            json_body(resp).await
        })
        .await
    }

    /// `POST /v1/call/{command}` with `args` as the body and, for a
    /// destructive command, the step-up header.
    pub async fn call(
        &self,
        command: &str,
        args: &Value,
        signature: Option<&str>,
    ) -> Result<Value, ClientError> {
        self.try_each(|base| async move {
            let mut req = self
                .calls
                .post(format!("{base}/v1/call/{command}"))
                .json(args);
            if let Some(sig) = signature {
                req = req.header(stepup::HEADER, sig);
            }
            let resp = req.send().await.map_err(classify)?;
            json_body(resp).await
        })
        .await
    }

    /// `GET /v1/events`: the response whose body is the event stream,
    /// returned once the headers are in. The caller reads chunks.
    pub async fn events(&self) -> Result<reqwest::Response, ClientError> {
        self.try_each(|base| async move {
            let resp = self
                .stream
                .get(format!("{base}/v1/events"))
                .header("accept", "text/event-stream")
                .send()
                .await
                .map_err(classify)?;
            ok_status(resp).await
        })
        .await
    }
}

/// The response if 2xx, else [`ClientError::Status`] carrying the body.
async fn ok_status(resp: reqwest::Response) -> Result<reqwest::Response, ClientError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(ClientError::Status {
        status: status.as_u16(),
        message: status_message(status.as_u16(), &body),
    })
}

/// The desktop's refusal as one line: a JSON body's `error` or
/// `message` string if it has one, else the body, else the status.
fn status_message(status: u16, body: &str) -> String {
    let body = body.trim();
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body) {
        for key in ["error", "message"] {
            if let Some(Value::String(s)) = map.get(key) {
                return s.clone();
            }
        }
    }
    if body.is_empty() {
        format!("desktop answered HTTP {status}")
    } else {
        body.to_string()
    }
}

async fn json_body<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
) -> Result<T, ClientError> {
    let resp = ok_status(resp).await?;
    let bytes = resp.bytes().await.map_err(classify)?;
    serde_json::from_slice(&bytes).map_err(|e| ClientError::Protocol(e.to_string()))
}

/// Sort a transport error into the two the caller distinguishes.
fn classify(err: reqwest::Error) -> ClientError {
    if is_tls_failure(&err) {
        return ClientError::Handshake(err.to_string());
    }
    if err.is_decode() {
        return ClientError::Protocol(err.to_string());
    }
    ClientError::Unreachable(err.to_string())
}

/// Whether a `rustls::Error` sits anywhere in the chain. rustls wraps
/// its errors in `io::Error` (kind `InvalidData`), and `io::Error`'s
/// `source()` skips over the wrapped error to ITS source, so the chain
/// walk also looks inside every `io::Error` it meets.
fn is_tls_failure(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if e.downcast_ref::<rustls::Error>().is_some() {
            return true;
        }
        if let Some(inner) = e
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
        {
            let inner: &(dyn std::error::Error + 'static) = inner;
            if is_tls_failure(inner) {
                return true;
            }
        }
        cur = e.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{DeviceKeys, SoftwareKeys};
    use crate::store::MemoryStore;
    use crate::testing::{Reply, TestServer};
    use serde_json::json;

    fn identity() -> SessionIdentity {
        let keys = SoftwareKeys::new(Arc::new(MemoryStore::default()));
        keys.generate().unwrap();
        keys.session_identity().unwrap()
    }

    #[test]
    fn base_urls_bracket_ipv6_literals() {
        assert_eq!(
            Client::base_url("192.0.2.10", 41919),
            "https://192.0.2.10:41919"
        );
        assert_eq!(
            Client::base_url("2001:db8::7", 41919),
            "https://[2001:db8::7]:41919"
        );
        assert_eq!(
            Client::base_url("[2001:db8::7]", 1),
            "https://[2001:db8::7]:1"
        );
    }

    #[test]
    fn a_status_message_prefers_the_desktops_own_words() {
        assert_eq!(status_message(403, "not paired"), "not paired");
        assert_eq!(
            status_message(403, r#"{"error":"pairing was denied"}"#),
            "pairing was denied"
        );
        assert_eq!(status_message(500, r#"{"message":"boom"}"#), "boom");
        assert_eq!(status_message(502, ""), "desktop answered HTTP 502");
    }

    #[test]
    fn the_config_offers_tls13_only_with_the_hybrid_group_first() {
        let cfg = tls_config(&identity(), &"ab".repeat(32)).unwrap();
        assert_eq!(
            cfg.crypto_provider().kx_groups[0].name(),
            rustls::NamedGroup::X25519MLKEM768
        );
        assert_eq!(cfg.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn a_rustls_error_is_found_inside_an_io_error() {
        let tls = TlsError::InvalidCertificate(CertificateError::ApplicationVerificationFailure);
        let io = std::io::Error::new(std::io::ErrorKind::InvalidData, tls);
        assert!(is_tls_failure(&io));
        assert!(!is_tls_failure(&std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused"
        )));
    }

    #[tokio::test]
    async fn a_paired_phone_gets_hello_and_the_key_exchange_is_post_quantum() {
        let id = identity();
        let server = TestServer::start().await;
        server.pair(&id.fingerprint());
        let client = Client::new(&id, &server.fp, vec![server.addr()], server.port()).unwrap();
        let hello = client.hello().await.unwrap();
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
        assert_eq!(hello.viewer_login.as_deref(), Some("octocat"));
        assert_eq!(server.last_kx(), Some(rustls::NamedGroup::X25519MLKEM768));
    }

    #[tokio::test]
    async fn the_wrong_server_fingerprint_is_a_handshake_failure() {
        let id = identity();
        let server = TestServer::start().await;
        server.pair(&id.fingerprint());
        let client =
            Client::new(&id, &"00".repeat(32), vec![server.addr()], server.port()).unwrap();
        let err = client.hello().await.unwrap_err();
        assert!(err.is_handshake(), "{err:?}");
    }

    #[tokio::test]
    async fn a_server_that_refuses_our_certificate_is_a_handshake_failure() {
        // What revocation looks like from the phone.
        let id = identity();
        let server = TestServer::start().await;
        let client = Client::new(&id, &server.fp, vec![server.addr()], server.port()).unwrap();
        let err = client.hello().await.unwrap_err();
        assert!(err.is_handshake(), "{err:?}");
    }

    #[tokio::test]
    async fn addresses_are_tried_in_order_and_the_one_that_answers_is_remembered() {
        let id = identity();
        let server = TestServer::start().await;
        server.pair(&id.fingerprint());
        // 192.0.2.1 is TEST-NET-1: nothing routes there, so it times out
        // (or is refused) rather than answering.
        let client = Client::new(
            &id,
            &server.fp,
            vec!["192.0.2.1".into(), server.addr()],
            server.port(),
        )
        .unwrap();
        assert_eq!(client.order(), vec![0, 1]);
        client.hello().await.unwrap();
        assert_eq!(
            client.order(),
            vec![1, 0],
            "the answering address moves first"
        );
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn no_address_answering_is_unreachable() {
        let id = identity();
        let server = TestServer::start().await;
        let port = server.port();
        drop(server);
        let client = Client::new(&id, &"ab".repeat(32), vec!["127.0.0.1".into()], port).unwrap();
        assert!(matches!(
            client.hello().await,
            Err(ClientError::Unreachable(_))
        ));
    }

    #[tokio::test]
    async fn call_posts_the_args_and_the_signature_header_and_returns_the_body() {
        let id = identity();
        let server = TestServer::start().await;
        server.pair(&id.fingerprint());
        server.reply(
            "/v1/call/remove_orphan",
            Reply::json(200, json!({"removed": true})),
        );
        let client = Client::new(&id, &server.fp, vec![server.addr()], server.port()).unwrap();
        let out = client
            .call(
                "remove_orphan",
                &json!({"path": "/srv/x"}),
                Some("v1;ts=1;nonce=n;ecdsa=e"),
            )
            .await
            .unwrap();
        assert_eq!(out, json!({"removed": true}));
        let req = server.requests().pop().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/call/remove_orphan");
        assert_eq!(
            req.header("x-headstate-signature"),
            Some("v1;ts=1;nonce=n;ecdsa=e")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&req.body).unwrap(),
            json!({"path": "/srv/x"})
        );
    }

    #[tokio::test]
    async fn a_non_2xx_is_a_status_error_with_the_desktops_message() {
        let id = identity();
        let server = TestServer::start().await;
        server.pair(&id.fingerprint());
        server.reply(
            "/v1/call/get_cached",
            Reply::text(500, "gh auth status failed"),
        );
        let client = Client::new(&id, &server.fp, vec![server.addr()], server.port()).unwrap();
        assert_eq!(
            client
                .call("get_cached", &json!({}), None)
                .await
                .unwrap_err(),
            ClientError::Status {
                status: 500,
                message: "gh auth status failed".into()
            }
        );
    }
}
