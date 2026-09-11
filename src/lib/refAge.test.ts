import { describe, expect, it } from "vitest";
import { refAge } from "./worktrees";

/// #702: every merge and upstream verdict is computed against refs on
/// disk, and nothing said how old those refs were. Measured on one
/// machine: a repository 12 days stale, whose rows read like the
/// present tense.
///
/// #815: and the threshold #702 chose was a calendar day, so the caveat
/// stayed silent through the whole window the bug actually lives in.
/// `main` here lands PRs hourly; 23-hour-old refs are stale enough to
/// call a merged branch unmerged.
describe("refAge", () => {
  const now = new Date("2026-09-09T12:00:00Z");
  const daysAgo = (n: number) =>
    new Date(now.getTime() - n * 86_400_000).toISOString();
  const hoursAgo = (n: number) =>
    new Date(now.getTime() - n * 3_600_000).toISOString();

  /// Silent under an HOUR, not under a day (#815). A fetch 90 seconds
  /// ago is genuinely current, and a caveat shown always is a caveat
  /// nobody reads -- so there is still a threshold, just a realistic one.
  it("says nothing about refs fetched in the last hour", () => {
    expect(refAge(hoursAgo(0), now)).toBeNull();
    expect(refAge(new Date(now.getTime() - 59 * 60_000).toISOString(), now)).toBeNull();
  });

  /// #815's threshold fix. This is the window the bug lives in: on a
  /// repository landing PRs hourly, refs from this morning are already
  /// stale enough to make a merged branch look unmerged -- and the page
  /// used to say nothing at all until a full day had passed.
  it("names an age measured in hours, which a day-long threshold hid", () => {
    expect(refAge(hoursAgo(1), now)).toBe("as of a fetch 1 hour ago");
    expect(refAge(hoursAgo(5), now)).toBe("as of a fetch 5 hours ago");
    expect(refAge(hoursAgo(23), now)).toBe("as of a fetch 23 hours ago");
  });

  it("switches to days once hours stop being readable", () => {
    expect(refAge(daysAgo(1), now)).toBe("as of a fetch 1 day ago");
    expect(refAge(daysAgo(12), now)).toBe("as of a fetch 12 days ago");
    // The boundary both ways: 24h is a day, 47h is still one day.
    expect(refAge(hoursAgo(24), now)).toBe("as of a fetch 1 day ago");
    expect(refAge(hoursAgo(47), now)).toBe("as of a fetch 1 day ago");
    expect(refAge(hoursAgo(48), now)).toBe("as of a fetch 2 days ago");
  });

  /// A future timestamp is "we cannot tell", not "-3 hours stale". It
  /// must never render a negative age, and must never be confused with
  /// the never-fetched case, which is a real claim.
  it("goes quiet on a timestamp from the future rather than going negative", () => {
    expect(refAge(hoursAgo(-5), now)).toBeNull();
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
