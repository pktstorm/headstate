import { describe, expect, it } from "vitest";
import { REQUIRED_PROTOCOL_VERSION, desktopTooOld } from "./protocol";

describe("desktopTooOld", () => {
  it("requires protocol 1, the version the desktop listener ships", () => {
    // Pinned on purpose: a bump is a spec change (design spec,
    // "Versioning and compatibility"), so this test should be edited
    // alongside it, not by accident.
    expect(REQUIRED_PROTOCOL_VERSION).toBe(1);
  });

  it("is false for a desktop at exactly the required version", () => {
    expect(desktopTooOld(REQUIRED_PROTOCOL_VERSION)).toBe(false);
  });

  it("is false for a newer desktop: the desktop accepts an older phone", () => {
    expect(desktopTooOld(REQUIRED_PROTOCOL_VERSION + 1)).toBe(false);
  });

  it("is true for a desktop below the required version", () => {
    expect(desktopTooOld(REQUIRED_PROTOCOL_VERSION - 1)).toBe(true);
    expect(desktopTooOld(0)).toBe(true);
  });

  it("is false while the version is unknown", () => {
    // Unpaired, connecting, or a report without the field: the
    // connection state has its own banner for those, and a desktop
    // from before the field existed is protocol 1 by definition.
    expect(desktopTooOld(null)).toBe(false);
  });
});
