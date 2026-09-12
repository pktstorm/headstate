import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/// #852: this column conveyed selection by BACKGROUND COLOUR ALONE, with
/// zero `aria-current` -- and had no test file at all, which is how.
///
/// `StatsSidebar` already states the rule: "`aria-current` rather than only a
/// colour: the selection is navigation state, and a screen reader reading a
/// list of repository names has no other way to know which one is open."
const repos = vi.hoisted(() => vi.fn<() => unknown>(() => []));
const scan = vi.hoisted(() => ({ loading: false }));
const selected = vi.hoisted(() => ({ repo: undefined as string | undefined }));

vi.mock("@/api/hooks", () => ({
  useWorktrees: () => ({ data: repos(), isLoading: scan.loading }),
}));
vi.mock("./ViewSwitcher", () => ({ ViewSwitcher: () => null }));
vi.mock("@/store/filters", () => ({
  useActiveFilters: () => ({ repo: selected.repo }),
  useFilters: () => ({ setFilter: vi.fn() }),
}));

import { RepoPickerSidebar } from "./RepoPickerSidebar";

const repo = (name: string) => ({
  identity: null,
  name,
  path: `/code/${name}`,
  worktrees: [],
});

beforeEach(() => {
  repos.mockReturnValue([]);
  scan.loading = false;
  selected.repo = undefined;
});

describe("RepoPickerSidebar", () => {
  it("lists every scanned repository", () => {
    repos.mockReturnValue([repo("alpha"), repo("beta")]);
    render(<RepoPickerSidebar reviewingCount={0} />);
    expect(screen.getByText("alpha")).toBeTruthy();
    expect(screen.getByText("beta")).toBeTruthy();
  });

  /// The rule this component's own comment documents: "'No repositories
  /// found in the scanned folders' is a DIAGNOSIS, not a holding message…
  /// sends someone to fix something that is not broken." Pinned here because
  /// nothing was pinning it.
  it("says it is looking rather than diagnosing, before the scan answers", () => {
    scan.loading = true;
    render(<RepoPickerSidebar reviewingCount={0} />);
    expect(screen.getByText(/looking for repositories/i)).toBeTruthy();
    expect(screen.queryByText(/no repositories found/i)).toBeNull();
  });

  it("gives the diagnosis once the scan has answered with nothing", () => {
    repos.mockReturnValue([]);
    render(<RepoPickerSidebar reviewingCount={0} />);
    expect(screen.getByText(/no repositories found in the scanned folders/i)).toBeTruthy();
  });

  describe("selection is not conveyed by colour alone", () => {
    it("marks the selected repository as current", () => {
      repos.mockReturnValue([repo("alpha"), repo("beta")]);
      selected.repo = "/code/alpha";
      render(<RepoPickerSidebar reviewingCount={0} />);
      expect(
        screen.getByText("alpha").closest("button")?.getAttribute("aria-current"),
      ).toBe("true");
      // And only that one: two `aria-current` rows would announce two
      // locations at once.
      expect(
        screen.getByText("beta").closest("button")?.getAttribute("aria-current"),
      ).toBeNull();
    });

    /// `undefined`, never `"false"`: absence is how "not current" is
    /// spelled, and `aria-current="false"` is announced by some readers, so
    /// an unselected row would say so out loud.
    it("omits the attribute on unselected rows rather than setting it false", () => {
      repos.mockReturnValue([repo("alpha"), repo("beta")]);
      render(<RepoPickerSidebar reviewingCount={0} />);
      const rows = screen.getAllByRole("button");
      expect(rows.some((r) => r.getAttribute("aria-current") === "false")).toBe(false);
      // With nothing scoped, no row claims to be the current one -- this
      // column has no "All repositories" entry to fall back to.
      expect(rows.some((r) => r.getAttribute("aria-current") !== null)).toBe(false);
    });
  });
});
