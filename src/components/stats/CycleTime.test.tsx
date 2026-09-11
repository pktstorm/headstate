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

  /// The gap between `prs` and `hours.length` is INFORMATION: an open pull
  /// request has no cycle time, so a median over the merged half of
  /// someone's work is a different figure from a median over all of it. This
  /// is the only place that difference is visible, and printing
  /// `hours.length` for both would make it read "3 of 3 merged" -- true of
  /// the distribution and false about the person.
  it("says how many are still open and not counted", () => {
    render(<CycleTime hours={[1, 2, 3]} prs={10} />);
    expect(screen.getByText(/7 still open, not counted/)).toBeTruthy();
  });

  it("says nothing about open pull requests when there are none", () => {
    render(<CycleTime hours={[1, 2, 3]} prs={3} />);
    expect(screen.queryByText(/still open/)).toBeNull();
  });

  /// An empty distribution is NOT "no data": there is data, and what it says
  /// is that none of these merged. A median over an empty set is undefined,
  /// and saying why beats a bare dash.
  it("explains an empty distribution rather than printing a bare dash", () => {
    render(<CycleTime hours={[]} prs={4} />);
    expect(screen.getByText("--")).toBeTruthy();
    expect(screen.getByText(/none of these 4 pull requests merged/i)).toBeTruthy();
  });

  it("distinguishes no pull requests from none merged", () => {
    render(<CycleTime hours={[]} prs={0} />);
    expect(screen.getByText(/no pull requests in this window/i)).toBeTruthy();
  });

  /// Singular wording, because "none of these 1 pull requests" is the kind
  /// of seam that makes a careful page look careless.
  it("reads correctly for a single unmerged pull request", () => {
    render(<CycleTime hours={[]} prs={1} />);
    expect(screen.getByText(/none of this pull request merged/i)).toBeTruthy();
  });
});
