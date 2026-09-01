import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Artifact } from "@/types/pr";

const state = vi.hoisted(() => ({
  artifacts: [] as Artifact[],
  loading: false,
  sizes: new Map<string, number>(),
  ages: new Map<string, number>(),
  pending: 0,
  total: 0,
}));

vi.mock("../api/hooks", () => ({
  useArtifacts: () => ({ data: state.artifacts, isLoading: state.loading }),
  useArtifactSizes: () => ({
    sizes: state.sizes,
    ages: state.ages,
    pending: state.pending,
    total: state.total,
  }),
}));

import { ArtifactsPage } from "./ArtifactsPage";

const art = (over: Partial<Artifact> = {}): Artifact => ({
  path: "/code/repo/target",
  kind: "cargo_target",
  repo_path: "/code/repo",
  size_bytes: null,
  modified_secs_ago: null,
  ...over,
});

describe("ArtifactsPage", () => {
  it("says so when there is nothing to show", () => {
    state.artifacts = [];
    render(<ArtifactsPage />);
    expect(screen.getByText(/No build output found/)).toBeTruthy();
  });

  /// Every row names what puts it back. "You can delete this" is only
  /// actionable next to the command that restores it, and that pairing
  /// is the whole safety argument for the view.
  it("names the command that regenerates each kind", () => {
    state.artifacts = [
      art(),
      art({ path: "/code/repo/node_modules", kind: "node_modules" }),
      art({ path: "/code/repo/.terraform", kind: "terraform" }),
    ];
    render(<ArtifactsPage />);
    expect(screen.getByText("cargo build")).toBeTruthy();
    expect(screen.getByText("npm install")).toBeTruthy();
    expect(screen.getByText("terraform init")).toBeTruthy();
  });

  /// The total is over a PARTIAL set until every batch answers, and
  /// claiming a finished number while measurement runs is the kind of
  /// quiet wrongness that makes a figure untrustworthy everywhere else.
  it("says the total is a lower bound while measuring", () => {
    state.artifacts = [art()];
    state.sizes = new Map([["/code/repo/target", 1_000_000_000]]);
    state.pending = 2;
    state.total = 5;
    render(<ArtifactsPage />);
    expect(screen.getByText(/at least/)).toBeTruthy();
    expect(screen.getByText(/measuring — 3 of 5/)).toBeTruthy();
  });

  it("drops the lower-bound wording once every batch has answered", () => {
    state.artifacts = [art()];
    state.sizes = new Map([["/code/repo/target", 1_000_000_000]]);
    state.pending = 0;
    state.total = 5;
    render(<ArtifactsPage />);
    expect(screen.queryByText(/at least/)).toBeNull();
    expect(screen.queryByText(/measuring/)).toBeNull();
  });

  /// A build writing into a directory does not show up in `git status`,
  /// because build output is gitignored. The mtime is the only signal
  /// there is, so it must be surfaced rather than folded away.
  it("flags a directory that was written to recently", () => {
    state.artifacts = [art()];
    state.ages = new Map([["/code/repo/target", 60]]);
    state.sizes = new Map();
    state.pending = 0;
    render(<ArtifactsPage />);
    expect(screen.getByText("written recently")).toBeTruthy();
  });

  it("does not flag a directory nobody has touched", () => {
    state.artifacts = [art()];
    state.ages = new Map([["/code/repo/target", 60 * 60 * 24 * 30]]);
    render(<ArtifactsPage />);
    expect(screen.queryByText("written recently")).toBeNull();
  });

  /// An unmeasured row must render a PLACEHOLDER, never "0 B".
  ///
  /// This is the half a sort test cannot catch: zero and unmeasured
  /// order identically at the bottom, so only the rendered cell
  /// distinguishes them. Showing 0 B for a directory nobody has measured
  /// is a number the user would act on -- and the 61 GB target on the
  /// machine that prompted this feature reads 0 B for its first minute.
  it("shows a placeholder, not 0 B, for an unmeasured row", () => {
    state.artifacts = [art({ path: "/code/a/target" })];
    state.sizes = new Map();
    state.ages = new Map();
    state.pending = 1;
    state.total = 1;
    const { container } = render(<ArtifactsPage />);
    expect(screen.queryByText("0 B")).toBeNull();
    expect(container.querySelector(".animate-pulse, [class*=animate-pulse]")).toBeTruthy();
  });

  /// Ordering, which is the other half. Note a zero-sort produces the
  /// SAME order as sorting unmeasured last -- zero is the minimum, so
  /// both land at the bottom. The rendered placeholder above is what
  /// actually separates the two; this pins that the largest measured
  /// directory leads, which is the view's whole point.
  it("sorts unmeasured rows last rather than as zero", () => {
    // Three rows, and the UNMEASURED one sorts alphabetically FIRST.
    // With only two rows a zero-sort produces the same order by
    // accident, so this needs a measured row on each side of it to
    // distinguish "last" from "as zero".
    state.artifacts = [
      art({ path: "/code/a-unmeasured/target" }),
      art({ path: "/code/b-small/target" }),
      art({ path: "/code/c-large/target" }),
    ];
    state.sizes = new Map([
      ["/code/b-small/target", 1_000_000],
      ["/code/c-large/target", 9_000_000_000],
    ]);
    state.ages = new Map();
    state.pending = 0;
    render(<ArtifactsPage />);

    const paths = screen
      .getAllByText(/\/code\/[abc][^/]*\/target/)
      .map((el) => el.textContent);
    expect(paths).toEqual([
      "/code/c-large/target",
      "/code/b-small/target",
      "/code/a-unmeasured/target",
    ]);
  });
});
