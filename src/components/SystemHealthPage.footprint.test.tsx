import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

/// The footprint panel with the REAL hooks behind it.
///
/// `SystemHealthPage.test.tsx` mocks `api/hooks`, which is right for
/// asserting what the component renders but leaves a hole exactly where
/// this issue's danger is: a stand-in that honours `enabled` proves the
/// component passes the flag, not that anything downstream respects it.
/// A regression could therefore live entirely in `hooks.ts` -- a
/// forgotten `enabled`, a query built outside the gate -- and the
/// component suite would stay green.
///
/// So this file mocks only `@tauri-apps/api/core`, the actual seam to
/// Rust, and asserts on the command names that crossed it. What it
/// guards is #661: `size_worktrees` is ~13s for 147 worktrees, and over
/// the remote surface a call that slow times out at 120s. Nothing in
/// this list may fire because a view was opened.
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
/// 178 directories, 9-40s for virtualenvs), so a panel that deferred
/// only the sizing would still have paid most of the cost on mount.
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
  load: [1, 1, 1],
  cpu_percent: 10,
  cpu_per_core: [10],
  memory: {
    total: 16 * 1024 ** 3,
    used: 8 * 1024 ** 3,
    available: 7 * 1024 ** 3,
    swap_total: 0,
    swap_used: 0,
  },
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
          app: { pid: 1, name: "headstate", cpu_percent: 2, memory: 1024 ** 2 },
          children: [],
          docker_daemon: null,
        });
      case "list_worktrees":
        return Promise.resolve([
          { identity: null, name: "hs", path: "/code/hs", worktrees: [] },
        ]);
      case "size_worktrees":
        return Promise.resolve([["/code/hs/wt", 1024 ** 3]]);
      case "scan_artifacts":
        return Promise.resolve([
          {
            path: "/code/hs/target",
            kind: "cargo-target",
            repo_path: "/code/hs",
            size_bytes: null,
            idle_secs: null,
          },
        ]);
      case "size_artifacts":
        return Promise.resolve([["/code/hs/target", 2 * 1024 ** 3, null]]);
      case "scan_venvs":
        return Promise.resolve([
          {
            path: "/venvs/a",
            project: "a",
            state: "active",
            source: null,
            size_bytes: null,
            idle_secs: null,
          },
        ]);
      case "size_venvs":
        return Promise.resolve([["/venvs/a", 512 * 1024 ** 2, null]]);
      case "docker_disk_usage":
        return Promise.resolve({
          images_bytes: 1024 ** 3,
          images_reclaimable_bytes: 0,
          build_cache_bytes: 0,
          volumes_bytes: 0,
          volumes_reclaimable_bytes: 0,
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

describe("the footprint panel over the real transport", () => {
  /// THE requirement, asserted at the seam that actually matters.
  it("sends no slow command when the view merely opens", async () => {
    show();
    // Settled first, so this is "nothing fired once everything had a
    // chance to" rather than "nothing had fired in the first tick",
    // which would pass trivially.
    await screen.findByRole("button", { name: /Measure disk use/ });
    await waitFor(() => expect(called("system_footprint")).toBe(true));
    await waitFor(() => expect(called("system_health")).toBe(true));

    for (const cmd of SLOW) expect(called(cmd)).toBe(false);
  });

  /// The live half IS allowed on the timer, and must be: it is a kernel
  /// read of the already-open process table, no subprocess and no
  /// directory walk. Asserted so a future change that made it expensive
  /// would have to come here and say so.
  it("does poll the cheap live reading", async () => {
    show();
    await waitFor(() => expect(called("system_footprint")).toBe(true));
  });

  /// Deferring is only correct if the action works. A gate that never
  /// opens is not a safe panel -- it is a missing feature, which is how
  /// #665 came to be reopened with its backend shipped and no caller.
  it("runs every disk command once, and only once, the button is pressed", async () => {
    show();
    fireEvent.click(await screen.findByRole("button", { name: /Measure disk use/ }));

    await waitFor(() => {
      for (const cmd of SLOW) expect(called(cmd)).toBe(true);
    });
  });

  /// The figures are the other views', summed from the same commands.
  /// Nothing here measures anything: 1 GB of worktrees, 2 GB of
  /// artifacts, 512 MB of venvs, and Docker's own accounting.
  it("shows the sizes those commands reported", async () => {
    show();
    fireEvent.click(await screen.findByRole("button", { name: /Measure disk use/ }));
    await waitFor(() => {
      expect(screen.getAllByText("1.0 GB", { exact: false }).length).toBe(2);
      expect(screen.getByText("2.0 GB", { exact: false })).toBeTruthy();
      expect(screen.getByText("512 MB", { exact: false })).toBeTruthy();
    });
  });
});
