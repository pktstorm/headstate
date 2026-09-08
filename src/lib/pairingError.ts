import { REQUIRED_PROTOCOL_VERSION } from "./protocol";

/// What went wrong pairing, as something a person can act on.
///
/// The companion's `PairingError` (`src-mobile/src/pairing.rs`) already
/// writes careful, user-facing sentences for all seven of its variants,
/// and they arrive here as the rejection's message string. This does not
/// replace them -- it adds the NEXT STEP, which the Rust side has no way
/// to know, and it corrects two cases where the accurate message is
/// misleading on a phone.
///
/// The two corrections are the reason this file exists rather than the
/// screen rendering `String(error)`:
///
/// 1. **Clock skew reads as an expired code.** `parse_qr` compares the
///    QR's `exp` against the PHONE's clock. A phone running fast reports
///    `Expired`, whose message says to show a new code on the desktop --
///    advice that will fail forever, because the next code will look
///    expired too. The real fix is the phone's clock, which the message
///    never mentions.
/// 2. **A protocol mismatch reads as the wrong app.** `parse_qr` rejects
///    a `v` it does not know as `BadQr`, which renders as "not a
///    Headstate pairing code". The actual condition is a desktop too old
///    to drive, which is a thing the user can fix by updating it.
///
/// Matching on message text is not ideal, and it is worth saying why it
/// is what happens here: `pair_from_qr` is a Tauri command, so its error
/// crosses the IPC boundary as a `String` and the variant is gone by the
/// time it arrives. Giving the command a structured error type would be
/// the better fix and is worth doing; until then the patterns below are
/// anchored to phrases from `pairing.rs` that are themselves asserted by
/// its tests, so a reworded message fails there first.

export interface PairingFailure {
  /// One line, in the user's terms.
  title: string;
  /// What to do about it. Empty when there is genuinely nothing.
  detail: string;
  /// Whether trying the same code again could plausibly work. Drives
  /// whether the screen offers "Try again" or only "Scan a new code".
  retryable: boolean;
  /// The underlying message, for the "Show details" disclosure.
  ///
  /// The friendly copy above is a guess at what a person should DO; when
  /// the guess is wrong -- "check both devices are on the same network"
  /// when they demonstrably are -- there was previously no way to learn
  /// anything more without a debugger (#633). This is that way, kept out
  /// of the default view so it costs nothing when the advice is right.
  ///
  /// Redacted: see `redactPairingDetail`.
  technical: string;
}

/// Strip anything secret from a message before it can be displayed.
///
/// The QR payload carries a pairing token, and a message that quotes the
/// payload back (a parse failure, most likely) would put it on screen
/// and into whatever the user pastes into an issue. Addresses, ports and
/// error kinds are the useful part and are not secret; the token, the
/// signing keys and the certificate are neither.
export function redactPairingDetail(message: string): string {
  return (
    message
      // `token` / `"token":"..."` in any JSON the message quotes.
      .replace(/("?\b(?:token|signing_keys|cert|certificate|key)"?\s*[:=]\s*)"[^"]*"/gi, '$1"[redacted]"')
      // A bare base64url run long enough to be key material.
      .replace(/\b[A-Za-z0-9_-]{32,}\b/g, "[redacted]")
  );
}

/// The QR carries a `v` the phone does not implement.
const UNSUPPORTED_VERSION = /version (\d+) is not supported/i;

export function describePairingFailure(error: unknown): PairingFailure {
  const message = error instanceof Error ? error.message : String(error);

  const version = UNSUPPORTED_VERSION.exec(message);
  if (version !== null) {
    // Deliberately NOT "not a Headstate pairing code", which is what the
    // raw message says. It is a Headstate code; it is from a Headstate
    // that is too old.
    //
    // Naming `REQUIRED_PROTOCOL_VERSION` for a QR version is sound only
    // because the two are held equal on purpose: both crates assert
    // `QR_VERSION == PROTOCOL_VERSION` (`src-mobile/src/pairing.rs:350`,
    // `src-tauri/src/remote/pairing.rs:1332`). Should they ever be
    // allowed to diverge, those assertions fail first and this line is
    // what needs rewording.
    return {
      title: "That desktop is too old to pair with",
      detail:
        `Its pairing code is version ${version[1]}, and this app needs version ` +
        `${REQUIRED_PROTOCOL_VERSION}. Update Headstate on the desktop, then show a new code.`,
      retryable: false,
      technical: redactPairingDetail(message),
    };
  }

  if (/expired/i.test(message)) {
    return {
      title: "That pairing code has expired",
      detail:
        "Show a new one on the desktop and scan again. If new codes keep coming up " +
        "expired, check that this phone's date and time are correct — a clock that is " +
        "ahead makes a valid code look expired.",
      retryable: false,
      technical: redactPairingDetail(message),
    };
  }

  if (/certificate that does not match/i.test(message)) {
    // The one failure that is a security event rather than a mishap.
    // Worded so that "try again" does not sound like the answer.
    return {
      title: "That desktop did not match its pairing code",
      detail:
        "The certificate it presented is not the one the code promised, so pairing was " +
        "stopped. Make sure you scanned the code from your own desktop, and try again " +
        "on a network you trust.",
      retryable: false,
      technical: redactPairingDetail(message),
    };
  }

  if (/refused the pairing/i.test(message)) {
    return {
      title: "The desktop refused the pairing",
      detail:
        "The request was declined, timed out waiting for someone to approve it, or the " +
        "code had already been used. Show a fresh code on the desktop and approve the " +
        "request when it appears.",
      retryable: false,
      technical: redactPairingDetail(message),
    };
  }

  if (/could not reach the desktop/i.test(message)) {
    return {
      title: "Could not reach that desktop",
      detail:
        "Check that it is awake, that Headstate is running on it with phone connections " +
        "allowed, and that both devices are on the same network — or connected through " +
        "the same VPN.",
      retryable: true,
      technical: redactPairingDetail(message),
    };
  }

  if (/not a Headstate pairing code/i.test(message)) {
    return {
      title: "That is not a Headstate pairing code",
      detail:
        "Scan the code shown by Headstate on your desktop, under Settings, Phone. " +
        `(${message})`,
      retryable: false,
      technical: redactPairingDetail(message),
    };
  }

  // Anything else -- key generation, the store, an HTTP status the
  // companion did not recognise. The message is still the Rust side's
  // own sentence, so show it rather than replacing it with a shrug.
  return {
    title: "Pairing failed",
    detail: redactPairingDetail(message),
    retryable: true,
    technical: redactPairingDetail(message),
  };
}
