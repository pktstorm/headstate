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

  /// This column no longer serves PR Stats (#825), which has its own
  /// `StatsSidebar`, so it must NOT highlight a repo row for that view.
  ///
  /// The inverse of the assertion this replaces, and the replacement is
  /// the point rather than a deletion: between #794 and #825 the rows were
  /// live on PR Stats and read by nothing, and the highlight was defended
  /// as "the honest thing to render: it says what was clicked". Now that
  /// the view has a column whose rows DO something, a highlight here would
  /// be for a filter key (`repo`) that PR Stats no longer uses -- its
  /// scope lives in `statsScopeKind` / `statsScopeValue`.
  ///
  /// Kept as a rendered assertion rather than trusting `repoActive`,
  /// because `App.tsx` is what stops this component reaching PR Stats at
  /// all and a routing change could quietly bring it back.
  it("does not highlight a repo row on PR Stats, which has its own sidebar", () => {
    useFilters.setState({ view: "pr-stats" });
    render(<RepoSidebar prs={PR_FIXTURES} />);

    fireEvent.click(screen.getByRole("button", { name: /octocat\/hello-world/ }));

    // The write still lands in PR Stats' own filter set -- `setFilter`
    // writes `[view]` and that is not this component's business -- but
    // nothing reads `repo` for that view any more, so nothing lights up.
    expect(useFilters.getState().filtersByView["my-prs"].repo).toBeUndefined();
    expect(
      screen.getByRole("button", { name: /octocat\/hello-world/ }).className,
    ).not.toContain("bg-[#1f6feb]");
    // And "All repositories" must not claim to be selected either: a view
    // this column does not serve should look unselected all the way down,
    // rather than defaulting to a highlight on the first row.
    expect(screen.getByText("All repositories").closest("button")?.className).not.toContain(
      "bg-[#1f6feb]",
    );
    // And the same in the accessible layer (#852), which is the half a
    // `className` assertion cannot see: a row that is not serving this view
    // must not announce itself as the current location either.
    expect(
      screen.getByText("All repositories").closest("button")?.getAttribute("aria-current"),
    ).toBeNull();
  });

  /// #852: selection was conveyed by BACKGROUND COLOUR ALONE.
  ///
  /// Every assertion in this file reached for `className` and
  /// `bg-[#1f6feb]`, which is precisely the problem restated: the blue WAS
  /// the selection, so the tests could only check the blue. `StatsSidebar`
  /// already states the rule: "`aria-current` rather than only a colour: the
  /// selection is navigation state, and a screen reader reading a list of
  /// repository names has no other way to know which one is open."
  describe("selection is not conveyed by colour alone", () => {
    it("marks the selected repository as current", () => {
      useFilters.setState({ view: "my-prs" });
      useFilters.getState().setFilter("repo", "octocat/hello-world");
      render(<RepoSidebar prs={PR_FIXTURES} />);
      const row = screen.getByRole("button", { name: /octocat\/hello-world/ });
      expect(row.getAttribute("aria-current")).toBe("true");
      expect(
        screen.getByText("All repositories").closest("button")?.getAttribute("aria-current"),
      ).toBeNull();
    });

    it("marks All repositories as current when nothing is scoped", () => {
      useFilters.setState({ view: "my-prs" });
      // Cleared explicitly: the file's `afterEach` calls `reset()`, which
      // deliberately KEEPS the repo (see the store test "reset clears
      // filters but keeps the repo"), so a repo set by an earlier test
      // survives into this one and "nothing is scoped" would be untrue.
      useFilters.getState().setFilter("repo", undefined);
      render(<RepoSidebar prs={PR_FIXTURES} />);
      expect(
        screen.getByText("All repositories").closest("button")?.getAttribute("aria-current"),
      ).toBe("true");
    });

    /// `undefined`, never `"false"`: absence is how "not current" is
    /// spelled, and `aria-current="false"` is announced by some readers.
    it("omits the attribute on unselected rows rather than setting it false", () => {
      useFilters.setState({ view: "my-prs" });
      render(<RepoSidebar prs={PR_FIXTURES} />);
      const rows = screen.getAllByRole("button");
      expect(rows.some((r) => r.getAttribute("aria-current") === "false")).toBe(false);
    });

    /// The `repoActive` guard applies to the accessible layer too: on a view
    /// this column does not serve, NO row is current. That guard exists
    /// because PR Stats was in exactly that state for a release.
    it("marks nothing as current on a view it does not serve", () => {
      useFilters.setState({ view: "pr-stats" });
      render(<RepoSidebar prs={PR_FIXTURES} />);
      const rows = screen.getAllByRole("button");
      expect(rows.some((r) => r.getAttribute("aria-current") !== null)).toBe(false);
    });
  });
});
