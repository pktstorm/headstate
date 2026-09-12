import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invoke = vi.hoisted(() =>
  vi.fn<(cmd: string, ...a: unknown[]) => Promise<unknown>>(() => Promise.resolve()),
);
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { useActOnPr } from "./hooks";

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function wrapper(qc: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
}

function client() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  qc.setQueryData(["viewer"], "me");
  return qc;
}

/// #742: one gesture, one refresh.
///
/// `refresh_now` searches every watched repository, which took 7-17
/// seconds on the account that reported this. Approving fires a refresh
/// and the auto-enqueue that follows fires another seconds later, so the
/// second abandoned the first mid-flight -- which surfaced as
/// "Background refresh failed". A real six-day session log held 25
/// `refresh_now start` lines with no matching completion, every one of
/// them preceded by a second action within seconds.
describe("refreshPrs coalescing", () => {
  // Fake timers, per the repo's own template (`src/lib/countdown.test.tsx`,
  // `src/splash.test.ts`), and `shouldAdvanceTime` as in
  // `hooks.venvs.test.tsx` so React Query's internals and `waitFor` still
  // make progress on their own.
  //
  // #853: the negative assertion below used 20ms of REAL slack to show
  // that no second refresh started. Real slack cannot prove a negative --
  // it only says nothing happened YET, and on a loaded runner (CI's
  // vitest shares the box with nothing else here, but the race check
  // runs the Rust suite at eight threads beside it) a second call can
  // land at 21ms and the test still passes. Advancing a fake clock makes
  // the window exact: everything scheduled within it has run, so "the
  // count did not climb" is a real negative rather than a deadline.
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    invoke.mockReset();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("joins a refresh already in flight instead of starting a rival", async () => {
    const slow = deferred<unknown>();
    let refreshCalls = 0;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "refresh_now") {
        refreshCalls += 1;
        return slow.promise;
      }
      return Promise.resolve();
    });

    const qc = client();
    const { result } = renderHook(() => useActOnPr(), { wrapper: wrapper(qc) });

    // The gesture: two actions in quick succession, the second arriving
    // while the first's refresh is still running. Deliberately not
    // awaited -- overlapping is the condition under test.
    void result.current("id1", "o/r", 7, "approve" as never);
    await waitFor(() => expect(refreshCalls).toBe(1));

    void result.current("id2", "o/r", 7, "enqueue" as never);

    // Room for the second action to reach its refresh. If it started one
    // of its own, this is where the count would climb. Advancing the fake
    // clock runs everything scheduled in that window, so the unchanged
    // count is an exact negative rather than a 20ms deadline (#853).
    await vi.advanceTimersByTimeAsync(20);
    expect(refreshCalls).toBe(1);

    slow.resolve([]);
    await waitFor(() => expect(refreshCalls).toBe(1));
  });

  it("starts a fresh refresh for a gesture after the first settles", async () => {
    // The guard must not latch. Coalescing that never releases would
    // mean the list silently stops updating after the first action of a
    // session, which is a worse bug than the one being fixed.
    let refreshCalls = 0;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "refresh_now") {
        refreshCalls += 1;
        return Promise.resolve([]);
      }
      return Promise.resolve();
    });

    const qc = client();
    const { result } = renderHook(() => useActOnPr(), { wrapper: wrapper(qc) });

    await result.current("id1", "o/r", 1, "approve" as never);
    expect(refreshCalls).toBe(1);

    await result.current("id2", "o/r", 2, "approve" as never);
    expect(refreshCalls).toBe(2);
  });
});
