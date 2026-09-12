import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Artifact, Venv } from "@/types/pr";

/// #846: this column had ZERO loading or error handling, and no test file
/// at all -- which is how it shipped that way.
///
/// The `= []` defaults on both scans made a REJECTED scan produce an empty
/// `groups` map, so the sidebar rendered one row ("Everything") and nothing
/// else: identical to a machine with no build output and no virtualenvs.
/// On a disk-cleanup tool that reads as "your machine is clean" when the
/// truth is it could not look, and nobody investigates good news.
const state = vi.hoisted(() => ({
  artifacts: [] as Artifact[],
  venvs: [] as Venv[],
  artifactsLoading: false,
  artifactsFailed: false,
  venvsLoading: false,
  venvsFailed: false,
}));

const refetchArtifacts = vi.hoisted(() => vi.fn());
const refetchVenvs = vi.hoisted(() => vi.fn());

vi.mock("../api/hooks", () => ({
  useArtifacts: () => ({
    data: state.artifacts,
    isLoading: state.artifactsLoading,
    isError: state.artifactsFailed,
    refetch: refetchArtifacts,
  }),
  useVenvs: () => ({
    data: state.venvs,
    isLoading: state.venvsLoading,
    isError: state.venvsFailed,
    refetch: refetchVenvs,
  }),
  // Sizes are not the subject here: the rows exist or they do not, and a
  // group with no size yet still renders its count.
  useArtifactSizes: () => ({ sizes: new Map(), ages: new Map(), pending: 0, total: 0 }),
  useVenvSizes: () => ({ sizes: new Map(), idle: new Map(), measuring: false, pending: 0, total: 0 }),
}));
vi.mock("./ViewSwitcher", () => ({ ViewSwitcher: () => null }));
// Which group the sidebar believes is selected, so the `aria-current` tests
// below can exercise a row that is current and a row that is not (#852).
const selected = vi.hoisted(() => ({ repo: undefined as string | undefined }));
vi.mock("@/store/filters", () => ({
  useActiveFilters: () => ({ repo: selected.repo }),
  useFilters: () => ({ setFilter: vi.fn() }),
}));

import { ArtifactSidebar } from "./ArtifactSidebar";

const art = (over: Partial<Artifact> = {}): Artifact => ({
  path: "/code/repo/target",
  kind: "cargo_target",
  repo_path: "/code/repo",
  size_bytes: null,
  ...over,
});

const venv = (over: Partial<Venv> = {}): Venv =>
  ({
    path: "/cache/p-AAAAAAAA-py3.13",
    project: "p",
    state: "orphaned",
    source: null,
    size_bytes: null,
    idle_secs: null,
    ...over,
  }) as Venv;

beforeEach(() => {
  selected.repo = undefined;
  refetchArtifacts.mockClear();
  refetchVenvs.mockClear();
  state.artifacts = [];
  state.venvs = [];
  state.artifactsLoading = false;
  state.artifactsFailed = false;
  state.venvsLoading = false;
  state.venvsFailed = false;
});

describe("ArtifactSidebar", () => {
  it("lists one row per kind actually found", () => {
    state.artifacts = [art(), art({ path: "/code/x/node_modules", kind: "node_modules" })];
    render(<ArtifactSidebar reviewingCount={0} />);
    expect(screen.getByText("Rust targets")).toBeTruthy();
    expect(screen.getByText("Node modules")).toBeTruthy();
  });

  it("adds the virtualenv group only when there are virtualenvs", () => {
    state.artifacts = [art()];
    render(<ArtifactSidebar reviewingCount={0} />);
    expect(screen.queryByText("Poetry virtualenvs")).toBeNull();
    state.venvs = [venv()];
    const { unmount } = render(<ArtifactSidebar reviewingCount={0} />);
    expect(screen.getAllByText("Poetry virtualenvs").length).toBeGreaterThan(0);
    unmount();
  });

  /// A successful scan that found nothing is the ordinary clean machine,
  /// and must stay quiet: an error or a holding message on every tidy
  /// machine would train the eye to skip both.
  it("says nothing when both scans answered and found nothing", () => {
    render(<ArtifactSidebar reviewingCount={0} />);
    expect(screen.queryByText(/could not scan/i)).toBeNull();
    expect(screen.queryByText(/looking for/i)).toBeNull();
    expect(screen.getByText("Everything")).toBeTruthy();
  });

  describe("when a scan fails", () => {
    /// The defect: the column said nothing at all, so a failure looked
    /// exactly like a clean machine.
    it("does not render a silently empty group list", () => {
      state.artifactsFailed = true;
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(screen.getByText(/could not scan for build output/i)).toBeTruthy();
    });

    /// NAMES which scan failed. The two feed different rows, and a user
    /// told only that "a scan failed" cannot tell whether the virtualenv
    /// group is absent because it failed or because there are none.
    it("names the artifact scan when only that one failed", () => {
      state.artifactsFailed = true;
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(screen.getByText(/could not scan for build output\./i)).toBeTruthy();
      expect(screen.queryByText(/virtualenvs/i)).toBeNull();
    });

    it("names the virtualenv scan when only that one failed", () => {
      state.venvsFailed = true;
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(screen.getByText(/could not scan for virtualenvs\./i)).toBeTruthy();
      expect(screen.queryByText(/build output/i)).toBeNull();
    });

    it("names both when both failed", () => {
      state.artifactsFailed = true;
      state.venvsFailed = true;
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(screen.getByText(/build output or virtualenvs/i)).toBeTruthy();
    });

    /// Retries only what actually FAILED. Re-running a scan that succeeded
    /// would throw away a result the page is still rendering -- and the
    /// virtualenv scan is 26 seconds.
    it("retries only the scan that failed", () => {
      state.artifactsFailed = true;
      render(<ArtifactSidebar reviewingCount={0} />);
      fireEvent.click(screen.getByRole("button", { name: /try again/i }));
      expect(refetchArtifacts).toHaveBeenCalled();
      expect(refetchVenvs).not.toHaveBeenCalled();
    });

    /// `failed` is checked before `loading` because a retry leaves both
    /// true for a moment, and flipping to "Looking…" mid-retry would read
    /// as the error having resolved itself.
    it("keeps saying it failed while the retry is in flight", () => {
      state.artifactsFailed = true;
      state.artifactsLoading = true;
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(screen.getByText(/could not scan/i)).toBeTruthy();
      expect(screen.queryByText(/looking for reclaimable space/i)).toBeNull();
    });

    /// The rows a HEALTHY scan found still render. A venv failure must not
    /// take the artifact groups with it -- that would replace one silent
    /// loss with another.
    it("still lists the groups the other scan found", () => {
      state.venvsFailed = true;
      state.artifacts = [art()];
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(screen.getByText("Rust targets")).toBeTruthy();
    });
  });

  /// #852: selection was conveyed by BACKGROUND COLOUR ALONE, with zero
  /// `aria-current`. `StatsSidebar` states the rule: "the selection is
  /// navigation state, and a screen reader reading a list of repository
  /// names has no other way to know which one is open."
  describe("selection is not conveyed by colour alone", () => {
    it("marks the selected group as current", () => {
      state.artifacts = [art(), art({ path: "/code/x/node_modules", kind: "node_modules" })];
      selected.repo = "cargo_target";
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(
        screen.getByText("Rust targets").closest("button")?.getAttribute("aria-current"),
      ).toBe("true");
      expect(
        screen.getByText("Node modules").closest("button")?.getAttribute("aria-current"),
      ).toBeNull();
    });

    it("marks Everything as current when no group is scoped", () => {
      state.artifacts = [art()];
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(
        screen.getByText("Everything").closest("button")?.getAttribute("aria-current"),
      ).toBe("true");
    });

    /// `undefined`, never `"false"`: absence is how "not current" is
    /// spelled, and `aria-current="false"` is announced by some readers.
    it("omits the attribute on unselected rows rather than setting it false", () => {
      state.artifacts = [art()];
      selected.repo = "cargo_target";
      render(<ArtifactSidebar reviewingCount={0} />);
      const rows = screen.getAllByRole("button");
      expect(rows.some((r) => r.getAttribute("aria-current") === "false")).toBe(false);
    });
  });

  /// A HOLDING message, not a diagnosis. An empty group list before the
  /// scans answer is "we have not looked yet", and saying anything about
  /// the machine's contents there is the `RepoPickerSidebar` mistake:
  /// "sends someone to fix something that is not broken".
  describe("while a scan is running", () => {
    it("says it is looking rather than claiming anything about the machine", () => {
      state.artifactsLoading = true;
      render(<ArtifactSidebar reviewingCount={0} />);
      expect(screen.getByText(/looking for reclaimable space/i)).toBeTruthy();
      expect(screen.queryByText(/could not scan/i)).toBeNull();
    });
  });
});
