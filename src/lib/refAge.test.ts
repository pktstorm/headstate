import { describe, expect, it } from "vitest";
import { refAge } from "./worktrees";

/// #702: every merge and upstream verdict is computed against refs on
/// disk, and nothing said how old those refs were. Measured on one
/// machine: a repository 12 days stale, whose rows read like the
/// present tense.
describe("refAge", () => {
  const now = new Date("2026-09-09T12:00:00Z");
  const daysAgo = (n: number) =>
    new Date(now.getTime() - n * 86_400_000).toISOString();

  /// Silent under a day. A caveat shown always is a caveat nobody
  /// reads, and a fetch this morning is not a caveat.
  it("says nothing about refs fetched today", () => {
    expect(refAge(daysAgo(0), now)).toBeNull();
    expect(refAge(new Date(now.getTime() - 3_600_000).toISOString(), now)).toBeNull();
  });

  it("names the age once it is worth naming", () => {
    expect(refAge(daysAgo(1), now)).toBe("as of a fetch 1 day ago");
    expect(refAge(daysAgo(12), now)).toBe("as of a fetch 12 days ago");
  });

  /// Never fetched is the STALEST state, not the freshest. Treating a
  /// missing time as "fresh enough to stay quiet" would silence the
  /// warning in exactly the case that most needs it.
  it("treats never fetched as the worst case, not the best", () => {
    expect(refAge(null, now)).toBe("never fetched");
    expect(refAge(undefined as unknown as null, now)).toBe("never fetched");
  });

  /// An unparseable timestamp is not evidence of freshness either.
  it("does not read a broken timestamp as current", () => {
    expect(refAge("not a date", now)).toBe("never fetched");
  });
});
