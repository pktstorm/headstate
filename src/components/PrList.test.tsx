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
});
