import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PR_FIXTURES } from "@/fixtures/prs";
import { PrioritiesStrip } from "./PrioritiesStrip";

describe("PrioritiesStrip", () => {
  it("shows PRs with failing CI or conflicts", () => {
    render(<PrioritiesStrip prs={PR_FIXTURES} />);
    expect(screen.getByText(/Fix flaky timezone test/)).toBeDefined();
  });

  it("does not show healthy PRs", () => {
    render(<PrioritiesStrip prs={PR_FIXTURES} />);
    expect(screen.queryByText(/Add retry to the fetch client/)).toBeNull();
  });

  /// The strip is only worth looking at if it never cries wolf. A PR whose
  /// mergeability GitHub has not finished computing is not a conflict.
  it("does not show a PR whose merge state is still checking", () => {
    render(<PrioritiesStrip prs={PR_FIXTURES} />);
    expect(screen.queryByText(/Bump the parser dependency/)).toBeNull();
  });

  it("renders a quiet line when nothing needs attention", () => {
    render(<PrioritiesStrip prs={[PR_FIXTURES[0]]} />);
    expect(screen.getByText(/Nothing blocked/i)).toBeDefined();
  });
});
