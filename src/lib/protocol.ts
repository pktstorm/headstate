/// The lowest wire protocol version this frontend can drive a desktop
/// with. The desktop reports its own from `GET /v1/hello` (see
/// `src-tauri/src/remote/listener.rs`, `PROTOCOL_VERSION`) and embeds it
/// in the pairing QR as `v`.
///
/// A bump here is a deliberate change to the remote surface or the
/// pairing payload, recorded in the design spec, never a side effect of
/// a release: the desktop accepts any phone at or below its own version,
/// so raising this number is what turns an older desktop away.
export const REQUIRED_PROTOCOL_VERSION = 1;

/// Whether the paired desktop is too old for this phone.
///
/// `reported` is what the desktop said, or null while the phone has not
/// heard (unpaired, still connecting, or a desktop from before the
/// field existed). Null is "not too old": the banner for "no answer
/// yet" is the connection state's own, and a desktop that predates the
/// field is protocol 1 by definition, which is what this build requires.
export function desktopTooOld(reported: number | null): boolean {
  return reported !== null && reported < REQUIRED_PROTOCOL_VERSION;
}
