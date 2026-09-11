import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionState } from "./api/connection";
import { PR_FIXTURES } from "./fixtures/prs";
import { REQUIRED_PROTOCOL_VERSION } from "./lib/protocol";
import { useFilters } from "./store/filters";
import { stubViewport } from "./test-utils";

// The shell talks to Tauri on mount. Stub the command surface so these
// tests exercise the layout, not the backend -- the same set App.test
// uses, since the same components mount.
vi.mock("./api/hooks", () => ({
  useUiPrefs: () => ({
    prefs: { hidden_views: [], close_hides_to_tray: true },
    set: () => Promise.resolve(),
  }),
  useCleanupPrefs: () => ({ prefs: undefined, set: () => Promise.resolve() }),
  useAutostart: () => ({ enabled: false, set: () => Promise.resolve() }),
  useRemoteEnabled: () => ({ enabled: false, set: () => Promise.resolve() }),
  useActOnPr: () => () => Promise.resolve(),
  useUpdatePrBranch: () => () => Promise.resolve(),
  useActOnPrs: () => () => Promise.resolve([]),
  useSetAutoMerge: () => () => Promise.resolve(),
  usePullRequests: () => ({ data: PR_FIXTURES, isSuccess: true, isLoading: false }),
  usePollError: () => null,
  useRefreshRequested: () => undefined,
  useRefreshFromGesture: () => () => Promise.resolve(),
  useTruncation: () => null,
  useIncomplete: () => 0,
  useReviewShortfall: () => 0,
  useViewCadence: () => undefined,
  usePollState: () => "idle",
  useRemovalProgress: () => null,
  useUpdateProgress: () => null,
  useCancelUpdateRun: () => () => Promise.resolve(),
  useUpdateRunOutcome: () => undefined,
  useUpdateRunResume: () => undefined,
  usePollInterval: () => ({ seconds: 120, set: () => Promise.resolve(120) }),
  useWorktreeDirs: () => ({ dirs: [], set: () => Promise.resolve([]) }),
  useNotifyPrefs: () => ({
    prefs: { enabled: true, ci_failed: true, conflicted: true },
    set: () => Promise.resolve(),
  }),
  useReviewing: () => ({ data: [], isLoading: false }),
  useReviewingCount: () => ({ data: 0 }),
  usePrDetail: () => ({ data: undefined, isLoading: true, isError: false, refetch: () => {} }),
  useDeleteHeadBranch: () => () => Promise.resolve(),
  useReviewPr: () => () => Promise.resolve(),
  useCommentOnPr: () => () => Promise.resolve(),
  useRerunChecks: () => () => Promise.resolve(),
  useViewer: () => ({ data: undefined }),
  useCycleTrend: () => ({ data: undefined }),
  usePeriods: () => ({
    data: {
      week_current: 5,
      week_previous: 3,
      opened_week_current: 6,
      opened_week_previous: 4,
      month_current: 20,
      month_previous: 18,
    },
    isLoading: false,
    isError: false,
    refetch: () => {},
  }),
  useHistory: () => ({
    data: {
      points: [],
      week_current: 0,
      week_previous: 0,
      opened_week_current: 0,
      opened_week_previous: 0,
      month_current: 0,
      month_previous: 0,
    },
    isLoading: false,
  }),
  useMergedDetail: () => ({ data: undefined, isLoading: false }),
}));

vi.mock("./components/AuthGate", () => ({
  AuthGate: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

const connection = vi.hoisted(() => ({ current: { kind: "local" } as ConnectionState }));
// Spread the real module rather than replacing it: only the hook needs
// to be driven from the test, and `isStale` -- which `StaleRibbon` and
// `useWritesPaused` both read -- should be the real logic under test,
// not a stub that could disagree with it.
vi.mock("./api/connection", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  useConnectionState: () => connection.current,
}));

const { default: App } = await import("./App");

function renderApp() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <App />
    </QueryClientProvider>,
  );
}

const EMPTY = {
  "my-prs": {},
  "to-review": {},
  worktrees: {},
  branches: {},
  docker: {},
  artifacts: {},
  packages: {},
  "claude-md": {}, "pr-stats": {}, "system-health": {},
};

beforeEach(() => {
  useFilters.setState({ filtersByView: EMPTY, view: "my-prs", panel: "list" } as never);
});

afterEach(() => {
  cleanup();
  stubViewport(null);
  connection.current = { kind: "local" };
});

describe("App shell on the desktop", () => {
  it("renders the repo sidebar inline, with no menu button and no banner", () => {
    stubViewport(1400);
    connection.current = {
      kind: "connected",
      desktop: "octocat's laptop",
      lastPoll: null,
      protocolVersion: REQUIRED_PROTOCOL_VERSION,
      stale: false,
    };
    renderApp();
    const nav = screen.getByRole("navigation");
    expect(within(nav).getByText("All repositories")).toBeTruthy();
    // "PR Stats" is inside the view switcher's menu now, not pinned to
    // the column (#794), so it is not in the DOM until the menu opens.
    // What this test is about is the SHELL -- inline sidebar, no menu
    // button, no banner -- so it checks the switcher is present and
    // leaves what the menu holds to `ViewSwitcher`'s own tests.
    expect(within(nav).getByRole("button", { name: /my pull requests/i })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /open navigation/i })).toBeNull();
    expect(screen.queryByText(/octocat's laptop/)).toBeNull();
  });
});

describe("App shell on a phone", () => {
  beforeEach(() => {
    stubViewport(390);
    connection.current = {
      kind: "connected",
      desktop: "octocat's laptop",
      lastPoll: new Date(Date.now() - 3 * 60_000).toISOString(),
      protocolVersion: REQUIRED_PROTOCOL_VERSION,
      stale: false,
    };
  });

  it("puts the repo sidebar behind a menu button", async () => {
    renderApp();
    // Not inline: the list gets the whole width.
    expect(screen.queryByRole("navigation")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    await waitFor(() => expect(screen.getByRole("navigation")).toBeTruthy());
    expect(within(screen.getByRole("navigation")).getByText("All repositories")).toBeTruthy();
  });

  it("closes the sheet once a repo is picked", async () => {
    renderApp();
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    await waitFor(() => expect(screen.getByRole("navigation")).toBeTruthy());
    fireEvent.click(within(screen.getByRole("navigation")).getByText("octocat/hello-world"));
    await waitFor(() => expect(screen.queryByRole("navigation")).toBeNull());
    expect(useFilters.getState().filtersByView["my-prs"].repo).toBe("octocat/hello-world");
  });

  /// Exactly one element carries the status-bar inset.
  ///
  /// The banner is above the header and already has `pt-safe`; the
  /// header briefly had it too, and the inset applied twice left a
  /// notch-height gap between them -- about a tenth of an iPhone screen.
  /// Asserted on the class rather than by measuring, because jsdom
  /// resolves `env()` to nothing and would pass either way.
  it("does not pay the safe-area inset twice", () => {
    renderApp();
    const banner = screen.getByRole("button", { name: /octocat's laptop/ });
    const header = screen.getByRole("heading", { level: 1 }).closest("header");
    expect(banner.className).toContain("pt-safe");
    // The header sits below the banner, so it must NOT add the inset.
    expect(header?.className ?? "").not.toContain("safe");
  });

  it("renders the connection banner above everything", () => {
    renderApp();
    const banner = screen.getByRole("button", { name: /octocat's laptop/ });
    expect(banner.textContent).toContain("reachable");
    // The desktop's own poll time moved out of this line: while the
    // desktop is reachable, GitHub's freshness is the useful timestamp
    // and the bottom bar no longer repeats it (#649). This test is
    // about the banner's POSITION, asserted below.
    expect(banner.textContent).not.toContain("last poll");
    // Above the header, not inside it or below the list.
    const header = screen.getByRole("heading", { level: 1 });
    expect(banner.compareDocumentPosition(header) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("still renders the pull requests as rows", () => {
    renderApp();
    // `getAll`: a blocked pull request is also named in the priorities
    // strip above the list.
    for (const pr of PR_FIXTURES) expect(screen.getAllByText(pr.title).length).toBeGreaterThan(0);
  });
});

/// Stats is absent from the COMPANION, not from narrow windows.
///
/// These were phone-viewport tests, which meant a desktop user who
/// dragged their window under 768px lost a page their build genuinely
/// has -- and `RepoSidebar` hid the entry to reach it by, on the same
/// wrong condition.
describe("Stats on the companion build", () => {
  async function renderMobileApp() {
    vi.stubEnv("VITE_TARGET", "mobile");
    vi.resetModules();
    const { default: MobileApp } = await import("./App");
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    return render(
      <QueryClientProvider client={qc}>
        <MobileApp />
      </QueryClientProvider>,
    );
  }

  afterEach(() => {
    vi.unstubAllEnvs();
  });

  /// The deliberate mobile classification from #794: PR Stats stays off
  /// the phone. It was never offered there, and promoting it from `panel`
  /// to `View` is a move between desktop surfaces, not a decision to ship
  /// it. `MOBILE_HIDDEN_VIEWS` is where that is recorded; this asserts
  /// the switcher honours it.
  it("offers no PR Stats entry in the view switcher", async () => {
    stubViewport(390);
    await renderMobileApp();
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    await waitFor(() => expect(screen.getByRole("navigation")).toBeTruthy());
    const nav = within(screen.getByRole("navigation"));
    // Open the switcher: the entry must be absent from the MENU, not
    // merely absent from a collapsed control that lists one view.
    fireEvent.click(nav.getByRole("button", { name: /my pull requests/i }));
    expect(nav.queryByText("PR Stats")).toBeNull();
    // A view the phone DOES have, so this proves the menu rendered.
    expect(nav.getByRole("menuitem", { name: /worktrees/i })).toBeTruthy();
  });

  it("shows the list rather than PR Stats even when that was the stored view", async () => {
    // `view` persists across launches exactly as `panel` did, so a
    // desktop that closed on PR Stats must not open a companion on a page
    // it does not have -- and with the entry hidden there would be no
    // switcher row to leave it by.
    useFilters.setState({ view: "pr-stats" } as never);
    stubViewport(390);
    await renderMobileApp();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Pull requests");
    expect(screen.getByText(/^\d+ Open$/)).toBeTruthy();
  });

  /// The stored value is NOT corrected, only declined. The phone and the
  /// desktop can share a persisted store, so writing "my-prs" back would
  /// silently move the desktop off PR Stats too.
  it("leaves the stored view alone rather than rewriting it", async () => {
    useFilters.setState({ view: "pr-stats" } as never);
    stubViewport(390);
    await renderMobileApp();
    expect(useFilters.getState().view).toBe("pr-stats");
  });

  /// The consequence of declining rather than correcting: two components
  /// route on `view`, and both must apply the same fallback. `App` renders
  /// the PR list; if `ViewSwitcher` read the raw stored value its collapsed
  /// button would say "PR Stats" above that list, naming a page not on
  /// screen.
  it("names the page it is actually showing in the switcher", async () => {
    useFilters.setState({ view: "pr-stats" } as never);
    stubViewport(390);
    await renderMobileApp();
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    await waitFor(() => expect(screen.getByRole("navigation")).toBeTruthy());
    const nav = within(screen.getByRole("navigation"));
    expect(nav.getByRole("button", { name: /my pull requests/i })).toBeTruthy();
    expect(nav.queryByText("PR Stats")).toBeNull();
  });

  it("keeps PR Stats on a narrow DESKTOP window", async () => {
    // The case the viewport guard got wrong (#598): a desktop dragged
    // under 768px still has every page a desktop has.
    vi.stubEnv("VITE_TARGET", "desktop");
    vi.resetModules();
    const { default: DesktopApp } = await import("./App");
    stubViewport(390);
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <DesktopApp />
      </QueryClientProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    await waitFor(() => expect(screen.getByRole("navigation")).toBeTruthy());
    const nav = within(screen.getByRole("navigation"));
    fireEvent.click(nav.getByRole("button", { name: /my pull requests/i }));
    expect(nav.getByRole("menuitem", { name: /pr stats/i })).toBeTruthy();
  });
});

/// The whole screen a phone opens on away from its desktop (#684).
///
/// The gates above `App` are the other half of that fix and are pinned in
/// `AuthGate.offline.test.tsx`; this asserts that what they let through is
/// worth having. `usePullRequests` is mocked to the fixtures because that
/// is what the companion genuinely serves off-network: `get_cached` is the
/// one read it answers from its stored snapshot
/// (`src-mobile/src/companion.rs`), so an unreachable desktop still
/// produces a real list.
describe("the phone's shell with the desktop unreachable", () => {
  beforeEach(() => {
    stubViewport(390);
    connection.current = {
      kind: "unreachable",
      desktop: "octocat's laptop",
      lastPoll: new Date(Date.now() - 42 * 60_000).toISOString(),
      stale: true,
    };
  });

  it("still renders the cached pull requests", () => {
    // "Show what it can." A saved copy is worth more than a wall, and
    // the two markers below are what make showing it honest.
    renderApp();
    for (const pr of PR_FIXTURES) expect(screen.getAllByText(pr.title).length).toBeGreaterThan(0);
  });

  it("marks the rows as a saved copy rather than presenting them as live", () => {
    // #602's rule: cached data is MARKED, never hidden and never passed
    // off as current. `StaleRibbon` is attached to the content, which is
    // the thing being doubted.
    renderApp();
    const ribbon = screen.getByRole("status");
    expect(ribbon.textContent).toContain("Showing a saved copy");
    expect(ribbon.textContent).toContain("octocat's laptop");
  });

  it("puts the desktop's status in the banner, in one line", () => {
    // The right home for this fact, and the issue says so: one line
    // naming the desktop and when it was last seen -- not a screen.
    renderApp();
    const banner = screen.getByRole("button", { name: /octocat's laptop is unreachable/ });
    expect(banner.textContent).toContain("last seen");
  });

  it("covers nothing with a full-screen error", () => {
    // The report's other half. An unreachable desktop is the ordinary
    // state of a phone, so nothing here may read as a failure of the
    // app: no crash screen, no auth accusation, no impossible advice.
    renderApp();
    expect(screen.queryByText(/something went wrong/i)).toBeNull();
    expect(screen.queryByText(/not signed in to GitHub/i)).toBeNull();
    expect(screen.queryByText(/brew install/i)).toBeNull();
    // And the app is genuinely there underneath, not merely uncovered.
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Pull requests");
  });
});
