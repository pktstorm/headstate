import { cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithQuery as render } from "@/test-utils";
import { useFilters } from "@/store/filters";
import type { StatsTree } from "@/types/pr";

const invoke = vi.hoisted(() => vi.fn<(...a: unknown[]) => Promise<unknown>>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

import { relativeDay, REPO_PREVIEW, StatsSidebar } from "./StatsSidebar";

/// Shaped from the REAL measured hierarchy rather than a uniform invention:
/// two organisations of very different sizes, one of them past
/// `REPO_PREVIEW`, with archived repositories in the tail. The live account
/// this feature was built against has 2 orgs (4 members / 49 repos and 4 /
/// 10), 6 personal repositories, and 2 archived repos whose last pushes
/// were 2026-04 and 2023-03 -- so the fixture keeps those proportions. A
/// uniform fixture would hide exactly the two things that shaped the
/// design: that one org is long enough to need capping and that the
/// archived rows sit at the quiet end of the order.
function tree(over: Partial<StatsTree> = {}): StatsTree {
  const repos = (owner: string, n: number, archivedTail = 0) =>
    Array.from({ length: n }, (_, i) => ({
      nameWithOwner: `${owner}/repo-${i}`,
      // Descending, the way GitHub returns them: row 0 is the most
      // recently pushed. Days apart so `relativeDay` produces distinct
      // labels and a reordering would be visible.
      pushedAt: new Date(Date.UTC(2026, 8, 11) - i * 86_400_000).toISOString(),
      isArchived: i >= n - archivedTail,
    }));
  return {
    viewer: "octocat",
    orgs: [
      {
        login: "acme",
        name: "Acme Corp",
        repos: repos("acme", 20, 2),
        reposTotal: 20,
        members: [
          { login: "octocat", name: "Mona Octocat", avatarUrl: null },
          { login: "hubot", name: null, avatarUrl: null },
        ],
        membersTotal: 2,
        readable: true,
      },
      {
        login: "initech",
        name: "Initech",
        repos: repos("initech", 3),
        reposTotal: 3,
        members: [{ login: "peter", name: null, avatarUrl: null }],
        membersTotal: 1,
        readable: true,
      },
    ],
    orgsTotal: 2,
    personal: repos("octocat", 2),
    personalTotal: 2,
    refusedFields: 0,
    spend: {
      points: 2,
      requests: 2,
      unmetered: 0,
      remaining: 4900,
      resetAt: null,
    },
    ...over,
  };
}

/// One repo row, found by the `owner/name` in its `title`.
///
/// NOT by accessible name, and the reason is worth recording because it cost
/// an hour of chasing a bug that was not there. A row's accessible name is
/// its label and its detail CONCATENATED with no separator -- `repo-2` with
/// a "2d" recency badge is the single string `"repo-22d"`. So
/// `/repo-2\b/` does not match it (the boundary falls between `2` and `2`),
/// and `/repo-1/` matches both `repo-1` and `repo-19`. Every anchoring
/// scheme over that string is either ambiguous or wrong for some row.
///
/// The `title` is the full `owner/name` and is exactly what the scope is
/// keyed on, so matching it is both unambiguous and a check that the row
/// carries the identity the click will write.
function repoRow(owner: string, n: number) {
  return screen.getByTitle(`${owner}/repo-${n}`);
}

const statsFilters = () => useFilters.getState().filtersByView["pr-stats"];

beforeEach(() => {
  // `cleanup` BEFORE each test, not only after. `screen` queries the whole
  // document, so a container left mounted by the previous test is still
  // visible to `getByRole` -- and `renderTree`'s wait for an "acme" button
  // was satisfied by the PREVIOUS test's DOM while the current render was
  // still pending. The tests then asserted against an empty sidebar and
  // failed in a full run while passing in isolation, which is the most
  // misleading shape a test failure has.
  cleanup();
  invoke.mockReset();
  useFilters.getState().reset();
  // The sidebar only enumerates on its own view, which is what `enabled`
  // gates -- so every test has to be ON that view or the tree never loads.
  useFilters.setState({ view: "pr-stats" });
});

afterEach(() => cleanup());

/// Resolve `stats_tree` and wait for the first org row to exist.
async function renderTree(t: StatsTree = tree()) {
  invoke.mockImplementation((cmd) =>
    cmd === "stats_tree" ? Promise.resolve(t) : Promise.resolve(null),
  );
  const r = render(<StatsSidebar />);
  // Wait for the pending line to GO, rather than for a row name to appear:
  // a name can be matched by a container another test left mounted, where
  // the disappearance of this render's own placeholder cannot.
  await waitFor(() =>
    expect(screen.queryByText(/Finding your organizations/)).toBeNull(),
  );
  return r;
}

/// Expand one org's disclosure.
function expand(name: string) {
  fireEvent.click(screen.getByRole("button", { name: new RegExp(name) }));
}

describe("StatsSidebar: the hierarchy", () => {
  /// The shape #825 specifies, in order. Asserted as a sequence rather than
  /// by presence, because the headings are what make the tree readable and
  /// a correct set of rows in the wrong nesting is a different sidebar.
  it("renders Organizations, then each org, then Personal", async () => {
    await renderTree();
    const text = document.body.textContent ?? "";
    expect(text.indexOf("Organizations")).toBeGreaterThanOrEqual(0);
    expect(text.indexOf("Organizations")).toBeLessThan(text.indexOf("acme"));
    expect(text.indexOf("acme")).toBeLessThan(text.indexOf("initech"));
    expect(text.indexOf("initech")).toBeLessThan(text.indexOf("Personal"));
  });

  /// Orgs are COLLAPSED on arrival, which is the list-length decision.
  /// With 20 + 3 repositories plus members, expanding everything would put
  /// the second org's Members below the fold -- and Members is what makes
  /// #823's team audience reachable at all.
  it("collapses orgs so the whole tree fits on arrival", async () => {
    await renderTree();
    expect(screen.queryByText("Repos")).toBeNull();
    expect(screen.queryByText("Members")).toBeNull();
    expand("acme");
    expect(screen.getByText("Repos")).toBeTruthy();
    expect(screen.getByText("Members")).toBeTruthy();
  });

  /// Expanding shows Repos (with its own All repos) then Members.
  it("holds Repos and Members under an expanded org", async () => {
    await renderTree();
    expand("acme");
    const text = document.body.textContent ?? "";
    expect(text.indexOf("Repos")).toBeLessThan(text.indexOf("Members"));
    // "All repos" exists under BOTH the org and Personal, so there are two.
    expect(screen.getAllByRole("button", { name: /All repos/ }).length).toBe(2);
  });

  /// The heading is a disclosure, NOT a scope. A control that both reveals
  /// a list and loads an expensive org-wide scope misfires every time a
  /// user meant only to look inside.
  it("expanding an org selects nothing", async () => {
    await renderTree();
    expand("acme");
    expect(statsFilters().statsScopeKind).toBeUndefined();
  });
});

describe("StatsSidebar: selection writes a scope", () => {
  /// `All repos` under an org is the ORG scope -- requirement 5's distinct
  /// scope, not a sum of the rows.
  it("All repos under an org selects the org scope", async () => {
    await renderTree();
    expand("acme");
    fireEvent.click(screen.getAllByRole("button", { name: /All repos/ })[0]);
    expect(statsFilters().statsScopeKind).toBe("org");
    expect(statsFilters().statsScopeValue).toBe("acme");
    // No subject: an org scope asks about the org, not about one person.
    expect(statsFilters().statsSubject).toBeUndefined();
  });

  it("a repo row selects the repo scope, keyed owner/name", async () => {
    await renderTree();
    expand("acme");
    fireEvent.click(repoRow("acme", 0));
    expect(statsFilters().statsScopeKind).toBe("repo");
    // The full `owner/name`, not the shortened label the row displays --
    // this is what `Scope::Repo` consumes.
    expect(statsFilters().statsScopeValue).toBe("acme/repo-0");
  });

  /// A member row is the feature's headline capability: it opens the stats
  /// page scoped to that PERSON, and it keeps the org scope because a
  /// person's activity is only meaningful somewhere.
  it("a member row selects that person within the org", async () => {
    await renderTree();
    expand("acme");
    fireEvent.click(screen.getByRole("button", { name: /Mona Octocat/ }));
    expect(statsFilters().statsSubject).toBe("octocat");
    expect(statsFilters().statsScopeKind).toBe("org");
    expect(statsFilters().statsScopeValue).toBe("acme");
  });

  /// Clicking a repository AFTER a person must clear the person.
  ///
  /// The bug this guards would be invisible: the page would render a
  /// repository's heading over one colleague's numbers, and nothing on
  /// screen would say the subject was still set.
  it("selecting a repo clears a previously selected person", async () => {
    await renderTree();
    expand("acme");
    fireEvent.click(screen.getByRole("button", { name: /Mona Octocat/ }));
    expect(statsFilters().statsSubject).toBe("octocat");
    fireEvent.click(repoRow("acme", 1));
    expect(statsFilters().statsSubject).toBeUndefined();
    expect(statsFilters().statsScopeKind).toBe("repo");
  });

  /// Personal's All repos is the `user` scope, spelled with the viewer's
  /// login -- `Scope::Personal` is `user:<login>`, and an unqualified
  /// search would be a much larger question.
  it("Personal All repos selects the user scope for the viewer", async () => {
    await renderTree();
    fireEvent.click(screen.getAllByRole("button", { name: /All repos/ })[0]);
    expect(statsFilters().statsScopeKind).toBe("user");
    expect(statsFilters().statsScopeValue).toBe("octocat");
  });

  /// The selection lands in PR STATS' own filter set, which is the #794
  /// arrangement this feature inherits and the reason `pr-stats` has an
  /// entry in `EMPTY_FILTERS` (#825 requirement 6).
  it("writes to pr-stats' own filter set and nobody else's", async () => {
    await renderTree();
    expand("acme");
    fireEvent.click(repoRow("acme", 0));
    expect(statsFilters().statsScopeValue).toBe("acme/repo-0");
    expect(
      useFilters.getState().filtersByView["my-prs"].statsScopeValue,
    ).toBeUndefined();
  });

  it("highlights the selected row", async () => {
    await renderTree();
    expand("acme");
    fireEvent.click(repoRow("acme", 0));
    await waitFor(() =>
      expect(repoRow("acme", 0).className).toContain("bg-[#1f6feb]"),
    );
    expect(repoRow("acme", 0).getAttribute("aria-current")).toBe("true");
  });
});

describe("StatsSidebar: honest degradation (#825 requirement 2)", () => {
  /// THE requirement, and the #769 lesson. An org the token cannot read
  /// must SAY so -- an empty Members list reads as "this org has no
  /// members", which is silence read as success.
  it("an unreadable org says so instead of rendering an empty Members list", async () => {
    const t = tree();
    t.orgs[0] = {
      ...t.orgs[0],
      repos: [],
      members: [],
      readable: false,
      reposTotal: 49,
      membersTotal: 4,
    };
    await renderTree(t);
    expand("acme");

    expect(screen.getByText(/Could not read this organization/)).toBeTruthy();
    // The counts from the orgs query survive, so the row says how big the
    // org is while admitting it could not be read.
    expect(screen.getByText(/49 repositories/)).toBeTruthy();
    expect(screen.getByText(/4.*members/)).toBeTruthy();
    // And it must NOT render the empty-but-readable wording, which is the
    // sentence that would be a lie here.
    expect(screen.queryByText(/No members listed/)).toBeNull();
    // The actionable fix is named: SSO authorisation is not guessable.
    expect(
      screen.getByText(/single sign-on|organization read access/),
    ).toBeTruthy();
  });

  /// Marked on the COLLAPSED row too, or the honest degradation is hidden
  /// behind a disclosure nobody opens.
  it("marks an unreadable org before it is expanded", async () => {
    const t = tree();
    t.orgs[0] = { ...t.orgs[0], repos: [], members: [], readable: false };
    await renderTree(t);
    expect(screen.getByLabelText("could not be read")).toBeTruthy();
  });

  /// Readable-and-empty is a DIFFERENT sentence. This is the pair that
  /// makes `readable` worth carrying: both have zero members, and only the
  /// flag tells them apart.
  it("a readable org with no members says that, not the refusal", async () => {
    const t = tree();
    t.orgs[0] = { ...t.orgs[0], members: [], membersTotal: 0, readable: true };
    await renderTree(t);
    expand("acme");
    expect(screen.getByText(/No members listed/)).toBeTruthy();
    expect(screen.queryByText(/Could not read this organization/)).toBeNull();
  });

  /// Pending and failed must not both render as absence, which is the same
  /// class of bug one level up.
  it("says it is loading rather than showing an empty tree", () => {
    invoke.mockImplementation(() => new Promise(() => {}));
    render(<StatsSidebar />);
    expect(screen.getByText(/Finding your organizations/)).toBeTruthy();
  });

  it("reports a failed enumeration instead of an empty tree", async () => {
    invoke.mockImplementation(() =>
      Promise.reject(new Error("bad credentials")),
    );
    render(<StatsSidebar />);
    await waitFor(() =>
      expect(
        screen.getByText(/Could not list your organizations/),
      ).toBeTruthy(),
    );
    expect(screen.getByText(/bad credentials/)).toBeTruthy();
  });

  /// A refusal that was not a whole org is surfaced rather than swallowed:
  /// the alternative is a quietly short list.
  it("reports refused fields", async () => {
    await renderTree(tree({ refusedFields: 3 }));
    expect(screen.getByText(/GitHub refused 3 fields/)).toBeTruthy();
  });
});

describe("StatsSidebar: truncation is never silent", () => {
  /// #824 item 8 applied to a row list. `totalCount` stays truthful above
  /// the 100-node page (MEASURED: 100 returned, totalCount 559), so a
  /// shorter list must say so -- expanding cannot reveal these.
  it("says when a repo list is shorter than GitHub's own count", async () => {
    const t = tree();
    t.orgs[0] = { ...t.orgs[0], reposTotal: 559 };
    await renderTree(t);
    expand("acme");
    expect(screen.getByText(/Showing 20 of 559 repositories/)).toBeTruthy();
  });

  it("says when a member list is truncated", async () => {
    const t = tree();
    t.orgs[0] = { ...t.orgs[0], membersTotal: 224 };
    await renderTree(t);
    expand("acme");
    expect(screen.getByText(/Showing 2 of 224 members/)).toBeTruthy();
  });

  it("says when the org list itself is truncated", async () => {
    await renderTree(tree({ orgsTotal: 70 }));
    expect(screen.getByText(/Showing 2 of 70 organizations/)).toBeTruthy();
  });

  /// The rendered cap is an EXPANSION, not a truncation: the count is
  /// stated and every row is reachable. This is the difference between
  /// capping a list and hiding part of one.
  it("caps the rendered rows but states the count and can expand", async () => {
    await renderTree();
    expand("acme");
    expect(screen.queryByTitle("acme/repo-19")).toBeNull();
    expect(repoRow("acme", 0)).toBeTruthy();
    // 20 repos, previewed at REPO_PREVIEW.
    expect(screen.getByText(`Show all 20`)).toBeTruthy();
    fireEvent.click(screen.getByText(`Show all 20`));
    expect(repoRow("acme", 19)).toBeTruthy();
    expect(screen.queryByText(/Show all/)).toBeNull();
  });

  it("shows no expansion control for a list inside the cap", async () => {
    await renderTree();
    expand("initech");
    // 3 repos, well under the preview.
    expect(repoRow("initech", 2)).toBeTruthy();
    expect(screen.queryByText(/Show all/)).toBeNull();
  });

  /// The cap must be a cap on the TAIL, which is only meaningful because
  /// the order is most-recently-active: the hidden rows are the quiet end.
  it("previews the most recently pushed rows, not an arbitrary slice", async () => {
    await renderTree();
    expand("acme");
    for (let i = 0; i < REPO_PREVIEW; i++) {
      expect(repoRow("acme", i)).toBeTruthy();
    }
    expect(screen.queryByTitle("acme/repo-12")).toBeNull();
  });
});

describe("StatsSidebar: archived repos", () => {
  /// The decision #825 asked to be made deliberately: archived repos are
  /// LISTED but marked, so a lead can still read a finished project's
  /// history, and excluded from aggregates.
  it("lists archived repos, marked", async () => {
    await renderTree();
    expand("acme");
    fireEvent.click(screen.getByText("Show all 20"));
    // The fixture's last two are archived.
    const archived = repoRow("acme", 19);
    expect(archived.textContent).toContain("archived");
    // Still clickable: the whole point of listing them.
    fireEvent.click(archived);
    expect(statsFilters().statsScopeValue).toBe("acme/repo-19");
  });

  /// `All repos` is annotated so the org total and the rows do not appear
  /// to disagree: the archived rows are inside the org scope's total but
  /// out of any per-repository aggregate.
  it("marks the org total as including archived repos", async () => {
    await renderTree();
    expand("acme");
    expect(
      screen.getAllByRole("button", { name: /All repos/ })[0].textContent,
    ).toContain("incl. archived");
  });

  it("does not claim archived repos when there are none", async () => {
    await renderTree();
    expand("initech");
    const all = screen
      .getAllByRole("button", { name: /All repos/ })
      .find((b) => b.textContent?.includes("3"));
    expect(all?.textContent).not.toContain("incl. archived");
  });
});

describe("StatsSidebar: no local git, and no statistics", () => {
  /// Requirement 3: the rows come from GitHub, never from a worktree or a
  /// remote. Asserted on the COMMANDS invoked, which is the only place a
  /// local-git dependency could actually enter.
  it("calls only stats_tree -- no worktree or repo scan", async () => {
    await renderTree();
    const called = invoke.mock.calls.map((c) => c[0]);
    expect(called).toContain("stats_tree");
    for (const forbidden of [
      "list_worktrees",
      "classify_worktrees",
      "list_branches",
      "scan_artifacts",
      "get_cached",
    ]) {
      expect(called).not.toContain(forbidden);
    }
  });

  /// Requirement 1 / the discovery-cheap rule: entering the view must not
  /// load statistics. A `stats_count` here would mean the sidebar spends
  /// per row rather than per click.
  it("loads no statistics until something is clicked", async () => {
    await renderTree();
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("stats_count");
    expand("acme");
    fireEvent.click(repoRow("acme", 0));
    // Still nothing: a click writes a SCOPE. #826 renders the numbers.
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("stats_count");
  });

  /// `enabled` threads the explicit-load rule from the caller. On another
  /// view the tree must not be fetched at all.
  it("enumerates nothing when the view is not PR Stats", () => {
    useFilters.setState({ view: "my-prs" });
    invoke.mockImplementation(() => Promise.resolve(tree()));
    render(<StatsSidebar />);
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("stats_tree");
  });
});

describe("StatsSidebar: a repo with no recent activity is still listed", () => {
  /// Requirement 4, which is what the old `repoCounts(prs)` column could
  /// not do: it listed only repositories with an OPEN PR. A three-year-dead
  /// repository is a legitimate scope, and its absence was unfixable there.
  it("lists a long-quiet repo and lets it be selected", async () => {
    const t = tree();
    t.orgs[1] = {
      ...t.orgs[1],
      repos: [
        {
          nameWithOwner: "initech/ancient",
          pushedAt: "2022-11-23T13:44:27Z",
          isArchived: false,
        },
      ],
      reposTotal: 1,
    };
    await renderTree(t);
    expand("initech");
    const row = screen.getByRole("button", { name: /ancient/ });
    expect(row).toBeTruthy();
    // And it says how stale it is, which is what makes the ordering legible.
    expect(row.textContent).toMatch(/\dy/);
    fireEvent.click(row);
    expect(statsFilters().statsScopeValue).toBe("initech/ancient");
  });

  /// A member row shows the LOGIN even when a display name exists: the
  /// stats are keyed on `author:<login>`, and a board of display names
  /// alone cannot be checked against GitHub's own UI.
  it("shows a member's login alongside their display name", async () => {
    await renderTree();
    expand("acme");
    const row = screen.getByRole("button", { name: /Mona Octocat/ });
    expect(row.textContent).toContain("Mona Octocat");
    expect(row.textContent).toContain("octocat");
    // A member with no display name falls back to the login alone.
    expect(screen.getByRole("button", { name: /hubot/ })).toBeTruthy();
  });
});

describe("relativeDay", () => {
  const now = Date.UTC(2026, 8, 11);

  /// The recency label is what makes "most-recently-active" legible. Each
  /// unit boundary is asserted because a wrong divisor would render
  /// plausible-looking nonsense -- "400d" rather than "1y".
  it("reports the smallest sensible unit", () => {
    expect(relativeDay(new Date(now).toISOString(), now)).toBe("today");
    expect(relativeDay(new Date(now - 3 * 86_400_000).toISOString(), now)).toBe(
      "3d",
    );
    expect(
      relativeDay(new Date(now - 29 * 86_400_000).toISOString(), now),
    ).toBe("29d");
    expect(
      relativeDay(new Date(now - 60 * 86_400_000).toISOString(), now),
    ).toBe("2mo");
    expect(
      relativeDay(new Date(now - 400 * 86_400_000).toISOString(), now),
    ).toBe("1y");
  });

  /// An absent timestamp must not be fabricated. A repository never pushed
  /// to genuinely has no value, and rendering the epoch would claim 1970
  /// for a repository created this morning.
  it("returns nothing for an absent or unparseable timestamp", () => {
    expect(relativeDay(null, now)).toBeUndefined();
    expect(relativeDay("not a date", now)).toBeUndefined();
  });

  /// Clock skew must not render "-1d".
  it("clamps a future timestamp to today", () => {
    expect(relativeDay(new Date(now + 5000).toISOString(), now)).toBe("today");
  });
});
