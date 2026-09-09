import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Branch, BranchScanFrame, Deletable } from "@/types/pr";

/// The event bus, standing in for Tauri's. Tests push frames through
/// `emit`; the hook receives them exactly as it would from the desktop.
///
/// Mocked at `@tauri-apps/api/event` rather than at `./transport`, so
/// the seam the mobile build swaps is itself under test: the phone
/// receives this event over the same path, which is the reason
/// `branch-scan-progress` needed a tenth entry on the `EVENT_NAMES`
/// allowlist in the first place (#657).
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

import { useBranchScan } from "./hooks";

const branch = (name: string, deletable: Deletable = { kind: "pending" }): Branch => ({
  name,
  location: "local",
  upstream: null,
  ahead: 0,
  behind: 0,
  committed: "2026-01-01T00:00:00Z",
  author: "octocat",
  tip: "abc1234",
  deletable,
});

const merged: Deletable = { kind: "merged", how: "squash" };

const emit = (f: BranchScanFrame) =>
  act(() => {
    bus.emit("branch-scan-progress", f);
  });

/// Flush the promise `listen` resolves through, so the subscription is
/// in place before a test emits into it.
const subscribed = () => act(async () => {});

describe("useBranchScan", () => {
  beforeEach(() => bus.reset());

  const show = () =>
    renderHook(({ r }: { r: string | undefined }) => useBranchScan(r), {
      initialProps: { r: "/code/app" as string | undefined },
    });

  it("starts with nothing and no total", () => {
    const { result } = show();
    expect(result.current).toEqual({ branches: [], total: null, classified: 0 });
  });

  /// The listing frame is what turns a blank page into a full one. It
  /// arrives before any verdict exists, and every row it carries is
  /// `pending` — a row with no answer, not a row with a negative one.
  it("takes every row and the total from the listing frame", async () => {
    const { result } = show();
    await subscribed();
    emit({
      kind: "listed",
      repo: "/code/app",
      total: 2,
      branches: [branch("a"), branch("b")],
    });
    expect(result.current.total).toBe(2);
    expect(result.current.classified).toBe(0);
    expect(result.current.branches.map((b) => b.name)).toEqual(["a", "b"]);
    expect(result.current.branches.every((b) => b.deletable.kind === "pending")).toBe(true);
  });

  it("fills verdicts in as they arrive and counts them", async () => {
    const { result } = show();
    await subscribed();
    emit({
      kind: "listed",
      repo: "/code/app",
      total: 2,
      branches: [branch("a"), branch("b")],
    });
    emit({ kind: "classified", repo: "/code/app", verdicts: [["a", merged]] });

    expect(result.current.classified).toBe(1);
    expect(result.current.branches[0].deletable).toEqual(merged);
    // Untouched rows stay pending. A verdict for one branch is not
    // evidence about another.
    expect(result.current.branches[1].deletable.kind).toBe("pending");
    // And the order never moves: the rows are rendered in it, so
    // reshuffling as answers land would move targets under the cursor.
    expect(result.current.branches.map((b) => b.name)).toEqual(["a", "b"]);
  });

  /// THE property the total exists for.
  ///
  /// The stream stops after one batch and nothing further ever comes.
  /// The state must be VISIBLY short — `classified` below `total` —
  /// because the page's only way to tell a dead stream from a finished
  /// one is that gap. Without it, a stream that died at 1 of 3 is
  /// indistinguishable from one that delivered everything.
  it("stays visibly short of its total when the stream dies part-way", async () => {
    const { result } = show();
    await subscribed();
    emit({
      kind: "listed",
      repo: "/code/app",
      total: 3,
      branches: [branch("a"), branch("b"), branch("c")],
    });
    emit({ kind: "classified", repo: "/code/app", verdicts: [["a", merged]] });

    // ... and the desktop dies here. Nothing else is ever emitted.
    expect(result.current.classified).toBe(1);
    expect(result.current.total).toBe(3);
    expect(result.current.classified).toBeLessThan(result.current.total!);
    // The rows that never got an answer say so rather than defaulting
    // to one, which is what `Deletable::Pending` exists to guarantee.
    expect(result.current.branches.filter((b) => b.deletable.kind === "pending")).toHaveLength(
      2,
    );
  });

  /// A complete stream reaches its total exactly. Asserted alongside
  /// the death case, because a hook that never reached its total would
  /// pass that one while permanently claiming every scan had stalled.
  it("reaches its total exactly when every verdict arrives", async () => {
    const { result } = show();
    await subscribed();
    emit({
      kind: "listed",
      repo: "/code/app",
      total: 2,
      branches: [branch("a"), branch("b")],
    });
    emit({
      kind: "classified",
      repo: "/code/app",
      verdicts: [
        ["a", merged],
        ["b", { kind: "unmerged", ahead: 2 }],
      ],
    });
    expect(result.current.classified).toBe(2);
  });

  /// The count must mean "rows on screen that are answered". A repeat
  /// batch — a retried frame, or one the desktop sent twice — would
  /// otherwise push it past its total and turn the honest signal into
  /// noise.
  it("does not count the same verdict twice", async () => {
    const { result } = show();
    await subscribed();
    emit({ kind: "listed", repo: "/code/app", total: 1, branches: [branch("a")] });
    emit({ kind: "classified", repo: "/code/app", verdicts: [["a", merged]] });
    emit({ kind: "classified", repo: "/code/app", verdicts: [["a", merged]] });
    expect(result.current.classified).toBe(1);
  });

  /// Nor count a name it has no row for. A verdict that beats its own
  /// listing frame, or names a branch the listing did not, must not
  /// advance a count the user reads as "this many rows are answered".
  it("ignores a verdict for a branch it has not listed", async () => {
    const { result } = show();
    await subscribed();
    emit({ kind: "listed", repo: "/code/app", total: 1, branches: [branch("a")] });
    emit({ kind: "classified", repo: "/code/app", verdicts: [["ghost", merged]] });
    expect(result.current.classified).toBe(0);
    expect(result.current.branches).toHaveLength(1);
  });

  /// The event is app-global; a scan is per-repository. Without the
  /// check, switching repository mid-scan folds the old repository's
  /// verdicts into the new repository's rows — the wrong deletability
  /// answer against the right-looking branch name.
  it("drops frames for another repository", async () => {
    const { result } = show();
    await subscribed();
    emit({ kind: "listed", repo: "/code/app", total: 1, branches: [branch("a")] });
    emit({ kind: "listed", repo: "/code/other", total: 9, branches: [branch("elsewhere")] });
    emit({ kind: "classified", repo: "/code/other", verdicts: [["a", merged]] });

    expect(result.current.total).toBe(1);
    expect(result.current.branches.map((b) => b.name)).toEqual(["a"]);
    expect(result.current.branches[0].deletable.kind).toBe("pending");
  });

  /// Changing repository clears what is held. Carrying the previous
  /// repository's rows over would show one repository's branches under
  /// another's name for as long as the new scan takes.
  it("starts from nothing when the repository changes", async () => {
    const { result, rerender } = renderHook(
      ({ r }: { r: string | undefined }) => useBranchScan(r),
      { initialProps: { r: "/code/app" as string | undefined } },
    );
    await subscribed();
    emit({ kind: "listed", repo: "/code/app", total: 1, branches: [branch("a")] });
    expect(result.current.branches).toHaveLength(1);

    rerender({ r: "/code/other" });
    expect(result.current).toEqual({ branches: [], total: null, classified: 0 });
  });

  /// A fresh listing REPLACES. It is the start of a new scan, and
  /// carrying verdicts across would show answers computed against refs
  /// that have since moved — the one stale answer this page cannot
  /// afford.
  it("replaces rather than merges when a second scan lists", async () => {
    const { result } = show();
    await subscribed();
    emit({ kind: "listed", repo: "/code/app", total: 1, branches: [branch("a")] });
    emit({ kind: "classified", repo: "/code/app", verdicts: [["a", merged]] });
    expect(result.current.classified).toBe(1);

    emit({ kind: "listed", repo: "/code/app", total: 1, branches: [branch("a")] });
    expect(result.current.classified).toBe(0);
    expect(result.current.branches[0].deletable.kind).toBe("pending");
  });

  it("subscribes to nothing when no repository is selected", async () => {
    // NOT `show(undefined)`: that would hit the parameter default and
    // silently test the selected-repository case instead.
    renderHook(({ r }: { r: string | undefined }) => useBranchScan(r), {
      initialProps: { r: undefined as string | undefined },
    });
    await subscribed();
    expect(bus.listeners.get("branch-scan-progress")?.size ?? 0).toBe(0);
  });

  it("unsubscribes on unmount", async () => {
    const { unmount } = show();
    await subscribed();
    expect(bus.listeners.get("branch-scan-progress")?.size).toBe(1);
    unmount();
    expect(bus.listeners.get("branch-scan-progress")?.size ?? 0).toBe(0);
  });
});
