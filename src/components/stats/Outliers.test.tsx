import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { BoardPr, MergedPr } from "@/types/pr";
import { Outliers } from "./Outliers";

const pr = (over: Partial<MergedPr> = {}): MergedPr => ({
  number: 42,
  title: "Add retry to the fetch client",
  url: "https://github.com/octocat/hello-world/pull/42",
  repo: "octocat/hello-world",
  cycle_time_hours: 2,
  size: 300,
  ...over,
});

/// The unscoped page's accessor. Passed explicitly in every case below
/// rather than defaulted in the component, because the whole reason it is a
/// prop is that the two callers spell the field differently -- a default
/// would silently read 0 for whichever shape it did not match.
const bySnake = (p: MergedPr) => p.cycle_time_hours;

describe("Outliers", () => {
  // The point of the component: a striking figure becomes reachable.
  it("links each PR to GitHub", () => {
    render(<Outliers slowest={[pr()]} largest={[]} slowestBy={bySnake} />);
    const link = screen.getByRole("link", { name: /add retry/i });
    expect(link.getAttribute("href")).toBe(
      "https://github.com/octocat/hello-world/pull/42",
    );
  });

  it("shows hours below a day and days above", () => {
    const { unmount } = render(
      <Outliers
        slowest={[pr({ cycle_time_hours: 5 })]}
        largest={[]}
        slowestBy={bySnake}
      />,
    );
    expect(screen.getByText("5.0h")).toBeTruthy();
    unmount();
    render(
      <Outliers
        slowest={[pr({ cycle_time_hours: 96 })]}
        largest={[]}
        slowestBy={bySnake}
      />,
    );
    expect(screen.getByText("4.0d")).toBeTruthy();
  });

  it("formats large line counts readably", () => {
    render(
      <Outliers slowest={[]} largest={[pr({ size: 10088 })]} slowestBy={bySnake} />,
    );
    expect(screen.getByText("10,088 lines")).toBeTruthy();
  });

  // Both lists come from the same sample, so an empty one means no data
  // rather than an error -- render nothing rather than an empty card.
  it("renders nothing for an empty list", () => {
    const { container } = render(
      <Outliers slowest={[]} largest={[]} slowestBy={bySnake} />,
    );
    expect(container.querySelectorAll("a").length).toBe(0);
    expect(screen.queryByText(/slowest to merge/i)).toBeNull();
  });

  it("says the figures are from a sample, not all time", () => {
    render(<Outliers slowest={[pr()]} largest={[]} slowestBy={bySnake} />);
    expect(screen.getByText(/in this sample/i)).toBeTruthy();
  });

  /// The scoped caller's shape, which spells the cycle time in camelCase.
  ///
  /// The reason the component takes an accessor rather than a field name:
  /// `MergedPr` and `BoardPr` are two serialized Rust types with two
  /// conventions, and renaming either to suit this component would be a
  /// change to the wire format for a rendering convenience. This case is
  /// what stops the generalisation from silently working for only one of
  /// them -- the bug a default accessor would have shipped.
  it("serves the scoped shape, which names the field differently", () => {
    const boardPr: BoardPr = {
      number: 7,
      title: "A long-running change",
      url: "https://github.com/octocat/hello-world/pull/7",
      repo: "octocat/hello-world",
      author: "octocat",
      cycleTimeHours: 48,
      size: 12,
    };
    render(
      <Outliers
        slowest={[boardPr]}
        largest={[]}
        slowestBy={(p) => p.cycleTimeHours}
        hint="across everyone in this scope and window"
      />,
    );
    expect(screen.getByText("2.0d")).toBeTruthy();
    // And the caller's wording replaces the sample caveat, because a scope
    // page measures the whole window rather than a fixed recent sample and
    // "in this sample" would understate it.
    expect(screen.getByText(/across everyone in this scope/i)).toBeTruthy();
    expect(screen.queryByText(/in this sample/i)).toBeNull();
  });
});
