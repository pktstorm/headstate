import { beforeEach, describe, expect, it } from "vitest";
import { ALL_VIEWS, useFilters } from "./filters";

const EMPTY = { "my-prs": {}, "to-review": {}, worktrees: {},
  branches: {}, docker: {}, artifacts: {}, packages: {}, "claude-md": {}, "pr-stats": {}, "system-health": {} } as const;
const active = () => {
  const s = useFilters.getState();
  return s.filtersByView[s.view];
};

describe("useFilters", () => {
  beforeEach(() =>
    useFilters.setState({ filtersByView: { ...EMPTY }, view: "my-prs", panel: "list" }),
  );

  it("sets an individual filter", () => {
    useFilters.getState().setFilter("repo", "octocat/hello-world");
    expect(active().repo).toBe("octocat/hello-world");
  });

  it("a preset replaces the filter set rather than merging", () => {
    useFilters.getState().setFilter("repo", "octocat/hello-world");
    useFilters.getState().applyPreset({ needsAttentionOnly: true });
    expect(active()).toEqual({ needsAttentionOnly: true });
  });

  // On "builds" rather than the old "stats": since #794 that value is
  // gone from the union, and Docker's is the only other sub-page left.
  it("a preset returns to the list panel", () => {
    useFilters.getState().setPanel("builds");
    useFilters.getState().applyPreset({ staleOnly: true });
    expect(useFilters.getState().panel).toBe("list");
  });

  // The reason filters are per-view: My PRs and Worktrees have entirely
  // different repo lists, so a selection in one is meaningless in the
  // other and would silently filter it to nothing.
  it("keeps each view's filters separate", () => {
    useFilters.getState().setFilter("repo", "octocat/hello-world");
    useFilters.getState().setView("worktrees");
    expect(active().repo).toBeUndefined();

    useFilters.getState().setFilter("repo", "some/other-repo");
    useFilters.getState().setView("my-prs");
    expect(active().repo).toBe("octocat/hello-world");
  });

  it("reset clears filters but keeps the repo, per view", () => {
    useFilters.getState().setFilter("repo", "octocat/hello-world");
    useFilters.getState().setFilter("staleOnly", true);
    useFilters.getState().reset();
    expect(active()).toEqual({ repo: "octocat/hello-world" });
  });

  // Switching views must not change which panel a view shows.
  it("panel is independent of view", () => {
    useFilters.getState().setPanel("builds");
    useFilters.getState().setView("to-review");
    useFilters.getState().setView("my-prs");
    expect(useFilters.getState().panel).toBe("builds");
  });

  // #794: PR Stats is reached by `setView`, which clears the selection
  // and the cursor like any other view change. Pinned because the route
  // in `App.tsx` puts the PR-detail branch FIRST, so a selection that
  // survived the switch would render one pull request over the summary.
  it("switching to PR Stats clears the selected pull request", () => {
    useFilters.getState().selectPr({ repo: "octocat/hello-world", number: 7 });
    useFilters.getState().setView("pr-stats");
    expect(useFilters.getState().selectedPr).toBeNull();
  });

  // The sidebar decision from #794: PR Stats keeps `RepoSidebar`, so a
  // click there has to land somewhere. It must not land in My PRs' set,
  // or picking a repo on one would silently re-filter the other. Nothing
  // reads `pr-stats.repo` yet -- `StatsPage` is whole-account -- but the
  // separation is what makes adding a scope later a one-page change
  // rather than an untangling.
  it("keeps PR Stats filters separate from My PRs", () => {
    useFilters.getState().setFilter("repo", "octocat/hello-world");
    useFilters.getState().setView("pr-stats");
    expect(active().repo).toBeUndefined();
    useFilters.getState().setFilter("repo", "some/other-repo");
    expect(active().repo).toBe("some/other-repo");
    useFilters.getState().setView("my-prs");
    expect(active().repo).toBe("octocat/hello-world");
  });
});

describe("persisted state migration", () => {
  // A store saved by v1 has a flat `filters` and a `view` enum that
  // conflated view with panel. Loading it into the new shape left
  // `filtersByView` undefined and crashed on first render -- invisible to
  // tests, which always start from empty, and hit immediately on a real
  // machine with saved state.
  const migrate = (useFilters.persist.getOptions().migrate ??
    ((s: unknown) => s)) as (s: unknown, v: number) => {
    filtersByView: Record<string, unknown>;
    view: string;
    panel: string;
  };

  it("lifts a v1 filter set into the active view", () => {
    const out = migrate({ filters: { repo: "octocat/hello-world" }, view: "list" }, 1);
    expect(out.view).toBe("my-prs");
    expect(out.panel).toBe("list");
    expect(out.filtersByView["my-prs"]).toEqual({ repo: "octocat/hello-world" });
  });

  // Was "maps the old dashboard enum to the stats panel". v2 sent it to
  // `panel: "stats"` because that was where the page lived; #794 moved
  // the page to a view, so the destination moved with it. Asserting the
  // panel as well is the point: "stats" is no longer a value `panel`
  // can hold, and a migration that still wrote it would leave a
  // persisted store the union says is impossible.
  it("maps the old dashboard enum to the PR Stats view", () => {
    const out = migrate({ filters: {}, view: "dashboard" }, 1);
    expect(out.view).toBe("pr-stats");
    expect(out.panel).toBe("list");
  });

  it("maps the old reviewing enum to the to-review view", () => {
    const out = migrate({ filters: {}, view: "reviewing" }, 1);
    expect(out.view).toBe("to-review");
  });

  it("survives a persisted value with nothing recognisable in it", () => {
    const out = migrate({}, 1);
    expect(out.view).toBe("my-prs");
    expect(out.filtersByView["my-prs"]).toEqual({});
  });

  // Every view must exist as a key, or reading the active one is undefined.
  it("always produces a complete filtersByView", () => {
    const out = migrate({ filters: { staleOnly: true }, view: "list" }, 1);
    // Every view must get an entry, or a v1 store rehydrated into the
    // current shape throws on first access -- the crash #145 shipped.
    //
    // Derived from the CURRENT view list rather than a hardcoded one, so
    // adding a view cannot leave this test passing against a stale
    // expectation. A literal list here would have to be edited by hand
    // every time, which is exactly when someone edits it to match
    // whatever the code now does and stops checking anything.
    expect(Object.keys(out.filtersByView).sort()).toEqual([...ALL_VIEWS].sort());
  });
});

/// #794 promoted Stats from a `panel` value to the `pr-stats` view, so a
/// store written by v2 can hold `panel: "stats"` -- a value no route
/// reads any more. Left alone it is not a crash but a silent loss: the
/// user closed the app on their stats page and reopens on the PR list
/// with nothing to say where it went.
describe("v2 -> v3: the stats panel becomes the PR Stats view", () => {
  const migrate = (useFilters.persist.getOptions().migrate ??
    ((s: unknown) => s)) as (s: unknown, v: number) => Record<string, unknown>;

  it("moves a My PRs store that was showing stats onto the view", () => {
    const out = migrate(
      { filtersByView: { "my-prs": { repo: "octocat/hello-world" } }, view: "my-prs", panel: "stats" },
      2,
    );
    expect(out.view).toBe("pr-stats");
    expect(out.panel).toBe("list");
  });

  // The filters are left exactly where they were. PR Stats keeps the
  // repo sidebar but has its OWN entry, so a repo chosen on My PRs
  // belongs to My PRs -- copying it across would make the stats look
  // pre-filtered by something the user never chose there.
  it("leaves the stored filters untouched", () => {
    const out = migrate(
      { filtersByView: { "my-prs": { repo: "octocat/hello-world" } }, view: "my-prs", panel: "stats" },
      2,
    );
    expect(out.filtersByView).toEqual({ "my-prs": { repo: "octocat/hello-world" } });
  });

  // `panel` is shared with Docker. A Docker user could not reach
  // "stats" through the UI, but a store that somehow holds both must not
  // teleport them off the view they were on -- dropping an unreachable
  // panel value is the smaller correction.
  it("does not move a non-My-PRs view, only clears the dead panel", () => {
    const out = migrate({ filtersByView: {}, view: "docker", panel: "stats" }, 2);
    expect(out.view).toBe("docker");
    expect(out.panel).toBe("list");
  });

  it("leaves a v2 store that was not on stats alone", () => {
    const before = { filtersByView: {}, view: "worktrees", panel: "list" };
    expect(migrate(before, 2)).toEqual(before);
  });

  // The hazard this whole migration exists for. `filtersByView` is
  // REPLACED into the store rather than merged, so a v2 store has no key
  // for a view added later -- and every consumer reads `.repo` off the
  // result. `merge` is what makes that safe; this asserts it covers the
  // new id, because the version that did not took the app down with a
  // black window.
  it("a v2 store rehydrates with an entry for every view, new ones included", () => {
    const merge = useFilters.persist.getOptions().merge!;
    const merged = merge(
      { filtersByView: { "my-prs": { repo: "octocat/hello-world" } }, view: "pr-stats", panel: "list" },
      useFilters.getState(),
    ) as { filtersByView: Record<string, unknown> };
    expect(Object.keys(merged.filtersByView).sort()).toEqual([...ALL_VIEWS].sort());
    expect(merged.filtersByView["pr-stats"]).toEqual({});
  });

  // A v1 store that sat unopened across BOTH changes has to run v1->v2
  // and then v2->v3. A chain of `if (from === n)` arms would apply one
  // and skip the other.
  it("carries a v1 store all the way to v3", () => {
    const out = migrate({ filters: { staleOnly: true }, view: "dashboard" }, 1);
    expect(out.view).toBe("pr-stats");
    expect(out.panel).toBe("list");
    expect(Object.keys(out.filtersByView as object).sort()).toEqual([...ALL_VIEWS].sort());
  });
});
