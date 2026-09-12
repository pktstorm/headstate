import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AuthorRow, ReviewerRow, StatsReviewers } from "@/types/pr";
import {
  LINES_CHANGED_LABEL,
  Leaderboards,
  REVIEWS_GIVEN_LABEL,
  REVIEWS_LABEL,
  TOP_N,
} from "./Leaderboard";

const row = (login: string, over: Partial<AuthorRow> = {}): AuthorRow => ({
  login,
  prs: 1,
  additions: 10,
  deletions: 5,
  changedFiles: 2,
  reviewsReceived: 0,
  cycleTimeHours: [1],
  ...over,
});

/// A reviews-GIVEN board, as `stats_reviewers` returns one.
const given = (
  rows: ReviewerRow[],
  over: Partial<StatsReviewers> = {},
): StatsReviewers => ({
  rows,
  unmeasured: [],
  refusedFields: 0,
  spend: { points: 1, requests: 1, unmetered: 0, remaining: 4999, resetAt: null },
  ...over,
});

describe("Leaderboards", () => {
  /// The three rankings must be able to DISAGREE, or only one of them is
  /// needed. This is the test that would catch all three being wired to the
  /// same measure -- a mistake that looks fine on any board where the most
  /// prolific author also writes the most lines.
  it("ranks each board on its own measure", () => {
    render(
      <Leaderboards
        complete
        rows={[
          row("prolific", { prs: 40, additions: 10, deletions: 0, reviewsReceived: 0 }),
          row("verbose", { prs: 1, additions: 90_000, deletions: 10_000, reviewsReceived: 0 }),
          row("reviewed", { prs: 2, additions: 20, deletions: 0, reviewsReceived: 99 }),
        ]}
      />,
    );
    expect(screen.getByText("40 PRs")).toBeTruthy();
    expect(screen.getByText(/100,000 · 2 files/)).toBeTruthy();
    expect(screen.getByText("99 reviews")).toBeTruthy();
  });

  /// A zero has no rank. Padding a top-five with zeroes presents people as
  /// ranked on a measure they do not appear in at all -- which on the review
  /// board would list colleagues as reviewed when nobody reviewed them.
  it("excludes a zero rather than padding the board with it", () => {
    render(<Leaderboards complete rows={[row("active", { reviewsReceived: 3 }), row("quiet")]} />);
    // `quiet` has pull requests and lines, so it appears on those boards...
    expect(screen.getAllByText("quiet").length).toBe(2);
    // ...and NOT on the review board, where its value is zero. Three boards
    // minus the one it is absent from.
    expect(screen.getAllByText("active").length).toBe(3);
  });

  it("says nobody qualified, per measure", () => {
    render(<Leaderboards complete rows={[]} />);
    expect(screen.getByText(/no pull requests in this window/i)).toBeTruthy();
    expect(screen.getByText(/no lines changed in this window/i)).toBeTruthy();
    // "received", specifically. With the reviews-GIVEN board beside it, two
    // charts both reading "No reviews in this window" would be one fact said
    // twice rather than two facts -- and a reader could not tell which board
    // the note belonged to.
    expect(screen.getByText(/no reviews received in this window/i)).toBeTruthy();
  });

  /// Ties break on login, so a board does not reshuffle between loads when
  /// nothing changed. `fetch.rs`'s waves complete out of order by design, so
  /// without an explicit tie-break two equal rows would swap on refresh --
  /// which reads as a bug in the data rather than in the sort.
  it("breaks ties deterministically", () => {
    const { container } = render(
      <Leaderboards complete rows={[row("zoe", { prs: 5 }), row("adam", { prs: 5 })]} />,
    );
    const first = container.querySelector("ol");
    expect(first?.textContent).toMatch(/adam[\s\S]*zoe/);
  });

  /// The honest label on the gameable metric is the mitigation #823
  /// settled on, so it is asserted rather than left to layout.
  it("labels the line count as including generated files", () => {
    render(<Leaderboards complete rows={[row("a")]} />);
    expect(screen.getByText(LINES_CHANGED_LABEL)).toBeTruthy();
  });

  /// The received board says RECEIVED and is not titled "top reviewers".
  /// `reviews { totalCount }` hangs off a pull request the author WROTE, so a
  /// board titled "top reviewers" over it would name the person whose code was
  /// reviewed most -- close to the opposite of what a reader would take it
  /// for. #829 refused to ship that and this pins the refusal.
  ///
  /// Asserted with `reviewersAvailable` FALSE, so the only review board on
  /// screen is the received one: the point is that THAT board does not claim
  /// to rank reviewers, which a test rendering both could not isolate.
  it("does not claim to rank reviewers on the received board", () => {
    render(<Leaderboards complete rows={[row("a", { reviewsReceived: 1 })]} />);
    expect(screen.getByText(REVIEWS_LABEL)).toBeTruthy();
    expect(screen.getByText(/most-reviewed/i)).toBeTruthy();
    expect(screen.queryByText(/^top \d+ reviewers$/i)).toBeNull();
  });

  /// Both review boards ship, and the LABELS are what keep them apart.
  ///
  /// The measured reason they must (live API, 2026-09-11): the two pull
  /// requests crediting the viewer as REVIEWER in an org window were both
  /// AUTHORED BY SOMEBODY ELSE and each carried one review -- so the author
  /// leads the received board and the reviewer the given one, on the same rows
  /// of data. A reader who confuses the two reads the leaderboard backwards.
  ///
  /// Synthetic logins below (`octocat` the author, `hubot` the reviewer), per
  /// `scripts/check-privacy.sh`: this is a public repo and the finding is
  /// about the SHAPE, not about who.
  it("ranks reviews given beside reviews received, labelled apart", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        rows={[row("octocat", { reviewsReceived: 2 })]}
        reviewers={given([{ login: "hubot", reviews: 2 }])}
      />,
    );
    // Two boards, two labels, neither standing in for the other.
    expect(screen.getByText(REVIEWS_LABEL)).toBeTruthy();
    expect(screen.getByText(REVIEWS_GIVEN_LABEL)).toBeTruthy();
    expect(screen.getByText(`Top ${TOP_N} reviewers`)).toBeTruthy();
    expect(screen.getByText(`Top ${TOP_N} most-reviewed`)).toBeTruthy();
    // And the two name DIFFERENT people, which is the whole point: `octocat`
    // is an author row (so it appears on the author boards INCLUDING
    // most-reviewed) and `hubot` appears only on the reviewer board, because
    // reviewing is not authoring. `hubot` being found exactly once is the
    // assertion that matters -- it is not derivable from the rows at all, so
    // it can only have come from the separate query.
    expect(screen.getAllByText("octocat").length).toBeGreaterThan(0);
    expect(screen.getAllByText("hubot").length).toBe(1);
  });

  /// A pending reviewer board must not print its empty note.
  ///
  /// MEASURED on this account: `org:FNX-Labs` over 30 days holds 569 merged
  /// pull requests and ZERO reviewed by any of its four members. So "no
  /// reviews given" is the true answer here -- which is exactly why the
  /// transient must not render as it. A reader could not tell the in-flight
  /// second from the answer.
  it("says it is counting rather than claiming zero while in flight", () => {
    render(
      <Leaderboards complete reviewersAvailable reviewersPending rows={[row("a")]} />,
    );
    expect(screen.getByText(/counting reviews/i)).toBeTruthy();
    expect(screen.queryByText(/no reviews given/i)).toBeNull();
  });

  /// A FAILED reviewer board is not a zero, and not an empty one.
  ///
  /// One search per person means this board can fail while the other three
  /// succeeded, so it says so for itself -- and names the asymmetry, because
  /// a reader seeing three complete boards would otherwise read the fourth's
  /// silence as an answer.
  it("distinguishes a failed reviewer board from an empty one", () => {
    render(
      <Leaderboards complete reviewersAvailable reviewersError rows={[row("a")]} />,
    );
    expect(screen.getByText(/could not count reviews/i)).toBeTruthy();
    expect(screen.queryByText(/no reviews given/i)).toBeNull();
  });

  /// A measured zero IS empty, and says so. The counterpart to the two tests
  /// above: all three states are reachable and none renders as another.
  it("says reviews were given by nobody when that is the measured answer", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        rows={[row("a")]}
        reviewers={given([{ login: "quiet", reviews: 0 }])}
      />,
    );
    expect(screen.getByText(/no reviews given in this window/i)).toBeTruthy();
    // A zero has no rank, so `quiet` is not listed on the reviewer board --
    // listing them would present a colleague as ranked on a measure they do
    // not appear in.
    expect(screen.queryByText("quiet")).toBeNull();
  });

  /// Unmeasured reviewers are NAMED, never shown as zero.
  ///
  /// A ranking missing one person can have the wrong name in first place, and
  /// "2 could not be measured" does not say whether the leader is one of them.
  it("names the reviewers it could not count", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        rows={[row("a")]}
        reviewers={given([{ login: "counted", reviews: 3 }], {
          unmeasured: ["missing-one", "missing-two"],
        })}
      />,
    );
    expect(screen.getByText(/missing-one, missing-two/)).toBeTruthy();
    expect(screen.getByText(/rather than shown as zero/i)).toBeTruthy();
  });

  /// No roster means the board is ABSENT, not empty.
  ///
  /// Only an org scope has members. On a repository, Personal or Everything
  /// scope nothing enumerated the reviewers, and an empty chart there would
  /// say "nobody reviewed" on the strength of never having looked.
  it("omits the reviewer board entirely when no roster was enumerated", () => {
    render(<Leaderboards complete rows={[row("a", { reviewsReceived: 1 })]} />);
    expect(screen.queryByText(`Top ${TOP_N} reviewers`)).toBeNull();
    expect(screen.queryByText(/no reviews given/i)).toBeNull();
    expect(screen.queryByText(/counting reviews/i)).toBeNull();
    // The received board is unaffected: it rides on the rows that are
    // already loaded.
    expect(screen.getByText(`Top ${TOP_N} most-reviewed`)).toBeTruthy();
  });

  /// The reviewer board states WHO it covers, because the gap is real and
  /// unfixable rather than a caveat for form's sake: the roster is the org's
  /// Members list, so an outside collaborator or a bot who reviewed is not
  /// counted. Nothing enumerated them -- a PR node says how many reviews it
  /// has, never who wrote them.
  it("says whose reviews the board counts", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        rows={[row("a")]}
        reviewers={given([
          { login: "one", reviews: 2 },
          { login: "two", reviews: 1 },
        ])}
      />,
    );
    expect(screen.getByText(/2 listed members/)).toBeTruthy();
    expect(screen.getByText(/an outside collaborator, a bot -- are not counted/i)).toBeTruthy();
  });

  /// A truncated roster is STATED, not implied (#851).
  ///
  /// The most severe of #851's four findings: the board ranked a roster cut
  /// at `tree::PAGE` (100) and titled itself "Top 5 reviewers" while the
  /// sidebar two columns away said "Showing 100 of 224 members". A top-five
  /// over an arbitrary subset presented as a top-five over the org.
  ///
  /// "Arbitrary" is the part that makes it worse than the repository
  /// truncation the module documents: `tree::PAGE`'s own doc reasons that a
  /// repository cut at 100 "drops the DEADEST repositories" because the
  /// order is most-recently-active. `membersWithRole` takes no `orderBy`, so
  /// there is no such consolation -- the top reviewer can be among the ones
  /// never asked about.
  it("says when the roster it ranked was cut short", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        reviewersTruncated
        rows={[row("a")]}
        reviewers={given([
          { login: "one", reviews: 2 },
          { login: "two", reviews: 1 },
        ])}
      />,
    );
    // The caveat, and specifically the claim that matters: somebody absent
    // may outrank everybody shown.
    expect(screen.getByText(/more members than the roster could list/i)).toBeTruthy();
    expect(
      screen.getByText(/may have reviewed more than anybody shown/i),
      "the caveat has to say the LEADER may be missing, not merely that some \
rows are absent -- that is the difference between a qualified ranking and a \
wrong one",
    ).toBeTruthy();
    // And it says the cut is arbitrary, which is what distinguishes it from
    // the repo truncation a reader may already know about.
    expect(screen.getByText(/not ordered by anything/i)).toBeTruthy();
  });

  /// A COMPLETE roster says nothing, so the caveat means something.
  ///
  /// The other half: a warning shown unconditionally is wallpaper. This is
  /// the assertion that would fail if the flag were ignored and the line
  /// always rendered -- which would pass the test above perfectly well.
  it("stays quiet when the roster was complete", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        rows={[row("a")]}
        reviewers={given([{ login: "one", reviews: 2 }])}
      />,
    );
    expect(screen.queryByText(/more members than the roster could list/i)).toBeNull();
    expect(screen.queryByText(/not ordered by anything/i)).toBeNull();
    // The who-it-covers line is NOT the same claim and still appears: that
    // one is about non-members, this one about unlisted members.
    expect(screen.getByText(/1 listed member/)).toBeTruthy();
  });

  /// The two roster caveats are independent and both appear together.
  ///
  /// A truncated roster AND named unmeasured people is a real combination --
  /// the board asked 100 of 224 and some of the 100 did not answer -- and
  /// the two facts are different: one is people never asked, the other
  /// people asked who failed. Collapsing them would lose a count either way.
  it("reports a short roster and unmeasured people separately", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        reviewersTruncated
        rows={[row("a")]}
        reviewers={given([{ login: "counted", reviews: 3 }], {
          unmeasured: ["asked-but-failed"],
        })}
      />,
    );
    expect(screen.getByText(/more members than the roster could list/i)).toBeTruthy();
    expect(screen.getByText(/asked-but-failed/)).toBeTruthy();
    expect(screen.getByText(/rather than shown as zero/i)).toBeTruthy();
  });

  /// Ties on the reviewer board break on login too, so the two boards order a
  /// tie identically and neither reshuffles between loads.
  it("breaks reviewer ties deterministically", () => {
    render(
      <Leaderboards
        complete
        reviewersAvailable
        rows={[row("a")]}
        reviewers={given([
          { login: "zoe", reviews: 4 },
          { login: "adam", reviews: 4 },
        ])}
      />,
    );
    const boards = Array.from(document.querySelectorAll("ol"));
    const reviewerBoard = boards[boards.length - 1];
    expect(reviewerBoard?.textContent).toMatch(/adam[\s\S]*zoe/);
  });

  /// The cut is TOP_N, and the heading says the same number the list shows.
  /// A heading quoting a different figure from the cut is how a "top five"
  /// over three rows happens.
  it("shows at most TOP_N rows and says so", () => {
    const rows = Array.from({ length: TOP_N + 4 }, (_, i) =>
      row(`user${i}`, { prs: 100 - i }),
    );
    const { container } = render(<Leaderboards complete rows={rows} />);
    const prBoard = container.querySelector("ol");
    expect(prBoard?.querySelectorAll("li").length).toBe(TOP_N);
    expect(screen.getByText(`Top ${TOP_N} pull request authors`)).toBeTruthy();
  });

  /// A partial board says so ABOVE all three rankings, because the
  /// partiality applies to every one of them -- a note attached to one board
  /// would read as though the others were complete.
  it("warns once, above every ranking, when the board is partial", () => {
    render(
      <Leaderboards
        complete={false}
        caveat="40 of 100 pull requests could not be retrieved."
        rows={[row("a")]}
      />,
    );
    expect(screen.getAllByText(/rankings are incomplete/i).length).toBe(1);
    expect(screen.getByText(/40 of 100/)).toBeTruthy();
  });

  /// And a complete board carries no caveat, so the warning keeps meaning
  /// something when it does appear.
  it("says nothing when the board is complete", () => {
    render(<Leaderboards complete rows={[row("a")]} />);
    expect(screen.queryByText(/incomplete/i)).toBeNull();
  });

  /// The warning renders on `!complete` ALONE, with or without a reason.
  ///
  /// `StatsPage` carries the detailed reason in a page-level banner that also
  /// covers the Mine view, so it omits `caveat` here to avoid two copies of
  /// one warning reading as two problems. An unexplained warning is still
  /// worth far more than a silent confident top-five, so the absence of a
  /// reason must not suppress it -- which is exactly what an earlier
  /// `!complete && caveat` condition did.
  it("warns even when no reason is supplied", () => {
    render(<Leaderboards complete={false} rows={[row("a")]} />);
    expect(screen.getByText(/rankings are incomplete/i)).toBeTruthy();
    expect(screen.getByText(/the order may be wrong/i)).toBeTruthy();
  });
});
