import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AuthorRow } from "@/types/pr";
import { LINES_CHANGED_LABEL, Leaderboards, REVIEWS_LABEL, TOP_N } from "./Leaderboard";

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
    expect(screen.getByText(/no reviews in this window/i)).toBeTruthy();
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

  /// The review board says RECEIVED, and is not titled "top reviewers".
  /// `reviews { totalCount }` hangs off a pull request the author WROTE, so
  /// a board titled "top reviewers" would name the person whose code was
  /// reviewed most -- close to the opposite of what a reader would take it
  /// for.
  it("does not claim to rank reviewers", () => {
    render(<Leaderboards complete rows={[row("a", { reviewsReceived: 1 })]} />);
    expect(screen.getByText(REVIEWS_LABEL)).toBeTruthy();
    expect(screen.getByText(/most-reviewed/i)).toBeTruthy();
    expect(screen.queryByText(/^top \d+ reviewers$/i)).toBeNull();
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
