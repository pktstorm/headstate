import { describe, expect, it } from "vitest";
import { upstreamReasonAged, upstreamToneAged } from "./worktrees";
import type { Upstream } from "@/types/pr";

/// #788: a `main` row read "up to date with upstream" in green, and
/// clicking Update then pulled in a large number of commits.
///
/// Not a comparison bug -- an asymmetry. The scan never fetches (a
/// deliberate decision: a network call per repository every time the view
/// opens, 40+ repositories and ~27,000 directories in one pass, and a
/// hang on an unreachable remote), so the badge compares local `main`
/// against a possibly-stale on-disk `origin/main`. The Update button DOES
/// fetch, via `git pull`. Both refs can be behind together, and the row
/// honestly reports that they agree.
///
/// Two mechanisms already existed and neither reached the reader at the
/// moment they needed it: the row's "(as of last fetch)" has no
/// magnitude, and the age note is on the page HEADER while the badge is
/// per ROW. These tests cover the row.
///
/// The load-bearing case is a RECENT-BUT-STALE fetch. A test that only
/// exercised the multi-day case would pass over exactly the bug reported:
/// the whole complaint is a row that looks current when it is hours
/// behind, and on a repository landing PRs hourly, hours is enough.
describe("the row's upstream claim, qualified by how old the refs are", () => {
  const now = new Date("2026-09-12T12:00:00Z");
  const hoursAgo = (n: number) =>
    new Date(now.getTime() - n * 3_600_000).toISOString();
  const daysAgo = (n: number) =>
    new Date(now.getTime() - n * 86_400_000).toISOString();

  const CURRENT: Upstream = { kind: "current" };
  const BEHIND: Upstream = { kind: "behind", n: 40 };

  /// The page's green. Asserted as a literal once, here, so the
  /// "is it still green?" questions below read as comparisons rather
  /// than as hex strings nobody can check.
  const GREEN = "text-[#3fb950]";
  const GREY = "text-[#8b949e]";
  const AMBER = "text-[#d29922]";

  /// THE REGRESSION #788 IS ABOUT, in hours.
  ///
  /// A repository fetched a few hours ago, whose `main` agrees with the
  /// `origin/main` on disk. The old row said "up to date with upstream
  /// (as of last fetch)" in green and nothing else -- a constant phrase
  /// that reads the same whether the fetch was 30 seconds or 30 days ago,
  /// which is why readers treated it as boilerplate and the green dot
  /// won the argument.
  ///
  /// Both halves are asserted together because either alone passes over
  /// the bug: text without a colour change leaves the green arguing
  /// against the caveat, and a colour change without a number gives the
  /// reader nothing to judge.
  it("names the age in HOURS beside an up-to-date claim, and stops calling it green", () => {
    const text = upstreamReasonAged(CURRENT, hoursAgo(4), now);
    expect(text).toBe("up to date with upstream · as of 4h ago");
    // The magnitude is PRESENT, which is the entire point. Asserted as a
    // digit-bearing substring as well as the exact string, so a future
    // rewording that dropped the number fails on the reason rather than
    // on the punctuation.
    expect(text).toMatch(/\d/);
    expect(upstreamToneAged(CURRENT, hoursAgo(4), now)).toBe(GREY);
    expect(upstreamToneAged(CURRENT, hoursAgo(4), now)).not.toBe(GREEN);
  });

  /// The specific figure #788's predecessor (#815) measured against, and
  /// the one a days-based threshold hid completely: 23 hours is inside
  /// the same calendar day and is a full working day of merges on an
  /// active repository.
  it("is not silent at 23 hours, which a day-long threshold hid entirely", () => {
    expect(upstreamReasonAged(CURRENT, hoursAgo(23), now)).toBe(
      "up to date with upstream · as of 23h ago",
    );
    expect(upstreamToneAged(CURRENT, hoursAgo(23), now)).toBe(GREY);
  });

  /// One hour is where the claim starts being wrong on this repository --
  /// the shortest window in which its `main` can actually move -- so the
  /// note appears AT an hour, not after a day.
  it("starts at one hour, the boundary in both directions", () => {
    expect(upstreamReasonAged(CURRENT, hoursAgo(1), now)).toBe(
      "up to date with upstream · as of 1h ago",
    );
    expect(upstreamToneAged(CURRENT, hoursAgo(1), now)).toBe(GREY);
    // And 59 minutes is still fresh: a caveat on every row of every
    // repository all the time is a caveat nobody reads, which is the
    // failure the old day-long threshold was guarding against.
    const justNow = new Date(now.getTime() - 59 * 60_000).toISOString();
    expect(upstreamReasonAged(CURRENT, justNow, now)).toBe(
      "up to date with upstream",
    );
    expect(upstreamToneAged(CURRENT, justNow, now)).toBe(GREEN);
  });

  /// Fresh refs get NO hedge at all, not even the old parenthetical.
  ///
  /// "up to date with upstream (as of last fetch)" on a repository
  /// fetched 90 seconds ago is the caveat-everywhere failure: it trains
  /// the reader to skip the parenthetical, so it is still there and
  /// unread on the row where it matters. Verified green, and verified
  /// that the phrase is gone.
  it("drops the parenthetical entirely when the refs really are current", () => {
    const text = upstreamReasonAged(CURRENT, hoursAgo(0), now);
    expect(text).toBe("up to date with upstream");
    expect(text).not.toContain("as of");
    expect(upstreamToneAged(CURRENT, hoursAgo(0), now)).toBe(GREEN);
  });

  it("switches to days once hours stop being readable", () => {
    expect(upstreamReasonAged(CURRENT, daysAgo(2), now)).toBe(
      "up to date with upstream · as of 2d ago",
    );
    // The boundary both ways: 24h is one day, 47h is still one day.
    expect(upstreamReasonAged(CURRENT, hoursAgo(24), now)).toBe(
      "up to date with upstream · as of 1d ago",
    );
    expect(upstreamReasonAged(CURRENT, hoursAgo(47), now)).toBe(
      "up to date with upstream · as of 1d ago",
    );
    expect(upstreamReasonAged(CURRENT, hoursAgo(48), now)).toBe(
      "up to date with upstream · as of 2d ago",
    );
  });

  /// ABSENT IS NOT ZERO, and absent is not success.
  ///
  /// This codebase's characteristic bug. #769's sizes summed a
  /// never-measured tree to 0 bytes and rendered "empty", inviting the
  /// deletion of a checkout nobody had looked inside. #841 read a missing
  /// health sample as healthy. Here the same mistake would be treating a
  /// null `fetched_at` as an age of zero -- printing "up to date · as of
  /// 0h ago" in green on a repository that has NEVER contacted its
  /// remote, which is the single most wrong sentence this page could
  /// produce.
  ///
  /// Asserted from three directions, because the plausible wrong answers
  /// are different shapes: a zero duration, a bare unhedged claim, and
  /// green.
  it("does NOT read a missing fetch time as fresh, as zero, or as green", () => {
    const text = upstreamReasonAged(CURRENT, null, now);
    expect(text).toBe("up to date with upstream · never fetched");
    // Not an age of zero.
    expect(text).not.toContain("0h");
    expect(text).not.toContain("0 hour");
    // Not a bare claim either -- silence here would be the reassurance
    // with the least behind it.
    expect(text).not.toBe("up to date with upstream");
    // And not green.
    expect(upstreamToneAged(CURRENT, null, now)).toBe(GREY);
    expect(upstreamToneAged(CURRENT, null, now)).not.toBe(GREEN);
  });

  /// `undefined` reaches here from a fixture or an older payload that
  /// omits the key, and it means the same thing null does. Checked
  /// separately because `=== null` and `== null` differ here and the
  /// strict one would let `undefined` fall through to the arithmetic.
  it("treats an absent key the same as an explicit null", () => {
    expect(
      upstreamReasonAged(CURRENT, undefined as unknown as null, now),
    ).toBe("up to date with upstream · never fetched");
    expect(upstreamToneAged(CURRENT, undefined as unknown as null, now)).toBe(
      GREY,
    );
  });

  /// A timestamp that will not parse is not evidence of freshness. It
  /// must not become `NaN` hours and it must not fall through to green.
  it("does not read a broken timestamp as current", () => {
    expect(upstreamReasonAged(CURRENT, "not a date", now)).toBe(
      "up to date with upstream · never fetched",
    );
    expect(upstreamReasonAged(CURRENT, "not a date", now)).not.toContain("NaN");
    expect(upstreamToneAged(CURRENT, "not a date", now)).toBe(GREY);
  });

  /// A future timestamp -- a clock that jumped, or an mtime ahead of the
  /// scan -- must never render a negative age. `refAge` goes silent on
  /// it deliberately: "we cannot tell how stale this is" is not itself a
  /// claim of staleness, and it must not be confused with never-fetched,
  /// which IS a claim.
  it("goes quiet on a timestamp from the future rather than going negative", () => {
    const ahead = new Date(now.getTime() + 5 * 3_600_000).toISOString();
    const text = upstreamReasonAged(CURRENT, ahead, now);
    expect(text).toBe("up to date with upstream");
    expect(text).not.toContain("-");
    expect(text).not.toContain("never fetched");
  });

  /// Staleness changes the tone of exactly ONE verdict, and `behind` is
  /// the one it must not change.
  ///
  /// Amber on this page means "you may want to act on this". Stale refs
  /// can only make "40 behind" an UNDERSTATEMENT -- the real number is
  /// the same or larger -- so the call to action holds, and greying it
  /// would hide a claim staleness cannot falsify. The age is still
  /// appended, because it tells the user the 40 is a floor.
  it("leaves a behind verdict amber however old the refs are, and still dates it", () => {
    expect(upstreamToneAged(BEHIND, daysAgo(9), now)).toBe(AMBER);
    expect(upstreamToneAged(BEHIND, null, now)).toBe(AMBER);
    expect(upstreamReasonAged(BEHIND, daysAgo(9), now)).toBe(
      "40 commits behind upstream (as of last fetch) · as of 9d ago",
    );
  });

  /// The states whose verdict does not read `origin/*` at all. Their
  /// colour cannot depend on the fetch age, because the fetch age is not
  /// evidence about them -- a local-only branch is local-only whenever
  /// you last looked.
  it("does not recolour verdicts that never consulted a remote ref", () => {
    for (const u of [
      { kind: "ahead", n: 3 },
      { kind: "untracked" },
      { kind: "detached" },
    ] as Upstream[]) {
      expect(upstreamToneAged(u, daysAgo(30), now)).toBe(
        upstreamToneAged(u, hoursAgo(0), now),
      );
    }
  });

  /// The row's note has to survive a cell that TRUNCATES. The verdict
  /// cell is deliberately the first to clip on the desktop (#818), so a
  /// note as long as the header's "as of a fetch 4 hours ago" would be
  /// the first thing lost -- shipping the fix as a tooltip nobody opens.
  ///
  /// A length bound rather than an exact string, so a rewording is free
  /// as long as it stays short. The figure is the header form's own
  /// length: the row's note must be strictly shorter than the prose it
  /// replaces, or it has not bought the space it was added for.
  it("is short enough to survive beside a verdict in a cell that clips", () => {
    const row = upstreamReasonAged(CURRENT, hoursAgo(4), now);
    expect(row.length).toBeLessThan(
      "up to date with upstream (as of last fetch)".length +
        " · as of a fetch 4 hours ago".length,
    );
    // And it still says "as of", which is what marks the number as the
    // age of the EVIDENCE rather than of the branch -- the same row shows
    // a commit age two cells away, and two unlabelled durations meaning
    // different things is worse than one long label.
    expect(row).toContain("as of");
  });
});
