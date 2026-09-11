import { cleanup, fireEvent, screen } from "@testing-library/react";
// The sidebar renders ViewSwitcher, which reads `useUiPrefs`.
import { renderWithQuery as render } from "@/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PR_FIXTURES } from "@/fixtures/prs";
import { useFilters } from "@/store/filters";
import { stubViewport } from "@/test-utils";
import { RepoSidebar } from "./RepoSidebar";

afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
  stubViewport(null);
  useFilters.getState().reset();
});

describe("RepoSidebar on the companion", () => {
  /// PR Stats is still desktop-only (#794), but this component no longer
  /// decides that: the pinned row it used to hide is gone, and the
  /// destination is a `ViewSwitcher` entry filtered by
  /// `MOBILE_HIDDEN_VIEWS`. What is pinned here is that the repo rows --
  /// which the phone DOES have -- are unaffected, since the old mobile
  /// branch wrapped the bottom of this column.
  it("keeps every repo row, with no Stats row left to hide", async () => {
    vi.stubEnv("VITE_TARGET", "mobile");
    vi.resetModules();
    const { RepoSidebar: Mobile } = await import("./RepoSidebar");
    render(<Mobile prs={PR_FIXTURES} />);
    expect(screen.queryByRole("button", { name: /^stats$/i })).toBeNull();
    expect(screen.getByText("All repositories")).toBeTruthy();
    expect(screen.getByText("octocat/hello-world")).toBeTruthy();
    expect(screen.getByText("octocat/spoon-knife")).toBeTruthy();
  });

  /// The viewport must not decide what this column holds, which is the
  /// rule #598 established and the reason `MOBILE_HIDDEN_VIEWS` is keyed
  /// on the build. 390px on a DESKTOP build is still a desktop.
  it("renders the same rows however narrow a desktop window is", () => {
    stubViewport(390);
    render(<RepoSidebar prs={PR_FIXTURES} />);
    expect(screen.getByText("All repositories")).toBeTruthy();
    expect(screen.getByText("octocat/hello-world")).toBeTruthy();
  });
});

describe("RepoSidebar", () => {
  it("lists only repos that currently have PRs", () => {
    render(<RepoSidebar prs={PR_FIXTURES} />);
    expect(screen.getByText("octocat/hello-world")).toBeDefined();
    expect(screen.getByText("octocat/spoon-knife")).toBeDefined();
  });

  it("always offers an All entry", () => {
    render(<RepoSidebar prs={PR_FIXTURES} />);
    expect(screen.getByText(/All/)).toBeDefined();
  });

  it("the All entry is selected by default", () => {
    render(<RepoSidebar prs={PR_FIXTURES} />);
    expect(screen.getByText("All repositories").closest("button")?.className).toContain(
      "bg-[#1f6feb]",
    );
  });

  it("selecting a repo writes through the shared filter store", () => {
    render(<RepoSidebar prs={PR_FIXTURES} />);
    fireEvent.click(screen.getByText("octocat/spoon-knife"));
    expect(useFilters.getState().filtersByView[useFilters.getState().view].repo).toBe("octocat/spoon-knife");
  });

  it("selecting All clears the repo filter", () => {
    useFilters.getState().setFilter("repo", "octocat/spoon-knife");
    render(<RepoSidebar prs={PR_FIXTURES} />);
    fireEvent.click(screen.getByText("All repositories"));
    expect(useFilters.getState().filtersByView[useFilters.getState().view].repo).toBeUndefined();
  });

  it("shows a count badge per repo, busiest first in DOM order", () => {
    render(<RepoSidebar prs={PR_FIXTURES} />);
    const buttons = screen.getAllByRole("button");
    // The view switcher heads the sidebar, then the repo rows, and
    // nothing after them. The pinned Stats row that used to close this
    // list is gone (#794) -- PR Stats is a switcher entry now, which is
    // where "Awaiting your review" went before it.
    expect(buttons.map((b) => b.textContent)).toEqual([
      "My pull requests",
      "All repositories3",
      "octocat/hello-world2",
      "octocat/spoon-knife1",
    ]);
  });

  /// The repo rows are the LAST thing in the column now. Asserting this
  /// rather than just the absence of a Stats button is the point: the row
  /// sat outside the scrolling region in a bordered block of its own, and
  /// a leftover empty block would still pass a presence check while
  /// drawing a stray rule under the list.
  it("ends with the repo rows, with nothing pinned below them", () => {
    const { container } = render(<RepoSidebar prs={PR_FIXTURES} />);
    const labels = screen.getAllByRole("button").map((b) => b.textContent);
    expect(labels[labels.length - 1]).toBe("octocat/spoon-knife1");
    // The scrolling repo list is the last child of the nav: no sibling
    // block left behind it.
    const nav = container.querySelector("nav")!;
    expect(nav.lastElementChild?.className).toContain("overflow-y-auto");
  });

  /// A repo selection highlights on PR Stats as well as on My PRs (#794).
  /// There is no pinned Stats row competing for the highlight any more,
  /// and a click that visibly does nothing is worse than one that filters
  /// nothing.
  it("highlights the selected repo on PR Stats too", () => {
    useFilters.setState({ view: "pr-stats" });
    render(<RepoSidebar prs={PR_FIXTURES} />);

    fireEvent.click(screen.getByRole("button", { name: /octocat\/hello-world/ }));

    // Written to PR Stats' own filter set, not My PRs'.
    expect(useFilters.getState().filtersByView["pr-stats"].repo).toBe("octocat/hello-world");
    expect(useFilters.getState().filtersByView["my-prs"].repo).toBeUndefined();
    expect(
      screen.getByRole("button", { name: /octocat\/hello-world/ }).className,
    ).toContain("bg-[#1f6feb]");
  });
});
