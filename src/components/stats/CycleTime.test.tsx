import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CycleTime } from "./CycleTime";

describe("CycleTime", () => {
  it("shows the median and the tail", () => {
    render(<CycleTime hours={[1, 2, 3]} prs={3} />);
    expect(screen.getByText("2.0h")).toBeTruthy();
    expect(screen.getByText(/median over 3 merged/)).toBeTruthy();
  });

  it("shows days above a day and hours below", () => {
    const { unmount } = render(<CycleTime hours={[48]} prs={1} />);
    expect(screen.getByText("2.0d")).toBeTruthy();
    unmount();
    render(<CycleTime hours={[5]} prs={1} />);
    expect(screen.getByText("5.0h")).toBeTruthy();
  });

  /// For a small distribution "p90" resolves to the largest value in it, so
  /// a single weekend pull request would be presented with the authority of
  /// a tail metric. `InsightCards` established this rule and the reason
  /// holds unchanged here.
  it("calls the tail slowest on a small sample and p90 on a large one", () => {
    const { unmount } = render(<CycleTime hours={[1, 2, 3]} prs={3} />);
    expect(screen.getByText(/slowest 3\.0h/)).toBeTruthy();
    expect(screen.queryByText(/p90/)).toBeNull();
    unmount();
    const many = Array.from({ length: 25 }, (_, i) => i + 1);
    render(<CycleTime hours={many} prs={25} />);
    expect(screen.getByText(/p90/)).toBeTruthy();
  });

  /// The gap between `prs` and `hours.length` is INFORMATION, and the label
  /// must say which information.
  ///
  /// On a merged-only board every counted pull request HAS merged, so the gap
  /// is a timestamp the app could not use -- not unfinished work. The first
  /// version said "still open, not counted", which called a merged pull
  /// request unfinished. Found in review, and this is the test that pins the
  /// corrected reading.
  it("says the gap is an unusable merge time, not an open pull request", () => {
    render(<CycleTime hours={[1, 2, 3]} prs={10} />);
    expect(screen.getByText(/7 excluded, no usable merge time/)).toBeTruthy();
    expect(screen.queryByText(/still open/i)).toBeNull();
  });

  it("says nothing about a gap when there is none", () => {
    render(<CycleTime hours={[1, 2, 3]} prs={3} />);
    expect(screen.queryByText(/excluded/)).toBeNull();
  });

  /// An empty distribution is NOT "no data": there is data, and a median over
  /// an empty set is undefined, so saying why beats a bare dash. And the why
  /// is again a timestamp problem rather than "none of these merged", which a
  /// merged-only board cannot produce.
  it("explains an empty distribution rather than printing a bare dash", () => {
    render(<CycleTime hours={[]} prs={4} />);
    expect(screen.getByText("--")).toBeTruthy();
    expect(screen.getByText(/no usable merge time on any of these 4/i)).toBeTruthy();
    expect(screen.queryByText(/merged in this window/i)).toBeNull();
  });

  it("distinguishes no pull requests from no usable times", () => {
    render(<CycleTime hours={[]} prs={0} />);
    expect(screen.getByText(/no pull requests in this window/i)).toBeTruthy();
  });

  /// Singular wording, because "any of these 1 pull requests" is the kind of
  /// seam that makes a careful page look careless.
  it("reads correctly for a single pull request", () => {
    render(<CycleTime hours={[]} prs={1} />);
    expect(screen.getByText(/no usable merge time on the one pull request/i)).toBeTruthy();
  });
});
