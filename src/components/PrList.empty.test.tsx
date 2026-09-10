import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PrList } from "./PrList";
import { PR_FIXTURES } from "@/fixtures/prs";
// The two tests that render real ROWS need the query provider those
// rows' action menus read; the empty-state tests here do not.
import { renderWithQuery } from "@/test-utils";

describe("PrList empty states", () => {
  // One fixed string used to cover both cases. With no filters set it read
  // as a bug, and told a new user nothing about what the app tracks.
  it("explains what the app tracks when nothing is filtered", () => {
    render(<PrList prs={[]} hasFilters={false} />);
    expect(screen.getByText(/no open pull requests/i)).toBeTruthy();
    expect(screen.getByText(/pull requests you opened/i)).toBeTruthy();
  });

  it("blames the filters only when filters are actually active", () => {
    render(<PrList prs={[]} hasFilters />);
    expect(screen.getByText(/match these filters/i)).toBeTruthy();
    expect(screen.queryByText(/pull requests you opened/i)).toBeNull();
  });
});

describe("PrList truncation notice", () => {
  // Silent truncation made the priorities strip -- whose whole job is
  // never to have a false negative -- filter a subset without saying so.
  it("says so when GitHub has more PRs than it returned", () => {
    render(<PrList prs={[]} hasFilters={false} total={137} />);
    expect(screen.getByText(/showing 0 of 137/i)).toBeTruthy();
  });

  it("stays silent in the normal case", () => {
    render(<PrList prs={[]} hasFilters={false} />);
    expect(screen.queryByText(/showing/i)).toBeNull();
  });

  it("stays silent when the total equals what was returned", () => {
    render(<PrList prs={[]} hasFilters={false} total={0} />);
    expect(screen.queryByText(/showing/i)).toBeNull();
  });

  /// The worst case in the six-day log (#745): 8 of 29 rendered with the
  /// other 21 unmentioned, which reads as "these are the open pull
  /// requests" to anyone scanning for work.
  it("counts what the poll fetched, not what the filters left", () => {
    renderWithQuery(<PrList prs={PR_FIXTURES} hasFilters total={29} fetched={8} />);
    expect(screen.getByText(/showing 8 of 29/i)).toBeTruthy();
  });

  /// A filter narrowing a COMPLETE list is not a truncation. Comparing
  /// GitHub's unfiltered count against the visible rows invented one.
  it("does not claim truncation when a filter merely narrowed a full list", () => {
    renderWithQuery(<PrList prs={PR_FIXTURES.slice(0, 1)} hasFilters total={0} fetched={3} />);
    expect(screen.queryByText(/showing/i)).toBeNull();
  });

  /// The marker has to be findable by the same affordance `StaleRibbon`
  /// uses, or a screen reader gets the confident list and none of the
  /// doubt attached to it.
  it("announces itself as a status", () => {
    render(<PrList prs={[]} hasFilters={false} total={29} fetched={8} />);
    expect(screen.getByRole("status").textContent).toMatch(/showing 8 of 29/i);
  });

  /// Naming a page size told the user about a limit that is not the
  /// cause: the poll pages through everything, and a short list means
  /// pages FAILED. A refresh is the action that actually helps.
  it("says the rest did not load rather than naming a page size", () => {
    render(<PrList prs={[]} hasFilters={false} total={29} fetched={8} />);
    const notice = screen.getByRole("status").textContent ?? "";
    expect(notice).toMatch(/did not load/i);
    expect(notice).toMatch(/refresh/i);
    expect(notice).not.toMatch(/100/);
  });
});
