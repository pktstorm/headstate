import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionState } from "./api/connection";
import { PR_FIXTURES } from "./fixtures/prs";
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
  useTruncation: () => null,
  useIncomplete: () => 0,
  useReviewShortfall: () => 0,
  useViewCadence: () => undefined,
  usePollState: () => "idle",
  useRemovalProgress: () => null,
  useUpdateRunOutcome: () => undefined,
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
vi.mock("./api/connection", () => ({
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
  "claude-md": {},
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
    connection.current = { kind: "connected", desktop: "octocat's laptop", lastPoll: null, protocolVersion: 1 };
    renderApp();
    const nav = screen.getByRole("navigation");
    expect(within(nav).getByText("All repositories")).toBeTruthy();
    expect(within(nav).getByText("Stats")).toBeTruthy();
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
      protocolVersion: 1,
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

  it("offers no Stats entry", async () => {
    renderApp();
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    await waitFor(() => expect(screen.getByRole("navigation")).toBeTruthy());
    expect(within(screen.getByRole("navigation")).queryByText("Stats")).toBeNull();
  });

  it("shows the list rather than Stats even when Stats was the last panel", () => {
    // The panel persists across launches, and a desktop that closed on
    // Stats must not open a phone on a page the phone does not have.
    useFilters.setState({ panel: "stats" } as never);
    renderApp();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Pull requests");
    expect(screen.getByText(/^\d+ Open$/)).toBeTruthy();
  });

  it("renders the connection banner above everything", () => {
    renderApp();
    const banner = screen.getByRole("button", { name: /octocat's laptop/ });
    expect(banner.textContent).toContain("reachable");
    expect(banner.textContent).toContain("last poll 3 minutes ago");
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
