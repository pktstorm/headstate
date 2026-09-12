import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invoke = vi.hoisted(() =>
  vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>((cmd, args) => {
    if (cmd === "size_artifacts") {
      const paths = (args as { paths: string[] }).paths;
      return Promise.resolve(paths.map((p) => [p, 2_684_354_560, 60]));
    }
    return Promise.resolve([]);
  }),
);
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { useOrphanSize } from "./hooks";

const wrapper = (qc: QueryClient) =>
  function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };

const fresh = () =>
  new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });

/// #845: the size the orphan confirmation states.
///
/// An orphan is the ONE row on the worktree page with no size available.
/// `sizeWorktrees` opens with `git worktree list` inside the repository,
/// and an orphan's repository is exactly what is gone -- so it rejects
/// rather than answering. `useAllWorktreeSizes` is gated on the
/// all-repositories view and never runs here either. The confirmation
/// therefore had nothing to say about how much was about to be destroyed,
/// and "2.5 GB" is what turns "copy it somewhere first" into a decision.
describe("useOrphanSize", () => {
  beforeEach(() => invoke.mockClear());

  /// It measures through `size_artifacts`, which takes EXPLICIT PATHS and
  /// involves no git -- the one existing command that can measure a
  /// directory nothing owns. Asserted on the command name because the
  /// misnomer is deliberate and load-bearing: the string is matched as a
  /// literal in two remote-surface allowlists, one of which ships in the
  /// phone app on its own release tag.
  it("measures the directory through the path-taking command, not the git one", async () => {
    const { result } = renderHook(() => useOrphanSize("/code/veil-coh", true), {
      wrapper: wrapper(fresh()),
    });
    await waitFor(() => expect(result.current.bytes).toBe(2_684_354_560));
    const calls = invoke.mock.calls.filter((c) => c[0] === "size_artifacts");
    expect(calls).toHaveLength(1);
    expect((calls[0][1] as { paths: string[] }).paths).toEqual(["/code/veil-coh"]);
    expect(invoke.mock.calls.some((c) => c[0] === "size_worktrees")).toBe(false);
  });

  /// The dialog being open is what starts the walk, not the row existing.
  /// A walk per orphan on mount measures directories nobody asked about.
  it("starts no walk until it is asked for one", () => {
    renderHook(() => useOrphanSize("/code/veil-coh", false), { wrapper: wrapper(fresh()) });
    expect(invoke.mock.calls.filter((c) => c[0] === "size_artifacts")).toHaveLength(0);
  });

  /// An empty result is NOT zero bytes.
  ///
  /// Zero reads as "this tree is empty, delete it", which for an
  /// unmeasurable directory is the most damaging thing the dialog could
  /// say -- the same rule `size_worktrees` states about flattening its own
  /// nulls. The dialog has a "not known" branch and this is what has to
  /// reach it.
  it("reports an unanswered walk as null rather than as zero", async () => {
    invoke.mockImplementation((cmd) =>
      cmd === "size_artifacts" ? Promise.resolve([]) : Promise.resolve([]),
    );
    const { result } = renderHook(() => useOrphanSize("/code/veil-coh", true), {
      wrapper: wrapper(fresh()),
    });
    await waitFor(() => expect(result.current.measuring).toBe(false));
    expect(result.current.bytes).toBeNull();
    expect(result.current.bytes).not.toBe(0);
  });

  /// A REFUSAL is its own state, distinct from a null result: it is the
  /// second thing about this orphan that could not be checked, and the
  /// dialog names it out loud rather than silently omitting the size.
  it("separates a refused walk from one that answered nothing", async () => {
    invoke.mockImplementation((cmd) =>
      cmd === "size_artifacts"
        ? Promise.reject(new Error("permission denied"))
        : Promise.resolve([]),
    );
    const { result } = renderHook(() => useOrphanSize("/code/veil-coh", true), {
      wrapper: wrapper(fresh()),
    });
    await waitFor(() => expect(result.current.failed).toBe(true));
    expect(result.current.bytes).toBeNull();
  });

  /// `retry: false`, matching every other sizing query here. A failed
  /// walk is not a flaky network call, and the dialog has a branch for
  /// "could not be measured" -- three expensive walks to reach the same
  /// branch only delay the question. Asserted by COUNTING the calls,
  /// because the defaults a caller's QueryClient sets are what actually
  /// decide this and a test that trusted its own `retry: false` wrapper
  /// would be asserting its own fixture.
  it("does not re-walk a refused directory", async () => {
    invoke.mockImplementation((cmd) =>
      cmd === "size_artifacts"
        ? Promise.reject(new Error("permission denied"))
        : Promise.resolve([]),
    );
    const qc = new QueryClient({ defaultOptions: { queries: { gcTime: 0 } } });
    const { result } = renderHook(() => useOrphanSize("/code/veil-coh", true), {
      wrapper: wrapper(qc),
    });
    await waitFor(() => expect(result.current.failed).toBe(true));
    expect(invoke.mock.calls.filter((c) => c[0] === "size_artifacts")).toHaveLength(1);
  });
});
