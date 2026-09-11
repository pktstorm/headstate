import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

/// System Health with the REAL hooks behind it.
///
/// `SystemHealthPage.test.tsx` mocks `api/hooks`, which is right for
/// asserting what the component renders but leaves a hole exactly where
/// the danger is: a stand-in that honours `enabled` proves the component
/// passes the flag, not that anything downstream respects it. A
/// regression could therefore live entirely in `hooks.ts` -- a forgotten
/// `enabled`, a query built outside the gate -- and the component suite
/// would stay green.
///
/// So this file mocks only `@tauri-apps/api/core`, the actual seam to
/// Rust, and asserts on the command names that crossed it. What it guards
/// is #661: `size_worktrees` is ~13s for 147 worktrees, and over the
/// remote surface a call that slow times out at 120s. Nothing in the list
/// below may fire because this view was opened.
///
/// # This file used to be about the footprint panels (#665)
///
/// It was written for the "What Headstate is costing" panel on the
/// overview and its sibling on the disk page, both of which put those
/// slow commands behind a "Measure disk use" button. #795 and #796
/// removed both panels, so the gate they were testing no longer exists --
/// which makes the requirement STRONGER rather than obsolete: there is
/// now no affordance anywhere in this view that can reach a slow command,
/// and the assertions below are what keep it that way.
///
/// Kept rather than deleted deliberately. The rejected alternative was
/// dropping the file with the panels, on the grounds that a component
/// which no longer calls a command cannot call it unasked. That reasoning
/// expires the moment someone adds a disk figure back to this page, and a
/// test nobody deleted is the cheapest way to make that addition an
/// explicit decision instead of an accident. The cost is one render.
///
/// `system_footprint` is stubbed below but no longer FIRES from the
/// landing page -- #795 removed its only caller there, and one test
/// asserts exactly that. The stub stays so the day a page calls it again
/// the assertion fails on the call rather than on an unresolved query.
const invoke = vi.hoisted(() =>
  vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(),
);
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

import { SystemHealthPage } from "./SystemHealthPage";

/// The commands that walk the filesystem, and the scans that find what
/// to walk. All seven are slow enough to matter; none may run unasked.
///
/// The DISCOVERY commands are in the list on purpose. `scan_artifacts`
/// and `scan_venvs` are seconds in their own right (measured: ~1.5s for
/// 178 directories, 9-40s for virtualenvs), so a page that deferred only
/// the sizing would still have paid most of the cost on mount.
const SLOW = [
  "list_worktrees",
  "size_worktrees",
  "scan_artifacts",
  "size_artifacts",
  "scan_venvs",
  "size_venvs",
  "docker_disk_usage",
];

const called = (cmd: string) => invoke.mock.calls.some((c) => c[0] === cmd);

const health = () => ({
  sampled_at: new Date().toISOString(),
  // Three DIFFERENT figures, so the one the tests wait on identifies a
  // single element. Identical loads render as three `1.00`s and a
  // `findByText` for one of them is ambiguous.
  load: [1.25, 0.94, 0.71],
  cpu_percent: 10,
  cpu_per_core: [10],
  memory: {
    total: 16 * 1024 ** 3,
    used: 8 * 1024 ** 3,
    available: 7 * 1024 ** 3,
    swap_total: 0,
    swap_used: 0,
  },
  gpus: [],
  disks: [],
  battery: null,
  thermal: "nominal",
  networks: [],
  uptime_secs: 3600,
});

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation((cmd) => {
    switch (cmd) {
      case "system_health":
        return Promise.resolve(health());
      case "system_health_history":
        return Promise.resolve([]);
      case "system_footprint":
        return Promise.resolve({
          sampled_at: new Date().toISOString(),
          top_cpu: [{ pid: 701, name: "acme-render", cpu_percent: 412, memory: 1024 ** 3 }],
          top_memory: [{ pid: 701, name: "acme-render", cpu_percent: 412, memory: 1024 ** 3 }],
          top_cpu_grouped: [],
          top_memory_grouped: [],
          process_count: 1436,
        });
      default:
        return Promise.resolve([]);
    }
  });
});

const show = () => {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  }
  return render(
    <Wrapper>
      <SystemHealthPage />
    </Wrapper>,
  );
};

describe("System Health over the real transport", () => {
  /// THE requirement, asserted at the seam that actually matters.
  it("sends no slow command when the view opens", async () => {
    show();
    // Settled first, so this is "nothing fired once everything had a
    // chance to" rather than "nothing had fired in the first tick",
    // which would pass trivially. The two cheap reads landing and the
    // figures being on screen is the proof the page actually rendered.
    await waitFor(() => expect(called("system_health")).toBe(true));
    await waitFor(() => expect(called("system_health_history")).toBe(true));
    await screen.findByText("1.25");

    for (const cmd of SLOW) expect(called(cmd)).toBe(false);
  });

  /// There is no longer any way to ASK for one either, and that is the
  /// other half of #795 and #796: the "Measure disk use" button went with
  /// the panels it gated. Asserted because its absence is the design --
  /// the Worktrees, Artifacts and Docker pages own those measurements and
  /// can act on them, and these panels could only report.
  it("offers no measurement affordance at all", async () => {
    show();
    await screen.findByText("1.25");
    expect(screen.queryByRole("button", { name: /measure disk use/i })).toBeNull();
  });

  /// `system_footprint` is NOT called here, and that is worth an
  /// assertion rather than a silence.
  ///
  /// It is the cheap live reading -- a kernel read of the already-open
  /// process table -- and it is still polled, by the CPU and Memory
  /// DETAIL pages. #795 removed the overview's only caller of it. So a
  /// future edit that fires it on the landing page is reintroducing a
  /// poll nothing on that page reads, and this is where that shows up.
  ///
  /// It belongs in THIS file rather than the component suite because
  /// only here is the assertion about what crossed the transport; with
  /// `api/hooks` mocked, a hook called and discarded looks identical to
  /// one never called.
  it("does not poll the process table from the overview", async () => {
    show();
    await screen.findByText("1.25");
    expect(called("system_footprint")).toBe(false);
  });
});
