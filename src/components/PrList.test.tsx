import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PR_FIXTURES } from "@/fixtures/prs";
import { PrList } from "@/components/PrList";

describe("PrList", () => {
  it("renders every PR with its number and title", () => {
    render(<PrList prs={PR_FIXTURES} />);
    expect(screen.getByText("Add retry to the fetch client")).toBeDefined();
    expect(screen.getByText(/#42/)).toBeDefined();
    expect(screen.getByText(/#43/)).toBeDefined();
  });

  it("renders label pills", () => {
    render(<PrList prs={PR_FIXTURES} />);
    expect(screen.getByText("enhancement")).toBeDefined();
    expect(screen.getByText("bug")).toBeDefined();
  });

  it("marks drafts", () => {
    render(<PrList prs={PR_FIXTURES} />);
    expect(screen.getByText(/draft/i)).toBeDefined();
  });

  it("shows the open count in the header", () => {
    render(<PrList prs={PR_FIXTURES} />);
    expect(screen.getByText(/3 Open/)).toBeDefined();
  });

  it("renders an empty state rather than a bare list", () => {
    render(<PrList prs={[]} />);
    expect(screen.getByText(/No pull requests/i)).toBeDefined();
  });

  /// Deliberately passes the fixtures in the WRONG order. PR_FIXTURES is
  /// already newest-first, so feeding it in unchanged would pass even against
  /// a component that does no sorting at all -- the assertion would be
  /// measuring the fixture, not the code.
  it("orders newest first even when handed the list reversed", () => {
    const oldestFirst = [...PR_FIXTURES].sort(
      (a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime(),
    );
    render(<PrList prs={oldestFirst} />);

    const rendered = screen
      .getAllByRole("link")
      .map((el) => el.textContent);
    const expected = [...PR_FIXTURES]
      .sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
      .map((pr) => pr.title);

    expect(rendered).toEqual(expected);
  });
});
