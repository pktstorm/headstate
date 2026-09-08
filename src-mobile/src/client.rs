//! The HTTPS client for the paired desktop: reqwest on rustls with the
//! aws-lc-rs provider (which carries ML-DSA as of 0.23.44), the ML-DSA-65
//! session certificate as the client identity, and a server verifier
//! that accepts one certificate -- the one whose SHA256 fingerprint was
//! pinned at pairing -- signed with ML-DSA-65 and nothing else.
//!
//! - TLS 1.3 only, X25519MLKEM768 offered first (the same provider and
//!   order as the desktop's listener). The provider is the one that can
//!   sign CertificateVerify with the ML-DSA-65 session key and verify
//!   the desktop's; the plain aws-lc-rs provider cannot load the key
//!   at all. See Cargo.toml for what its `aws-lc-rs-unstable` feature
//!   does and does not mean.
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
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, PeerMisbehaved,
    SignatureScheme,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use crate::discovery;
use crate::keys::{fingerprint_of, SessionIdentity};
use crate::stepup;

/// The protocol this client speaks; `/v1/hello` reports the desktop's,
/// and `connection.rs` calls a desktop below this too old to drive.
///
/// 2 (#521): both TLS certificates are ML-DSA-65. A 5.0 desktop
/// (protocol 1) presents a P-256 certificate this client refuses at
/// the handshake anyway, and its verifier would refuse ours.
pub const PROTOCOL_VERSION: u32 = 2;

/// The one signature scheme the desktop may sign the handshake with.
const SERVER_SIGNATURE_SCHEME: SignatureScheme = SignatureScheme::ML_DSA_65;

/// Per address, and only the TCP connect -- not the TLS handshake, and
/// not the request. `CALL_TIMEOUT` bounds the whole thing.
///
/// A LAN peer that is up answers a connect in single-digit
/// milliseconds; an overlay hop (Tailscale, WireGuard) over cellular is
/// the slow case, and still an order of magnitude inside this. What the
/// old three seconds bought was a pathological WAN that this app does
/// not have -- and it cost that on EVERY dead address, serially, before
/// the phone could say the desktop was unreachable.
///
/// It is deliberately not tighter than a second. Below that a cold
/// radio waking for the first packet starts to look like a dead
/// address, and the cost of a false "unreachable" is a user who thinks
/// their desktop is off.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// How long one attempt gets to itself before the next starts beside
/// it.
///
/// RFC 8305 suggests 250ms for happy-eyeballs, and that is the right
/// order here for the same reason: long enough that a healthy address
/// wins outright and nothing speculative is ever sent, short enough
/// that a dead one does not hold the user. A LAN peer answers a connect
/// in single-digit milliseconds, so in the common case the second
/// attempt is never started at all.
const ATTEMPT_STAGGER: Duration = Duration::from_millis(250);

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

/// aws-lc-rs, which carries the ML-DSA signing keys and verifiers
/// itself as of rustls 0.23.44 -- it was `rustls-post-quantum`'s
/// provider until #588 -- with X25519MLKEM768 first, spelled out rather
/// than left to rustls's `prefer-post-quantum` feature, exactly as the
/// desktop does. That explicit order is what kept the key exchange
/// hybrid when the post-quantum crate (and the feature it enabled) went
/// away.
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
/// without the key -- and accepts ML-DSA-65 as that signature's scheme
/// and nothing else, so a desktop from before protocol 2 (P-256) is
/// refused at the handshake rather than reasoned about afterwards.
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

    /// ML-DSA-65 only. The provider's table maps the scheme to webpki's
    /// ML-DSA-65 verifier, which also checks that the certificate's
    /// public key is an ML-DSA-65 key.
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        if dss.scheme != SERVER_SIGNATURE_SCHEME {
            return Err(PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme.into());
        }
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    /// What ClientHello's signature_algorithms offers the desktop.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SERVER_SIGNATURE_SCHEME]
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
/// How long a rediscovery browse waits. The module doc for
/// `discovery.rs` names three seconds; a multicast reply from a desktop
/// on the same LAN arrives in milliseconds, and the rest of the window
/// is for a busy network rather than for a desktop that is not there.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Client {
    calls: reqwest::Client,
    stream: reqwest::Client,
    addrs: Vec<String>,
    port: u16,
    /// Index into `addrs` of the last address that answered; tried
    /// first next time.
    preferred: Mutex<Option<usize>>,
    /// The paired desktop's fingerprint, for matching its mDNS record.
    /// Only a hint about WHERE to connect; the pinned fingerprint on the
    /// TLS handshake is what proves the desktop is the right one.
    server_fp: String,
    /// An address learned from mDNS, kept until it too stops answering.
    ///
    /// The QR's addresses go stale the moment the desktop's DHCP lease
    /// changes or it moves networks, and nothing else ever updates them
    /// -- `addrs` is fixed at pairing and re-read verbatim on every
    /// start. So a laptop that renewed its lease overnight left the
    /// phone permanently unreachable, recoverable only by re-pairing.
    discovered: Mutex<Option<String>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("addrs", &self.addrs)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// Wait for the first of `running` to settle, removing and returning it.
///
/// With `widen`, gives up after [`ATTEMPT_STAGGER`] and returns `None`
/// so the caller can start another attempt alongside these; without it
/// there is nothing left to add, so it waits as long as the attempts
/// themselves take.
///
/// Hand-rolled rather than `FuturesUnordered` because the mobile crate
/// deliberately carries no `futures` dependency, and this is the only
/// place that needs it. Polling every attempt on each wake is O(n) in a
/// list that is at most a handful of addresses.
///
/// The settled future's value is captured AS it becomes ready -- taken
/// out of the poll rather than recovered by polling again, since a
/// future that has returned `Ready` must not be polled a second time.
async fn poll_settled<T, Fut>(
    running: &mut Vec<(String, Pin<Box<Fut>>)>,
    widen: bool,
) -> Option<(String, Result<T, ClientError>)>
where
    Fut: Future<Output = Result<T, ClientError>>,
{
    let mut done: Option<(usize, Result<T, ClientError>)> = None;
    let settled = std::future::poll_fn(|cx| {
        for (i, (_, fut)) in running.iter_mut().enumerate() {
            if let Poll::Ready(v) = fut.as_mut().poll(cx) {
                done = Some((i, v));
                return Poll::Ready(());
            }
        }
        Poll::Pending
    });

    if widen {
        // The stagger elapsing is not a failure: it is the signal to
        // start the next address beside the ones already running, which
        // keep running.
        if tokio::time::timeout(ATTEMPT_STAGGER, settled)
            .await
            .is_err()
        {
            return None;
        }
    } else {
        settled.await;
    }

    let (i, value) = done?;
    let (base, _) = running.remove(i);
    Some((base, value))
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
            server_fp: server_fp.to_string(),
            discovered: Mutex::new(None),
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

    /// Every base URL to try, in order: an address mDNS found last time,
    /// then the stored ones.
    fn bases(&self) -> Vec<String> {
        let found = self
            .discovered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut out: Vec<String> = found.into_iter().collect();
        out.extend(
            self.order()
                .into_iter()
                .map(|i| Self::base_url(&self.addrs[i], self.port)),
        );
        out
    }

    /// Ask the LAN where the paired desktop is now.
    ///
    /// Only after every stored address has failed: while one of them
    /// answers there is nothing to look up, and a multicast browse on
    /// each call would be three seconds of latency for a question
    /// already answered. `None` is not an error -- a different network,
    /// no multicast, a listener that is off, and an iOS build without
    /// the local-network entitlement all look identical from here -- so
    /// the caller reports the same unreachable it would have anyway.
    ///
    /// Blocking, on a multicast channel, so it runs on the blocking
    /// pool rather than parking a tokio worker for the timeout.
    async fn rediscover(&self) -> Option<String> {
        let prefix = discovery::fp_prefix(&self.server_fp);
        let found = tauri::async_runtime::spawn_blocking(move || {
            discovery::browse(&prefix, DISCOVERY_TIMEOUT)
        })
        .await
        .ok()
        .flatten()?;
        let (ip, port) = found;
        let base = Self::base_url(&ip.to_string(), port);
        log::info!("companion: found the desktop at {base} over mDNS");
        Some(base)
    }

    /// Run `f` against each base URL until one answers, starting them
    /// STAGGERED rather than strictly one after another.
    ///
    /// # Why not purely serial
    ///
    /// The QR carries a LAN address and an overlay address, and the
    /// phone has no way to know which is live. Serially, a phone that
    /// has left the desktop's LAN paid the full connect timeout on the
    /// dead address before trying the one that works -- on the first
    /// call after every network change, and on every call until one
    /// succeeded and `preferred` was set.
    ///
    /// # Why not all at once
    ///
    /// Firing every address simultaneously is worse than it sounds
    /// here. Each attempt that gets past TCP does a full mTLS handshake
    /// with ML-DSA-65 certificates on both sides, and `bases()` can
    /// return an address found over mDNS PLUS every stored one. On a
    /// phone that is several post-quantum handshakes for a question one
    /// of them was going to answer, paid in radio and battery.
    ///
    /// # What this does
    ///
    /// RFC 8305's shape, minus the address-family interleaving that
    /// does not apply: start the first attempt, and only if it has not
    /// settled within [`ATTEMPT_STAGGER`] start the next alongside it.
    /// A reachable first address -- the overwhelmingly common case,
    /// since `bases()` puts the last one that worked first -- therefore
    /// costs exactly one attempt, and nothing is fired speculatively
    /// until an address has actually been slow.
    ///
    /// # Error precedence
    ///
    /// Unchanged, and load-bearing. An address that cannot be REACHED is
    /// skipped; any other outcome -- a handshake refusal, a status, a
    /// bad reply -- is the desktop's own answer and is returned at once,
    /// because every address leads to the same desktop and asking it
    /// again elsewhere would get the same answer. Under concurrency that
    /// has to hold against the clock too: a refusal from the second
    /// attempt must win over a slower `Unreachable` from the first,
    /// rather than being raced out by it. `select!` on a set that drops
    /// the losers gives exactly that -- the first NON-`Unreachable`
    /// outcome returns and the rest are cancelled where they stand.
    async fn try_each<T, F, Fut>(&self, f: F) -> Result<T, ClientError>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<T, ClientError>>,
    {
        let bases = self.bases();
        // EVERY attempt, not just the most recent. A phone that cannot
        // reach its desktop is the hardest failure to diagnose from the
        // outside -- the user sees one red box and has no way to learn
        // whether the address was refused, timed out, or was never
        // tried. Overwriting a single `last` threw away exactly the
        // facts that distinguish those (#633).
        let mut attempts: Vec<String> = Vec::new();
        // Attempts started and not yet settled, each tagged with the
        // base it is for so a win can be remembered.
        let mut running: Vec<(String, Pin<Box<Fut>>)> = Vec::new();
        let mut next = 0;

        loop {
            if next < bases.len() {
                running.push((bases[next].clone(), Box::pin(f(bases[next].clone()))));
                next += 1;
            } else if running.is_empty() {
                break;
            }

            // Wait for whichever attempt settles first, but no longer
            // than the stagger while there is still an address to add.
            // `poll_settled` returns None on the stagger elapsing, which
            // is the signal to widen rather than to give up.
            let more = next < bases.len();
            let Some((base, result)) = poll_settled(&mut running, more).await else {
                continue;
            };
            match result {
                Ok(v) => {
                    self.remember(&base);
                    return Ok(v);
                }
                Err(ClientError::Unreachable(m)) => {
                    log::info!("companion: {base} did not answer: {m}");
                    attempts.push(format!("{base}: {m}"));
                }
                // The desktop's own answer. Returning here drops every
                // other in-flight attempt.
                Err(e) => return Err(e),
            }
        }
        // Every address we knew about is dead. Before reporting the
        // desktop unreachable, ask the LAN where it went: this is the
        // DHCP-renewal case, which otherwise stranded the phone for
        // good.
        if let Some(base) = self.rediscover().await {
            match f(base.clone()).await {
                Ok(v) => {
                    *self.discovered.lock().unwrap_or_else(|e| e.into_inner()) = Some(base);
                    return Ok(v);
                }
                Err(ClientError::Unreachable(m)) => {
                    log::info!("companion: the address from mDNS did not answer: {m}");
                    attempts.push(format!("{base} (found by mDNS): {m}"));
                }
                Err(e) => return Err(e),
            }
        }
        // One line per address tried, so the message names what was
        // attempted and why each failed. Addresses and error kinds only:
        // the pairing token, the session certificate and any key
        // material stay out of a string the UI will display.
        Err(ClientError::Unreachable(if attempts.is_empty() {
            "no addresses to try".into()
        } else {
            attempts.join("; ")
        }))
    }

    /// Record which base URL answered, so it is tried first next time.
    fn remember(&self, base: &str) {
        if let Some(i) =
            (0..self.addrs.len()).find(|i| Self::base_url(&self.addrs[*i], self.port) == base)
        {
            *self.preferred.lock().unwrap_or_else(|e| e.into_inner()) = Some(i);
        }
        // A discovered address is already at the front of `bases()`, so
        // there is nothing to reorder when that is what answered.
    }

    /// The address currently believed good: what `bases()` would try
    /// first, or `None` before anything has answered.
    ///
    /// For the caller to persist. `Client` deliberately holds no store
    /// handle -- it is the network layer, and giving it one to write a
    /// preference through would put disk I/O behind every request.
    /// `Companion` owns the store and writes this back after a
    /// successful call, so the next COLD START begins with the address
    /// that last worked instead of re-walking the QR's list from the
    /// top and paying a discovery browse again.
    pub fn best_address(&self) -> Option<String> {
        // A bare address in both branches, matching `Desktop::addrs`.
        // `discovered` holds a base URL because that is what `bases()`
        // hands to the request; it is unwrapped back here rather than
        // stored twice, so there is one representation on disk.
        if let Some(base) = self
            .discovered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
        {
            return Self::addr_of(base);
        }
        let preferred = *self.preferred.lock().unwrap_or_else(|e| e.into_inner());
        preferred.map(|i| self.addrs[i].clone())
    }

    /// The host out of a base URL this module built, undoing
    /// [`Self::base_url`] -- including the brackets it puts around an
    /// IPv6 literal.
    fn addr_of(base: &str) -> Option<String> {
        let rest = base.strip_prefix("https://")?;
        let host = match rest.rsplit_once(':') {
            // `]` means the colon we found is inside an IPv6 literal
            // rather than the port separator.
            Some((h, _)) if !h.ends_with(']') => h,
            _ => rest,
        };
        Some(
            host.strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host)
                .to_string(),
        )
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
        // The phone signed CertificateVerify with ML-DSA-65 -- what the
        // desktop's verifier saw -- and verified the desktop's ML-DSA-65
        // certificate, the only kind `PinnedServer` accepts.
        assert_eq!(server.last_sig(), Some(SignatureScheme::ML_DSA_65));
        assert!(server.cert_is_ml_dsa_65());
    }

    /// What ClientHello offers, and what the verifier will check.
    #[test]
    fn the_verifier_accepts_ml_dsa_65_and_nothing_else() {
        let provider = provider();
        let verifier = PinnedServer {
            fp: "ab".repeat(32),
            algs: provider.signature_verification_algorithms,
        };
        assert_eq!(
            verifier.supported_verify_schemes(),
            vec![SignatureScheme::ML_DSA_65]
        );
        assert!(provider
            .signature_verification_algorithms
            .supported_schemes()
            .contains(&SignatureScheme::ML_DSA_65));
        // NOT asserted: that plain aws-lc-rs cannot verify ML-DSA-65.
        // True when written, false since rustls 0.23.44. See #588.
    }

    /// A 5.0 desktop presents a P-256 certificate. Even with its
    /// fingerprint pinned, the handshake fails: this client offers no
    /// scheme it can sign with. That is what a phone on protocol 2 sees
    /// of a desktop on protocol 1, before `/v1/hello` can say so.
    #[tokio::test]
    async fn a_desktop_with_a_p256_certificate_is_a_handshake_failure() {
        let id = identity();
        let server = TestServer::start_with_p256_certificate().await;
        server.pair(&id.fingerprint());
        let client = Client::new(&id, &server.fp, vec![server.addr()], server.port()).unwrap();
        let err = client.hello().await.unwrap_err();
        assert!(err.is_handshake(), "{err:?}");
        assert!(server.requests().is_empty(), "nothing reached HTTP");
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

    /// Every address that failed is named, not just the last one.
    ///
    /// This is what makes "could not reach that desktop" diagnosable
    /// (#633): a user on the same network as their desktop needs to see
    /// WHICH addresses were tried and how each failed, and the previous
    /// code overwrote a single `last`, so the first failure was lost.
    #[tokio::test]
    async fn unreachable_names_every_address_that_was_tried() {
        let id = identity();
        // Both TEST-NET-1: nothing routes there, so both fail and the
        // error has to account for both.
        let client = Client::new(
            &id,
            "aa:bb",
            vec!["192.0.2.1".into(), "192.0.2.2".into()],
            9,
        )
        .unwrap();
        let err = client.hello().await.unwrap_err();
        let ClientError::Unreachable(m) = err else {
            panic!("expected Unreachable, got {err:?}");
        };
        assert!(m.contains("192.0.2.1"), "{m}");
        assert!(m.contains("192.0.2.2"), "{m}");
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

    /// A dead address must not hold the user for its whole timeout when
    /// a live one is sitting behind it.
    ///
    /// Serially this cost `CONNECT_TIMEOUT` before the second address
    /// was tried at all -- on the first call after every network
    /// change, which is exactly when a phone changes networks. The
    /// assertion is on the CLOCK, because "eventually answers" was
    /// already true of the serial version.
    #[tokio::test]
    async fn a_live_address_behind_a_dead_one_answers_without_waiting_it_out() {
        let id = identity();
        let server = TestServer::start().await;
        server.pair(&id.fingerprint());
        // 192.0.2.1 is TEST-NET-1: nothing routes there, so the attempt
        // hangs until CONNECT_TIMEOUT rather than being refused.
        let client = Client::new(
            &id,
            &server.fp,
            vec!["192.0.2.1".into(), server.addr()],
            server.port(),
        )
        .unwrap();

        let started = std::time::Instant::now();
        client.hello().await.unwrap();
        let took = started.elapsed();
        assert!(
            took < CONNECT_TIMEOUT,
            "waited {took:?} for a live second address; the stagger should have \
             started it after {ATTEMPT_STAGGER:?}"
        );
        // And it is remembered, so the next call skips the dead one.
        assert_eq!(client.order(), vec![1, 0]);
    }

    /// The desktop's own answer beats a slower "cannot reach you".
    ///
    /// This is the precedence rule the serial version got for free and
    /// concurrency puts at risk: a handshake refusal from the SECOND
    /// address must win, not be raced out by the first address finally
    /// timing out. Every address leads to the same desktop, so asking
    /// another one would only get the same refusal.
    #[tokio::test]
    async fn a_refusal_wins_over_a_slower_unreachable() {
        let id = identity();
        let server = TestServer::start().await;
        // NOT paired: the server refuses this phone's certificate, which
        // is a handshake failure rather than an unreachable.
        let client = Client::new(
            &id,
            &server.fp,
            vec!["192.0.2.1".into(), server.addr()],
            server.port(),
        )
        .unwrap();

        let started = std::time::Instant::now();
        let err = client.hello().await.unwrap_err();
        assert!(
            err.is_handshake(),
            "expected the desktop's refusal, got {err:?}"
        );
        assert!(
            started.elapsed() < CONNECT_TIMEOUT,
            "the refusal waited for the dead address to time out"
        );
    }

    /// Nothing speculative while the first address is healthy.
    ///
    /// The point of staggering rather than firing everything at once:
    /// each extra attempt that gets past TCP is a full mTLS handshake
    /// with ML-DSA-65 certificates on both ends, which on a phone is
    /// paid in radio and battery. A reachable first address must cost
    /// exactly one.
    #[tokio::test]
    async fn a_healthy_first_address_starts_no_second_attempt() {
        let id = identity();
        let server = TestServer::start().await;
        server.pair(&id.fingerprint());
        // The live address first, then one that would answer too. If
        // the second were started, the server would see two requests.
        let client = Client::new(
            &id,
            &server.fp,
            vec![server.addr(), server.addr()],
            server.port(),
        )
        .unwrap();
        client.hello().await.unwrap();
        assert_eq!(
            server.requests().len(),
            1,
            "a healthy first address must not trigger a second attempt"
        );
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
