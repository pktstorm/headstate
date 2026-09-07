import { describe, expect, it } from "vitest";
import { describePairingFailure } from "./pairingError";
import { REQUIRED_PROTOCOL_VERSION } from "./protocol";

/// The message strings below are copied from `PairingError`'s
/// `#[error(...)]` attributes in `src-mobile/src/pairing.rs`. They are
/// what actually crosses the IPC boundary, because a Tauri command's
/// error is a `String` by the time it reaches the webview.
const RUST = {
  expired: "this pairing code has expired; show a new one on the desktop",
  mismatch: "the desktop presented a certificate that does not match the pairing code",
  denied: "the desktop refused the pairing: token already used",
  unreachable: "could not reach the desktop: connection refused",
  oldVersion: "not a Headstate pairing code: version 1 is not supported",
  malformed: "not a Headstate pairing code: expected value at line 1 column 1",
};

describe("describePairingFailure", () => {
  it("tells the user to check the clock when a code looks expired", () => {
    // The correction that matters most. `parse_qr` compares the QR's
    // `exp` against the PHONE's clock, so a phone running fast reports
    // Expired for a perfectly valid code -- and the Rust message's
    // advice ("show a new one on the desktop") then fails forever,
    // because the next code looks expired too.
    const f = describePairingFailure(new Error(RUST.expired));
    expect(f.title).toMatch(/expired/i);
    expect(f.detail).toMatch(/date and time/i);
    expect(f.retryable).toBe(false);
  });

  it("reports an old desktop as old, not as the wrong app", () => {
    // `parse_qr` rejects an unknown `v` as BadQr, which renders as "not
    // a Headstate pairing code". It IS a Headstate code -- from a
    // Headstate too old to drive -- and that is something the user can
    // fix by updating the desktop.
    const f = describePairingFailure(new Error(RUST.oldVersion));
    expect(f.title).toMatch(/too old/i);
    expect(f.title).not.toMatch(/not a Headstate/i);
    expect(f.detail).toContain("version 1");
    expect(f.detail).toContain(String(REQUIRED_PROTOCOL_VERSION));
    expect(f.detail).toMatch(/update headstate on the desktop/i);
  });

  it("still calls a genuinely malformed code what it is", () => {
    // The version case must not swallow every BadQr: a payload that is
    // not JSON at all really is not a Headstate pairing code.
    const f = describePairingFailure(new Error(RUST.malformed));
    expect(f.title).toMatch(/not a Headstate pairing code/i);
    expect(f.retryable).toBe(false);
  });

  it("does not invite a retry after a fingerprint mismatch", () => {
    // The one failure that is a security event rather than a mishap.
    const f = describePairingFailure(new Error(RUST.mismatch));
    expect(f.title).toMatch(/did not match/i);
    expect(f.retryable).toBe(false);
  });

  it("explains a refusal as denied, timed out, or already used", () => {
    const f = describePairingFailure(new Error(RUST.denied));
    expect(f.title).toMatch(/refused/i);
    expect(f.detail).toMatch(/already been used/i);
  });

  it("offers a retry, and network things to check, when unreachable", () => {
    const f = describePairingFailure(new Error(RUST.unreachable));
    expect(f.title).toMatch(/could not reach/i);
    expect(f.detail).toMatch(/same network/i);
    expect(f.retryable).toBe(true);
  });

  it("keeps the Rust sentence for anything it does not recognise", () => {
    // Key generation, the store, an unexpected HTTP status: the Rust
    // side still wrote a real sentence, so show it rather than
    // replacing it with a shrug.
    const f = describePairingFailure(new Error("the secure enclave refused to sign"));
    expect(f.detail).toBe("the secure enclave refused to sign");
    expect(f.retryable).toBe(true);
  });

  it("handles a bare string, which is what Tauri actually rejects with", () => {
    const f = describePairingFailure(RUST.expired);
    expect(f.title).toMatch(/expired/i);
  });
});
