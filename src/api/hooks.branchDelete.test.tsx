import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BranchDeleteFrame } from "@/types/pr";

/// The event bus, standing in for Tauri's — the same setup
/// `hooks.branchScan.test.tsx` uses, and mocked at the same seam.
///
/// `@tauri-apps/api/event` rather than `./transport`, so the path the
/// mobile build swaps is itself under test. That path is why
/// `branch-delete-progress` needed an entry on the `EVENT_NAMES`
/// allowlist: a phone receives this event over it, and a hook reaching
/// Tauri directly would work on the desktop and silently never fire on
/// the phone (#724).
const bus = vi.hoisted(() => {
  const listeners = new Map<string, Set<(e: { payload: unknown }) => void>>();
  return {
    listeners,
    emit(name: string, payload: unknown) {
      for (const cb of listeners.get(name) ?? []) cb({ payload });
    },
    reset() {
      listeners.clear();
    },
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(() => new Promise(() => {})) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) => {
    const set = bus.listeners.get(name) ?? new Set();
    set.add(cb);
    bus.listeners.set(name, set);
    return Promise.resolve(() => set.delete(cb));
  }),
}));

import { useBranchDeleteProgress } from "./hooks";

const emit = (f: BranchDeleteFrame) =>
  act(() => {
    bus.emit("branch-delete-progress", f);
  });

/// Flush the promise `listen` resolves through, so the subscription is
/// in place before a test emits into it.
const subscribed = () => act(async () => {});

const REPO = "/code/app";

describe("useBranchDeleteProgress", () => {
  beforeEach(() => bus.reset());

  const show = () =>
    renderHook(({ r }: { r: string | undefined }) => useBranchDeleteProgress(r), {
      initialProps: { r: REPO as string | undefined },
    });

  it("reports nothing while no deletion is running", () => {
    const { result } = show();
    expect(result.current).toBeNull();
  });

  /// The whole point of the change. A deletion opens with a full
  /// uncached scan of the repository, and on the reported 562-branch
  /// batch that is minutes long — reported as one counter it would
  /// read 0/562 throughout, which is indistinguishable from a hang.
  it("names the checking phase, and its count moves while nothing is deleted", async () => {
    const { result } = show();
    await subscribed();

    emit({ kind: "checking", repo: REPO, done: 0, total: 562 });
    expect(result.current).toEqual({ phase: "checking", done: 0, total: 562 });

    emit({ kind: "checking", repo: REPO, done: 128, total: 562 });
    expect(result.current).toEqual({ phase: "checking", done: 128, total: 562 });
  });

  /// The phase, not merely the numbers, is what the page renders on.
  /// A user who reads "deleting" during the re-check and cancels
  /// believes refs are already gone when none are.
  it("moves from checking to deleting, and the counters are of different things", async () => {
    const { result } = show();
    await subscribed();

    // The gate scans the whole repository — 900 branches — for a batch
    // of 562. The two totals are deliberately different.
    emit({ kind: "checking", repo: REPO, done: 900, total: 900 });
    expect(result.current).toEqual({ phase: "checking", done: 900, total: 900 });

    emit({ kind: "deleting", repo: REPO, done: 47, total: 562, failed: 0 });
    expect(result.current).toEqual({
      phase: "deleting",
      done: 47,
      total: 562,
      failed: 0,
    });
  });

  /// Failures visible as they happen, not only in the summary at the
  /// end: a batch losing thirty branches to refusals is worth knowing
  /// with five hundred still to go.
  it("carries the refusal count while the batch is still running", async () => {
    const { result } = show();
    await subscribed();

    emit({ kind: "deleting", repo: REPO, done: 100, total: 562, failed: 30 });
    expect(result.current).toMatchObject({ phase: "deleting", failed: 30 });

    // And it persists on later frames rather than being a one-shot the
    // page could render between and miss.
    emit({ kind: "deleting", repo: REPO, done: 200, total: 562, failed: 30 });
    expect(result.current).toMatchObject({ done: 200, failed: 30 });
  });

  /// Cleared on the last frame rather than leaving "562 of 562" up
  /// after the work is over — the rule `useRemovalProgress` follows.
  it("clears when the deleting phase reaches its total", async () => {
    const { result } = show();
    await subscribed();

    emit({ kind: "deleting", repo: REPO, done: 561, total: 562, failed: 0 });
    expect(result.current).not.toBeNull();

    emit({ kind: "deleting", repo: REPO, done: 562, total: 562, failed: 0 });
    expect(result.current).toBeNull();
  });

  /// The event is app-global while a deletion is per-repository. A
  /// page showing one repository must not render another's counters.
  it("drops frames for a different repository", async () => {
    const { result } = show();
    await subscribed();

    emit({ kind: "checking", repo: "/code/other", done: 5, total: 9 });
    expect(result.current).toBeNull();

    emit({ kind: "checking", repo: REPO, done: 5, total: 9 });
    expect(result.current).toEqual({ phase: "checking", done: 5, total: 9 });
  });

  /// Switching repository must not carry the previous one's progress
  /// across. Compared during render rather than reset in an effect,
  /// which would paint the stale phase for a frame first.
  it("forgets a running deletion when the repository changes", async () => {
    const { result, rerender } = show();
    await subscribed();

    emit({ kind: "deleting", repo: REPO, done: 3, total: 10, failed: 1 });
    expect(result.current).not.toBeNull();

    rerender({ r: "/code/other" });
    expect(result.current).toBeNull();
  });

  it("reports nothing when there is no repository at all", async () => {
    const { result } = renderHook(() => useBranchDeleteProgress(undefined));
    await subscribed();
    expect(result.current).toBeNull();
  });
});
