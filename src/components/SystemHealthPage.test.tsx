import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HealthSample } from "@/types/pr";

const liveFn = vi.hoisted(() => vi.fn<() => Promise<HealthSample>>());
const historyFn = vi.hoisted(() => vi.fn<() => Promise<HealthSample[]>>());

// The two hooks, not the whole `api/hooks` module's transitive world:
// that file imports every command the app has, and mocking it wholesale
// would tie this test to all of them. The stand-ins keep the real
// TanStack behaviour so loading and error states are the genuine ones.
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
  disks: [
    { mount: "/", total: 500 * 1024 ** 3, available: 100 * 1024 ** 3, is_root: true },
  ],
  battery: { percent: 82, on_ac: false },
  thermal: "nominal",
  networks: [{ name: "en0", rx_bytes: 1024 ** 3, tx_bytes: 512 * 1024 ** 2 }],
  uptime_secs: 3 * 86_400 + 4 * 3600,
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

describe("SystemHealthPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    liveFn.mockResolvedValue(sample());
    historyFn.mockResolvedValue([]);
  });

  it("shows the live readings once they arrive", async () => {
    show();
    expect(await screen.findByText("1.25")).toBeTruthy();
    expect(screen.getByText("0.94")).toBeTruthy();
    expect(screen.getByText("0.71")).toBeTruthy();
    expect(screen.getByText("18%")).toBeTruthy();
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
    expect(screen.getByText(/100 GB free of 500 GB/)).toBeTruthy();
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
  it("explains the breaks beside each chart", async () => {
    show();
    await screen.findByText("1.25");
    const notes = screen.getAllByText(/Headstate was not running/);
    expect(notes.length).toBe(2);
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
    expect(within(grid as HTMLElement).getAllByRole("heading").length).toBe(5);
  });
});
