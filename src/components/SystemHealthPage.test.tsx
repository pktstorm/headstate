import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Artifact,
  DockerDiskUsage,
  Footprint,
  HealthSample,
  Venv,
  WorktreeRepo,
} from "@/types/pr";
import { stubViewport } from "@/test-utils";

const liveFn = vi.hoisted(() => vi.fn<() => Promise<HealthSample>>());
const historyFn = vi.hoisted(() => vi.fn<() => Promise<HealthSample[]>>());
const footprintFn = vi.hoisted(() => vi.fn<() => Promise<Footprint>>());

/// Every source the DISK half can reach, as one spy each.
///
/// Separate spies rather than one, because the assertion that matters
/// is per-command: "nothing sized on mount" has to be provable about
/// each of the four, and a single counter would pass while three of
/// them fired. The DISCOVERY calls are spied too -- `scan_artifacts`
/// and `scan_venvs` are seconds in their own right, so a panel that
/// deferred only the sizing would still have paid most of the cost on
/// view open.
const disk = vi.hoisted(() => ({
  worktrees: vi.fn<() => Promise<WorktreeRepo[]>>(),
  worktreeSizes: vi.fn<(path: string) => Promise<Map<string, number>>>(),
  artifacts: vi.fn<() => Promise<Artifact[]>>(),
  artifactSizes: vi.fn<() => Promise<Map<string, number>>>(),
  venvs: vi.fn<() => Promise<Venv[]>>(),
  venvSizes: vi.fn<() => Promise<Map<string, number>>>(),
  dockerDisk: vi.fn<() => Promise<DockerDiskUsage>>(),
}));

// The hooks this page uses, not the whole `api/hooks` module's
// transitive world: that file imports every command the app has, and
// mocking it wholesale would tie this test to all of them. The
// stand-ins keep the real TanStack behaviour so loading and error
// states are the genuine ones -- and, for the disk hooks, so that
// `enabled` genuinely decides whether the query function runs. That is
// the whole subject of the tests below: a stand-in that ignored
// `enabled` would make them pass while the real thing walked 147
// worktrees on view open.
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
  // The three size hooks return the aggregate shapes their real
  // counterparts do -- a map plus progress counters -- rather than a
  // raw query, because that is the contract the panel reads.
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
    useQuery({
      queryKey: ["docker-disk"],
      queryFn: disk.dockerDisk,
      enabled,
      retry: false,
    }),
}));

// The build target, as a mock: `IS_MOBILE_BUILD` is read at module
// scope, so re-importing the component to flip it would lose every
// other mock in this file. Same shape as `WorktreesPage.test.tsx`.
const mobileBuild = vi.hoisted(() => ({ current: false }));
vi.mock("@/lib/target", () => ({
  get IS_MOBILE_BUILD() {
    return mobileBuild.current;
  },
  get IS_DESKTOP_BUILD() {
    return !mobileBuild.current;
  },
}));

// The connection state drives the desktop's NAME and the unreachable
// wording. Defaults to `local`, which is what the desktop build always
// has, so every pre-existing test in this file keeps its old meaning.
const connection = vi.hoisted(() => ({
  current: { kind: "local" } as { kind: string; desktop?: string; lastPoll?: string | null },
}));
vi.mock("@/api/connection", () => ({
  useConnectionState: () => connection.current,
  isStale: () => false,
}));

import { SystemHealthPage } from "./SystemHealthPage";
// The pure logic lives in `lib/health` rather than in the component, so
// the gap detection -- the single most important piece of correctness
// here -- is testable without rendering anything.
import {
  SAMPLE_INTERVAL_MS,
  barColor,
  formatUptime,
  medianSpacing,
  splitOnGaps,
  thermalColor,
  type Point,
} from "@/lib/health";

/// A fully populated sample, so each test overrides only the one field
/// it is about. Everything present by default means a test asserting an
/// ABSENT reading has to say so explicitly, which is the point.
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
  gpus: [
    {
      name: "Apple M2 Max",
      utilization_percent: 7,
      memory_used: 1.5 * 1024 ** 3,
      memory_total: 10 * 1024 ** 3,
      unified_memory: true,
    },
  ],
  disks: [
    { mount: "/", total: 500 * 1024 ** 3, available: 100 * 1024 ** 3, is_root: true },
  ],
  // Charge 82, capacity 84 -- deliberately different numbers, so a
  // panel rendering one in place of the other is visible.
  battery: { percent: 82, on_ac: false, capacity_percent: 84, cycle_count: 413 },
  thermal: "nominal",
  networks: [{ name: "en0", rx_bytes: 1024 ** 3, tx_bytes: 512 * 1024 ** 2 }],
  uptime_secs: 3 * 86_400 + 4 * 3600,
  ...over,
});

/// A footprint with all three groups present, so a test asserting an
/// ABSENT process has to say so explicitly -- the same discipline as
/// `sample` above, and for the same reason: absence is the interesting
/// case here and must never be the accidental default.
const footprint = (over: Partial<Footprint> = {}): Footprint => ({
  sampled_at: new Date().toISOString(),
  app: { pid: 4242, name: "headstate", cpu_percent: 3, memory: 220 * 1024 ** 2 },
  children: [
    { pid: 5001, name: "git", cpu_percent: 180, memory: 64 * 1024 ** 2 },
    { pid: 5002, name: "gh", cpu_percent: 4, memory: 32 * 1024 ** 2 },
  ],
  docker_daemon: {
    pid: 900,
    name: "com.docker.backend",
    cpu_percent: 1,
    memory: 4 * 1024 ** 3,
  },
  ...over,
});

/// A series at the sampler's own cadence, ending now.
function series(values: (number | null)[], stepMs = SAMPLE_INTERVAL_MS): Point[] {
  const end = Date.now();
  return values.map((v, i) => ({
    t: end - (values.length - 1 - i) * stepMs,
    v,
  }));
}

const show = () => {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <SystemHealthPage />
    </QueryClientProvider>,
  );
};

describe("splitOnGaps", () => {
  /// The requirement the whole view rests on. A laptop closed overnight
  /// leaves two points twelve hours apart, and a line drawn between
  /// them asserts a value nobody measured.
  it("cuts the series where the app was not running", () => {
    const step = SAMPLE_INTERVAL_MS;
    const end = Date.now();
    const points: Point[] = [
      { t: end - 10 * 3600_000 - 2 * step, v: 10 },
      { t: end - 10 * 3600_000 - step, v: 12 },
      // Ten hours of nothing: the machine was asleep.
      { t: end - step, v: 30 },
      { t: end, v: 32 },
    ];
    const runs = splitOnGaps(points);
    expect(runs).toHaveLength(2);
    expect(runs[0].map((p) => p.v)).toEqual([10, 12]);
    expect(runs[1].map((p) => p.v)).toEqual([30, 32]);
  });

  /// The other half of the same requirement: an unbroken run must NOT
  /// be chopped up. A gap detector that fires on ordinary spacing would
  /// turn every chart into confetti and be just as untrustworthy.
  it("leaves an evenly sampled series in one piece", () => {
    const runs = splitOnGaps(series([1, 2, 3, 4, 5, 6]));
    expect(runs).toHaveLength(1);
    expect(runs[0]).toHaveLength(6);
  });

  /// Downsampling returns every Nth row, so adjacent points in a dense
  /// series are legitimately N minutes apart. That is the resolution,
  /// not a gap, and the threshold is derived from the series' own
  /// spacing precisely so this case stays whole.
  it("does not mistake the downsampled stride for a gap", () => {
    // Twelve minutes apart -- what a full 24 hours downsamples to.
    const runs = splitOnGaps(series([1, 2, 3, 4, 5], 12 * SAMPLE_INTERVAL_MS));
    expect(runs).toHaveLength(1);
  });

  /// A null VALUE is the same kind of unknown as a closed app: the
  /// platform reported nothing for that moment. Interpolating across it
  /// would be the same lie in a different costume.
  it("breaks the line at a value that was never measured", () => {
    const runs = splitOnGaps(series([1, 2, null, 4, 5]));
    expect(runs).toHaveLength(2);
    expect(runs[0].map((p) => p.v)).toEqual([1, 2]);
    expect(runs[1].map((p) => p.v)).toEqual([4, 5]);
  });

  /// One measurement between two closures is real and is kept. Dropping
  /// it would under-report what the app actually saw.
  it("keeps a run of a single point", () => {
    const end = Date.now();
    const runs = splitOnGaps([
      { t: end - 6 * 3600_000, v: 40 },
      { t: end, v: 50 },
    ]);
    expect(runs).toHaveLength(2);
    expect(runs[0]).toHaveLength(1);
  });

  /// The blind spot in a purely relative threshold: two points six
  /// hours apart have a MEDIAN spacing of six hours, so nothing in the
  /// series exceeds `median * factor` and the pair would join into a
  /// line across a period nobody measured. The absolute ceiling is what
  /// catches it.
  it("cuts a wide pair even though the gap is its own median spacing", () => {
    const end = Date.now();
    const runs = splitOnGaps([
      { t: end - 6 * 3600_000, v: 40 },
      { t: end, v: 50 },
    ]);
    expect(runs).toHaveLength(2);
  });

  it("returns nothing for an empty series", () => {
    expect(splitOnGaps([])).toEqual([]);
  });
});

describe("medianSpacing", () => {
  /// The median, not the mean: the mean of a series containing a
  /// six-hour hole is dragged up BY that hole, raising the threshold
  /// above the very gap being looked for and hiding it.
  it("is not dragged upward by the gap it is meant to find", () => {
    const end = Date.now();
    const points: Point[] = [];
    for (let i = 20; i > 0; i -= 1) {
      points.push({ t: end - 8 * 3600_000 - i * SAMPLE_INTERVAL_MS, v: 1 });
    }
    points.push({ t: end, v: 1 });
    expect(medianSpacing(points)).toBe(SAMPLE_INTERVAL_MS);
  });

  it("never falls below the sampler's own cadence", () => {
    const end = Date.now();
    expect(
      medianSpacing([
        { t: end - 1000, v: 1 },
        { t: end, v: 1 },
      ]),
    ).toBe(SAMPLE_INTERVAL_MS);
  });
});

describe("formatUptime", () => {
  it("reads as days, hours and minutes", () => {
    expect(formatUptime(3 * 86_400 + 4 * 3600)).toBe("3d 4h");
    expect(formatUptime(5 * 3600 + 30 * 60)).toBe("5h 30m");
    expect(formatUptime(90)).toBe("1m");
  });
});

describe("barColor", () => {
  /// Thresholds are high on purpose: a machine at 70% memory is a
  /// machine being used. Colouring that amber teaches people to ignore
  /// the colour, which costs the red band its meaning.
  it("stays green through ordinary use and escalates only near full", () => {
    expect(barColor(10)).toBe("#3fb950");
    expect(barColor(70)).toBe("#3fb950");
    expect(barColor(80)).toBe("#d29922");
    expect(barColor(95)).toBe("#f85149");
  });
});

describe("thermalColor", () => {
  /// `nominal` is grey, not green: a machine at rest is the ordinary
  /// case, and green would read as an achievement.
  it("reserves colour for the states that need attention", () => {
    expect(thermalColor("nominal")).toBe("#8b949e");
    expect(thermalColor("fair")).toBe("#58a6ff");
    expect(thermalColor("serious")).toBe("#d29922");
    expect(thermalColor("critical")).toBe("#f85149");
  });
});

/// Everything the disk half could reach, primed to resolve.
///
/// Primed deliberately, not left rejecting: a test that proves nothing
/// fired is only worth something if the calls WOULD have succeeded had
/// they been made. A spy that throws would pass the same assertion for
/// the wrong reason.
function primeDisk() {
  disk.worktrees.mockResolvedValue([
    { identity: "pktstorm/headstate", name: "headstate", path: "/code/hs", worktrees: [] },
  ]);
  disk.worktreeSizes.mockResolvedValue(new Map([["/code/hs/wt", 3 * 1024 ** 3]]));
  disk.artifacts.mockResolvedValue([
    {
      path: "/code/hs/target",
      kind: "cargo-target",
      repo_path: "/code/hs",
      size_bytes: null,
      idle_secs: null,
    } as unknown as Artifact,
  ]);
  disk.artifactSizes.mockResolvedValue(new Map([["/code/hs/target", 5 * 1024 ** 3]]));
  disk.venvs.mockResolvedValue([
    { path: "/venvs/a", project: "a", source: null, size_bytes: null, idle_secs: null } as
      unknown as Venv,
  ]);
  disk.venvSizes.mockResolvedValue(new Map([["/venvs/a", 700 * 1024 ** 2]]));
  disk.dockerDisk.mockResolvedValue({
    images_bytes: 2 * 1024 ** 3,
    images_reclaimable_bytes: 0,
    build_cache_bytes: 1024 ** 3,
    volumes_bytes: 0,
    volumes_reclaimable_bytes: 0,
  });
}

/// The footprint panel itself, so row assertions do not collect the
/// network table's rows from the panel above it.
const footprintPanel = () =>
  screen.getByText("What Headstate is costing").closest("section") as HTMLElement;

/// Every spy the disk half can reach, for the "nothing fired" assertion.
const diskSpies = () => [
  disk.worktrees,
  disk.worktreeSizes,
  disk.artifacts,
  disk.artifactSizes,
  disk.venvs,
  disk.venvSizes,
  disk.dockerDisk,
];

describe("SystemHealthPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    liveFn.mockResolvedValue(sample());
    historyFn.mockResolvedValue([]);
    footprintFn.mockResolvedValue(footprint());
    primeDisk();
    // Back to the desktop build on a local machine, so a mobile test
    // that flips these cannot leak into the one after it.
    mobileBuild.current = false;
    connection.current = { kind: "local" };
  });

  it("shows the live readings once they arrive", async () => {
    show();
    expect(await screen.findByText("1.25")).toBeTruthy();
    expect(screen.getByText("0.94")).toBeTruthy();
    expect(screen.getByText("0.71")).toBeTruthy();
    // `getAllBy`: the CPU figure is deliberately in two places now --
    // the pressure row above and the CPU panel here (#683). Both
    // render the same sample, so asserting it appears is still the
    // point; asserting it appears exactly once never was.
    expect(screen.getAllByText("18%").length).toBeGreaterThan(0);
    expect(screen.getByText("3d 4h")).toBeTruthy();
  });

  /// "We have not looked yet" and "we looked and there is nothing" are
  /// opposite answers. Panels full of dashes on first paint would say
  /// the second while the first is true.
  it("says it is still reading before the first sample lands", () => {
    liveFn.mockReturnValue(new Promise(() => {}));
    show();
    expect(screen.getByText(/Reading this machine's health/)).toBeTruthy();
  });

  /// THE absent-value rule. A desktop with no battery reports null, and
  /// "0%" there would say the machine is about to die.
  it("renders a machine with no battery as not measured, never as zero", async () => {
    liveFn.mockResolvedValue(sample({ battery: null }));
    show();
    await screen.findByText("1.25");
    expect(screen.queryByText("0%")).toBeNull();
    expect(screen.getAllByText(/Not measured/).length).toBeGreaterThan(0);
    expect(screen.getByText(/No battery on this machine/)).toBeTruthy();
  });


  /// #773's second half: the rate on the OVERVIEW, not only behind a
  /// click.
  ///
  /// The issue is explicit about why. #720's alert fires, the user
  /// opens this page, and the number that answers "why did I get that
  /// alert" should be on the panel they land on.
  it("shows the drain rate on the overview, not only on the detail page", async () => {
    liveFn.mockResolvedValue(
      sample({
        battery: {
          percent: 64,
          on_ac: false,
          capacity_percent: 84,
          cycle_count: 413,
          power: { watts: -18.4, milliamps: -1454, millivolts: 12654 },
        },
      }),
    );
    show();
    await screen.findByText("-18.4 W");
    expect(screen.getByText("Out of the battery")).toBeTruthy();
  });

  /// The #720 condition, said in words on the overview panel too --
  /// colour is never the only cue on this page.
  it("says on the overview when a plugged-in machine is losing charge", async () => {
    liveFn.mockResolvedValue(
      sample({
        battery: {
          percent: 64,
          on_ac: true,
          capacity_percent: 84,
          cycle_count: 413,
          power: { watts: -6.2, milliamps: -490, millivolts: 12654 },
        },
      }),
    );
    show();
    await screen.findByText(/losing charge while plugged in/i);
  });

  /// Absent is not zero, in the field where it is hardest to see: a
  /// full battery on mains genuinely draws 0 W, so a fabricated zero
  /// here is indistinguishable from a measurement.
  it("reports an unpublished rate as not measured, never as zero watts", async () => {
    liveFn.mockResolvedValue(
      sample({
        battery: {
          percent: 64,
          on_ac: true,
          capacity_percent: 84,
          cycle_count: 413,
          power: null,
        },
      }),
    );
    show();
    await screen.findByText("64%");
    expect(screen.queryByText("0.0 W")).toBeNull();
    expect(screen.getByText(/This platform does not publish it/)).toBeTruthy();
  });

  /// A desktop's absent rate is not a separate fact from its absent
  /// battery, and two "Not measured" rows for one absence is noise.
  it("does not add a rate row for a machine with no battery at all", async () => {
    liveFn.mockResolvedValue(sample({ battery: null }));
    show();
    await screen.findByText(/No battery on this machine/);
    expect(screen.queryByText("Rate")).toBeNull();
  });

  /// Windows reports no load average at all. Three silent dashes would
  /// look like a bug in Headstate rather than a platform limit.
  it("explains a platform with no load average instead of showing zeros", async () => {
    liveFn.mockResolvedValue(sample({ load: null, cpu_percent: null }));
    show();
    expect(
      await screen.findByText(/does not report load averages/),
    ).toBeTruthy();
    expect(screen.queryByText("0.00")).toBeNull();
  });

  /// A machine with swap turned off reports a zero total. That is "there
  /// is no swap", not "0% of swap is in use".
  it("distinguishes no swap from empty swap", async () => {
    liveFn.mockResolvedValue(
      sample({
        memory: { ...sample().memory, swap_total: 0, swap_used: 0 },
      }),
    );
    show();
    expect(await screen.findByText(/No swap configured/)).toBeTruthy();
  });

  it("reports an absent thermal reading rather than inventing one", async () => {
    liveFn.mockResolvedValue(sample({ thermal: null }));
    show();
    await screen.findByText("1.25");
    expect(screen.getAllByText(/Not measured/).length).toBeGreaterThan(0);
  });

  /// The label invites exactly one wrong reading -- that it is a
  /// temperature. The copy beside it has to say it is not, and say why
  /// degrees are unavailable.
  it("says the thermal label is a pressure rating and not a temperature", async () => {
    show();
    await screen.findByText("1.25");
    expect(screen.getByText("nominal")).toBeTruthy();
    const note = screen.getByText(/not a\s+temperature/i);
    expect(note.textContent).toMatch(/elevated privileges/i);
    // And it must never present the label in degrees.
    expect(screen.queryByText(/°/)).toBeNull();
  });

  it("names every mounted volume with what is free of what", async () => {
    show();
    await screen.findByText("1.25");
    expect(screen.getByText("/")).toBeTruthy();
    // Also in the Disk pressure card (#683), same sample, so the
    // question is whether the panel says it -- not whether the page
    // says it once.
    expect(screen.getAllByText(/100 GB free of 500 GB/).length).toBeGreaterThan(0);
    // The volume the app lives on is called out rather than left to be
    // guessed from the mount point.
    expect(screen.getByText("system")).toBeTruthy();
  });

  it("lists interfaces as totals since boot, not as speeds", async () => {
    show();
    await screen.findByText("1.25");
    expect(screen.getByText("en0")).toBeTruthy();
    expect(screen.getByText(/Totals since the machine booted/)).toBeTruthy();
  });

  /// The chart itself must show the break, not merely be capable of it.
  /// `data-runs` is the count of separately drawn runs, so two means the
  /// line really was cut rather than smoothed over.
  it("draws a history with a hole in it as two separate runs", async () => {
    const end = Date.now();
    const at = (msAgo: number) => new Date(end - msAgo).toISOString();
    historyFn.mockResolvedValue([
      sample({ sampled_at: at(10 * 3600_000 + 2 * 60_000), cpu_percent: 10 }),
      sample({ sampled_at: at(10 * 3600_000 + 60_000), cpu_percent: 12 }),
      // Ten hours with the app closed.
      sample({ sampled_at: at(60_000), cpu_percent: 30 }),
      sample({ sampled_at: at(0), cpu_percent: 32 }),
    ]);
    show();
    const chart = await screen.findByTestId("sparkline-CPU");
    expect(chart.getAttribute("data-runs")).toBe("2");
    // Two polylines, and crucially no single line joining all four
    // points across the hole.
    expect(chart.querySelectorAll("polyline")).toHaveLength(2);
  });

  /// The counterpart: continuous data must draw as one line, or the
  /// gap rendering would be noise rather than signal.
  it("draws a continuous history as one unbroken line", async () => {
    const end = Date.now();
    historyFn.mockResolvedValue(
      [0, 1, 2, 3, 4].map((i) =>
        sample({
          sampled_at: new Date(end - (4 - i) * 60_000).toISOString(),
          cpu_percent: 20 + i,
        }),
      ),
    );
    show();
    const chart = await screen.findByTestId("sparkline-CPU");
    expect(chart.getAttribute("data-runs")).toBe("1");
    expect(chart.querySelectorAll("polyline")).toHaveLength(1);
  });

  /// An empty series is "we have not collected anything yet", which is
  /// not the same as a flat line at zero -- and a flat line at zero is
  /// exactly what a naive chart would draw.
  it("says there is no history yet rather than drawing a flat zero", async () => {
    show();
    await screen.findByText("1.25");
    const charts = screen.getAllByLabelText(/no history yet/);
    expect(charts.length).toBeGreaterThan(0);
    expect(document.querySelectorAll("polyline")).toHaveLength(0);
  });

  /// A history that fails must not take the live panels with it: the
  /// current numbers are half of what people open this view for.
  it("keeps the live readings when only the history fails", async () => {
    historyFn.mockRejectedValue(new Error("db locked"));
    show();
    expect(await screen.findByText("1.25")).toBeTruthy();
    await waitFor(() =>
      expect(screen.getByText(/24-hour history could not be loaded/)).toBeTruthy(),
    );
  });

  it("reports a failure to read the machine at all", async () => {
    liveFn.mockRejectedValue(new Error("collector unavailable"));
    show();
    expect(
      await screen.findByText(/Could not read this machine's health/),
    ).toBeTruthy();
  });

  /// Per-core bars are meters, so their value is readable without
  /// inferring it from a pixel width.
  it("renders one meter per core at that core's value", async () => {
    show();
    await screen.findByText("1.25");
    const core0 = screen.getByLabelText("Core 0");
    const core1 = screen.getByLabelText("Core 1");
    expect(core0.getAttribute("aria-valuenow")).toBe("10");
    expect(core1.getAttribute("aria-valuenow")).toBe("26");
  });

  /// The GPU's utilization is a meter, like every other bar here, so
  /// its value is readable without inferring it from a pixel width.
  it("renders the GPU's utilization and the memory it holds", async () => {
    show();
    await screen.findByText("1.25");
    expect(screen.getByText("Apple M2 Max")).toBeTruthy();
    const bar = screen.getByLabelText("Apple M2 Max utilization");
    expect(bar.getAttribute("aria-valuenow")).toBe("7");
  });

  /// **The unified-memory rule (#686).** The Memory panel reports the
  /// same physical pool, so without this sentence a reader would add
  /// the GPU's gigabytes to the system's and conclude the machine has
  /// more RAM than it does -- the two panels would look like they
  /// disagreed about the size of the machine.
  it("says the GPU shares one pool with the system, not a second one", async () => {
    show();
    await screen.findByText("1.25");
    expect(screen.getByText(/unified memory/)).toBeTruthy();
    expect(screen.getByText(/should not be added to it/)).toBeTruthy();
  });

  /// A discrete GPU has its own VRAM, so the unified-memory note would
  /// be false there and must not appear.
  it("does not claim unified memory for a card with its own VRAM", async () => {
    liveFn.mockResolvedValue(
      sample({
        gpus: [
          {
            name: "card0",
            utilization_percent: 42,
            memory_used: 2 * 1024 ** 3,
            memory_total: 8 * 1024 ** 3,
            unified_memory: false,
          },
        ],
      }),
    );
    show();
    await screen.findByText("1.25");
    expect(screen.getByText("card0")).toBeTruthy();
    expect(screen.queryByText(/unified memory/)).toBeNull();
    expect(screen.getByText("Total VRAM")).toBeTruthy();
  });

  /// **No panel at all, rather than a panel of absences.**
  ///
  /// This is the rule #686 states explicitly, and it is where this
  /// page's usual "Not measured" would be the WRONG answer: a GPU
  /// panel full of dashes claims a GPU was found and could not be
  /// read. On Windows, and on Intel/NVIDIA Linux, nothing was found
  /// because there is no unprivileged way to look.
  it("renders no GPU panel when the platform reported none", async () => {
    liveFn.mockResolvedValue(sample({ gpus: [] }));
    show();
    await screen.findByText("1.25");
    expect(screen.queryByText("GPU")).toBeNull();
    expect(screen.queryByText("Utilization")).toBeNull();
  });

  /// A GPU that reported utilization but no memory figures shows the
  /// one it has and says nothing about the ones it does not -- never a
  /// confident 0 bytes.
  it("keeps a GPU's unreported memory absent rather than zero", async () => {
    liveFn.mockResolvedValue(
      sample({
        gpus: [
          {
            name: "card0",
            utilization_percent: 55,
            memory_used: null,
            memory_total: null,
            unified_memory: false,
          },
        ],
      }),
    );
    show();
    await screen.findByText("1.25");
    expect(screen.getByLabelText("card0 utilization").getAttribute("aria-valuenow")).toBe(
      "55",
    );
    expect(screen.queryByText("0 B")).toBeNull();
    expect(screen.getAllByText(/Not measured/).length).toBeGreaterThan(0);
  });

  /// Two GPUs are two cards, not one picked arbitrarily -- an Intel Mac
  /// with integrated and discrete graphics reports both.
  it("renders every GPU the machine reported", async () => {
    liveFn.mockResolvedValue(
      sample({
        gpus: [
          {
            name: "Intel Iris",
            utilization_percent: 3,
            memory_used: null,
            memory_total: null,
            unified_memory: false,
          },
          {
            name: "Radeon Pro",
            utilization_percent: 61,
            memory_used: null,
            memory_total: null,
            unified_memory: false,
          },
        ],
      }),
    );
    show();
    await screen.findByText("1.25");
    expect(screen.getByText("Intel Iris")).toBeTruthy();
    expect(screen.getByText("Radeon Pro")).toBeTruthy();
  });

  /// A machine that reported no per-core figures gets the absent
  /// treatment too -- not a row of empty bars, which would read as a
  /// perfectly idle CPU.
  it("says so when no per-core figures were reported", async () => {
    liveFn.mockResolvedValue(sample({ cpu_per_core: [] }));
    show();
    await screen.findByText("1.25");
    expect(screen.queryByLabelText("Core 0")).toBeNull();
    expect(screen.getAllByText(/Not measured/).length).toBeGreaterThan(0);
  });

  /// "Available" is not "total minus used" on any modern platform, and
  /// a reader who assumes it is will conclude one of the numbers is
  /// wrong. The hint is what stops that.
  it("explains that available memory includes reclaimable cache", async () => {
    show();
    await screen.findByText("1.25");
    expect(screen.getByText(/Reclaimable, incl\. cache/)).toBeTruthy();
  });

  /// Each chart carries the explanation beside it. An explanation two
  /// panels away is one nobody reads at the moment they need it.
  ///
  /// Three charts on this fixture: CPU, memory, and the GPU (#686),
  /// which the default sample reports.
  it("explains the breaks beside each chart", async () => {
    show();
    await screen.findByText("1.25");
    const notes = screen.getAllByText(/Headstate was not running/);
    expect(notes.length).toBe(3);
  });

  /// The GPU chart goes away with the GPU panel, so a machine with no
  /// discoverable GPU is back to two.
  it("drops the GPU chart along with its panel", async () => {
    liveFn.mockResolvedValue(sample({ gpus: [] }));
    show();
    await screen.findByText("1.25");
    expect(screen.getAllByText(/Headstate was not running/).length).toBe(2);
  });
});

describe("the view's place in the app", () => {
  /// The list is the single source: the migration's completeness test
  /// and the settings toggles both read from it, so a view missing here
  /// is a view that crashes on rehydrate.
  it("is one of the app's views", async () => {
    const { ALL_VIEWS } = await import("../store/filters");
    expect([...ALL_VIEWS]).toContain("system-health");
  });

  /// It describes the machine, so it has no repository axis. A picker
  /// beside it would be a control that changes nothing -- or, on a
  /// machine with no scanned checkouts, an empty list under a heading,
  /// which reads as a page that failed to load.
  it("gets a sidebar with no repository picker", async () => {
    const { useFilters } = await import("../store/filters");
    useFilters.getState().setView("system-health");
    expect(useFilters.getState().filtersByView["system-health"]).toEqual({});
    expect(useFilters.getState().filtersByView["system-health"].repo).toBeUndefined();
  });
});

describe("the panels are laid out for a desktop", () => {
  /// jsdom has no `matchMedia`, so `useIsMobile` reports false and this
  /// renders the desktop layout -- which is the one this issue is
  /// about. Asserted rather than assumed, so a future change to the
  /// viewport shim does not silently retarget every test above.
  it("renders without a viewport shim, on the desktop path", async () => {
    liveFn.mockResolvedValue(sample());
    historyFn.mockResolvedValue([]);
    expect(typeof window.matchMedia).not.toBe("function");
    const { container } = show();
    await screen.findByText("1.25");
    const grid = container.querySelector(".md\\:grid-cols-2");
    expect(grid).not.toBeNull();
    // Seven panel headings -- the original six plus GPU (#686), which
    // the default sample reports -- plus the "Disk" subheading inside
    // the footprint panel. Counted rather than asserted loosely because
    // a panel silently disappearing from the layout is exactly the kind
    // of regression this catches -- #665 shipped once with its whole
    // panel missing.
    const headings = within(grid as HTMLElement).getAllByRole("heading");
    expect(headings.filter((h) => h.tagName === "H2").length).toBe(7);
    expect(headings.map((h) => h.textContent)).toContain(
      "What Headstate is costing",
    );
    expect(headings.map((h) => h.textContent)).toContain("GPU");
  });

  /// And the GPU panel is the one that comes and goes: a machine that
  /// reported none is back to six, with nothing left behind.
  ///
  /// The counterpart to the count above. Between them they pin both
  /// halves of the #686 rule -- the panel appears when there is
  /// something truthful in it, and vanishes entirely when there is not.
  it("drops to six panels on a machine with no discoverable GPU", async () => {
    liveFn.mockResolvedValue(sample({ gpus: [] }));
    historyFn.mockResolvedValue([]);
    const { container } = show();
    await screen.findByText("1.25");
    const grid = container.querySelector(".md\\:grid-cols-2");
    const headings = within(grid as HTMLElement).getAllByRole("heading");
    expect(headings.filter((h) => h.tagName === "H2").length).toBe(6);
    expect(headings.map((h) => h.textContent)).not.toContain("GPU");
  });
});

/// The panel #665 is actually about.
///
/// Two halves with opposite rules: the live half polls and must never
/// print a zero for a process that is not running; the disk half is
/// slow and must never run without being asked.
describe("what Headstate is costing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    liveFn.mockResolvedValue(sample());
    historyFn.mockResolvedValue([]);
    footprintFn.mockResolvedValue(footprint());
    primeDisk();
  });

  /// THE test.
  ///
  /// `size_worktrees` is ~13s for 147 worktrees, and over the remote
  /// surface a call that slow times out at 120s -- issue #661, exactly.
  /// So nothing here may run because a view was opened. Every disk
  /// source is asserted individually: a single counter would pass while
  /// three of the four fired.
  ///
  /// Discovery is included on purpose. `scan_artifacts` and
  /// `scan_venvs` take seconds themselves, so a panel that deferred
  /// only the sizing would still have paid most of the cost on mount.
  it("measures no disk at all until asked", async () => {
    show();
    // Waited for the page to be fully painted before asserting, so this
    // is "nothing fired once everything settled" rather than "nothing
    // had fired yet in the first tick", which would pass trivially.
    await screen.findByText("1.25");
    await screen.findByRole("button", { name: /Measure disk use/ });
    await waitFor(() => expect(footprintFn).toHaveBeenCalled());

    for (const spy of diskSpies()) expect(spy).not.toHaveBeenCalled();
  });

  /// The other half of the same requirement: deferring is only correct
  /// if the action actually works. A gate that never opens is not a
  /// safe panel, it is a missing feature -- which is how this issue got
  /// reopened.
  it("measures the disk once, and only once, the button is pressed", async () => {
    show();
    const button = await screen.findByRole("button", { name: /Measure disk use/ });
    for (const spy of diskSpies()) expect(spy).not.toHaveBeenCalled();

    fireEvent.click(button);

    await waitFor(() => {
      expect(disk.worktrees).toHaveBeenCalled();
      expect(disk.artifacts).toHaveBeenCalled();
      expect(disk.venvs).toHaveBeenCalled();
      expect(disk.dockerDisk).toHaveBeenCalled();
    });
    // Sizing follows discovery, since it needs the paths discovery found.
    await waitFor(() => {
      expect(disk.worktreeSizes).toHaveBeenCalled();
      expect(disk.artifactSizes).toHaveBeenCalled();
      expect(disk.venvSizes).toHaveBeenCalled();
    });
  });

  /// The figures must be the ones the other views show, summed from
  /// the same commands -- not a second count that could disagree.
  it("shows the sizes the other views measured", async () => {
    show();
    fireEvent.click(await screen.findByRole("button", { name: /Measure disk use/ }));

    // Each row carries the figure its own source reported, summed but
    // not recomputed: 3 GB of worktrees, 5 GB of artifacts, 700 MB of
    // virtualenvs, and Docker's images + build cache + volumes (2 GB +
    // 1 GB + 0) as the single 3 GB the Docker page shows.
    expect(await screen.findByText("3.0 GB", { exact: false })).toBeTruthy();
    await waitFor(() => {
      expect(screen.getByText("5.0 GB", { exact: false })).toBeTruthy();
      expect(screen.getByText("700 MB", { exact: false })).toBeTruthy();
      // Two 3.0 GB figures: worktrees, and Docker's three lines added.
      expect(screen.getAllByText("3.0 GB", { exact: false }).length).toBe(2);
    });
    expect(screen.getByText(/same figures the Worktrees, Artifacts and Docker/)).toBeTruthy();
  });

  /// The mirror of "absent is never zero", and the case that is easy to
  /// get backwards: once the scan HAS run and found nothing, saying
  /// "not measured" reports a completed look as a failure to look. A
  /// machine with no virtualenvs is not an unmeasured machine.
  it("says none found, not not-measured, when a scan came back empty", async () => {
    disk.venvs.mockResolvedValue([]);
    show();
    fireEvent.click(await screen.findByRole("button", { name: /Measure disk use/ }));
    await waitFor(() => expect(screen.getByText("None found")).toBeTruthy());
    // The sizing command must not have run for a set with nothing in it.
    expect(disk.venvSizes).not.toHaveBeenCalled();
  });

  /// The cost is stated before the click, not discovered after it. A
  /// button that says only "Measure" and then appears to hang for
  /// thirty seconds is how a user learns to distrust the view.
  it("says what measuring will cost before it is asked to", async () => {
    show();
    await screen.findByRole("button", { name: /Measure disk use/ });
    expect(screen.getByText(/takes tens of seconds/)).toBeTruthy();
  });

  /// ABSENT IS NEVER ZERO, on the live half.
  ///
  /// An empty `children` is the ORDINARY state -- `git` runs in bursts
  /// and is gone between refreshes -- so it must read as "none right
  /// now", never as a table of tools sitting at 0%. A zeroed row here
  /// would tell a user their fan-out is idle when it never started.
  it("renders tools that are not running as absent, never as zero", async () => {
    footprintFn.mockResolvedValue(footprint({ children: [], docker_daemon: null }));
    show();
    await screen.findByText(/None running at this moment/);
    // No fabricated ROWS: nothing claims a tool exists at no cost.
    //
    // Checked as rows rather than by name, because the sentence
    // explaining the empty state names `git` and `gh` in prose -- and
    // that copy is the point of this test, not a collision with it.
    // What must not exist is a table row for a process that is not
    // running. Two rows survive inside this panel: the header and the
    // app itself, which IS running.
    const rows = within(footprintPanel()).getAllByRole("row");
    expect(rows).toHaveLength(2);
    expect(rows[1].textContent).toContain("headstate");
    expect(screen.queryByText("com.docker.backend")).toBeNull();
    expect(screen.queryByText("0%")).toBeNull();
    expect(screen.queryByText("0 B")).toBeNull();
    // And Docker not running says so, rather than showing a zero-cost
    // daemon -- which is the common case, not the exotic one.
    expect(screen.getByText("Not running.")).toBeTruthy();
  });

  /// The same rule for our own process, whose absence would be a failed
  /// lookup rather than an idle app.
  it("says so when the platform will not report our own process", async () => {
    footprintFn.mockResolvedValue(footprint({ app: null }));
    show();
    expect(
      await screen.findByText(/did not report our own process/),
    ).toBeTruthy();
  });

  /// The live half, when everything is running.
  it("names each running process with its pid, cpu and memory", async () => {
    show();
    expect(await screen.findByText("headstate")).toBeTruthy();
    // By ROW, since `git` and `gh` also appear in the prose beside the
    // tables. A row is what proves the process was actually listed.
    const rows = within(footprintPanel())
      .getAllByRole("row")
      .map((r) => r.textContent);
    expect(rows.some((t) => t?.includes("git") && t.includes("5001"))).toBe(true);
    expect(rows.some((t) => t?.includes("gh") && t.includes("5002"))).toBe(true);
    expect(rows.some((t) => t?.includes("com.docker.backend"))).toBe(true);
    // Memory as a size, not a byte count nobody can read.
    expect(screen.getByText("64 MB")).toBeTruthy();
  });

  /// CPU here is a share of ONE core, so `git` on three of them reads
  /// 280%. Clamping it to 100 -- which every other percentage on this
  /// page legitimately does -- would report a busy process as a merely
  /// saturated one and hide the fan-out the panel exists to show.
  it("does not clamp a process using more than one core to 100%", async () => {
    footprintFn.mockResolvedValue(
      footprint({
        children: [{ pid: 5001, name: "git", cpu_percent: 280, memory: 1024 ** 2 }],
      }),
    );
    show();
    expect(await screen.findByText("280%")).toBeTruthy();
    expect(screen.getAllByText(/of one core/).length).toBeGreaterThan(0);
  });

  /// The daemon is not ours and not our child. Reporting it is right --
  /// a 4 GB daemon is a cost the user attributes to this app -- but it
  /// must be reported apart from the groups we do own.
  it("keeps the Docker daemon separate from Headstate's own processes", async () => {
    show();
    await screen.findByText("com.docker.backend");
    expect(screen.getByText(/Not started by Headstate/)).toBeTruthy();
  });
});

/// #683: the pressure row, above everything else.
describe("the at-a-glance pressure cards", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    liveFn.mockResolvedValue(sample());
    historyFn.mockResolvedValue([]);
    footprintFn.mockResolvedValue(footprint());
    primeDisk();
    mobileBuild.current = false;
    connection.current = { kind: "local" };
  });

  it("answers CPU, memory and disk without scrolling", async () => {
    show();
    // Scoped to the row: the same figures also appear in the panels
    // below, which is correct -- they are one source of truth read
    // twice -- but makes a page-wide query ambiguous.
    const row = await screen.findByRole("group", { name: /pressure at a glance/i });
    // The fixture is 8 of 16 GB used, and a 500 GB root with 100 GB
    // free -- so 50% and 80%.
    expect(within(row).getByText("50%")).toBeTruthy();
    expect(within(row).getByText("80%")).toBeTruthy();
    expect(within(row).getByText("18%")).toBeTruthy();
  });

  /// A percentage alone cannot tell a nearly-full small disk from a
  /// nearly-full large one, which is the difference between "ignore
  /// this" and "act today".
  it("shows the absolute figures behind each percentage", async () => {
    show();
    const row = await screen.findByRole("group", { name: /pressure at a glance/i });
    expect(within(row).getByText(/of 16 GB/)).toBeTruthy();
    expect(within(row).getByText(/free of 500 GB/)).toBeTruthy();
  });

  /// The page's central rule, applied where people look first. A
  /// confident 0% for something never measured is the one thing this
  /// view exists not to do.
  it("says a missing reading is not measured, never zero", async () => {
    liveFn.mockResolvedValue(sample({ cpu_percent: null }));
    show();
    const row = await screen.findByRole("group", { name: /pressure at a glance/i });
    expect(within(row).getByText("Not measured")).toBeTruthy();
    expect(within(row).queryByText("0%")).toBeNull();
  });

  /// Colour is a second cue, never the only one: the bar carries the
  /// band, the digits carry the value, and a reader who cannot
  /// distinguish the bands still gets the answer.
  it("labels each bar for a reader who cannot see its colour", async () => {
    show();
    expect(await screen.findByLabelText(/Memory: 50 percent/i)).toBeTruthy();
    expect(screen.getByLabelText(/Disk: 80 percent/i)).toBeTruthy();
  });

  /// The row must not become a second source of truth: the panels
  /// below stay exactly as they were.
  it("leaves the panels below untouched", async () => {
    show();
    await screen.findByRole("group", { name: /pressure at a glance/i });
    // The panel headings, which the row must not have replaced.
    for (const title of ["Load averages, per-core use, and the last 24 hours",
                         "Every mounted volume"]) {
      expect(screen.getByText(title)).toBeTruthy();
    }
  });
});

/// #666: the same view on the phone, describing the DESKTOP.
///
/// Every test here flips `mobileBuild.current` explicitly. A test that
/// forgets renders the desktop build whatever its name says -- the same
/// trap `stubViewport` sets for viewport-gated code, and the reason
/// #655 shipped a component with no mobile coverage at all.
describe("SystemHealthPage on the phone", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    liveFn.mockResolvedValue(sample());
    historyFn.mockResolvedValue([]);
    // The footprint panel is part of this page too, so it is primed
    // here as well -- otherwise these tests would assert against a
    // page half of which never resolved, which is not the page a user
    // sees.
    footprintFn.mockResolvedValue(footprint());
    primeDisk();
    mobileBuild.current = true;
    connection.current = { kind: "connected", desktop: "studio", lastPoll: null };
  });

  it("says whose machine it is describing", async () => {
    show();
    // The distinction the whole issue is about: this page is full of
    // CPU and battery readings, rendered on a device that has its own.
    expect(await screen.findByText(/not this phone/i)).toBeTruthy();
    expect(screen.getByText("studio")).toBeTruthy();
  });

  it("says the desktop is unreachable rather than showing zeros", async () => {
    connection.current = { kind: "unreachable", desktop: "studio", lastPoll: null };
    liveFn.mockRejectedValue(new Error("unreachable"));
    show();
    expect(await screen.findByText(/cannot reach studio/i)).toBeTruthy();
    // The failure this codebase avoids everywhere else: a zero that
    // means "not measured". No reading may be on screen.
    expect(screen.queryByText("0%")).toBeNull();
    expect(screen.queryByText("18%")).toBeNull();
  });

  it("does not blame the desktop for a gap the phone may have caused", async () => {
    historyFn.mockResolvedValue([sample()]);
    show();
    // On the phone a hole has two causes, and naming only the first
    // would assert the desktop was off during a period it may have
    // been running perfectly well.
    const note = await screen.findAllByText(/could not reach it/i);
    expect(note.length).toBeGreaterThan(0);
  });

  it("names the desktop when its battery is absent", async () => {
    liveFn.mockResolvedValue(sample({ battery: null }));
    show();
    expect(await screen.findByText(/that desktop has no battery/i)).toBeTruthy();
  });

  it("falls back to a neutral name before the desktop is known", async () => {
    connection.current = { kind: "unknown" };
    show();
    // No hostname yet is not the same as no pairing; the page still
    // has to say it is not describing the phone.
    expect(await screen.findByText(/the paired desktop/i)).toBeTruthy();
  });
});

/// The other half of the same requirement: the desktop rendering is
/// unchanged. Asserted at a real desktop width rather than by
/// inspection, because that is what the issue asks for.
describe("SystemHealthPage on the desktop is untouched by the mobile work", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    liveFn.mockResolvedValue(sample());
    historyFn.mockResolvedValue([]);
    footprintFn.mockResolvedValue(footprint());
    primeDisk();
    mobileBuild.current = false;
    connection.current = { kind: "local" };
  });

  it("never mentions a phone or a paired desktop at 1400px", async () => {
    const viewport = stubViewport(1400);
    try {
      show();
      await screen.findAllByText("18%");
      expect(screen.queryByText(/not this phone/i)).toBeNull();
      expect(screen.queryByText(/paired desktop/i)).toBeNull();
      expect(screen.queryByText(/could not reach it/i)).toBeNull();
    } finally {
      viewport.resize(1400);
    }
  });

  it("still says a gap means Headstate was not running", async () => {
    historyFn.mockResolvedValue([sample()]);
    show();
    const note = await screen.findAllByText(/Headstate was not running/i);
    expect(note.length).toBeGreaterThan(0);
    // The phone's second cause must not leak on to the desktop, where
    // there is no phone and the claim would be false.
    expect(screen.queryByText(/could not reach it/i)).toBeNull();
  });

  it("keeps its own wording when a local reading fails", async () => {
    liveFn.mockRejectedValue(new Error("boom"));
    show();
    expect(await screen.findByText(/could not read this machine's health/i)).toBeTruthy();
    expect(screen.queryByText(/cannot reach/i)).toBeNull();
  });
});
