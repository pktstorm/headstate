import { describe, expect, it } from "vitest";
import { isStale } from "./connection";
import type { ConnectionState } from "./connection";

/// The marker the frontend computed a value for and then threw away.
///
/// `connection.rs` sets `stale` on every report, and `ConnectionReport`
/// declared it with the comment "what the list's stale marker reads" --
/// but `fromReport` never copied it onto `ConnectionState`, and no
/// component consumed it. So with the desktop asleep the phone rendered
/// a full, confident pull-request list, hours old, with every write
/// button live.

const connected = (over: Partial<ConnectionState> = {}): ConnectionState =>
  ({
    kind: "connected",
    desktop: "octocat's laptop",
    lastPoll: null,
    protocolVersion: 2,
    stale: false,
    ...over,
  }) as ConnectionState;

describe("isStale", () => {
  it("is false for the desktop, which holds the data itself", () => {
    expect(isStale({ kind: "local" })).toBe(false);
  });

  it("is false before the companion has answered", () => {
    // `unknown` is "we do not know yet", not "the data is old". Marking
    // it stale would flash the ribbon on every launch.
    expect(isStale({ kind: "unknown" })).toBe(false);
  });

  it("is false when unpaired, which is showing nothing to be stale about", () => {
    expect(isStale({ kind: "unpaired" })).toBe(false);
  });

  it("is false for an ordinary connected desktop", () => {
    expect(isStale(connected())).toBe(false);
  });

  it("is true for a connected desktop the companion may not drive", () => {
    // Reachable and yet stale: a desktop below the required protocol
    // answers, but its answers must not be acted on.
    expect(isStale(connected({ stale: true } as Partial<ConnectionState>))).toBe(true);
  });

  it("is true when unreachable, which is the case the ribbon exists for", () => {
    expect(
      isStale({
        kind: "unreachable",
        desktop: "octocat's laptop",
        lastPoll: new Date().toISOString(),
        stale: true,
      }),
    ).toBe(true);
  });

  it("is true when revoked", () => {
    expect(
      isStale({ kind: "revoked", desktop: "octocat's laptop", lastPoll: null, stale: true }),
    ).toBe(true);
  });
});
