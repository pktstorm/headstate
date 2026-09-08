import { describe, expect, it } from "vitest";
import { errorMessage, isCancelled } from "./cancelled";

/// Every destructive command on the phone is gated by Face ID or the
/// Android biometric prompt, and dismissing that sheet is a decision --
/// the action does not happen. It used to arrive at the UI as an opaque
/// failure indistinguishable from a lockout or an invalidated key,
/// because `plugin_error` in `keys.rs` collapsed `Cancelled`,
/// `AuthFailed` and `Malformed` into one `Crypto` variant and kept only
/// the string.

/// Must match `CANCELLED` in `src-mobile/src/companion.rs`.
const MARKER = "headstate:cancelled";

describe("isCancelled", () => {
  it("recognises the companion's cancel marker", () => {
    expect(isCancelled(MARKER)).toBe(true);
    expect(isCancelled(new Error(MARKER))).toBe(true);
  });

  it("does not swallow a real failure", () => {
    // The distinction that matters: a lockout and an invalidated key
    // both need to be SHOWN, and each says something different about
    // what to do next.
    expect(isCancelled("the confirmation failed: too many attempts")).toBe(false);
    expect(
      isCancelled("the confirmation failed: the signing key was invalidated; re-pair this phone"),
    ).toBe(false);
    expect(isCancelled("octocat's laptop is unreachable")).toBe(false);
  });

  it("does not match on a substring", () => {
    // A message that merely mentions cancelling is not the marker, or
    // a desktop could suppress a real error by wording it carefully.
    expect(isCancelled("the request was cancelled by the server")).toBe(false);
    expect(isCancelled(`${MARKER} and then something else`)).toBe(false);
  });

  it("handles the shapes a rejection actually arrives in", () => {
    expect(isCancelled(undefined)).toBe(false);
    expect(isCancelled(null)).toBe(false);
    expect(isCancelled({})).toBe(false);
  });
});

describe("errorMessage", () => {
  it("is null for a cancel, so the caller shows nothing", () => {
    expect(errorMessage(MARKER)).toBeNull();
  });

  it("is the message for anything else", () => {
    expect(errorMessage("boom")).toBe("boom");
    expect(errorMessage(new Error("boom"))).toBe("boom");
  });
});
