import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Artifact,
  DockerDiskUsage,
  Footprint,
  FootprintProcess,
  HealthSample,
  Venv,
  WorktreeRepo,
} from "@/types/pr";
import { stubViewport } from "@/test-utils";

/// The per-class detail pages (#687).
///
/// Separate from `SystemHealthPage.test.tsx` because that file's
/// subject is the OVERVIEW, which this issue must leave alone -- and
/// mixing the two would make it easy for a change here to quietly
/// rewrite an assertion there. The overview's own suite is the guard
/// that it stayed put; this one covers what was added beside it.
///
/// # Every process name below is synthetic
///
/// This repository is public and its screenshots go in store listings.
/// A real process list names what a person runs, so the fixtures use
/// invented names (`acme-render`, `widget-daemon`) rather than a
/// capture from any machine. Per `CONTRIBUTING.md`.

const liveFn = vi.hoisted(() => vi.fn<() => Promise<HealthSample>>());
const historyFn = vi.hoisted(() => vi.fn<() => Promise<HealthSample[]>>());
const footprintFn = vi.hoisted(() => vi.fn<() => Promise<Footprint>>());
const disk = vi.hoisted(() => ({
  worktrees: vi.fn<() => Promise<WorktreeRepo[]>>(),
  worktreeSizes: vi.fn<(path: string) => Promise<Map<string, number>>>(),
  artifacts: vi.fn<() => Promise<Artifact[]>>(),
  artifactSizes: vi.fn<() => Promise<Map<string, number>>>(),
  venvs: vi.fn<() => Promise<Venv[]>>(),
  venvSizes: vi.fn<() => Promise<Map<string, number>>>(),
  dockerDisk: vi.fn<() => Promise<DockerDiskUsage>>(),
}));

// Only the hooks this page uses, keeping the real TanStack behaviour so
// `enabled` genuinely decides whether a query function runs -- the same
// reasoning as the overview's suite.
vi.mock("../api/hooks", () => ({
  useSystemHealth: (enabled: boolean) =>
    useQuery({ queryKey: ["system-health"], queryFn: liveFn, enabled, retry: false }),
  useSystemHealthHistory: (enabled: boolean) =>
    useQuery({
      queryKey: ["system-health-history"],
      queryFn: historyFn,
      enabled,
      retry: false,
    }),
  useSystemFootprint: (enabled: boolean) =>
    useQuery({
      queryKey: ["system-footprint"],
      queryFn: footprintFn,
      enabled,
      retry: false,
    }),
  useWorktrees: (enabled = true) =>
    useQuery({ queryKey: ["worktrees"], queryFn: disk.worktrees, enabled, retry: false }),
  useArtifacts: (enabled: boolean) =>
    useQuery({ queryKey: ["artifacts"], queryFn: disk.artifacts, enabled, retry: false }),
  useVenvs: (enabled: boolean) =>
    useQuery({ queryKey: ["venvs"], queryFn: disk.venvs, enabled, retry: false }),
  useAllWorktreeSizes: (paths: string[], enabled: boolean) => {
    const q = useQuery({
      queryKey: ["worktree-sizes", paths.join()],
      queryFn: () => disk.worktreeSizes(paths.join()),
      enabled: enabled && paths.length > 0,
      retry: false,
    });
    return {
      sizes: q.data ?? new Map<string, number>(),
      pending: q.isFetching ? 1 : 0,
      total: paths.length,
    };
  },
  useArtifactSizes: (artifacts: Artifact[], enabled: boolean) => {
    const q = useQuery({
      queryKey: ["artifact-sizes"],
      queryFn: disk.artifactSizes,
      enabled: enabled && artifacts.length > 0,
      retry: false,
    });
    return {
      sizes: q.data ?? new Map<string, number>(),
      ages: new Map<string, number>(),
      pending: q.isFetching ? 1 : 0,
      total: 1,
    };
  },
  useVenvSizes: (venvs: Venv[], enabled: boolean) => {
    const q = useQuery({
      queryKey: ["venv-sizes"],
      queryFn: disk.venvSizes,
      enabled: enabled && venvs.length > 0,
      retry: false,
    });
    return {
      sizes: q.data ?? new Map<string, number>(),
      idle: new Map<string, number>(),
      measuring: q.isFetching,
      pending: q.isFetching ? 1 : 0,
      total: 1,
    };
  },
  useDockerDiskUsage: (enabled: boolean) =>
    useQuery({ queryKey: ["docker-disk"], queryFn: disk.dockerDisk, enabled, retry: false }),
  // The sidebar renders `ViewSwitcher`, which reads the hidden-views
  // preference. Nothing hidden, so the switcher offers every view --
  // this file's subject is the class list beneath it.
  useUiPrefs: () => ({ prefs: { hidden_views: [] } }),
}));

const mobileBuild = vi.hoisted(() => ({ current: false }));
vi.mock("@/lib/target", () => ({
  get IS_MOBILE_BUILD() {
    return mobileBuild.current;
  },
  get IS_DESKTOP_BUILD() {
    return !mobileBuild.current;
  },
}));

vi.mock("@/api/connection", () => ({
  useConnectionState: () => ({ kind: "local" }),
  isStale: () => false,
}));

import { SystemHealthPage } from "./SystemHealthPage";
import { HEALTH_PAGES, SystemHealthSidebar } from "./SystemHealthSidebar";
import { ALL_HEALTH_PAGES, useFilters } from "@/store/filters";

const sample = (over: Partial<HealthSample> = {}): HealthSample => ({
  sampled_at: new Date().toISOString(),
  load: [1.25, 0.94, 0.71],
  cpu_percent: 18,
  cpu_per_core: [10, 26],
  memory: {
    total: 16 * 1024 ** 3,
    used: 8 * 1024 ** 3,
    available: 7 * 1024 ** 3,
    swap_total: 2 * 1024 ** 3,
    swap_used: 512 * 1024 ** 2,
  },
  // Empty: GPU (#705) has its own panel on the overview and no detail
  // page here -- that is the obvious follow-up to this issue, not part
  // of it. Empty rather than absent because that is what an
  // unreadable-GPU platform actually reports.
  gpus: [],
  disks: [
    { mount: "/", total: 500 * 1024 ** 3, available: 200 * 1024 ** 3, is_root: true },
    { mount: "/Volumes/spare", total: 1024 ** 4, available: 900 * 1024 ** 3, is_root: false },
  ],
  battery: { percent: 82, on_ac: false },
  thermal: "nominal",
  networks: [
    { name: "en0", rx_bytes: 8 * 1024 ** 3, tx_bytes: 2 * 1024 ** 3 },
    { name: "lo0", rx_bytes: 1024 ** 2, tx_bytes: 1024 ** 2 },
  ],
  uptime_secs: 3 * 86_400 + 4 * 3600,
  ...over,
});

/// Synthetic processes. Invented names, never a captured list -- this
/// repo is public. See the file header.
const proc = (
  pid: number,
  name: string,
  cpu_percent: number,
  memoryMb: number,
): FootprintProcess => ({ pid, name, cpu_percent, memory: memoryMb * 1024 ** 2 });

const footprint = (over: Partial<Footprint> = {}): Footprint => ({
  sampled_at: new Date().toISOString(),
  app: proc(4242, "headstate", 3, 220),
  children: [proc(5001, "git", 180, 64)],
  docker_daemon: null,
  top_cpu: [
    proc(701, "acme-render", 412, 900),
    proc(702, "widget-daemon", 96, 120),
    proc(4242, "headstate", 3, 220),
  ],
  top_memory: [
    proc(701, "acme-render", 412, 900),
    proc(4242, "headstate", 3, 220),
    proc(703, "cache-keeper", 0, 480),
  ],
  process_count: 1436,
  ...over,
});

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <SystemHealthPage />
    </QueryClientProvider>,
  );
}

function renderSidebar() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <SystemHealthSidebar />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mobileBuild.current = false;
  liveFn.mockResolvedValue(sample());
  historyFn.mockResolvedValue([]);
  footprintFn.mockResolvedValue(footprint());
  disk.worktrees.mockResolvedValue([]);
  disk.artifacts.mockResolvedValue([]);
  disk.venvs.mockResolvedValue([]);
  useFilters.setState({ healthPage: "overview" });
});

afterEach(() => {
  stubViewport(null);
  useFilters.setState({ healthPage: "overview" });
});

describe("the overview is still the landing page", () => {
  it("opens on the overview and not on a detail page", async () => {
    renderPage();
    // The pressure cards are the overview's own, and no detail page
    // renders them: seeing them is proof we did not land elsewhere.
    await screen.findByRole("group", { name: /system pressure at a glance/i });
    expect(
      screen.queryByRole("heading", { level: 1, name: "CPU" }),
    ).toBeNull();
  });

  it("keeps the footprint panel on the overview", async () => {
    // The Disk page folds the footprint measurement in, but the panel
    // #665 asked for stays where it was. "Moved" and "also available"
    // are different changes and the issue asked for the second.
    renderPage();
    await screen.findByText("What Headstate is costing");
  });
});

describe("the CPU page names the processes responsible", () => {
  beforeEach(() => useFilters.setState({ healthPage: "cpu" }));

  it("lists the machine's top processes, biggest first", async () => {
    renderPage();
    await screen.findByText("What is using the CPU");
    const table = await screen.findByRole("table");
    const rows = within(table).getAllByRole("row").slice(1);
    // The order the Rust side chose, preserved: the first row is the
    // answer, and a client that re-sorted could disagree with the
    // count of what was left out.
    expect(rows[0].textContent).toContain("acme-render");
    expect(rows[1].textContent).toContain("widget-daemon");
  });

  it("says how many processes it is NOT showing", async () => {
    renderPage();
    // Eight rows presented as the whole machine is its own lie. The
    // count of the rest is what keeps a bounded list honest.
    await screen.findByText(/of 1436 processes running/i);
    await screen.findByText(/other 1433 are not listed/i);
  });

  it("shows CPU above 100% without clamping it", async () => {
    // `cpu_percent` is a share of ONE core, so a process using four
    // legitimately reads 412%. Clamping would report a busy process as
    // a merely saturated one, and hide the fan-out worth seeing.
    renderPage();
    await screen.findByText("412%");
  });

  it("renders an absent process list as not measured, never as an empty table", async () => {
    // A desktop too old to report this is not a machine with no
    // processes running -- which cannot happen on a booted machine.
    //
    // Reachable, not hypothetical, and it was checked when review
    // questioned it: the phone and desktop ship on independent tags and
    // are compatible on the wire PROTOCOL_VERSION, which adding fields
    // to a response does not bump. The version gate that would refuse
    // an older desktop applies to writes only, and this is a read. See
    // the note on `Footprint.top_cpu` in `types/pr.ts` for the full
    // argument.
    footprintFn.mockResolvedValue(footprint({ top_cpu: undefined }));
    renderPage();
    const panel = (await screen.findByText("What is using the CPU"))
      .closest("section") as HTMLElement;

    // Scoped to THIS panel: a page-wide search for "Not measured" would
    // pass on any other absent reading and prove nothing about this one.
    await waitFor(() =>
      expect(within(panel).getByText(/not measured/i)).toBeTruthy(),
    );
    // And no table at all, rather than a table of nothing. An empty
    // grid under a heading reads as "we looked and found none", which
    // is the opposite of what happened.
    expect(within(panel).queryByRole("table")).toBeNull();
    // The count sentence must be absent too: there is no total to
    // report, and inventing one would be the same class of lie.
    expect(within(panel).queryByText(/processes running/i)).toBeNull();
  });

  it("omits the count sentence when the total is missing but the list is not", async () => {
    // The three fields arrive together in practice, but they are
    // independently optional on the wire, and a list without a total
    // must not have one fabricated for it.
    footprintFn.mockResolvedValue(footprint({ process_count: undefined }));
    renderPage();
    const panel = (await screen.findByText("What is using the CPU"))
      .closest("section") as HTMLElement;
    await waitFor(() => expect(within(panel).getByRole("table")).toBeTruthy());
    expect(within(panel).queryByText(/processes running/i)).toBeNull();
  });

  it("explains that one pinned core hides inside the average", async () => {
    renderPage();
    await screen.findByText(/one core at 100% while the rest are idle is normal/i);
  });
});

describe("the Memory page names what is holding it", () => {
  beforeEach(() => useFilters.setState({ healthPage: "memory" }));

  it("lists the biggest resident sets, in the order Rust chose", async () => {
    renderPage();
    await screen.findByText("What is holding the memory");
    const table = await screen.findByRole("table");
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows[0].textContent).toContain("acme-render");
    // Chosen by MEMORY, so the 0%-CPU cache process is present and the
    // busy-but-small one is not -- proof the page reads `top_memory`
    // rather than re-sorting `top_cpu`, which would have dropped it.
    expect(rows[2].textContent).toContain("cache-keeper");
    expect(within(table).queryByText("widget-daemon")).toBeNull();
  });

  it("warns that resident sets do not add up to the total", async () => {
    renderPage();
    await screen.findByText(/counted once in every process holding it/i);
  });

  it("says a machine with no swap has none rather than 0% used", async () => {
    liveFn.mockResolvedValue(
      sample({
        memory: {
          total: 16 * 1024 ** 3,
          used: 8 * 1024 ** 3,
          available: 7 * 1024 ** 3,
          swap_total: 0,
          swap_used: 0,
        },
      }),
    );
    renderPage();
    await screen.findByText(/no swap configured, so nothing can be paged out/i);
  });
});

describe("the Disk page", () => {
  beforeEach(() => useFilters.setState({ healthPage: "disk" }));

  it("lists every volume and marks the system one", async () => {
    renderPage();
    await screen.findByText("Volumes");
    expect(screen.getByText("/Volumes/spare")).toBeTruthy();
    // The root volume carries the badge and the spare does not: the
    // one the app lives on is what a user filling their disk cares
    // about first, so it is named rather than left to be guessed.
    // `getAllByText` because the panel's own explanation says the word
    // too -- the badge is the first, inside the volume row.
    const badges = screen.getAllByText("system");
    expect(badges[0].className).toContain("rounded");
  });

  it("folds in the footprint measurement, still behind its button", async () => {
    renderPage();
    await screen.findByText("What Headstate is using");
    // Gated exactly as it is on the overview: this is the ~13s
    // `size_worktrees` of #661, and a drill-down must not become the
    // place it runs unasked.
    await screen.findByRole("button", { name: /measure disk use/i });
    expect(disk.worktrees).not.toHaveBeenCalled();
    expect(disk.artifacts).not.toHaveBeenCalled();
    expect(disk.venvs).not.toHaveBeenCalled();
    expect(disk.dockerDisk).not.toHaveBeenCalled();
  });
});

describe("the Network page", () => {
  beforeEach(() => useFilters.setState({ healthPage: "network" }));

  it("puts the busiest interface first", async () => {
    // Whatever order the platform listed them in: the interface that
    // carried the traffic is the one being looked for.
    liveFn.mockResolvedValue(
      sample({
        networks: [
          { name: "lo0", rx_bytes: 1024, tx_bytes: 1024 },
          { name: "en0", rx_bytes: 8 * 1024 ** 3, tx_bytes: 2 * 1024 ** 3 },
        ],
      }),
    );
    renderPage();
    await screen.findByText("Per interface");
    const meters = screen.getAllByRole("meter");
    expect(meters[0].getAttribute("aria-label")).toContain("en0");
  });

  it("says the bars are a share of traffic, not saturation", async () => {
    // Every other bar on this page means "how full". Reusing the shape
    // for a different meaning without saying so is how a reader
    // concludes an interface is 80% saturated.
    renderPage();
    await screen.findByText(/share of all traffic, not how saturated it is/i);
  });
});

describe("the Power page", () => {
  beforeEach(() => useFilters.setState({ healthPage: "power" }));

  it("distinguishes a mains-powered machine from a flat battery", async () => {
    liveFn.mockResolvedValue(sample({ battery: null }));
    renderPage();
    await screen.findByText(/mains-powered, which is not the same as a battery at zero/i);
  });

  it("couples thermal and battery only when both facts are in hand", async () => {
    liveFn.mockResolvedValue(
      sample({ thermal: "serious", battery: { percent: 40, on_ac: false } }),
    );
    renderPage();
    await screen.findByText(/warm and on battery/i);
  });

  it("says nothing about the coupling when the machine is on AC", async () => {
    // An invented warning about a state the machine is not in would be
    // the page's own rule broken.
    liveFn.mockResolvedValue(
      sample({ thermal: "serious", battery: { percent: 40, on_ac: true } }),
    );
    renderPage();
    await screen.findByText("Thermal pressure");
    expect(screen.queryByText(/warm and on battery/i)).toBeNull();
  });
});

describe("the page list stays complete", () => {
  /// Every page in the union is offered, and nothing is offered that
  /// is not in it.
  ///
  /// The same guard `SettingsDialog.test.tsx` puts on `ALL_VIEWS`, and
  /// for the same reason: `HEALTH_PAGES` is a hand-written array beside
  /// a union, so adding a page to the union without adding it here
  /// gives you a page reachable by no control -- and the sidebar looks
  /// complete either way, which is why nobody would notice. Derived
  /// from the union rather than written out, because a second literal
  /// list is one that gets edited to match and stops checking anything.
  it("offers exactly the pages the union declares", () => {
    expect(HEALTH_PAGES.map((p) => p.id).sort()).toEqual([...ALL_HEALTH_PAGES].sort());
  });

  it("renders a body for every page, not a fallback", async () => {
    // The detail shell's final branch is an `else`, so a page added to
    // the union and to HEALTH_PAGES but not to the switch would render
    // the Power page under someone else's heading -- wrong content
    // under a correct title, which is the hardest kind to spot.
    //
    // Each page's own first panel title, which no other page uses.
    const expected: Record<string, RegExp> = {
      cpu: /what is using the cpu/i,
      memory: /what is holding the memory/i,
      disk: /^volumes$/i,
      network: /^since boot$/i,
      power: /^battery$/i,
    };
    for (const page of ALL_HEALTH_PAGES) {
      if (page === "overview") continue;
      useFilters.setState({ healthPage: page });
      const { unmount } = renderPage();
      // Awaited: the live sample has to land before any page has a
      // body, so a synchronous assertion would fail on all five.
      await screen.findByText(expected[page]);
      unmount();
    }
  });
});

describe("navigation", () => {
  it("moves between pages from the sidebar", async () => {
    renderSidebar();
    fireEvent.click(screen.getByRole("button", { name: "Memory" }));
    expect(useFilters.getState().healthPage).toBe("memory");
    // Marked as a LOCATION rather than a pressed toggle: these are
    // navigation, and "pressed" describes a control that did something.
    expect(
      screen.getByRole("button", { name: "Memory" }).getAttribute("aria-current"),
    ).toBe("page");
  });

  it("returns to the overview when the view changes", async () => {
    // Coming back to System Health lands on the landing page. A
    // drill-down answered a question that is now behind you.
    useFilters.setState({ healthPage: "cpu" });
    useFilters.getState().setView("worktrees");
    expect(useFilters.getState().healthPage).toBe("overview");
  });

  it("is not persisted, so a relaunch opens on the overview", () => {
    // The store's `partialize` decides what survives a relaunch.
    // Persisting the drill-down would skip the page that says whether
    // there is anything worth drilling into today.
    useFilters.setState({ healthPage: "cpu" });
    const stored = JSON.parse(
      window.localStorage.getItem("headstate-filters") ?? "{}",
    ) as { state?: Record<string, unknown> };
    expect(stored.state ?? {}).not.toHaveProperty("healthPage");
  });
});

describe("navigation at phone width", () => {
  // NOTE: without `stubViewport` these render the DESKTOP layout
  // whatever the describe block is called -- jsdom has no `matchMedia`.
  beforeEach(() => stubViewport(400));

  it("puts the class list on the overview, since there is no sidebar", async () => {
    renderPage();
    const nav = await screen.findByRole("navigation", { name: /system health sections/i });
    // Every class the sidebar offers except the one you are on.
    for (const label of ["CPU", "Memory", "Disk", "Network", "Power and uptime"]) {
      expect(within(nav).getByRole("button", { name: new RegExp(label) })).toBeTruthy();
    }
    expect(within(nav).queryByRole("button", { name: /^Overview/ })).toBeNull();
  });

  it("opens a detail page from a card and comes back", async () => {
    renderPage();
    const nav = await screen.findByRole("navigation", { name: /system health sections/i });
    fireEvent.click(within(nav).getByRole("button", { name: /CPU/ }));
    await screen.findByText("What is using the CPU");

    // The back control is the ONLY way off a detail page on a phone,
    // which is why it is asserted rather than assumed.
    fireEvent.click(screen.getByRole("button", { name: /system health overview/i }));
    await screen.findByRole("group", { name: /system pressure at a glance/i });
  });

  it("shows no class cards on the desktop, where the sidebar has them", async () => {
    stubViewport(1400);
    renderPage();
    await screen.findByRole("group", { name: /system pressure at a glance/i });
    expect(
      screen.queryByRole("navigation", { name: /system health sections/i }),
    ).toBeNull();
  });

  it("shows no back control on the desktop, where the sidebar is persistent", async () => {
    stubViewport(1400);
    useFilters.setState({ healthPage: "cpu" });
    renderPage();
    await screen.findByText("What is using the CPU");
    expect(
      screen.queryByRole("button", { name: /system health overview/i }),
    ).toBeNull();
  });
});
