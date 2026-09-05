import { describe, expect, it } from "vitest";
import { formatFingerprint } from "./fingerprint";

const HEX = "ab12cd34ef5601237890abcdef0123456789abcdef0123456789abcdef012345";

/// The phone shows the same 64 hex characters in the same groups, so
/// the two can be compared block by block. Anything that changes the
/// grouping -- a prefix, case, whitespace -- would defeat that.
describe("formatFingerprint", () => {
  it("groups bare hex into blocks of four", () => {
    expect(formatFingerprint(HEX)).toBe(
      "ab12 cd34 ef56 0123 7890 abcd ef01 2345 6789 abcd ef01 2345 6789 abcd ef01 2345",
    );
  });

  it("strips the sha256: prefix the QR payload carries", () => {
    expect(formatFingerprint(`sha256:${HEX}`)).toBe(formatFingerprint(HEX));
  });

  it("lowercases and ignores stray whitespace", () => {
    expect(formatFingerprint(` ${HEX.toUpperCase()} `)).toBe(formatFingerprint(HEX));
  });

  it("does not pad a short string", () => {
    expect(formatFingerprint("abcde")).toBe("abcd e");
    expect(formatFingerprint("")).toBe("");
  });
});
