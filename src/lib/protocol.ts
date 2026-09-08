/// The lowest wire protocol version this frontend can drive a desktop
/// with. The desktop reports its own from `GET /v1/hello` (see
/// `src-tauri/src/remote/listener.rs`, `PROTOCOL_VERSION`) and embeds it
/// in the pairing QR as `v`.
///
/// A bump here is a deliberate change to the remote surface or the
/// pairing payload, recorded in the design spec, never a side effect of
/// a release: the desktop accepts any phone at or below its own version,
/// so raising this number is what turns an older desktop away.
///
/// 2: both TLS certificates are ML-DSA-65 (#521). A 5.0 desktop
/// (protocol 1) is refused at the TLS handshake before `/v1/hello` can
/// say so; this is what makes the banner name the version it needs.
export const REQUIRED_PROTOCOL_VERSION = 2;

/// Whether the paired desktop is too old for this phone.
///
/// `reported` is what the desktop said, or null while the phone has not
/// heard -- unpaired, still connecting, or a desktop from before the
/// field existed.
///
/// **Null is "not too old" here, and that is deliberate: this decides a
/// BANNER, not permission.** The old comment justified it by claiming
/// "a desktop that predates the field is protocol 1 by definition,
/// which is what this build requires" -- which contradicts itself, since
/// `REQUIRED_PROTOCOL_VERSION` is 2 and protocol 1 is exactly what must
/// be refused. Read literally it argued for failing closed.
///
/// Failing closed here would be wrong anyway. Null is overwhelmingly
/// "we have not heard yet", which is every connecting state and the
/// moment after pairing, and an "update your desktop" banner during
/// those would be a false alarm the user cannot act on.
///
/// The refusal that matters is not here. `connection.rs` blocks writes
/// for any connected desktop whose version is not known to be at least
/// `PROTOCOL_VERSION` (`Some(v) if v >= PROTOCOL_VERSION => None`, and
/// every other case is blocked), and cert pinning fails a protocol-1
/// desktop at the TLS handshake before `/v1/hello` can answer. So a
/// desktop the phone must not drive is already refused with a message
/// naming it; this function only decides whether to ALSO show the
/// banner, and it stays quiet until it has a version to name.
export function desktopTooOld(reported: number | null): boolean {
  return reported !== null && reported < REQUIRED_PROTOCOL_VERSION;
}
